use crate::{director::lingo::datum::Datum, player::{sprite::ColorRef, DatumRef, DirPlayer, ScriptError}};

pub struct ColorDatumHandlers {}

impl ColorDatumHandlers {
  // pub fn call(datum: DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
  //   match handler_name.as_str() {
  //     _ => Err(ScriptError::new(format!("No handler {handler_name} for color")))
  //   }
  // }

  pub fn get_prop(player: &mut DirPlayer, datum: DatumRef, prop: &String) -> Result<DatumRef, ScriptError> {
    let color_ref = player.get_datum(datum).to_color_ref()?;
    match prop.as_str() {
      "red" => {
        match color_ref {
          ColorRef::Rgb(r, _, _timeout_name) => Ok(player.alloc_datum(Datum::Int(*r as i32))),
          ColorRef::PaletteIndex(i) => match i {
            0 => Ok(player.alloc_datum(Datum::Int(255))),
            255 => Ok(player.alloc_datum(Datum::Int(0))),
            _ => Ok(player.alloc_datum(Datum::Int(255))),
          },
        }
      },
      "green" => {
        match color_ref {
          ColorRef::Rgb(_, g, _timeout_name) => Ok(player.alloc_datum(Datum::Int(*g as i32))),
          ColorRef::PaletteIndex(i) => match i {
            0 => Ok(player.alloc_datum(Datum::Int(255))),
            255 => Ok(player.alloc_datum(Datum::Int(0))),
            _ => Ok(player.alloc_datum(Datum::Int(0))),
          },
        }
      },
      "blue" => {
        match color_ref {
          ColorRef::Rgb(_, _, b) => Ok(player.alloc_datum(Datum::Int(*b as i32))),
          ColorRef::PaletteIndex(i) => match i {
            0 => Ok(player.alloc_datum(Datum::Int(255))),
            255 => Ok(player.alloc_datum(Datum::Int(0))),
            _ => Ok(player.alloc_datum(Datum::Int(255))),
          },
        }
      },
      "ilk" => {
        Ok(player.alloc_datum(Datum::Symbol("color".to_owned())))
      },
      _ => {
        Err(ScriptError::new(format!("Cannot get color property {}", prop)))
      },
    }
  }

  // pub fn set_prop(player: &mut DirPlayer, datum: DatumRef, prop: &String, value: DatumRef) -> Result<(), ScriptError> {
  //   match prop.as_str() {
  //     _ => {
  //       Err(ScriptError::new(format!("Cannot set color property {}", prop)))
  //     },
  //   }
  // }
}
