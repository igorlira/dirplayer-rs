use log::debug;

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
                Datum::String(s) => Ok(player.alloc_datum(Datum::Int(s.chars().count() as i32))),
                Datum::StringChunk(..) => {
                    let s = obj.string_value()?;
                    Ok(player.alloc_datum(Datum::Int(s.chars().count() as i32)))
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
            } else if let Datum::Symbol(s) = obj {
                // In Director, string(#symbol) returns "symbol" without the # prefix
                Datum::String(s.clone())
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
            let start = (player
                .get_datum(&args[1])
                .int_value()
                .unwrap_or(1)
                .max(1) - 1) as usize;
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
            let mut chars = str_value.chars();

            let byte_val = if let Some(c) = chars.next() {
                c as i32
            } else {
                0
            };

            Ok(player.alloc_datum(Datum::Int(byte_val)))
        })
    }

    pub fn num_to_char(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let num = player.get_datum(&args[0]).int_value()?;
            let byte_val = (num & 0xFF) as u8 as char;

            // Build a single-byte string directly from raw bytes (Latin-1 1:1)
            let result_string = byte_val.to_string();

            Ok(player.alloc_datum(Datum::String(result_string)))
        })
    }

    pub fn url_encode(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // urlEncode([#empty: sSessionID]) - takes a prop list and converts to URL parameters
            let prop_list_datum = player.get_datum(&args[0]);

            let result = match prop_list_datum {
                Datum::PropList(prop_list, ..) => {
                    // Convert prop list to URL parameters: [#empty: "value"] -> "empty=encoded_value"
                    let mut url_params = String::new();
                    for (key_ref, value_ref) in prop_list {
                        let key = player.get_datum(key_ref);
                        let value = player.get_datum(value_ref);

                        let key_str = match key {
                            Datum::Symbol(s) => s.clone(),
                            Datum::String(s) => s.clone(),
                            _ => continue,
                        };

                        let value_str = match value {
                            Datum::String(s) => s.clone(),
                            Datum::Int(n) => n.to_string(),
                            Datum::Float(f) => f.to_string(),
                            Datum::Symbol(s) => s.clone(),
                            Datum::Void => String::new(),
                            _ => continue,
                        };

                        if !url_params.is_empty() {
                            url_params.push('&');
                        }

                        // URL encode the value using the same character mapping as ActionScript
                        let mut encoded_value = String::new();
                        for ch in value_str.chars() {
                            let encoded_char = match ch {
                                ':' => "%3A", ';' => "%3B", '<' => "%3C", '=' => "%3D", '>' => "%3E", '?' => "%3F",
                                '@' => "%40", '[' => "%5B", ']' => "%5D", '{' => "%7B", '}' => "%7D", '~' => "%7E",
                                ' ' => "%20", '!' => "%21", '"' => "%22", '#' => "%23", '$' => "%24", '%' => "%25",
                                '&' => "%26", '\'' => "%27", '(' => "%28", ')' => "%29", '*' => "%2A", '+' => "%2B",
                                ',' => "%2C", '-' => "%2D", '.' => "%2E", '/' => "%2F", '©' => "%26%23169", '®' => "%26%23174",
                                _ => {
                                    encoded_value.push(ch);
                                    continue;
                                }
                            };
                            encoded_value.push_str(encoded_char);
                        }

                        url_params.push_str(&format!("{}={}", key_str, encoded_value));
                    }
                    url_params
                },
                Datum::String(s) => {
                    // Direct string encoding (fallback)
                    let mut encoded = String::new();
                    for ch in s.chars() {
                        let encoded_char = match ch {
                            ':' => "%3A", ';' => "%3B", '<' => "%3C", '=' => "%3D", '>' => "%3E", '?' => "%3F",
                            '@' => "%40", '[' => "%5B", ']' => "%5D", '{' => "%7B", '}' => "%7D", '~' => "%7E",
                            ' ' => "%20", '!' => "%21", '"' => "%22", '#' => "%23", '%' => "%25",
                            '&' => "%26", '\'' => "%27", '(' => "%28", ')' => "%29", '*' => "%2A", '+' => "%2B",
                            ',' => "%2C", '©' => "%26%23169", '®' => "%26%23174",
                            _ => {
                                encoded.push(ch);
                                continue;
                            }
                        };
                        encoded.push_str(encoded_char);
                    }
                    encoded
                },
                _ => return Err(ScriptError::new("urlEncode: argument must be a prop list or string".to_string()))
            };

            debug!("urlEncode() = '{}'", result);
            Ok(player.alloc_datum(Datum::String(result)))
        })
    }
}
