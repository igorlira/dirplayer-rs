use log::debug;

use crate::player::keyboard_map;

/// Director key code from the browser's `event.key` NAME.
///
/// `event.keyCode` is deprecated and some environments report 0 for every
/// key. Movies read `the keyCode` for Escape (53), Delete (117), Return
/// (36) and friends, so without a fallback those keys silently do nothing
/// while letters (read through `the key`) keep working.
fn sw_code_from_key_name(key: &str) -> Option<u16> {
    let code = match key {
        "Escape" | "Esc" => 53,
        "Delete" => 117,
        "Enter" => 36,
        "Backspace" => 51,
        "Tab" => 48,
        " " | "Space" | "Spacebar" => 49,
        "ArrowLeft" => 123,
        "ArrowRight" => 124,
        "ArrowDown" => 125,
        "ArrowUp" => 126,
        "Home" => 115,
        "End" => 119,
        "PageUp" => 116,
        "PageDown" => 121,
        _ => {
            // Single characters: reuse the JS-keyCode table via the uppercase
            // ASCII code, which is what a browser would otherwise have sent.
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            let js = (c.to_ascii_uppercase() as u32) as u16;
            return keyboard_map::get_keyboard_key_map_js_to_sw().get(&js).copied();
        }
    };
    Some(code)
}

pub struct KeyboardKey {
    pub key: String,
    pub code: u16,
}

pub struct KeyboardManager {
    pub down_keys: Vec<KeyboardKey>,
    /// The key most recently pressed, kept after its release: `the key` and
    /// `the keyCode` report it inside `on keyUp`, when the key is no longer
    /// down. Cleared by nothing but the next key press.
    pub last_key: Option<KeyboardKey>,
    /// Timestamp of the most recent `key_down`; `None` if no key has been
    /// pressed since the movie started. Used by `the lastKey` to compute
    /// ticks since the last key event.
    pub last_key_time: Option<chrono::DateTime<chrono::Local>>,
}

impl KeyboardManager {
    pub fn new() -> Self {
        Self {
            down_keys: Vec::new(),
            last_key: None,
            last_key_time: None,
        }
    }

    pub fn key_down(&mut self, key: String, code: u16) {
        let code_mapped = keyboard_map::get_keyboard_key_map_js_to_sw().get(&code);
        debug!("Key down: {} {} (mapped to: {:?})", key, code, code_mapped);
        // Fall back to the key NAME when the browser gave no usable keyCode.
        let mapped_code = match code_mapped {
            Some(m) => *m,
            None => sw_code_from_key_name(&key).unwrap_or(code),
        };
        self.last_key_time = Some(chrono::Local::now());

        // Map JS key names to Director key values
        let mapped_key = match key.as_str() {
            "Enter" => "\r".to_string(),
            "Tab" => "\t".to_string(),
            "Backspace" => "\x08".to_string(),
            _ => key,
        };

        self.last_key = Some(KeyboardKey {
            key: mapped_key.clone(),
            code: mapped_code,
        });

        // Check if this code is already in the down_keys list
        if !self.down_keys.iter().any(|x| x.code == mapped_code) {
            self.down_keys.push(KeyboardKey {
                key: mapped_key,
                code: mapped_code,
            });
        }
    }

    /// The key `the key` and `the keyCode` describe: the one most recently
    /// pressed among those still down, else the last one pressed.
    fn current_key(&self) -> Option<&KeyboardKey> {
        self.down_keys.last().or(self.last_key.as_ref())
    }

    pub fn key_up(&mut self, _: &str, code: u16) {
        // Map the code the same way as key_down does
        let code_mapped = keyboard_map::get_keyboard_key_map_js_to_sw().get(&code);
        let code_to_remove = *code_mapped.unwrap_or(&code);

        self.down_keys.retain(|x| x.code != code_to_remove);
    }

    pub fn is_key_down(&self, key: &str) -> bool {
        self.down_keys.iter().any(|x| x.key == key)
    }

    pub fn is_command_down(&self) -> bool {
        self.is_key_down("Meta")
    }

    pub fn is_control_down(&self) -> bool {
        self.is_key_down("Control")
    }

    pub fn is_shift_down(&self) -> bool {
        self.is_key_down("Shift")
    }

    pub fn is_alt_down(&self) -> bool {
        // German / Nordic / many EU layouts assign the right Alt key the
        // "AltGraph" identifier instead of "Alt". Treat both as the same
        // modifier so `the optionDown`/`the altDown` Lingo accessors stay
        // truthy when the user is holding AltGr.
        self.is_key_down("Alt") || self.is_key_down("AltGraph")
    }

    /// True specifically when the AltGr key (right-Alt on German/Nordic
    /// layouts) is held. Used by text-insertion to distinguish "typing a
    /// layout-modified character like @, €, |" from "Ctrl+letter shortcut".
    pub fn is_alt_graph_down(&self) -> bool {
        self.is_key_down("AltGraph")
    }

    pub fn key_code(&self) -> u16 {
        self.current_key().map_or(0, |key| key.code)
    }

    /// Translate a stored browser key name (e.g. `e.key` = "ArrowLeft") to the
    /// single character Director's `the key` / `the keyPressed` report. Director
    /// returns its arrow-key char constants — numToChar(28..31) — so movies that
    /// test `charToNum(the keyPressed) = 28` (left), 29 (right), 30 (up),
    /// 31 (down) work. bogey_nights' dog-movement `case` falls to exactly these
    /// tests for single-arrow (non-diagonal) presses. Returns None for keys with
    /// no Director char equivalent (function keys, "Shift", etc.).
    fn director_char_for(key: &str) -> Option<char> {
        match key {
            "ArrowLeft" => Some('\u{1C}'),  // 28
            "ArrowRight" => Some('\u{1D}'), // 29
            "ArrowUp" => Some('\u{1E}'),    // 30
            "ArrowDown" => Some('\u{1F}'),  // 31
            _ => None,
        }
    }

    pub fn key(&self) -> String {
        let Some(key) = self.current_key().map(|key| &key.key) else {
            return "".to_string();
        };
        if let Some(ch) = Self::director_char_for(key) {
            return ch.to_string();
        }
        if key.len() == 1 && key.as_bytes()[0] < 0x80 {
            key.clone()
        } else {
            "".to_string()
        }
    }

    pub fn key_pressed(&self) -> String {
        if self.down_keys.is_empty() {
            return "".to_string();
        }
        let key = &self.down_keys.last().unwrap().key;
        if let Some(ch) = Self::director_char_for(key) {
            return ch.to_string();
        }
        key.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::KeyboardManager;

    #[test]
    fn the_key_survives_the_release() {
        // `on keyUp` runs after the key has left the down list, and Director
        // still reports the released key there.
        let mut kb = KeyboardManager::new();
        kb.key_down(" ".to_string(), 32);
        let code = kb.key_code();
        assert_eq!(kb.key(), " ");
        kb.key_up(" ", 32);
        assert_eq!(kb.key(), " ");
        assert_eq!(kb.key_code(), code);
        assert!(!kb.is_key_down(" "));
        assert_eq!(kb.key_pressed(), "", "keyPressed is about keys still down");
    }

    #[test]
    fn a_key_still_down_wins_over_the_last_release() {
        let mut kb = KeyboardManager::new();
        kb.key_down("a".to_string(), 65);
        kb.key_down("b".to_string(), 66);
        kb.key_up("b", 66);
        assert_eq!(kb.key(), "a");
    }
}

