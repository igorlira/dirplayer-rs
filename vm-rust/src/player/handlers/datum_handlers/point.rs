use crate::{
    director::lingo::datum::{datum_bool, Datum},
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

pub struct PointDatumHandlers {}

impl PointDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "getAt" => Self::get_at(datum, args),
            "setAt" => Self::set_at(datum, args),
            "inside" => Self::inside(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for point"
            ))),
        }
    }

    pub fn inside(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let point = player.get_datum(datum).to_int_point()?;
            let rect = player.get_datum(&args[0]).to_int_rect()?;
            Ok(player.alloc_datum(datum_bool(
                rect.0 <= point.0 && point.0 < rect.2 && rect.1 <= point.1 && point.1 < rect.3,
            )))
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let rect = player.get_datum(datum);
            let rect = match rect {
                Datum::IntPoint(point_vec) => Ok(point_vec),
                _ => Err(ScriptError::new("Cannot get prop of non-point".to_string())),
            }?;
            let list_val = [rect.0, rect.1];
            let index = player.get_datum(&args[0]).int_value()?;
            Ok(player.alloc_datum(Datum::Int(list_val[(index - 1) as usize] as i32)))
        })
    }

    pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let pos = player.get_datum(&args[0]).int_value()?;
            let value = player.get_datum(&args[1]).int_value()?;

            let point = player.get_datum_mut(datum);
            let point = match point {
                Datum::IntPoint(point_vec) => Ok(point_vec),
                _ => Err(ScriptError::new("Cannot get prop of non-point".to_string())),
            }?;
            match pos {
                1 => point.0 = value,
                2 => point.1 = value,
                _ => return Err(ScriptError::new("Invalid index for point".to_string())),
            }
            Ok(DatumRef::Void)
        })
    }

    pub fn get_prop(
        player: &DirPlayer,
        datum: &DatumRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        let rect = player.get_datum(datum);
        let (left, top) = match rect {
            Datum::IntPoint(point) => Ok(point),
            _ => Err(ScriptError::new("Cannot get prop of non-point".to_string())),
        }?;
        match prop.as_str() {
            "locH" => Ok(Datum::Int(*left as i32)),
            "locV" => Ok(Datum::Int(*top as i32)),
            "ilk" => Ok(Datum::Symbol("point".to_string())),
            _ => Err(ScriptError::new(format!(
                "Cannot get point property {}",
                prop
            ))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        let value = player.get_datum(value_ref);
        match prop.as_str() {
            "locH" => {
                let value = value.int_value()?;
                let point = player.get_datum_mut(datum).to_int_point_mut()?;
                point.0 = value;
                Ok(())
            }
            "locV" => {
                let value = value.int_value()?;
                let point = player.get_datum_mut(datum).to_int_point_mut()?;
                point.1 = value;
                Ok(())
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set point property {}",
                prop
            ))),
        }
    }
}
