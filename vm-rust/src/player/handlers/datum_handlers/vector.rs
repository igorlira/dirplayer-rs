use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

pub struct VectorDatumHandlers {}

impl VectorDatumHandlers {
    /// Convert a Datum (Vector or List) into a [f32;3] array
    fn datum_to_vec(player: &DirPlayer, datum: &Datum) -> Result<[f32; 3], ScriptError> {
        match datum {
            Datum::Vector(arr) => Ok(*arr),
            Datum::List(_, list, _) if list.len() == 3 => Ok([
                player.get_datum(&list[0]).float_value()? as f32,
                player.get_datum(&list[1]).float_value()? as f32,
                player.get_datum(&list[2]).float_value()? as f32,
            ]),
            _ => Err(ScriptError::new("Expected a vector".to_string())),
        }
    }

    /// Convert a [f32;3] array into a Datum::Vector
    fn vec_to_datum(player: &mut DirPlayer, vec: [f32; 3]) -> DatumRef {
        player.alloc_datum(Datum::Vector(vec))
    }

    /// Call a handler by name
    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &[DatumRef],
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "getAt" => Self::get_at(datum, args),
            "setAt" => Self::set_at(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for vector"
            ))),
        }
    }

    /// Get a vector component by index (1-based)
    pub fn get_at(datum: &DatumRef, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let vec = Self::datum_to_vec(player, player.get_datum(datum))?;
            let index = (player.get_datum(&args[0]).int_value()? - 1) as usize;
            if index >= 3 {
                return Err(ScriptError::new(
                    "Index out of range for vector".to_string(),
                ));
            }
            Ok(player.alloc_datum(Datum::Float(vec[index])))
        })
    }

    /// Set a vector component by index (1-based)
    pub fn set_at(datum: &DatumRef, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let mut vec = match player.get_datum_mut(datum) {
                Datum::Vector(arr) => *arr,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set prop of non-vector".to_string(),
                    ))
                }
            };

            let index = (player.get_datum(&args[0]).int_value()? - 1) as usize;
            if index >= 3 {
                return Err(ScriptError::new(
                    "Index out of range for vector".to_string(),
                ));
            }

            let value = player.get_datum(&args[1]).float_value()? as f32;
            vec[index] = value;

            *player.get_datum_mut(datum) = Datum::Vector(vec);
            Ok(DatumRef::Void)
        })
    }

    /// Get a vector property (x, y, z, ilk)
    pub fn get_prop(
        player: &DirPlayer,
        datum: &DatumRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let [x, y, z] = Self::datum_to_vec(player, player.get_datum(datum))?;
        match prop {
            "x" => Ok(Datum::Float(x)),
            "y" => Ok(Datum::Float(y)),
            "z" => Ok(Datum::Float(z)),
            "ilk" => Ok(Datum::Symbol("vector".to_string())),
            _ => Err(ScriptError::new(format!(
                "Cannot get vector property {}",
                prop
            ))),
        }
    }

    /// Set a vector property (x, y, z)
    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &str,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        let mut vec = match player.get_datum_mut(datum) {
            Datum::Vector(arr) => *arr,
            _ => {
                return Err(ScriptError::new(
                    "Cannot set prop of non-vector".to_string(),
                ))
            }
        };

        let value = player.get_datum(value_ref).float_value()? as f32;

        match prop {
            "x" => vec[0] = value,
            "y" => vec[1] = value,
            "z" => vec[2] = value,
            _ => {
                return Err(ScriptError::new(format!(
                    "Cannot set vector property {}",
                    prop
                )))
            }
        }

        *player.get_datum_mut(datum) = Datum::Vector(vec);
        Ok(())
    }

    /// Vector addition
    pub fn add(a: &DatumRef, b: &DatumRef) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let va = Self::datum_to_vec(player, player.get_datum(a))?;
            let vb = Self::datum_to_vec(player, player.get_datum(b))?;
            Ok(Self::vec_to_datum(
                player,
                [va[0] + vb[0], va[1] + vb[1], va[2] + vb[2]],
            ))
        })
    }

    /// Vector subtraction
    pub fn sub(a: &DatumRef, b: &DatumRef) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let va = Self::datum_to_vec(player, player.get_datum(a))?;
            let vb = Self::datum_to_vec(player, player.get_datum(b))?;
            Ok(Self::vec_to_datum(
                player,
                [va[0] - vb[0], va[1] - vb[1], va[2] - vb[2]],
            ))
        })
    }

    /// Vector multiplication (scalar or component-wise)
    pub fn mul(a: &DatumRef, b: &DatumRef) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let va = Self::datum_to_vec(player, player.get_datum(a))?;
            match player.get_datum(b) {
                Datum::Float(f) => Ok(Self::vec_to_datum(
                    player,
                    [
                        va[0] * (*f as f32),
                        va[1] * (*f as f32),
                        va[2] * (*f as f32),
                    ],
                )),
                Datum::Vector(vb) => Ok(Self::vec_to_datum(
                    player,
                    [va[0] * vb[0], va[1] * vb[1], va[2] * vb[2]],
                )),
                _ => Err(ScriptError::new(
                    "Invalid operand for vector multiplication".to_string(),
                )),
            }
        })
    }

    /// Vector division (scalar or component-wise)
    pub fn div(a: &DatumRef, b: &DatumRef) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let va = Self::datum_to_vec(player, player.get_datum(a))?;
            match player.get_datum(b) {
                Datum::Float(f) => {
                    if *f == 0.0 {
                        return Err(ScriptError::new("Division by zero".to_string()));
                    }
                    Ok(Self::vec_to_datum(
                        player,
                        [
                            va[0] / (*f as f32),
                            va[1] / (*f as f32),
                            va[2] / (*f as f32),
                        ],
                    ))
                }
                Datum::Vector(vb) => {
                    if vb[0] == 0.0 || vb[1] == 0.0 || vb[2] == 0.0 {
                        return Err(ScriptError::new(
                            "Division by zero in vector components".to_string(),
                        ));
                    }
                    Ok(Self::vec_to_datum(
                        player,
                        [va[0] / vb[0], va[1] / vb[1], va[2] / vb[2]],
                    ))
                }
                _ => Err(ScriptError::new(
                    "Invalid operand for vector division".to_string(),
                )),
            }
        })
    }
}
