use crate::{
    director::lingo::datum::Datum,
    player::{DatumRef, DirPlayer, ScriptError, reserve_player_mut, symbols::{builtin::BuiltInSymbol, symbol::Symbol}},
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
        handler_name: Symbol,
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

            let arg0 = || arg_values.get(0).copied().unwrap_or(0.0);
            let arg1 = || arg_values.get(1).copied().unwrap_or(0.0);

            let result: f64 = match handler_name.into_builtin_or_error()? {
                BuiltInSymbol::Abs   => arg0().abs(),
                BuiltInSymbol::Ceil  => arg0().ceil(),
                BuiltInSymbol::Floor => arg0().floor(),
                BuiltInSymbol::Round => arg0().round(),
                BuiltInSymbol::Sin   => arg0().sin(),
                BuiltInSymbol::Cos   => arg0().cos(),
                BuiltInSymbol::Tan   => arg0().tan(),
                BuiltInSymbol::Asin  => arg0().asin(),
                BuiltInSymbol::Acos  => arg0().acos(),
                BuiltInSymbol::Atan  => arg0().atan(),
                BuiltInSymbol::Atan2 => arg0().atan2(arg1()),
                BuiltInSymbol::Sqrt  => arg0().sqrt(),
                BuiltInSymbol::Exp   => arg0().exp(),
                BuiltInSymbol::Log   => arg0().ln(),
                BuiltInSymbol::Pow   => arg0().powf(arg1()),
                BuiltInSymbol::Min   => arg_values.iter().copied().fold(f64::INFINITY, f64::min),
                BuiltInSymbol::Max   => arg_values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                _ => return Err(ScriptError::new(format!("Unknown math function '{handler_name}'")))
            };

            Ok(player.alloc_datum(Datum::Float(result)))
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        _datum: &DatumRef,
        prop: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        match prop.into_builtin_or_error()? {
            BuiltInSymbol::Ilk => Ok(player.alloc_datum(Datum::Symbol(BuiltInSymbol::Math.into()))),
            BuiltInSymbol::Pi  => Ok(player.alloc_datum(Datum::Float(PI))),
            _ => Err(ScriptError::new(format!("Unknown math property '{prop}'"))),
        }
    }

    pub fn set_prop(
        _player: &mut DirPlayer,
        _datum: &DatumRef,
        prop: Symbol,
        _value: &DatumRef,
    ) -> Result<(), ScriptError> {
        Err(ScriptError::new(format!("Cannot set math property '{prop}'")))
    }
}
