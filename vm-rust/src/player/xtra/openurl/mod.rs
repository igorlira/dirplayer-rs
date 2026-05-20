//! OpenURL Xtra (Gary Smith, 1997) — single global handler that opens a URL in
//! the default browser.
//!
//! Lingo signature:
//!   `gsOpenURL string URL` -> integer 1 on success, 0 on failure.
//!
//! In a browser host the "default browser" is already this page; `window.open`
//! either pops a new tab (success) or is blocked by the popup blocker
//! (failure).

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, reserve_player_ref, DatumRef, ScriptError},
};

pub struct OpenUrlXtra;

impl OpenUrlXtra {
    pub fn has_handler(name: &str) -> bool {
        name.eq_ignore_ascii_case("gsOpenURL")
    }

    pub fn call_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match_ci!(name, {
            "gsOpenURL" => gs_open_url(args),
            _ => Err(ScriptError::new(format!(
                "OpenURL: no handler {}",
                name
            ))),
        })
    }
}

fn gs_open_url(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let url = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("gsOpenURL requires a URL argument".to_string())
        })?;
        player.get_datum(arg).string_value()
    })?;

    let ok = open_url_in_browser(&url);
    reserve_player_mut(|player| {
        Ok(player.alloc_datum(Datum::Int(if ok { 1 } else { 0 })))
    })
}

fn open_url_in_browser(url: &str) -> bool {
    match web_sys::window() {
        Some(window) => window.open_with_url_and_target(url, "_blank").is_ok(),
        None => false,
    }
}
