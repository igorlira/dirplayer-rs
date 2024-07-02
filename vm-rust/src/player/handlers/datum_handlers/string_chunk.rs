use itertools::Itertools;

use crate::{director::lingo::datum::{Datum, StringChunkExpr, StringChunkSource, StringChunkType}, player::{cast_member::CastMemberType, reserve_player_mut, DatumRef, DirPlayer, ScriptError, VOID_DATUM_REF}};

use super::string::string_get_lines;

pub struct StringChunkHandlers { }
pub struct StringChunkUtils { }

impl StringChunkUtils {
  pub fn delete(player: &mut DirPlayer, original_str_src: &StringChunkSource, chunk_expr: &StringChunkExpr) -> Result<(), ScriptError> {
    let new_string = {
      let original_str = match original_str_src {
        StringChunkSource::Datum(original_str_ref) => player.get_datum(original_str_ref).string_value()?,
        StringChunkSource::Member(member_ref) => player.movie.cast_manager.find_member_by_ref(&member_ref).unwrap().member_type.as_field().unwrap().text.clone()
      };
      Self::string_by_deleting_chunk(&original_str, &chunk_expr)
    }?;
    match original_str_src {
      StringChunkSource::Datum(original_str_ref) => {
        let original_str_value = player.get_datum_mut(original_str_ref).to_string_mut()?;
        *original_str_value = new_string;
      },
      StringChunkSource::Member(member_ref) => {
        let member = &mut player.movie.cast_manager.find_mut_member_by_ref(&member_ref).unwrap().member_type;
        match member {
          CastMemberType::Field(field) => field.text = new_string,
          CastMemberType::Text(member) => member.text = new_string,
          _ => return Err(ScriptError::new("Cannot delete chunk from non-text member".to_string()))
        }
      }
    }
    Ok(())
  }

  pub fn string_by_deleting_chunk(string: &String, chunk_expr: &StringChunkExpr) -> Result<String, ScriptError> {
    match chunk_expr.chunk_type {
      StringChunkType::Char => {
        let mut new_string = string.clone();
        let (start, end) = Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.len());
        new_string.replace_range(start..end, "");
        Ok(new_string)
      },
      _ => {
        Err(ScriptError::new("Only char chunk type is supported for string by deleting chunk".to_string()))
      }
    }
  }

  fn vm_range_to_host(range: (i32, i32), max_length: usize) -> (usize, usize) {
    let (start, end) = range;
    let start_index = std::cmp::max(0, start - 1) as usize;
    let end_index = if end == 0 {
      (start_index + 1) as usize
    } else if end == -1 || end > max_length as i32 {
      max_length as usize
    } else {
      end as usize
    };
    let start_index = std::cmp::min(std::cmp::max(start_index, 0), max_length);
    let end_index = std::cmp::max(start_index, std::cmp::min(end_index, max_length));
    (start_index, end_index)
  }

  #[allow(dead_code)]
  fn host_range_to_vm(range: (i32, i32)) -> (i32, i32) {
    let (start, end) = range;
    (start + 1, end)
  }

  pub fn resolve_chunk_list(string: &String, chunk_type: StringChunkType, item_delimiter: &String) -> Result<Vec<String>, ScriptError> {
    match chunk_type {
      StringChunkType::Item => {
        let items = string.split(item_delimiter).map(|x| x.to_string());
        Ok(items.collect_vec())
      },
      StringChunkType::Word => {
        let words = string.split_whitespace().map(|x| x.to_string());
        Ok(words.collect_vec())
      },
      StringChunkType::Char => {
        let chars = string.chars().map(|c| c.to_string());
        Ok(chars.collect_vec())
      },
      StringChunkType::Line => {
        let lines = string_get_lines(string);
        Ok(lines)
      },
    }
  }

  pub fn resolve_last_chunk(string: &String, chunk_type: StringChunkType, item_delimiter: &String) -> Result<String, ScriptError> {
    match chunk_type {
      StringChunkType::Item => {
        let items = string.split(item_delimiter).map(|x| x.to_string());
        Ok(items.last().unwrap_or("".to_string()).to_string())
      },
      StringChunkType::Word => {
        let words = string.split_whitespace().map(|x| x.to_string());
        Ok(words.last().unwrap_or("".to_string()).to_string())
      },
      StringChunkType::Char => {
        Ok(string.chars().last().map(|x| x.to_string()).unwrap_or("".to_string()))
      },
      StringChunkType::Line => {
        let lines = string_get_lines(string);
        Ok(lines.last().unwrap_or(&"".to_string()).to_string())
      },
    }
  }

  pub fn resolve_chunk_count(string: &String, chunk_type: StringChunkType, item_delimiter: &String) -> Result<usize, ScriptError> {
    match chunk_type {
      StringChunkType::Item => {
        Ok(string.chars().filter(|c| item_delimiter.chars().next().unwrap() == *c).count() + 1)
      },
      StringChunkType::Word => {
        Ok(string.split_whitespace().count())
      },
      StringChunkType::Char => {
        Ok(string.len())
      },
      StringChunkType::Line => {
        Ok(string_get_lines(string).len())
      },
    }
  }

  pub fn resolve_chunk_expr_string(string: &String, chunk_expr: &StringChunkExpr) -> Result<String, ScriptError> {    
    // let type_str: String = chunk_expr.chunk_type.to_owned().into();

    //console_warn!("-============ resolve_chunk_expr_string =============-");
    //console_warn!("input string: {}", string);
    //console_warn!("type: {}", type_str);
    //console_warn!("vm range ({}, {})", chunk_expr.start, chunk_expr.end);
    //console_warn!("host range ({}, {})", start, end);
    //console_warn!("delimiter: {} (len {})", chunk_expr.item_delimiter, chunk_expr.item_delimiter.len());
    //console_warn!("chunk list: {:?}", chunk_list);

    if string.len() == 0 {
      return Ok("".to_string());
    }

    let result = match chunk_expr.chunk_type {
      StringChunkType::Item => {
        let chunk_list = Self::resolve_chunk_list(string, chunk_expr.chunk_type.clone(), &chunk_expr.item_delimiter)?;
        let (start, end) = Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

        if chunk_list.len() == 0 {
          return Ok("".to_string());
        }
        chunk_list[start..end].join(&chunk_expr.item_delimiter)
      },
      StringChunkType::Word => {
        let chunk_list = Self::resolve_chunk_list(string, chunk_expr.chunk_type.clone(), &chunk_expr.item_delimiter)?;
        let (start, end) = Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

        if chunk_list.len() == 0 {
          return Ok("".to_string());
        }
        chunk_list[start..end].join(" ")
      },
      StringChunkType::Char => {
        let (start, end) = Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), string.len());
        let bytes = string.bytes().skip(start).take(end - start);
        unsafe { String::from_utf8_unchecked(bytes.collect_vec()) }
      },
      StringChunkType::Line => {
        let chunk_list = Self::resolve_chunk_list(string, chunk_expr.chunk_type.clone(), &chunk_expr.item_delimiter)?;
        let (start, end) = Self::vm_range_to_host((chunk_expr.start, chunk_expr.end), chunk_list.len());

        if chunk_list.len() == 0 {
          return Ok("".to_string());
        }
        chunk_list[start..end].join("\r\n")
      },
    };

    //console_warn!("result: {}", result);
    //console_warn!("-============  =============-");

    Ok(result)
  }
}

impl StringChunkHandlers {
  pub fn count(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let value = player.get_datum(datum).string_value()?;
      let operand = player.get_datum(&args[0]).string_value()?;
      let delimiter = &player.movie.item_delimiter;
      let count = StringChunkUtils::resolve_chunk_count(&value, StringChunkType::from(&operand), delimiter)?;
      Ok(player.alloc_datum(Datum::Int(count as i32)))
    })
  }

  pub fn get_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let datum = player.get_datum(datum);
      let prop_name = player.get_datum(&args[0]).string_value()?;
      let start = player.get_datum(&args[1]).int_value()?;
      let end = if args.len() > 2 { player.get_datum(&args[2]).int_value()? } else { start };
      let chunk_expr = StringChunkExpr {
        chunk_type: StringChunkType::from(&prop_name),
        start,
        end,
        item_delimiter: player.movie.item_delimiter.clone(),
      };

      let str_value = StringChunkUtils::resolve_chunk_expr_string(&datum.string_value()?, &chunk_expr)?;
      Ok(player.alloc_datum(Datum::String(str_value)))
    })
  }

  pub fn set_prop(_: &mut DirPlayer, _: &DatumRef, prop: &String, _value_ref: &DatumRef) -> Result<(), ScriptError> {
    match prop.as_str() {
      "font" | "fontStyle" => {
        // TODO
      },
      _ => {
        return Err(ScriptError::new(format!("Cannot set property {prop} for string chunk datum")))
      }
    }
    Ok(())
  }

  fn delete(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let (original_str_ref, chunk_expr, ..) = player.get_datum(datum).to_string_chunk()?;
      StringChunkUtils::delete(player, &original_str_ref.clone(), &chunk_expr.clone())?;
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn call(datum: &DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      "count" => Self::count(datum, args),
      "getProp" => Self::get_prop(datum, args),
      "delete" => Self::delete(datum, args),
      _ => Err(ScriptError::new(format!("No handler {handler_name} for string chunk datum")))
    }
  }
}
