use crate::{
    director::lingo::datum::{
        Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType,
    },
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
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
            "length" => Ok(Datum::Int(value.len() as i32)),
            "ilk" => Ok(Datum::Symbol("string".to_owned())),
            "string" => Ok(Datum::String(value.clone())),
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

            let parts: Vec<DatumRef> = value
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
        "char" | "chars" => Ok(value.len() as u32),
        "item" | "items" => Ok(string_get_items(value, delimiter).len() as u32),
        "word" | "words" => Ok(if value.len() > 0 {
            value.split_whitespace().count() as u32
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

#[allow(dead_code)]
pub fn string_get_words(value: &String) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn string_get_lines(value: &String) -> Vec<String> {
    let line_break = if value.contains("\r\n") {
        "\r\n"
    } else if value.contains("\n") {
        "\n"
    } else {
        "\r"
    };
    value.split(line_break).map(|s| s.to_string()).collect()
}
