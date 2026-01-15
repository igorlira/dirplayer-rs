use crate::{
    director::lingo::datum::Datum,
    player::{DatumRef, DirPlayer, ScriptError},
};

pub struct FloatDatumHandlers {}

impl FloatDatumHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        datum_ref: &DatumRef,
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        let float_value = player.get_datum(datum_ref).float_value()?;
        match prop.as_str() {
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
            _ => Err(ScriptError::new(format!(
                "Cannot get float property {}",
                prop
            ))),
        }
    }
}
