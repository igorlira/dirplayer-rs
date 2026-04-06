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

            let (c_start, c_end) = match &player.movie.file {
                Some(file) => (file.config.min_member as u32, file.config.max_member as u32),
                None => return Err(ScriptError::new("findEmpty: no movie file loaded".to_string())),
            };

            let cast_lib = if member_ref.cast_lib > 0 {
                member_ref.cast_lib as u32
            } else {
                1
            };
            let cast = player.movie.cast_manager.get_cast(cast_lib)?;

            let member_num = member_ref.cast_member as u32;
            if member_num > c_end {
                return Ok(player.alloc_datum(Datum::Int(member_num as i32)));
            }

            let start = if member_num > c_start { member_num } else { c_start };
            for slot in start..=c_end {
                if !cast.members.contains_key(&slot) {
                    return Ok(player.alloc_datum(Datum::Int(slot as i32)));
                }
            }
            Ok(player.alloc_datum(Datum::Int(c_end as i32 + 1)))
        })
    }
}
