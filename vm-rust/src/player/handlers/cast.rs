use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, ScriptError},
};

pub struct CastHandlers {}

impl CastHandlers {
    pub fn cast_lib(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let name_or_number = player.get_datum(&args[0]);
            let cast = match name_or_number {
                Datum::Int(n) => Some(player.movie.cast_manager.get_cast(*n as u32)?),
                Datum::String(s) => player.movie.cast_manager.get_cast_by_name(&s),
                _ => return Err(ScriptError::new(format!("Invalid argument for castLib"))),
            };

            match cast {
                Some(c) => Ok(player.alloc_datum(Datum::CastLib(c.number))),
                None => Err(ScriptError::new(format!("Cast not found"))),
            }
        })
    }

    pub fn find_empty(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let member_ref = player.get_datum(&args[0]).to_member_ref()?;
            let cast_lib = if member_ref.cast_lib > 0 {
                member_ref.cast_lib as u32
            } else {
                1
            };
            let cast = player.movie.cast_manager.get_cast(cast_lib)?;
            // "the position AFTER a specified cast member" — the dictionary's own
            // example calls `findEmpty(member(100))` "the first empty cast member
            // on or after cast member 100", so the given slot counts itself.
            let slot = cast.find_empty_slot(member_ref.cast_member as u32);
            Ok(player.alloc_datum(Datum::Int(slot as i32)))
        })
    }
}
