use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{
        Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType,
    },
    player::{eval::try_eval_lingo_expr_static, reserve_player_ref, reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

use super::string_chunk::StringChunkUtils;

pub struct StringDatumHandlers {}
pub struct StringDatumUtils {}

impl StringDatumUtils {
    pub fn get_prop_ref(
        player: &DirPlayer,
        datum_ref: &DatumRef,
        prop_name: &String,
        start: i32,
        end: i32,
    ) -> Result<Datum, ScriptError> {
        let datum = player.get_datum(datum_ref);
        if let Datum::String(str_val) = datum {
            match prop_name.to_lowercase().as_str() {
                "item" | "word" | "char" | "line" => {
                    let chunk_expr = StringChunkExpr {
                        chunk_type: StringChunkType::from(prop_name),
                        start,
                        end,
                        item_delimiter: player.movie.item_delimiter.to_owned(),
                    };
                    let resolved_str =
                        StringChunkUtils::resolve_chunk_expr_string(str_val, &chunk_expr)?;
                    Ok(Datum::StringChunk(
                        StringChunkSource::Datum(datum_ref.clone()),
                        chunk_expr,
                        resolved_str,
                    ))
                }
                _ => Err(ScriptError::new(format!(
                    "getPropRef: invalid prop_name {prop_name} for string"
                ))),
            }
        } else {
            return Err(ScriptError::new(format!(
                "getPropRef: datum is not a string"
            )));
        }
    }

    pub fn get_built_in_prop(value: &String, prop_name: &String) -> Result<Datum, ScriptError> {
        match prop_name.as_str() {
            "length" => Ok(Datum::Int(value.chars().count() as i32)),
            "ilk" => Ok(Datum::Symbol("string".to_owned())),
            "string" => Ok(Datum::String(value.clone())),
            "value" => {
                // Strip Lingo comments and fix unbalanced brackets before parsing
                let cleaned = strip_lingo_comments(value);
                let cleaned = trim_unbalanced_brackets(cleaned.trim());
                // Evaluate the string as a Lingo expression (prop lists, lists, numbers, etc.)
                match try_eval_lingo_expr_static(cleaned) {
                    Ok(datum_ref) => {
                        reserve_player_ref(|player| Ok(player.get_datum(&datum_ref).clone()))
                    }
                    Err(_) => Ok(Datum::String(value.clone())),
                }
            }
            "charToNum" => {
                let code = value.chars().next().map_or(0, |c| c as i32);
                Ok(Datum::Int(code))
            }
            "charSpacing" => Ok(Datum::Int(0)),
            "char" => {
                // String chunk type accessed as property — return the string itself
                // so subsequent indexing (e.g., .char[1]) can work via character indexing
                Ok(Datum::String(value.clone()))
            }
            "marker" => {
                // Quirky director behavior:
                // Any string can run .marker on it to get the frame number of the marker, if it does not match a marker name, it returns 0
                reserve_player_ref(|player| {
                    let marker_name_lower = value.to_lowercase();
                    let frame_num = player
                        .movie
                        .score
                        .frame_labels
                        .iter()
                        .find(|label| label.label.to_lowercase() == marker_name_lower)
                        .map_or(0, |label| label.frame_num as i32);
                    Ok(Datum::Int(frame_num))
                })
            }
            _ => Err(ScriptError::new(format!(
                "Invalid string built-in property {prop_name}"
            ))),
        }
    }
}

impl StringDatumHandlers {
    pub fn count(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(datum).string_value()?;
            let operand = player.get_datum(&args[0]).string_value()?;
            let delimiter = player.movie.item_delimiter;
            let count = string_get_count(&value, &operand, delimiter)?;
            Ok(player.alloc_datum(Datum::Int(count as i32)))
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(datum).string_value()?;
            let index = player.get_datum(&args[0]).int_value()? as usize;
            // 1-based index
            let ch = value.chars().nth(index.wrapping_sub(1)).unwrap_or(' ');
            Ok(player.alloc_datum(Datum::String(ch.to_string())))
        })
    }

    pub fn duplicate(datum: &DatumRef, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(datum).string_value()?;
            Ok(player.alloc_datum(Datum::String(value)))
        })
    }

    pub fn get_chunk_prop_ref(
        datum: &DatumRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = player.get_datum(&args[0]).string_value()?;
            let start = player.get_datum(&args[1]).int_value()?;
            let end = if args.len() > 2 {
                player.get_datum(&args[2]).int_value()?
            } else {
                start
            };

            let prop_ref = StringDatumUtils::get_prop_ref(player, datum, &prop_name, start, end)?;
            Ok(player.alloc_datum(prop_ref))
        })
    }

    pub fn get_chunk_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = player.get_datum(&args[0]).string_value()?;
            let start = player.get_datum(&args[1]).int_value()?;
            let end = if args.len() > 2 {
                player.get_datum(&args[2]).int_value()?
            } else {
                start
            };

            let str_value = StringDatumUtils::get_prop_ref(player, datum, &prop_name, start, end)?
                .string_value()?;
            Ok(player.alloc_datum(Datum::String(str_value)))
        })
    }

    pub fn split(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(datum).string_value()?;
            let delimiter = if args.is_empty() {
                "&".to_string() // TODO: verify the correct default delimiter
            } else {
                player.get_datum(&args[0]).string_value()?
            };

            let parts: VecDeque<DatumRef> = value
                .split(&delimiter)
                .map(|s| player.alloc_datum(Datum::String(s.to_string())))
                .collect();

            // Create the list datum properly
            Ok(player.alloc_datum(Datum::List(
                DatumType::String, // type of elements
                parts,
                false, // not sorted
            )))
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "count" => Self::count(datum, args),
            "getAt" => Self::get_at(datum, args),
            "duplicate" => Self::duplicate(datum, args),
            "getPropRef" => Self::get_chunk_prop_ref(datum, args),
            "getProp" => Self::get_chunk_prop(datum, args),
            "split" => Self::split(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for string datum"
            ))),
        }
    }
}

pub fn string_get_count(
    value: &String,
    operand: &String,
    delimiter: char,
) -> Result<u32, ScriptError> {
    match operand.as_str() {
        "char" | "chars" => Ok(value.chars().count() as u32),
        "item" | "items" => Ok(string_get_items(value, delimiter).len() as u32),
        "word" | "words" => Ok(if value.len() > 0 {
            string_get_words(value).len() as u32
        } else {
            0
        }),
        "line" | "lines" => Ok(string_get_lines(value).len() as u32),
        _ => Err(ScriptError::new(format!(
            "Invalid operand {operand} for string_get_count"
        ))),
    }
}

pub fn string_get_items(value: &String, delimiter: char) -> Vec<String> {
    if delimiter == '\r' || delimiter == '\n' {
        string_get_lines(value)
    } else {
        value.split(delimiter).map(|s| s.to_string()).collect()
    }
}

fn is_director_whitespace(byte: char) -> bool {
    if byte.is_ascii_control() || byte.is_ascii_whitespace() {
        return true;
    } else {
        return false;
    }
}

#[allow(dead_code)]
pub fn string_get_words(value: &str) -> Vec<String> {
    value
        .split(is_director_whitespace)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn string_get_lines(value: &String) -> Vec<String> {
    // In Director, the number of lines in "" returns 0
    if value.is_empty() {
        return Vec::new();
    }
    let line_break = if value.contains("\r\n") {
        "\r\n"
    } else if value.contains("\n") {
        "\n"
    } else {
        "\r"
    };
    value.split(line_break).map(|s| s.to_string()).collect()
}

/// Remove trailing unbalanced `]` and `)` from an expression string.
fn trim_unbalanced_brackets(input: &str) -> String {
    let mut depth_square: i32 = 0;
    let mut depth_paren: i32 = 0;
    let mut in_string = false;
    let mut last_balanced_end = 0;

    for (i, ch) in input.char_indices() {
        if in_string {
            if ch == '"' { in_string = false; }
        } else {
            match ch {
                '"' => in_string = true,
                '[' => depth_square += 1,
                ']' => depth_square -= 1,
                '(' => depth_paren += 1,
                ')' => depth_paren -= 1,
                _ => {}
            }
        }
        if depth_square >= 0 && depth_paren >= 0 {
            last_balanced_end = i + ch.len_utf8();
        }
    }
    input[..last_balanced_end].trim().to_string()
}

/// Strip Lingo `--` comments from a string, respecting quoted strings.
fn strip_lingo_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_string = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            result.push(ch);
            in_string = true;
        } else if ch == '-' {
            if chars.peek() == Some(&'-') {
                // Comment: skip rest of line
                for c in chars.by_ref() {
                    if c == '\n' || c == '\r' {
                        result.push(c);
                        break;
                    }
                }
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}
