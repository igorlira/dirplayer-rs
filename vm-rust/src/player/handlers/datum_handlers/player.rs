use crate::{
    director::lingo::datum::Datum,
    player::{
        datum_formatting::format_datum, reserve_player_mut, reserve_player_ref, DatumRef,
        ScriptError, ScriptErrorCode,
    },
};

pub struct PlayerDatumHandlers {}

impl PlayerDatumHandlers {
    pub fn call(handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "count" => Self::count(args),
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
