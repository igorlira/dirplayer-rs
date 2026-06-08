use crate::{
    director::lingo::datum::Datum,
    player::{
        reserve_player_mut, reserve_player_ref, DatumRef,
        ScriptError, ScriptErrorCode,
    },
};
use super::super::types::TypeHandlers;

pub struct PlayerDatumHandlers {}

impl PlayerDatumHandlers {
    pub fn call(handler_name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "count" => Self::count(args),
            "cursor" => TypeHandlers::cursor(args),
            // `_key.keyPressed()` — no-arg form returns the currently-pressed
            // key character (Director 11.5: `_key.keyPressed() = SPACE`); the
            // single-arg form `_key.keyPressed(charOrCode)` tests a specific
            // key and is shared with the top-level `keyPressed()` builtin.
            // parent_dialog's updateDialog (the key-rebind UI) calls the
            // no-arg form: `nn = _key.keyPressed()`.
            "keyPressed" | "keypressed" => {
                if args.is_empty() {
                    reserve_player_mut(|player| {
                        let k = player.keyboard_manager.key_pressed();
                        Ok(player.alloc_datum(Datum::String(k)))
                    })
                } else {
                    crate::player::handlers::manager::BuiltInHandlerManager::key_pressed(args)
                }
            }
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
                "windowList" => Ok(player.alloc_datum(Datum::Int(0))),
                _ => Err(ScriptError::new(
                    format!("Invalid call _player.count({subject})").to_string(),
                )),
            }
        })
    }
}
