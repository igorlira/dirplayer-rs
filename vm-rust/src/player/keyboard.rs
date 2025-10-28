use log::debug;

use crate::player::keyboard_map;

pub struct KeyboardKey {
    pub key: String,
    pub code: u16,
}

pub struct KeyboardManager {
    pub down_keys: Vec<KeyboardKey>,
}

impl KeyboardManager {
    pub fn new() -> Self {
        Self {
            down_keys: Vec::new(),
        }
    }

    pub fn key_down(&mut self, key: String, code: u16) {
        let code_mapped = keyboard_map::get_keyboard_key_map_js_to_sw().get(&code);
        debug!("Key down: {} {} (mapped to: {:?})", key, code, code_mapped);
        let mapped_code = *code_mapped.unwrap_or(&code);

        // Check if this code is already in the down_keys list
        if !self.down_keys.iter().any(|x| x.code == mapped_code) {
            self.down_keys.push(KeyboardKey {
                key: key,
                code: mapped_code,
            });
        }
    }

    pub fn key_up(&mut self, _: &String, code: u16) {
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
        self.is_key_down("Alt")
    }

    pub fn key_code(&self) -> u16 {
        if self.down_keys.len() == 0 {
            return 0;
        }

        let key = self.down_keys.last().unwrap();
        key.code
    }

    pub fn key(&self) -> String {
        if self.down_keys.len() == 0 {
            return "".to_string();
        }
        self.down_keys.last().unwrap().key.clone()
    }
}
