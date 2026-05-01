use crate::{
    director::lingo::datum::{datum_bool, Datum},
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

pub struct PointDatumHandlers {}

impl PointDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "getAt" => Self::get_at(datum, args),
            "setAt" => Self::set_at(datum, args),
            "inside" => Self::inside(datum, args),
            "duplicate" => Self::duplicate(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for point"
            ))),
        }
    }

    pub fn duplicate(datum: &DatumRef, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (vals, flags) = player.get_datum(datum).to_point_inline()?;
            Ok(player.alloc_datum(Datum::Point(vals, flags)))
        })
    }

    pub fn inside(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (point, _pf) = player.get_datum(datum).to_point_inline()?;
            let (rect, _rf) = player.get_datum(&args[0]).to_rect_inline()?;

            let px = point[0] as i32;
            let py = point[1] as i32;
            let x1 = rect[0] as i32;
            let y1 = rect[1] as i32;
            let x2 = rect[2] as i32;
            let y2 = rect[3] as i32;

            let inside = x1 <= px && px < x2 && y1 <= py && py < y2;

            Ok(player.alloc_datum(datum_bool(inside)))
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (vals, flags) = player.get_datum(datum).to_point_inline()?;

            let index = player.get_datum(&args[0]).int_value()?; // 1 or 2
            if !(1..=2).contains(&index) {
                return Err(ScriptError::new("Invalid index for point".to_string()));
            }

            let i = (index - 1) as usize;
            let component = Datum::inline_component_to_datum(vals[i], Datum::inline_is_float(flags, i));
            Ok(player.alloc_datum(component))
        })
    }

    pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let index = player.get_datum(&args[0]).int_value()?;

            if !(1..=2).contains(&index) {
                return Err(ScriptError::new("Invalid index for point".to_string()));
            }

            let new_val = player.get_datum(&args[1]).clone();
            let (val, is_float) = Datum::datum_to_inline_component(&new_val)?;

            let i = (index - 1) as usize;
            let (vals, flags) = player.get_datum_mut(datum).to_point_inline_mut()?;
            vals[i] = val;
            Datum::inline_set_float(flags, i, is_float);

            Ok(DatumRef::Void)
        })
    }

    pub fn get_prop(
        player: &DirPlayer,
        datum: &DatumRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let (vals, flags) = player.get_datum(datum).to_point_inline()?;

        match prop {
            "locH" => Ok(Datum::inline_component_to_datum(vals[0], Datum::inline_is_float(flags, 0))),
            "locV" => Ok(Datum::inline_component_to_datum(vals[1], Datum::inline_is_float(flags, 1))),
            "ilk"  => Ok(Datum::Symbol("point".to_string())),
            _ => Err(ScriptError::new(format!("Cannot get point property {}", prop))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &str,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        let new_val = player.get_datum(value_ref).clone();
        let (val, is_float) = Datum::datum_to_inline_component(&new_val)?;

        let idx = match prop {
            "locH" => 0usize,
            "locV" => 1usize,
            _ => return Err(ScriptError::new(format!("Cannot set point property {}", prop))),
        };

        let (vals, flags) = player.get_datum_mut(datum).to_point_inline_mut()?;
        vals[idx] = val;
        Datum::inline_set_float(flags, idx, is_float);

        Ok(())
    }
}
