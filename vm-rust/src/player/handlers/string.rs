use crate::{director::lingo::datum::Datum, player::{datum_formatting::format_concrete_datum, reserve_player_mut, DatumRef, ScriptError}};

pub struct StringHandlers {}

impl StringHandlers {
  pub fn space(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      Ok(player.alloc_datum(Datum::String(" ".to_string())))
    })
  }

  pub fn offset(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let str_to_find = player.get_datum(&args[0]).string_value()?;
      let find_in = player.get_datum(&args[1]).string_value()?;
      let result = find_in.find(&str_to_find).map(|x| x as i32).unwrap_or(-1);
      Ok(player.alloc_datum(Datum::Int(result + 1)))
    })
  }

  pub fn length(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      match obj {
        Datum::String(s) => Ok(player.alloc_datum(Datum::Int(s.len() as i32))),
        Datum::StringChunk(..) => {
          let s = obj.string_value()?;
          Ok(player.alloc_datum(Datum::Int(s.len() as i32)))
        }
        _ => Err(ScriptError::new("Cannot get length of non-string".to_string())),
      }
    })
  }

  pub fn string(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let result_obj = if obj.is_string() {
        Datum::String(obj.string_value()?.to_string())
      } else if obj.is_void() {
        Datum::String("".to_string())
      } else {
        Datum::String(format_concrete_datum(obj, player))
      };
      Ok(player.alloc_datum(result_obj))
    })
  }

  pub fn chars(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let string = player.get_datum(&args[0]).string_value()?;
      let start = player.get_datum(&args[1]).int_value()? - 1;
      let end: i32 = player.get_datum(&args[2]).int_value()?;
      let substr = string.chars().skip(start as usize).take((end - start) as usize).collect::<String>();

      Ok(player.alloc_datum(Datum::String(substr)))
    })
  }

  pub fn char_to_num(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let str_value = player.get_datum(&args[0]).string_value()?;
      let num = str_value.chars().next().map(|c| c as i32).unwrap_or(0);
      Ok(player.alloc_datum(Datum::Int(num)))
    })
  }

  pub fn num_to_char(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let num = player.get_datum(&args[0]).int_value()?;
      let char_value = std::char::from_u32(num as u32).unwrap().to_string();
      Ok(player.alloc_datum(Datum::String(char_value)))
    })
  }
}
