use crate::{director::lingo::datum::Datum, player::{DatumRef, DirPlayer, ScriptError}};

pub struct VoidDatumHandlers {}

impl VoidDatumHandlers {
  #[allow(dead_code, unused_variables)]
  pub fn call(datum: DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      _ => Err(ScriptError::new(format!("No handler {handler_name} for void")))
    }
  }

  pub fn get_prop(player: &mut DirPlayer, _: &DatumRef, prop: &String) -> Result<DatumRef, ScriptError> {
    match prop.as_str() {
      "ilk" => {
        Ok(player.alloc_datum(Datum::Symbol("void".to_owned())))
      }
      "length" => {
        Ok(player.alloc_datum(Datum::Int(0)))
      }
      _ => {
        Err(ScriptError::new(format!("Cannot get Void property {}", prop)))
      },
    }
  }
}
