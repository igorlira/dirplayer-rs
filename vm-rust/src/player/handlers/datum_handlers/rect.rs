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

    pub fn duplicate(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let rect = player.get_datum(datum).to_rect()?;
            let new_rect_refs = [
                player.alloc_datum(player.get_datum(&rect[0]).clone()),
                player.alloc_datum(player.get_datum(&rect[1]).clone()),
                player.alloc_datum(player.get_datum(&rect[2]).clone()),
                player.alloc_datum(player.get_datum(&rect[3]).clone()),
            ];
            let new_rect = Datum::Rect(new_rect_refs);
            Ok(player.alloc_datum(new_rect))
        })
    }

    pub fn offset(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let rect = player.get_datum(datum).to_rect()?;
            let dx = player.get_datum(&args[0]).int_value()?;
            let dy = player.get_datum(&args[1]).int_value()?;

            let new_rect_refs = [
                player.alloc_datum(Datum::Int(player.get_datum(&rect[0]).int_value()? + dx)),
                player.alloc_datum(Datum::Int(player.get_datum(&rect[1]).int_value()? + dy)),
                player.alloc_datum(Datum::Int(player.get_datum(&rect[2]).int_value()? + dx)),
                player.alloc_datum(Datum::Int(player.get_datum(&rect[3]).int_value()? + dy)),
            ];

            Ok(player.alloc_datum(Datum::Rect(new_rect_refs)))
        })
    }

    pub fn intersect(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let r1 = player.get_datum(datum).to_rect()?;
            let r2 = player.get_datum(&args[0]).to_rect()?;

            // Compute intersection as floats (to preserve mixed Int/Float)
            let l = Datum::to_f64(&player, &r1[0])?.min(Datum::to_f64(&player, &r2[0])?);
            let t = Datum::to_f64(&player, &r1[1])?.min(Datum::to_f64(&player, &r2[1])?);
            let r = Datum::to_f64(&player, &r1[2])?.max(Datum::to_f64(&player, &r2[2])?);
            let b = Datum::to_f64(&player, &r1[3])?.max(Datum::to_f64(&player, &r2[3])?);

            let rect_refs = [
                player.alloc_datum(Datum::from_f64(l)),
                player.alloc_datum(Datum::from_f64(t)),
                player.alloc_datum(Datum::from_f64(r)),
                player.alloc_datum(Datum::from_f64(b)),
            ];

            Ok(player.alloc_datum(Datum::Rect(rect_refs)))
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let arr = match player.get_datum(datum) {
                Datum::Rect(arr) => arr,
                _ => return Err(ScriptError::new("Cannot getAt of non-rect".to_string())),
            };

            let index = player.get_datum(&args[0]).int_value()?; // 1..4
            if !(1..=4).contains(&index) {
                return Err(ScriptError::new("Invalid index for rect".to_string()));
            }

            let value_ref = &arr[(index - 1) as usize];
            let value = player.get_datum(value_ref);

            match value {
                Datum::Int(_) | Datum::Float(_) => Ok(value_ref.clone()),
                other => Err(ScriptError::new(format!(
                    "Rect component is not numeric: {}",
                    other.type_str()
                ))),
            }
        })
    }

    pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let index = player.get_datum(&args[0]).int_value()?;
            let new_val = player.get_datum(&args[1]).clone();

            if !(1..=4).contains(&index) {
                return Err(ScriptError::new("Invalid index for rect".to_string()));
            }

            let new_ref = match new_val {
                Datum::Int(n) => player.alloc_datum(Datum::Int(n)),
                Datum::Float(f) => player.alloc_datum(Datum::Float(f)),
                other => return Err(ScriptError::new(format!(
                    "Rect component must be numeric, got {}",
                    other.type_str()
                ))),
            };

            let arr = match player.get_datum_mut(datum) {
                Datum::Rect(arr) => arr,
                _ => return Err(ScriptError::new("Cannot setAt of non-rect".to_string())),
            };

            arr[(index - 1) as usize] = new_ref;

            Ok(DatumRef::Void)
        })
    }

    pub fn get_prop(player: &DirPlayer, datum: &DatumRef, prop: &str) -> Result<Datum, ScriptError> {
        let rect = player.get_datum(datum);
        let rect_arr = match rect {
            Datum::Rect(arr) => arr,
            _ => return Err(ScriptError::new("Cannot get prop of non-rect".to_string())),
        };

        let left = Datum::to_f64(player, &rect_arr[0])?;
        let top = Datum::to_f64(player, &rect_arr[1])?;
        let right = Datum::to_f64(player, &rect_arr[2])?;
        let bottom = Datum::to_f64(player, &rect_arr[3])?;

        match prop {
            "ilk" => Ok(Datum::Symbol("rect".to_string())),
            "width" => Ok(Datum::from_f64(right - left)),
            "height" => Ok(Datum::from_f64(bottom - top)),
            "left" => Ok(Datum::from_f64(left)),
            "top" => Ok(Datum::from_f64(top)),
            "right" => Ok(Datum::from_f64(right)),
            "bottom" => Ok(Datum::from_f64(bottom)),
            _ => Err(ScriptError::new(format!("Cannot get rect property {}", prop))),
        }
    }

    pub fn set_prop(player: &mut DirPlayer, datum: &DatumRef, prop: &str, value_ref: &DatumRef) -> Result<(), ScriptError> {
        let idx = match prop {
            "left" => 0,
            "top" => 1,
            "right" => 2,
            "bottom" => 3,
            _ => return Err(ScriptError::new(format!("Cannot set rect property {}", prop))),
        };

        let new_val = player.get_datum(value_ref).clone();
        let new_ref = match new_val {
            Datum::Int(n) => player.alloc_datum(Datum::Int(n)),
            Datum::Float(f) => player.alloc_datum(Datum::Float(f)),
            other => return Err(ScriptError::new(format!(
                "Rect property must be numeric, got {}",
                other.type_str()
            ))),
        };

        let arr = match player.get_datum_mut(datum) {
            Datum::Rect(arr) => arr,
            _ => return Err(ScriptError::new("Cannot set prop of non-rect".to_string())),
        };

        arr[idx] = new_ref;

        Ok(())
    }
}
