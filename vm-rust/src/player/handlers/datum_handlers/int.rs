use crate::{
    director::lingo::datum::Datum,
    player::{DatumRef, DirPlayer, ScriptError},
};

pub struct IntDatumHandlers {}

impl IntDatumHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        datum_ref: &DatumRef,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let int_value = player.get_datum(datum_ref).int_value()?;

        match prop {
            "abs" => Ok(player.alloc_datum(Datum::Int(int_value.abs()))),
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("integer".to_string()))),
            "integer" => Ok(datum_ref.clone()),
            "float" => Ok(player.alloc_datum(Datum::Float(int_value as f64))),
            "number" => Ok(datum_ref.clone()),
            "char" => {
                if int_value >= 0 && int_value <= 255 {
                    let ch = char::from_u32(int_value as u32).unwrap_or('?');
                    Ok(player.alloc_datum(Datum::String(ch.to_string())))
                } else {
                    Err(ScriptError::new(format!(
                        "Integer {} out of range for char (must be 0-255)", 
                        int_value
                    )))
                }
            }
            "string" => Ok(player.alloc_datum(Datum::String(int_value.to_string()))),
            "magnitude" => Ok(player.alloc_datum(Datum::Int(int_value.abs()))),
            "sprite" => Ok(player.alloc_datum(Datum::SpriteRef(int_value as i16))),
            // Director allows trig/math functions as numeric properties via dot
            // syntax: `n.cos` is equivalent to `cos(n)`. Output is always Float.
            // Estate's gcam.transform.rotation.z * pi() / 180.cos parses as
            // (((-z * pi()) / 180).cos) * forward — chained off an int literal.
            "sin" => Ok(player.alloc_datum(Datum::Float((int_value as f64).sin()))),
            "cos" => Ok(player.alloc_datum(Datum::Float((int_value as f64).cos()))),
            "tan" => Ok(player.alloc_datum(Datum::Float((int_value as f64).tan()))),
            "asin" => Ok(player.alloc_datum(Datum::Float((int_value as f64).asin()))),
            "acos" => Ok(player.alloc_datum(Datum::Float((int_value as f64).acos()))),
            "atan" => Ok(player.alloc_datum(Datum::Float((int_value as f64).atan()))),
            "sqrt" => Ok(player.alloc_datum(Datum::Float((int_value as f64).sqrt()))),
            "log" => Ok(player.alloc_datum(Datum::Float((int_value as f64).ln()))),
            "exp" => Ok(player.alloc_datum(Datum::Float((int_value as f64).exp()))),
            _ => Err(ScriptError::new(format!(
                "Cannot get int property {}",
                prop
            ))),
        }
    }
}
