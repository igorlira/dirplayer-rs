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
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "getAt" => Self::get_at(datum, args),
            "setAt" => Self::set_at(datum, args),
            "intersect" => Self::intersect(datum, args),
            "duplicate" => Self::duplicate(datum, args),
            "offset" => Self::offset(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for rect"
            ))),
        }
    }

    pub fn duplicate(datum: &DatumRef, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (vals, flags) = player.get_datum(datum).to_rect_inline()?;
            Ok(player.alloc_datum(Datum::Rect(vals, flags)))
        })
    }

    pub fn offset(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (vals, _flags) = player.get_datum(datum).to_rect_inline()?;
            let dx = player.get_datum(&args[0]).int_value()?;
            let dy = player.get_datum(&args[1]).int_value()?;

            // offset always produces int results
            Ok(player.alloc_datum(Datum::Rect([
                vals[0] + dx as f64,
                vals[1] + dy as f64,
                vals[2] + dx as f64,
                vals[3] + dy as f64,
            ], 0)))
        })
    }

    pub fn intersect(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (r1, _f1) = player.get_datum(datum).to_rect_inline()?;
            let (r2, _f2) = player.get_datum(&args[0]).to_rect_inline()?;

            // intersect uses min/max
            let l = r1[0].min(r2[0]);
            let t = r1[1].min(r2[1]);
            let r = r1[2].max(r2[2]);
            let b = r1[3].max(r2[3]);

            // Use from_f64 logic for flags
            let result = Datum::build_rect(
                &Datum::from_f64(l),
                &Datum::from_f64(t),
                &Datum::from_f64(r),
                &Datum::from_f64(b),
            )?;

            Ok(player.alloc_datum(result))
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (vals, flags) = player.get_datum(datum).to_rect_inline()?;

            let index = player.get_datum(&args[0]).int_value()?; // 1..4
            if !(1..=4).contains(&index) {
                return Err(ScriptError::new("Invalid index for rect".to_string()));
            }

            let i = (index - 1) as usize;
            let component = Datum::inline_component_to_datum(vals[i], Datum::inline_is_float(flags, i));
            Ok(player.alloc_datum(component))
        })
    }

    pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let index = player.get_datum(&args[0]).int_value()?;
            let new_val = player.get_datum(&args[1]).clone();

            if !(1..=4).contains(&index) {
                return Err(ScriptError::new("Invalid index for rect".to_string()));
            }

            let (val, is_float) = Datum::datum_to_inline_component(&new_val)?;

            let i = (index - 1) as usize;
            let (vals, flags) = player.get_datum_mut(datum).to_rect_inline_mut()?;
            vals[i] = val;
            Datum::inline_set_float(flags, i, is_float);

            Ok(DatumRef::Void)
        })
    }

    pub fn get_prop(player: &DirPlayer, datum: &DatumRef, prop: &str) -> Result<Datum, ScriptError> {
        let (vals, flags) = player.get_datum(datum).to_rect_inline()?;

        let left = vals[0];
        let top = vals[1];
        let right = vals[2];
        let bottom = vals[3];

        match prop {
            "ilk" => Ok(Datum::Symbol("rect".to_string())),
            "width" => Ok(Datum::from_f64(right - left)),
            "height" => Ok(Datum::from_f64(bottom - top)),
            "left" => Ok(Datum::inline_component_to_datum(left, Datum::inline_is_float(flags, 0))),
            "top" => Ok(Datum::inline_component_to_datum(top, Datum::inline_is_float(flags, 1))),
            "right" => Ok(Datum::inline_component_to_datum(right, Datum::inline_is_float(flags, 2))),
            "bottom" => Ok(Datum::inline_component_to_datum(bottom, Datum::inline_is_float(flags, 3))),
            _ => Err(ScriptError::new(format!("Cannot get rect property {}", prop))),
        }
    }

    pub fn set_prop(player: &mut DirPlayer, datum: &DatumRef, prop: &str, value_ref: &DatumRef) -> Result<(), ScriptError> {
        let idx = match prop {
            "left" => 0usize,
            "top" => 1usize,
            "right" => 2usize,
            "bottom" => 3usize,
            _ => return Err(ScriptError::new(format!("Cannot set rect property {}", prop))),
        };

        let new_val = player.get_datum(value_ref).clone();
        let (val, is_float) = Datum::datum_to_inline_component(&new_val)?;

        let (vals, flags) = player.get_datum_mut(datum).to_rect_inline_mut()?;
        vals[idx] = val;
        Datum::inline_set_float(flags, idx, is_float);

        Ok(())
    }
}
