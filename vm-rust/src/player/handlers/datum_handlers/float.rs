use crate::{
    director::lingo::datum::Datum,
    player::{DatumRef, DirPlayer, ScriptError},
};

pub struct FloatDatumHandlers {}

impl FloatDatumHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        datum_ref: &DatumRef,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let float_value = player.get_datum(datum_ref).float_value()?;
        match prop {
            "abs" => Ok(player.alloc_datum(Datum::Float(float_value.abs()))),
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("float".to_string()))),
            "integer" => Ok(player.alloc_datum(Datum::Int(float_value.round() as i32))),
            "float" => Ok(datum_ref.clone()),
            "char" => {
                let int_value = float_value.round() as i32;
                if int_value >= 0 && int_value <= 255 {
                    let ch = char::from_u32(int_value as u32).unwrap_or('?');
                    Ok(player.alloc_datum(Datum::String(ch.to_string())))
                } else {
                    Err(ScriptError::new(format!(
                        "Float {} out of range for char (must be 0-255)", 
                        float_value
                    )))
                }
            }
            "string" => Ok(player.alloc_datum(Datum::String(float_value.to_string()))),
            "magnitude" => Ok(player.alloc_datum(Datum::Float(float_value.abs()))),
            // Vector-like access: return 0.0 for x/y/z when a float is used where a vector was expected
            "x" | "y" | "z" => Ok(player.alloc_datum(Datum::Float(0.0))),
            // Director allows trig/math functions as numeric properties via the dot
            // syntax: `n.cos` is equivalent to `cos(n)`, `n.sqrt` to `sqrt(n)`, etc.
            // Inputs/outputs are in radians for trig, matching Director's globals.
            "sin" => Ok(player.alloc_datum(Datum::Float(float_value.sin()))),
            "cos" => Ok(player.alloc_datum(Datum::Float(float_value.cos()))),
            "tan" => Ok(player.alloc_datum(Datum::Float(float_value.tan()))),
            "asin" => Ok(player.alloc_datum(Datum::Float(float_value.asin()))),
            "acos" => Ok(player.alloc_datum(Datum::Float(float_value.acos()))),
            "atan" => Ok(player.alloc_datum(Datum::Float(float_value.atan()))),
            "sqrt" => Ok(player.alloc_datum(Datum::Float(float_value.sqrt()))),
            "log" => Ok(player.alloc_datum(Datum::Float(float_value.ln()))),
            "exp" => Ok(player.alloc_datum(Datum::Float(float_value.exp()))),
            _ => Err(ScriptError::new(format!(
                "Cannot get float property {}",
                prop
            ))),
        }
    }
}
