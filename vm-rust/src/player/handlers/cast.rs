use crate::{director::lingo::datum::Datum, player::{get_datum, reserve_player_mut, DatumRef, ScriptError}};


pub struct CastHandlers { }

impl CastHandlers {
  pub fn cast_lib(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let name_or_number = get_datum(args[0], &player.datums);
      let cast = match name_or_number {
        Datum::Int(n) => {
          Some(player.movie.cast_manager.get_cast(*n as u32)?)
        },
        Datum::String(s) => {
          player.movie.cast_manager.get_cast_by_name(&s)
        },
        _ => {
          return Err(ScriptError::new(format!("Invalid argument for castLib")))
        }
      };

      match cast {
        Some(c) => {
          Ok(player.alloc_datum(Datum::CastLib(c.number)))
        },
        None => Err(ScriptError::new(format!("Cast not found")))
      }
    })
  }
}