use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

use std::f64::consts::PI;

pub struct MathObject {
    pub id: u32,
}

impl MathObject {
    pub fn new(id: u32) -> Self {
        MathObject { id }
    }
}

pub struct MathDatumHandlers;

impl MathDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let math_id = player.get_datum(datum).to_math_ref()?;
            let _math_obj = player
                .math_objects
                .get(&math_id)
                .ok_or_else(|| ScriptError::new(format!("Math object {} not found", math_id)))?;

            let arg_values: Vec<f64> = args
                .iter()
                .filter_map(|a| player.get_datum(a).float_value().ok().map(|v| v as f64))
                .collect();

            let name = handler_name.to_lowercase();
            let result: f64 = match name.as_str() {
                // Basic
                "abs" => arg_values.get(0).copied().unwrap_or(0.0).abs(),
                "ceil" => arg_values.get(0).copied().unwrap_or(0.0).ceil(),
                "floor" => arg_values.get(0).copied().unwrap_or(0.0).floor(),
                "round" => arg_values.get(0).copied().unwrap_or(0.0).round(),

                // Trig
                "sin" => arg_values.get(0).copied().unwrap_or(0.0).sin(),
                "cos" => arg_values.get(0).copied().unwrap_or(0.0).cos(),
                "tan" => arg_values.get(0).copied().unwrap_or(0.0).tan(),
                "asin" => arg_values.get(0).copied().unwrap_or(0.0).asin(),
                "acos" => arg_values.get(0).copied().unwrap_or(0.0).acos(),
                "atan" => arg_values.get(0).copied().unwrap_or(0.0).atan(),

                // Power/log
                "sqrt" => arg_values.get(0).copied().unwrap_or(0.0).sqrt(),
                "exp" => arg_values.get(0).copied().unwrap_or(0.0).exp(),
                "log" => arg_values.get(0).copied().unwrap_or(0.0).ln(),
                "pow" => {
                    let base = arg_values.get(0).copied().unwrap_or(0.0);
                    let exp = arg_values.get(1).copied().unwrap_or(1.0);
                    base.powf(exp)
                }

                // Min / Max (✅ use f64::min and f64::max)
                "min" => arg_values.iter().copied().fold(f64::INFINITY, f64::min),
                "max" => arg_values.iter().copied().fold(f64::NEG_INFINITY, f64::max),

                _ => {
                    return Err(ScriptError::new(format!(
                        "Unknown math function '{}'",
                        handler_name
                    )))
                }
            };

            Ok(player.alloc_datum(Datum::Float(result as f32)))
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        let name = prop.to_lowercase();
        match name.as_str() {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("math".to_owned()))),
            "pi" => Ok(player.alloc_datum(Datum::Float(PI as f32))),
            _ => Err(ScriptError::new(format!(
                "Unknown math property '{}'",
                prop
            ))),
        }
    }

    pub fn set_prop(
        _player: &mut DirPlayer,
        _datum: &DatumRef,
        prop: &String,
        _value: &DatumRef,
    ) -> Result<(), ScriptError> {
        Err(ScriptError::new(format!(
            "Cannot set math property '{}'",
            prop
        )))
    }
}
