use crate::{director::lingo::datum::Datum, player::{DatumRef, DirPlayer, ScriptError}};

pub struct SoundDatumHandlers {}

impl SoundDatumHandlers {
  #[allow(dead_code, unused_variables)]
  pub fn call(datum: &DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      _ => Err(ScriptError::new(format!("No handler {handler_name} for sound")))
    }
  }


  pub fn get_prop(_player: &DirPlayer, _datum: &DatumRef, prop: &String) -> Result<Datum, ScriptError> {
    match prop.as_str() {
      "volume" => {
        Ok(Datum::Int(255)) // TODO
      },
      _ => {
        Err(ScriptError::new(format!("Cannot get rect property {}", prop)))
      },
    }
  }

  pub fn set_prop(_player: &mut DirPlayer, _datum: &DatumRef, prop: &String, _value_ref: &DatumRef) -> Result<(), ScriptError> {
    match prop.as_str() {
      "volume" => {
        // TODO
        Ok(())
      },
      _ => {
        Err(ScriptError::new(format!("Cannot set rect property {}", prop)))
      },
    }
  }
}
