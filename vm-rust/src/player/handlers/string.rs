use crate::{
    director::lingo::datum::Datum,
    player::{datum_formatting::format_concrete_datum, reserve_player_mut, DatumRef, ScriptError},
};

pub struct StringHandlers {}

impl StringHandlers {
    pub fn space(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(" ".to_string()))))
    }

    pub fn offset(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let str_to_find = player.get_datum(&args[0]).string_value()?;
            let find_in = player.get_datum(&args[1]).string_value()?;

            // Lingo edge cases
            if str_to_find.is_empty() {
                return Ok(player.alloc_datum(Datum::Int(1)));
            }

            if find_in.is_empty() {
                return Ok(player.alloc_datum(Datum::Int(0)));
            }

            // Case-insensitive search (like Mac Lingo)
            let find_in_lower = find_in.to_lowercase();
            let str_to_find_lower = str_to_find.to_lowercase();

            let result = find_in_lower
                .find(&str_to_find_lower)
                .map(|byte_index| {
                    // Count characters up to the found byte index
                    let char_index = find_in[..byte_index].chars().count() as i32;
                    char_index + 1 // 1-based indexing
                })
                .unwrap_or(0); // Not found → return 0

            Ok(player.alloc_datum(Datum::Int(result)))
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
                _ => Err(ScriptError::new(
                    "Cannot get length of non-string".to_string(),
                )),
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
            let string = player
                .get_datum(&args[0])
                .string_value()
                .unwrap_or_default();
            let start = player
                .get_datum(&args[1])
                .int_value()
                .unwrap_or(1)
                .saturating_sub(1) as usize;
            let mut end = player.get_datum(&args[2]).int_value().unwrap_or(0) as usize;

            let len = string.chars().count();
            end = end.min(len); // clamp to string length

            if start >= len || end < start + 1 {
                return Ok(player.alloc_datum(Datum::String("".to_string())));
            }

            let substr: String = string.chars().skip(start).take(end - start).collect();

            Ok(player.alloc_datum(Datum::String(substr)))
        })
    }

    pub fn char_to_num(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let str_value = player.get_datum(&args[0]).string_value()?;
            let utf8_bytes = str_value.as_bytes();

            let byte_val = if utf8_bytes.is_empty() {
                0
            } else {
                utf8_bytes[0] as i32
            };

            Ok(player.alloc_datum(Datum::Int(byte_val)))
        })
    }

    pub fn num_to_char(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let num = player.get_datum(&args[0]).int_value()?;
            let byte_val = (num & 0xFF) as u8;

            // Build a single-byte string directly from raw bytes (Latin-1 1:1)
            let result_string = unsafe { String::from_utf8_unchecked(vec![byte_val]) };

            Ok(player.alloc_datum(Datum::String(result_string)))
        })
    }
}
