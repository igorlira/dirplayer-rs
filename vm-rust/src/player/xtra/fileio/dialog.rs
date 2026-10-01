//! The file dialogs behind FileIO's displayOpen and displaySave.
//!
//! A projector puts up the operating system's file dialog here and the
//! movie waits for the answer. In the browser the page shows its own dialog
//! (dirplayer-js-api, file-dialog.js) over the stage, and the handler awaits
//! it the same way. The dialog lists the files this browser holds for the
//! movie (the persisted virtual filesystem), filtered by the movie's filter
//! mask, and can bring in a file from the player's own computer, which is
//! stored in the virtual filesystem under its own name and handed back as
//! the chosen path. Bytes travel as bytes: files written by a Director
//! projector are in the Windows code page, not UTF-8.

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "dirplayer-js-api")]
extern "C" {
    /// Shows the dialog and resolves with `null` (cancel) or
    /// `{ name, bytes? }`: the chosen file name, plus its contents when the
    /// file came from outside the virtual filesystem.
    #[wasm_bindgen(js_name = showFileDialog)]
    fn js_show_file_dialog(request: JsValue) -> js_sys::Promise;

    /// A file named in a displaySave answer has been written and closed.
    /// The page may offer it as a download or write it to a file the player
    /// picked. Must return at once: the movie is running.
    #[wasm_bindgen(js_name = onFileDialogSaveWritten)]
    fn js_on_save_written(name: &str, data: &[u8]);
}

/// One file the dialog can offer.
pub struct DialogFile {
    pub name: String,
    /// Milliseconds since the epoch of the last write, 0 when unknown.
    pub modified: f64,
    pub size: usize,
}

pub enum DialogAnswer {
    Cancel,
    /// A file name, and its bytes when it came from the player's computer.
    File { name: String, bytes: Option<Vec<u8>> },
}

/// The glob patterns in a FileIO filter mask, lowercased.
///
/// On Windows the mask is `description,pattern` pairs separated by commas,
/// and one pattern entry may hold several patterns separated by `;`
/// ("Text files,*.txt;*.log,All files,*.*"). A lone entry that looks like a
/// pattern is taken as one. A Mac mask is a four letter file type ("TEXT"),
/// which says nothing about names: no patterns, every file matches.
pub fn mask_patterns(mask: &str) -> Vec<String> {
    let parts: Vec<&str> = mask.split(',').map(|p| p.trim()).collect();
    let candidates: Vec<&str> = if parts.len() >= 2 {
        parts.iter().skip(1).step_by(2).copied().collect()
    } else {
        parts
    };
    let mut patterns: Vec<String> = Vec::new();
    for entry in candidates {
        for p in entry.split(';') {
            let p = p.trim().to_lowercase();
            if !(p.contains('*') || p.contains('?') || p.contains('.')) {
                continue;
            }
            if !patterns.contains(&p) {
                patterns.push(p);
            }
        }
    }
    // "*.*" and "*" match everything, which is the same as no filter.
    if patterns.iter().any(|p| p == "*.*" || p == "*") {
        return Vec::new();
    }
    patterns
}

/// The extensions the patterns name, like "sav" for "*.sav", for the
/// browser's own file picker.
pub fn mask_extensions(patterns: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in patterns {
        if let Some(ext) = p.strip_prefix("*.") {
            if !ext.is_empty() && !ext.contains(['*', '?', '.']) && !out.iter().any(|e| e == ext) {
                out.push(ext.to_string());
            }
        }
    }
    out
}

fn glob_match(pattern: &[char], name: &[char]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some('*') => (0..=name.len()).any(|i| glob_match(&pattern[1..], &name[i..])),
        Some('?') => !name.is_empty() && glob_match(&pattern[1..], &name[1..]),
        Some(c) => name.first() == Some(c) && glob_match(&pattern[1..], &name[1..]),
    }
}

/// Whether a file name passes the mask's patterns (case-insensitive, as on
/// Windows). No patterns: everything passes.
pub fn name_matches(name: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let name: Vec<char> = name.to_lowercase().chars().collect();
    patterns.iter().any(|p| {
        let p: Vec<char> = p.chars().collect();
        glob_match(&p, &name)
    })
}

fn set(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

/// Put up the open dialog and wait for the player.
pub async fn show_open_dialog(mask: &str, files: &[DialogFile]) -> DialogAnswer {
    show_dialog("open", "", "", mask, files).await
}

/// Put up the save dialog and wait for the player.
pub async fn show_save_dialog(title: &str, default_name: &str, mask: &str) -> DialogAnswer {
    show_dialog("save", title, default_name, mask, &[]).await
}

async fn show_dialog(
    kind: &str,
    title: &str,
    default_name: &str,
    mask: &str,
    files: &[DialogFile],
) -> DialogAnswer {
    let patterns = mask_patterns(mask);
    let request = Object::new();
    set(&request, "kind", &JsValue::from_str(kind));
    set(&request, "title", &JsValue::from_str(title));
    set(&request, "defaultName", &JsValue::from_str(default_name));
    set(&request, "mask", &JsValue::from_str(mask));
    let extensions = Array::new();
    for ext in mask_extensions(&patterns) {
        extensions.push(&JsValue::from_str(&ext));
    }
    set(&request, "extensions", &extensions);
    let list = Array::new();
    for f in files {
        let o = Object::new();
        set(&o, "name", &JsValue::from_str(&f.name));
        set(&o, "modified", &JsValue::from_f64(f.modified));
        set(&o, "size", &JsValue::from_f64(f.size as f64));
        list.push(&o);
    }
    set(&request, "files", &list);

    let answer = match JsFuture::from(js_show_file_dialog(request.into())).await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("FileIO dialog failed: {:?}", e);
            return DialogAnswer::Cancel;
        }
    };
    if answer.is_null() || answer.is_undefined() {
        return DialogAnswer::Cancel;
    }
    let name = Reflect::get(&answer, &JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    if name.is_empty() {
        return DialogAnswer::Cancel;
    }
    let bytes = Reflect::get(&answer, &JsValue::from_str("bytes"))
        .ok()
        .filter(|v| v.is_instance_of::<Uint8Array>())
        .map(|v| Uint8Array::from(v).to_vec());
    DialogAnswer::File { name, bytes }
}

/// Tell the page a saved file is complete.
pub fn notify_save_written(name: &str, data: &[u8]) {
    js_on_save_written(name, data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_mask_pairs() {
        let p = mask_patterns("*.SAV,*.SAV");
        assert_eq!(p, vec!["*.sav".to_string()]);
        assert!(name_matches("Game1.SAV", &p));
        assert!(name_matches("game1.sav", &p));
        assert!(!name_matches("settings.txt", &p));
        assert_eq!(mask_extensions(&p), vec!["sav".to_string()]);
    }

    #[test]
    fn several_patterns_and_all_files() {
        let p = mask_patterns("Text,*.txt;*.log,Data,*.dat");
        assert_eq!(p, vec!["*.txt", "*.log", "*.dat"]);
        assert!(name_matches("a.LOG", &p));
        assert!(mask_patterns("All,*.*").is_empty());
    }

    #[test]
    fn mac_type_and_empty_mask_match_everything() {
        assert!(mask_patterns("TEXT").is_empty());
        assert!(mask_patterns("").is_empty());
        assert!(name_matches("anything.bin", &mask_patterns("TEXT")));
    }

    #[test]
    fn question_mark_glob() {
        let p = mask_patterns("*.d?r");
        assert!(name_matches("movie.dir", &p));
        assert!(name_matches("movie.dxr", &p));
        assert!(!name_matches("movie.dcrx", &p));
    }
}
