use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

pub struct RectDatumHandlers {}
pub struct RectUtils {}

impl RectUtils {
    pub fn union(rect1: (i32, i32, i32, i32), rect2: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
        let left = rect1.0.min(rect2.0);
        let top = rect1.1.min(rect2.1);
        let right = rect1.2.max(rect2.2);
        let bottom = rect1.3.max(rect2.3);
        (left, top, right, bottom)
    }

    pub fn intersect(
        rect1: (i32, i32, i32, i32),
        rect2: (i32, i32, i32, i32),
    ) -> (i32, i32, i32, i32) {
        let left = rect1.0.max(rect2.0);
        let top = rect1.1.max(rect2.1);
        let right = rect1.2.min(rect2.2);
        let bottom = rect1.3.min(rect2.3);
        // If rectangles don't overlap, return empty rect (0,0,0,0)
        if left >= right || top >= bottom {
            return (0, 0, 0, 0);
        }
        (left, top, right, bottom)
    }
}

impl RectDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "getAt" => Self::get_at(datum, args),
            "setAt" => Self::set_at(datum, args),
            "intersect" => Self::intersect(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for rect"
            ))),
        }
    }

    pub fn intersect(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let rect1 = player.get_datum(datum).to_int_rect()?;
            let rect2 = player.get_datum(&args[0]).to_int_rect()?;
            let (left, top, right, bottom) = RectUtils::intersect(rect1, rect2);
            Ok(player.alloc_datum(Datum::IntRect((left, top, right, bottom))))
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let rect = player.get_datum(datum);
            let rect = match rect {
                Datum::IntRect(rect_vec) => Ok(rect_vec),
                _ => Err(ScriptError::new("Cannot get prop of non-rect".to_string())),
            }?;
            let list_val = [rect.0, rect.1, rect.2, rect.3];
            let index = player.get_datum(&args[0]).int_value()?;
            Ok(player.alloc_datum(Datum::Int(list_val[(index - 1) as usize] as i32)))
        })
    }

    pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let pos = player.get_datum(&args[0]).int_value()?;
            let value = player.get_datum(&args[1]).int_value()?;

            let rect = player.get_datum_mut(datum);
            let rect = match rect {
                Datum::IntRect(rect_vec) => Ok(rect_vec),
                _ => Err(ScriptError::new("Cannot get prop of non-rect".to_string())),
            }?;
            match pos {
                1 => rect.0 = value,
                2 => rect.1 = value,
                3 => rect.2 = value,
                4 => rect.3 = value,
                _ => return Err(ScriptError::new("Invalid index for rect".to_string())),
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
        let (left, top, right, bottom) = match rect {
            Datum::IntRect(rect_vec) => Ok(rect_vec),
            _ => Err(ScriptError::new("Cannot get prop of non-rect".to_string())),
        }?;
        match prop.as_str() {
            "width" => Ok(Datum::Int(*right as i32 - *left as i32)),
            "height" => Ok(Datum::Int(*bottom as i32 - *top as i32)),
            "left" => Ok(Datum::Int(*left as i32)),
            "top" => Ok(Datum::Int(*top as i32)),
            "right" => Ok(Datum::Int(*right as i32)),
            "bottom" => Ok(Datum::Int(*bottom as i32)),
            _ => Err(ScriptError::new(format!(
                "Cannot get rect property {}",
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
        match prop.as_str() {
            "left" => {
                let value = player.get_datum(value_ref).int_value()?;
                let rect = player.get_datum_mut(datum).to_int_rect_mut()?;
                rect.0 = value;
                Ok(())
            }
            "top" => {
                let value = player.get_datum(value_ref).int_value()?;
                let rect = player.get_datum_mut(datum).to_int_rect_mut()?;
                rect.1 = value;
                Ok(())
            }
            "right" => {
                let value = player.get_datum(value_ref).int_value()?;
                let rect = player.get_datum_mut(datum).to_int_rect_mut()?;
                rect.2 = value;
                Ok(())
            }
            "bottom" => {
                let value = player.get_datum(value_ref).int_value()?;
                let rect = player.get_datum_mut(datum).to_int_rect_mut()?;
                rect.3 = value;
                Ok(())
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set rect property {}",
                prop
            ))),
        }
    }
}
