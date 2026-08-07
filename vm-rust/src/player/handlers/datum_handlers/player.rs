use crate::{
    director::lingo::datum::Datum,
    player::{
        reserve_player_mut, reserve_player_ref, DatumRef,
        ScriptError, ScriptErrorCode,
        symbols::{builtin::BuiltInSymbol, symbol::Symbol},
    },
};
use super::super::types::TypeHandlers;

pub struct PlayerDatumHandlers {}

impl PlayerDatumHandlers {
    pub fn call(handler_name: Symbol, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match handler_name.into_builtin() {
            Some(BuiltInSymbol::Count) => Self::count(args),
            Some(BuiltInSymbol::Cursor) => TypeHandlers::cursor(args),
            // `_key.keyPressed()` — no-arg form returns the currently-pressed
            // key character (Director 11.5: `_key.keyPressed() = SPACE`); the
            // single-arg form `_key.keyPressed(charOrCode)` tests a specific
            // key and is shared with the top-level `keyPressed()` builtin.
            // parent_dialog's updateDialog (the key-rebind UI) calls the
            // no-arg form: `nn = _key.keyPressed()`.
            Some(BuiltInSymbol::KeyPressed) => {
                if args.is_empty() {
                    reserve_player_mut(|player| {
                        let k = player.keyboard_manager.key_pressed();
                        Ok(player.alloc_datum(Datum::String(k)))
                    })
                } else {
                    crate::player::handlers::manager::BuiltInHandlerManager::key_pressed(args)
                }
            }
            // `_player.getPref(name)` / `_player.setPref(name, value)` — the Player
            // object's form of the top-level getPref()/setPref() (Director 11.5
            // Scripting Dictionary: "Function; retrieves the content of the
            // specified file", VOID when the file doesn't exist, and only .txt /
            // .htm are valid extensions). Same storage either way, so delegate to
            // the existing implementation rather than duplicating it — AreaZero's
            // `[M] Generic Handlers.LoadData` reads its save file with
            // `_player.getPref(gGame.PrefFile)`.
            Some(BuiltInSymbol::GetPref) => {
                crate::player::handlers::movie::MovieHandlers::get_pref(args)
            }
            Some(BuiltInSymbol::SetPref) => {
                crate::player::handlers::movie::MovieHandlers::set_pref(args)
            }
            // `_player.windowList[1]` — Director 11.5 Scripting Dictionary,
            // `windowList` (Player property, read-only): "displays a list of
            // references to all known movie windows … The Stage is also
            // considered a window." We open no auxiliary MIAWs, so the list is
            // exactly [the Stage], and index 1 is the Stage.
            //
            // AreaZero's `[M] Main.InitGlobals` does
            // `gSystem[#parent] = _player.windowList[1].movie`, which compiles to
            // getPropRef(_player, "windowList", 1) and previously raised.
            Some(BuiltInSymbol::GetProp) | Some(BuiltInSymbol::GetAt) | Some(BuiltInSymbol::GetPropRef) => reserve_player_mut(|player| {
                let subject = player.get_datum(&args[0]).string_value()?;
                if subject.eq_ignore_ascii_case("windowList") {
                    let index = args.get(1)
                        .map(|a| player.get_datum(a).int_value())
                        .transpose()?
                        .unwrap_or(1);
                    return Ok(if index == 1 {
                        player.alloc_datum(Datum::Stage)
                    } else {
                        DatumRef::Void
                    });
                }
                Err(ScriptError::new(format!(
                    "Invalid call _player.{handler_name}({subject})"
                )))
            }),
            _ => reserve_player_ref(|player| {
                Err(ScriptError::new_code(
                    ScriptErrorCode::HandlerNotFound,
                    format!("No handler {handler_name} for player datum"),
                ))
            }),
        }
    }

    fn count(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let subject = player.get_datum(&args[0]).string_value().unwrap();
            match subject.as_str() {
                // The Stage counts as a window (see `windowList` above), so the
                // list is never empty even with no MIAWs open.
                "windowList" => Ok(player.alloc_datum(Datum::Int(1))),
                _ => Err(ScriptError::new(
                    format!("Invalid call _player.count({subject})").to_string(),
                )),
            }
        })
    }
}
