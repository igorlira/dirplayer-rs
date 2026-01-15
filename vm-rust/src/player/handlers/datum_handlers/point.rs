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
            let point = player.get_datum(datum).to_point()?;
            let rect = player.get_datum(&args[0]).to_rect()?;

            let px = player.get_datum(&point[0]).int_value()?;
            let py = player.get_datum(&point[1]).int_value()?;

            let x1 = player.get_datum(&rect[0]).int_value()?;
            let y1 = player.get_datum(&rect[1]).int_value()?;
            let x2 = player.get_datum(&rect[2]).int_value()?;
            let y2 = player.get_datum(&rect[3]).int_value()?;

            let inside = x1 <= px && px < x2 && y1 <= py && py < y2;

            Ok(player.alloc_datum(datum_bool(inside)))
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let point_arr = match player.get_datum(datum) {
                Datum::Point(arr) => arr,
                _ => return Err(ScriptError::new("Cannot getAt of non-point".to_string())),
            };

            let index = player.get_datum(&args[0]).int_value()?; // 1 or 2
            if !(1..=2).contains(&index) {
                return Err(ScriptError::new("Invalid index for point".to_string()));
            }

            let value_ref = &point_arr[(index - 1) as usize];
            let value = player.get_datum(value_ref);

            match value {
                Datum::Int(_) | Datum::Float(_) => Ok(value_ref.clone()),
                other => Err(ScriptError::new(format!(
                    "Point component is not numeric: {}",
                    other.type_str()
                ))),
            }
        })
    }

    pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let index = player.get_datum(&args[0]).int_value()?;

            if !(1..=2).contains(&index) {
                return Err(ScriptError::new("Invalid index for point".to_string()));
            }

            let new_val = player.get_datum(&args[1]).clone();

            let new_ref = match new_val {
                Datum::Int(n) => player.alloc_datum(Datum::Int(n)),
                Datum::Float(f) => player.alloc_datum(Datum::Float(f)),
                other => {
                    return Err(ScriptError::new(format!(
                        "Point component must be numeric, got {}",
                        other.type_str()
                    )))
                }
            };

            let point_arr = match player.get_datum_mut(datum) {
                Datum::Point(arr) => arr,
                _ => return Err(ScriptError::new("Cannot setAt of non-point".to_string())),
            };

            point_arr[(index - 1) as usize] = new_ref;

            Ok(DatumRef::Void)
        })
    }

    pub fn get_prop(
        player: &DirPlayer,
        datum: &DatumRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        let point_arr = match player.get_datum(datum) {
            Datum::Point(arr) => arr,
            _ => return Err(ScriptError::new("Cannot get prop of non-point".to_string())),
        };

        let x = Datum::to_f64(player, &point_arr[0])?;
        let y = Datum::to_f64(player, &point_arr[1])?;

        match prop.as_str() {
            "locH" => Ok(Datum::from_f64(x)),
            "locV" => Ok(Datum::from_f64(y)),
            "ilk"  => Ok(Datum::Symbol("point".to_string())),
            _ => Err(ScriptError::new(format!("Cannot get point property {}", prop))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        let new_val = player.get_datum(value_ref).clone();

        let idx = match prop.as_str() {
            "locH" => 0,
            "locV" => 1,
            _ => return Err(ScriptError::new(format!("Cannot set point property {}", prop))),
        };

        let new_ref = match new_val {
            Datum::Int(n) => player.alloc_datum(Datum::Int(n)),
            Datum::Float(f) => player.alloc_datum(Datum::Float(f)),
            other => return Err(ScriptError::new(format!(
                "Point property must be numeric, got {}",
                other.type_str()
            ))),
        };

        let point_arr = match player.get_datum_mut(datum) {
            Datum::Point(arr) => arr,
            _ => return Err(ScriptError::new("Cannot set prop of non-point".to_string())),
        };

        point_arr[idx] = new_ref;

        Ok(())
    }
}
