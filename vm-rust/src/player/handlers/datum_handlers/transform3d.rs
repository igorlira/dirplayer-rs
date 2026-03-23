//! Lingo Transform object handler.
//! A Transform is a mutable 4x4 row-major matrix used for 3D position/rotation/scale.

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

pub struct Transform3dDatumHandlers;

impl Transform3dDatumHandlers {
    pub fn get_prop(player: &mut DirPlayer, datum: &DatumRef, prop: &str) -> Result<Datum, ScriptError> {
        let m = match player.get_datum(datum) {
            Datum::Transform3d(m) => *m,
            _ => return Err(ScriptError::new("Expected Transform3d".into())),
        };

        match prop {
            "position" => Ok(Datum::Vector([m[12], m[13], m[14]])),
            "rotation" => {
                let (rx, ry, rz) = matrix_to_euler(&m);
                Ok(Datum::Vector([rx, ry, rz]))
            }
            "scale" => {
                let sx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt();
                let sy = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt();
                let sz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt();
                Ok(Datum::Vector([sx, sy, sz]))
            }
            "xAxis" => Ok(Datum::Vector([m[0], m[1], m[2]])),
            "yAxis" => Ok(Datum::Vector([m[4], m[5], m[6]])),
            "zAxis" => Ok(Datum::Vector([m[8], m[9], m[10]])),
            _ => Err(ScriptError::new(format!("Unknown transform property '{}'", prop))),
        }
    }

    pub fn set_prop(player: &mut DirPlayer, datum: &DatumRef, prop: &str, value: &DatumRef) -> Result<(), ScriptError> {
        let val = player.get_datum(value).clone();
        let m = match player.get_datum_mut(datum) {
            Datum::Transform3d(m) => m,
            _ => return Err(ScriptError::new("Expected Transform3d".into())),
        };

        match prop {
            "position" => {
                if let Datum::Vector(v) = val {
                    // Guard: only set finite values
                    if v[0].is_finite() { m[12] = v[0]; }
                    if v[1].is_finite() { m[13] = v[1]; }
                    if v[2].is_finite() { m[14] = v[2]; }
                }
                Ok(())
            }
            "rotation" => {
                if let Datum::Vector(v) = val {
                    // Preserve position and scale, rebuild rotation
                    let pos = [
                        if m[12].is_finite() { m[12] } else { 0.0 },
                        if m[13].is_finite() { m[13] } else { 0.0 },
                        if m[14].is_finite() { m[14] } else { 0.0 },
                    ];
                    // Guard: if current matrix has NaN, use scale 1.0
                    let sx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt();
                    let sy = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt();
                    let sz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt();
                    let sx = if sx.is_finite() && sx > 1e-10 { sx } else { 1.0 };
                    let sy = if sy.is_finite() && sy > 1e-10 { sy } else { 1.0 };
                    let sz = if sz.is_finite() && sz > 1e-10 { sz } else { 1.0 };
                    let rot = euler_to_matrix(v[0], v[1], v[2]);
                    // Apply scale to rotation columns
                    m[0] = rot[0]*sx;  m[1] = rot[1]*sx;  m[2] = rot[2]*sx;
                    m[4] = rot[4]*sy;  m[5] = rot[5]*sy;  m[6] = rot[6]*sy;
                    m[8] = rot[8]*sz;  m[9] = rot[9]*sz;  m[10] = rot[10]*sz;
                    m[12] = pos[0]; m[13] = pos[1]; m[14] = pos[2];
                }
                Ok(())
            }
            "scale" => {
                if let Datum::Vector(v) = val {
                    // Normalize existing rotation columns, then apply new scale
                    let cur_sx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt();
                    let cur_sy = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt();
                    let cur_sz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt();
                    if cur_sx > 0.0 { let s = v[0] / cur_sx; m[0] *= s; m[1] *= s; m[2] *= s; }
                    if cur_sy > 0.0 { let s = v[1] / cur_sy; m[4] *= s; m[5] *= s; m[6] *= s; }
                    if cur_sz > 0.0 { let s = v[2] / cur_sz; m[8] *= s; m[9] *= s; m[10] *= s; }
                }
                Ok(())
            }
            _ => Err(ScriptError::new(format!("Cannot set transform property '{}'", prop))),
        }
    }

    pub fn call(datum: &DatumRef, handler_name: &str, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "identity" => Self::identity(datum),
            "translate" => Self::translate(datum, args, true),    // Director translate = pre-multiply (moves in local space)
            "preTranslate" => Self::translate(datum, args, false),
            "rotate" => Self::rotate(datum, args, true),     // Director rotate = pre-multiply (R*M, transforms position)
            "preRotate" => Self::rotate(datum, args, false), // Director preRotate = post-multiply (M*R, doesn't transform position)
            "scale" => Self::scale(datum, args, true),
            "preScale" => Self::scale(datum, args, false),
            "inverse" => Self::inverse(datum),
            "duplicate" => Self::duplicate(datum),
            "multiply" => Self::multiply(datum, args),
            "interpolate" => Self::interpolate(datum, args),
            "interpolateTo" => Self::interpolate_to(datum, args),
            "getAt" => Self::get_at(datum, args),
            "setAt" => Self::set_at(datum, args),
            _ => Err(ScriptError::new(format!("No handler '{}' for transform", handler_name))),
        }
    }

    fn identity(datum: &DatumRef) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            *player.get_datum_mut(datum) = Datum::Transform3d(IDENTITY);
            Ok(DatumRef::Void)
        })
    }

    fn translate(datum: &DatumRef, args: &[DatumRef], pre: bool) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (dx, dy, dz) = Self::read_xyz(player, args)?;
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };

            let t = [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                dx,  dy,  dz,  1.0,
            ];

            let result = if pre { mat4_mul(&t, &m) } else { mat4_mul(&m, &t) };
            *player.get_datum_mut(datum) = Datum::Transform3d(result);
            Ok(DatumRef::Void)
        })
    }

    fn rotate(datum: &DatumRef, args: &[DatumRef], pre: bool) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (rx, ry, rz) = Self::read_xyz(player, args)?;
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };

            let r = euler_to_matrix(rx, ry, rz);
            let result = if pre { mat4_mul(&r, &m) } else { mat4_mul(&m, &r) };
            *player.get_datum_mut(datum) = Datum::Transform3d(result);
            Ok(DatumRef::Void)
        })
    }

    fn scale(datum: &DatumRef, args: &[DatumRef], pre: bool) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (sx, sy, sz) = Self::read_xyz(player, args)?;
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };

            let s = [
                sx,  0.0, 0.0, 0.0,
                0.0, sy,  0.0, 0.0,
                0.0, 0.0, sz,  0.0,
                0.0, 0.0, 0.0, 1.0,
            ];

            let result = if pre { mat4_mul(&s, &m) } else { mat4_mul(&m, &s) };
            *player.get_datum_mut(datum) = Datum::Transform3d(result);
            Ok(DatumRef::Void)
        })
    }

    fn inverse(datum: &DatumRef) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };
            let inv = mat4_invert_affine(&m);
            Ok(player.alloc_datum(Datum::Transform3d(inv)))
        })
    }

    fn duplicate(datum: &DatumRef) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };
            Ok(player.alloc_datum(Datum::Transform3d(m)))
        })
    }

    fn multiply(datum: &DatumRef, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };
            let other = match player.get_datum(&args[0]) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d argument".into())),
            };
            let result = mat4_mul(&m, &other);
            Ok(player.alloc_datum(Datum::Transform3d(result)))
        })
    }

    fn interpolate(datum: &DatumRef, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };
            let target = match player.get_datum(&args[0]) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d argument".into())),
            };
            let t = player.get_datum(&args[1]).float_value()? / 100.0; // percent → 0-1

            let mut result = [0.0f64; 16];
            for i in 0..16 {
                result[i] = m[i] + (target[i] - m[i]) * t;
            }
            Ok(player.alloc_datum(Datum::Transform3d(result)))
        })
    }

    fn interpolate_to(datum: &DatumRef, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };
            let target = match player.get_datum(&args[0]) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d argument".into())),
            };
            let t = player.get_datum(&args[1]).float_value()? / 100.0;

            let mut result = [0.0f64; 16];
            for i in 0..16 {
                result[i] = m[i] + (target[i] - m[i]) * t;
            }
            *player.get_datum_mut(datum) = Datum::Transform3d(result);
            Ok(DatumRef::Void)
        })
    }

    fn get_at(datum: &DatumRef, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let m = match player.get_datum(datum) {
                Datum::Transform3d(m) => *m,
                _ => return Err(ScriptError::new("Expected Transform3d".into())),
            };
            let index = (player.get_datum(&args[0]).int_value()? - 1) as usize;
            if index >= 16 {
                return Err(ScriptError::new("Transform index out of range".into()));
            }
            Ok(player.alloc_datum(Datum::Float(m[index])))
        })
    }

    fn set_at(datum: &DatumRef, args: &[DatumRef]) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let index = (player.get_datum(&args[0]).int_value()? - 1) as usize;
            let value = player.get_datum(&args[1]).float_value()?;
            if index >= 16 {
                return Err(ScriptError::new("Transform index out of range".into()));
            }
            if let Datum::Transform3d(m) = player.get_datum_mut(datum) {
                m[index] = value;
            }
            Ok(DatumRef::Void)
        })
    }

    /// Read (x, y, z) from args - either 3 separate floats or a single vector
    fn read_xyz(player: &DirPlayer, args: &[DatumRef]) -> Result<(f64, f64, f64), ScriptError> {
        if args.len() >= 3 {
            let x = player.get_datum(&args[0]).float_value()?;
            let y = player.get_datum(&args[1]).float_value()?;
            let z = player.get_datum(&args[2]).float_value()?;
            Ok((x, y, z))
        } else if args.len() >= 1 {
            match player.get_datum(&args[0]) {
                Datum::Vector(v) => Ok((v[0], v[1], v[2])),
                _ => {
                    let x = player.get_datum(&args[0]).float_value()?;
                    Ok((x, 0.0, 0.0))
                }
            }
        } else {
            Ok((0.0, 0.0, 0.0))
        }
    }
}

// ─── Matrix math ───

/// Column-major 4x4 matrix multiply: C = A * B
fn mat4_mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut r = [0.0f64; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] =
                a[0 * 4 + row] * b[col * 4 + 0] +
                a[1 * 4 + row] * b[col * 4 + 1] +
                a[2 * 4 + row] * b[col * 4 + 2] +
                a[3 * 4 + row] * b[col * 4 + 3];
        }
    }
    r
}

/// Invert a column-major affine transform
fn mat4_invert_affine(m: &[f64; 16]) -> [f64; 16] {
    // Column-major: R[row][col] = m[col*4 + row]
    let (tx, ty, tz) = (m[12], m[13], m[14]);
    // -R^T * t
    let itx = -(m[0] * tx + m[1] * ty + m[2] * tz);
    let ity = -(m[4] * tx + m[5] * ty + m[6] * tz);
    let itz = -(m[8] * tx + m[9] * ty + m[10] * tz);
    [
        m[0], m[4], m[8],  0.0,  // R^T col 0
        m[1], m[5], m[9],  0.0,  // R^T col 1
        m[2], m[6], m[10], 0.0,  // R^T col 2
        itx,  ity,  itz,   1.0,
    ]
}

/// Euler angles to column-major rotation matrix (IFX convention: R = Rx * Ry * Rz)
pub fn euler_to_matrix(rx_deg: f64, ry_deg: f64, rz_deg: f64) -> [f64; 16] {
    // Guard against NaN/infinity — use 0 for any invalid input
    let rx = if rx_deg.is_finite() { rx_deg } else { 0.0 }.to_radians();
    let ry = if ry_deg.is_finite() { (-ry_deg) } else { 0.0 }.to_radians();
    let rz = if rz_deg.is_finite() { rz_deg } else { 0.0 }.to_radians();

    let (sx, cx) = (rx.sin(), rx.cos());
    let (sy, cy) = (ry.sin(), ry.cos());
    let (sz, cz) = (rz.sin(), rz.cos());

    // R = Rz * Ry * Rx, true column-major: m[col*4+row]
    [
        cy*cz,                     cy*sz,                     -sy,                     0.0,  // col 0
        sx*sy*cz - cx*sz,          sx*sy*sz + cx*cz,          sx*cy,                   0.0,  // col 1
        cx*sy*cz + sx*sz,          cx*sy*sz - sx*cz,          cx*cy,                   0.0,  // col 2
        0.0,                       0.0,                       0.0,                     1.0,  // col 3
    ]
}

/// Extract euler angles from rotation matrix (matching euler_to_matrix convention)
fn matrix_to_euler(m: &[f64; 16]) -> (f64, f64, f64) {
    // Normalize rotation columns to remove scale before extracting angles
    let s0 = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt().max(1e-10);
    let s1 = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt().max(1e-10);
    let s2 = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt().max(1e-10);
    let n = [m[0]/s0, m[1]/s0, m[2]/s0, 0.0,
             m[4]/s1, m[5]/s1, m[6]/s1, 0.0,
             m[8]/s2, m[9]/s2, m[10]/s2, 0.0,
             0.0, 0.0, 0.0, 1.0];

    // Guard: if matrix contains NaN, return zero rotation
    if !n[0].is_finite() || !n[2].is_finite() || !n[10].is_finite() {
        return (0.0, 0.0, 0.0);
    }

    // IFX convention: Y = -asin(m[2]) where m[2] = R[2][0]
    let sy = (-n[2]).clamp(-1.0, 1.0);
    let ry = sy.asin();
    let cy = ry.cos();

    let (rx, rz);
    if cy.abs() > 1e-6 {
        // X = atan2(R[2][1], R[2][2]) = atan2(m[6], m[10])
        rx = (n[6] / cy).atan2(n[10] / cy);
        // Z = atan2(R[1][0], R[0][0]) = atan2(m[1], m[0])
        rz = (n[1] / cy).atan2(n[0] / cy);
    } else {
        rx = 0.0;
        rz = n[4].atan2(n[5]);
    }

    (rx.to_degrees(), -ry.to_degrees(), rz.to_degrees())
}
