use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

pub struct VectorDatumHandlers {}

impl VectorDatumHandlers {
    /// Convert a Datum (Vector or List) into a [f64;3] array
    fn datum_to_vec(player: &DirPlayer, datum: &Datum) -> Result<[f64; 3], ScriptError> {
        match datum {
            Datum::Vector(arr) => Ok(*arr),
            Datum::List(_, list, _) if list.len() == 3 => Ok([
                player.get_datum(&list[0]).float_value()?,
                player.get_datum(&list[1]).float_value()?,
                player.get_datum(&list[2]).float_value()?,
            ]),
            Datum::Void | Datum::Int(0) => Ok([0.0, 0.0, 0.0]),
            _ => Err(ScriptError::new(format!("Expected a vector, got {}", datum.type_str()))),
        }
    }

    /// Convert a [f64;3] array into a Datum::Vector
    fn vec_to_datum(player: &mut DirPlayer, vec: [f64; 3]) -> DatumRef {
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
            "duplicate" => reserve_player_mut(|player| {
                let vec = Self::datum_to_vec(player, player.get_datum(datum))?;
                Ok(player.alloc_datum(Datum::Vector(vec)))
            }),
            "distanceTo" => reserve_player_mut(|player| {
                let a = Self::datum_to_vec(player, player.get_datum(datum))?;
                let b = Self::datum_to_vec(player, player.get_datum(&args[0]))?;
                let dx = a[0] - b[0];
                let dy = a[1] - b[1];
                let dz = a[2] - b[2];
                Ok(player.alloc_datum(Datum::Float((dx*dx + dy*dy + dz*dz).sqrt())))
            }),
            "getNormalized" => reserve_player_mut(|player| {
                let [x, y, z] = Self::datum_to_vec(player, player.get_datum(datum))?;
                let len = (x*x + y*y + z*z).sqrt();
                if len > 1e-10 {
                    Ok(player.alloc_datum(Datum::Vector([x/len, y/len, z/len])))
                } else {
                    Ok(player.alloc_datum(Datum::Vector([0.0, 0.0, 0.0])))
                }
            }),
            "normalize" => reserve_player_mut(|player| {
                let [x, y, z] = Self::datum_to_vec(player, player.get_datum(datum))?;
                let len = (x*x + y*y + z*z).sqrt();
                if len > 1e-10 {
                    *player.get_datum_mut(datum) = Datum::Vector([x/len, y/len, z/len]);
                }
                Ok(DatumRef::Void)
            }),
            "crossProduct" | "cross" => reserve_player_mut(|player| {
                let a = Self::datum_to_vec(player, player.get_datum(datum))?;
                let b = Self::datum_to_vec(player, player.get_datum(&args[0]))?;
                Ok(player.alloc_datum(Datum::Vector([
                    a[1]*b[2] - a[2]*b[1],
                    a[2]*b[0] - a[0]*b[2],
                    a[0]*b[1] - a[1]*b[0],
                ])))
            }),
            "dotProduct" | "dot" => reserve_player_mut(|player| {
                let a = Self::datum_to_vec(player, player.get_datum(datum))?;
                let b = Self::datum_to_vec(player, player.get_datum(&args[0]))?;
                Ok(player.alloc_datum(Datum::Float(a[0]*b[0] + a[1]*b[1] + a[2]*b[2])))
            }),
            "angleBetween" => reserve_player_mut(|player| {
                let a = Self::datum_to_vec(player, player.get_datum(datum))?;
                let b = Self::datum_to_vec(player, player.get_datum(&args[0]))?;
                let len_a = (a[0]*a[0] + a[1]*a[1] + a[2]*a[2]).sqrt();
                let len_b = (b[0]*b[0] + b[1]*b[1] + b[2]*b[2]).sqrt();
                let angle = if len_a > 1e-10 && len_b > 1e-10 {
                    let cos_angle = (a[0]*b[0] + a[1]*b[1] + a[2]*b[2]) / (len_a * len_b);
                    cos_angle.clamp(-1.0, 1.0).acos().to_degrees()
                } else {
                    0.0
                };
                Ok(player.alloc_datum(Datum::Float(angle)))
            }),
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

            let value = player.get_datum(&args[1]).float_value()? as f64;
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
            "magnitude" | "length" => Ok(Datum::Float((x * x + y * y + z * z).sqrt())),
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

        let value = player.get_datum(value_ref).float_value()? as f64;

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

        // Write back to parent transform if this vector came from transform.position/rotation
        if let Some((_, parent_ref, sub_prop)) = player.transform_sub_refs.iter()
            .find(|(vec_ref, _, _)| vec_ref == datum).cloned() {
            if let Datum::Transform3d(m) = player.get_datum_mut(&parent_ref) {
                match sub_prop.as_str() {
                    "position" => {
                        m[12] = vec[0]; m[13] = vec[1]; m[14] = vec[2];
                    }
                    "rotation" => {
                        let pos = [m[12], m[13], m[14]];
                        let sx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt();
                        let sy = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt();
                        let sz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt();
                        let rot = crate::player::handlers::datum_handlers::transform3d::euler_to_matrix(vec[0], vec[1], vec[2]);
                        m[0] = rot[0]*sx;  m[1] = rot[1]*sx;  m[2] = rot[2]*sx;
                        m[4] = rot[4]*sy;  m[5] = rot[5]*sy;  m[6] = rot[6]*sy;
                        m[8] = rot[8]*sz;  m[9] = rot[9]*sz;  m[10] = rot[10]*sz;
                        m[12] = pos[0]; m[13] = pos[1]; m[14] = pos[2];
                    }
                    "scale" => {
                        // Set column lengths to new scale while preserving rotation direction
                        let old_sx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt().max(1e-10);
                        let old_sy = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt().max(1e-10);
                        let old_sz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt().max(1e-10);
                        let fx = vec[0] / old_sx;
                        let fy = vec[1] / old_sy;
                        let fz = vec[2] / old_sz;
                        m[0] *= fx; m[1] *= fx; m[2] *= fx;
                        m[4] *= fy; m[5] *= fy; m[6] *= fy;
                        m[8] *= fz; m[9] *= fz; m[10] *= fz;
                    }
                    _ => {}
                }
            }
        }

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
                        va[0] * (*f as f64),
                        va[1] * (*f as f64),
                        va[2] * (*f as f64),
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
                            va[0] / (*f as f64),
                            va[1] / (*f as f64),
                            va[2] / (*f as f64),
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
