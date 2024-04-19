use crate::{director::lingo::datum::Datum, player::{DatumRef, DirPlayer, ScriptError}};

pub struct IntDatumHandlers {}

impl IntDatumHandlers {
  pub fn get_prop(player: &mut DirPlayer, _: DatumRef, prop: &String) -> Result<DatumRef, ScriptError> {
    match prop.as_str() {
      "ilk" => {
        Ok(player.alloc_datum(Datum::Symbol("integer".to_string())))
      },
      _ => {
        Err(ScriptError::new(format!("Cannot get int property {}", prop)))
      },
    }
  }
}
