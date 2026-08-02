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

            let result: f64 = match handler_name.into_builtin() {
                Some(BuiltInSymbol::Abs)   => arg0().abs(),
                Some(BuiltInSymbol::Ceil)  => arg0().ceil(),
                Some(BuiltInSymbol::Floor) => arg0().floor(),
                Some(BuiltInSymbol::Round) => arg0().round(),
                Some(BuiltInSymbol::Sin)   => arg0().sin(),
                Some(BuiltInSymbol::Cos)   => arg0().cos(),
                Some(BuiltInSymbol::Tan)   => arg0().tan(),
                Some(BuiltInSymbol::Asin)  => arg0().asin(),
                Some(BuiltInSymbol::Acos)  => arg0().acos(),
                Some(BuiltInSymbol::Atan)  => arg0().atan(),
                Some(BuiltInSymbol::Atan2) => arg0().atan2(arg1()),
                Some(BuiltInSymbol::Sqrt)  => arg0().sqrt(),
                Some(BuiltInSymbol::Exp)   => arg0().exp(),
                Some(BuiltInSymbol::Log)   => arg0().ln(),
                Some(BuiltInSymbol::Pow)   => arg0().powf(arg1()),
                Some(BuiltInSymbol::Min)   => arg_values.iter().copied().fold(f64::INFINITY, f64::min),
                Some(BuiltInSymbol::Max)   => arg_values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
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
        match prop.into_builtin() {
            Some(BuiltInSymbol::Ilk) => Ok(player.alloc_datum(Datum::Symbol(BuiltInSymbol::Math.into()))),
            Some(BuiltInSymbol::Pi)  => Ok(player.alloc_datum(Datum::Float(PI))),
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
