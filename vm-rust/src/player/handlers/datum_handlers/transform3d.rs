//! Lingo Transform object handler.
//! A Transform is a mutable 4x4 row-major matrix used for 3D position/rotation/scale.

use std::collections::HashSet;
use std::cell::RefCell;

thread_local! {
    /// Track which Transform3d datum IDs were mutated in-place (dirty).
    /// sync_persistent_transforms only writes dirty datums to node_transforms.
    pub static DIRTY_TRANSFORM_IDS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

pub fn mark_transform_dirty(datum_ref: &crate::player::DatumRef) {
    DIRTY_TRANSFORM_IDS.with(|d| d.borrow_mut().insert(datum_ref.unwrap()));
}

pub fn take_dirty_ids() -> HashSet<usize> {
    DIRTY_TRANSFORM_IDS.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

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
            "axisAngle" => {
                // Extract axis-angle from the rotation part of the matrix
                let (axis, angle) = matrix_to_axis_angle(&m);
                let axis_ref = player.alloc_datum(Datum::Vector(axis));
                let angle_ref = player.alloc_datum(Datum::Float(angle));
                Ok(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    std::collections::VecDeque::from([axis_ref, angle_ref]),
                    false,
                ))
            }
            _ => Err(ScriptError::new(format!("Unknown transform property '{}'", prop))),
        }
    }

    pub fn set_prop(player: &mut DirPlayer, datum: &DatumRef, prop: &str, value: &DatumRef) -> Result<(), ScriptError> {
        mark_transform_dirty(datum);
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
                    // Debug: log position sets with large Z (overlay models at Z≈-500)
                    if v[2].abs() > 400.0 {
                        static T3D_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                        if T3D_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                            web_sys::console::log_1(&format!(
                                "[T3D-POS] transform.position = ({:.1},{:.1},{:.1}) datum_id={:?}",
                                v[0], v[1], v[2], datum
                            ).into());
                        }
                    }
                }
                Ok(())
            }
            "rotation" => {
                if let Datum::Vector(v) = val {
                    // Log non-zero Z rotation (steering)
                    if v[2].abs() > 0.1 {
                        static ROT_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                        if ROT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 5 {
                            web_sys::console::log_1(&format!(
                                "[T3D-ROT] transform.rotation = ({:.1},{:.1},{:.1}) pos=({:.1},{:.1},{:.1})",
                                v[0], v[1], v[2], m[12], m[13], m[14]
                            ).into());
                        }
                    }
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
            "axisAngle" | "axisangle" => {
                // axisAngle = [vector(axis), angle_degrees]
                // Extract values before getting mutable borrow on transform
                let (axis, angle_deg) = if let Datum::List(_, items, _) = &val {
                    if items.len() >= 2 {
                        let axis = match player.get_datum(&items[0]) {
                            Datum::Vector(v) => *v,
                            _ => return Err(ScriptError::new("axisAngle: expected vector for axis".into())),
                        };
                        let angle_deg = player.get_datum(&items[1]).to_float()?;
                        (Some(axis), angle_deg)
                    } else { (None, 0.0) }
                } else { (None, 0.0) };

                if let Some(axis) = axis {
                    let m = match player.get_datum_mut(datum) {
                        Datum::Transform3d(m) => m,
                        _ => return Err(ScriptError::new("Expected Transform3d".into())),
                    };
                    let pos = [m[12], m[13], m[14]];
                    let sx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt();
                    let sy = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt();
                    let sz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt();
                    let sx = if sx.is_finite() && sx > 1e-10 { sx } else { 1.0 };
                    let sy = if sy.is_finite() && sy > 1e-10 { sy } else { 1.0 };
                    let sz = if sz.is_finite() && sz > 1e-10 { sz } else { 1.0 };
                    let rot = axis_angle_to_matrix(&axis, angle_deg);
                    m[0] = rot[0]*sx;  m[1] = rot[1]*sx;  m[2] = rot[2]*sx;  m[3] = 0.0;
                    m[4] = rot[4]*sy;  m[5] = rot[5]*sy;  m[6] = rot[6]*sy;  m[7] = 0.0;
                    m[8] = rot[8]*sz;  m[9] = rot[9]*sz;  m[10] = rot[10]*sz; m[11] = 0.0;
                    m[12] = pos[0]; m[13] = pos[1]; m[14] = pos[2]; m[15] = 1.0;
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
            "getProp" | "getPropRef" => {
                // transform.rotation[3] → getProp(#rotation, 3)
                reserve_player_mut(|player| {
                    let prop_name = player.get_datum(&args[0]).string_value()?;
                    let prop_datum = Self::get_prop(player, datum, &prop_name)?;
                    if args.len() > 1 {
                        let index = player.get_datum(&args[1]).int_value()?;
                        let prop_ref = player.alloc_datum(prop_datum);
                        let prop_val = player.get_datum(&prop_ref).clone();
                        match prop_val {
                            Datum::Vector(v) => {
                                let idx = (index as usize).saturating_sub(1);
                                if idx < 3 {
                                    Ok(player.alloc_datum(Datum::Float(v[idx])))
                                } else {
                                    Ok(player.alloc_datum(Datum::Float(0.0)))
                                }
                            }
                            Datum::List(_, items, _) => {
                                let idx = (index as usize).saturating_sub(1);
                                if idx < items.len() {
                                    Ok(items[idx].clone())
                                } else {
                                    Ok(DatumRef::Void)
                                }
                            }
                            other => Ok(player.alloc_datum(other)),
                        }
                    } else {
                        Ok(player.alloc_datum(prop_datum))
                    }
                })
            }
            "count" => {
                // transform.rotation.count → 3
                reserve_player_mut(|player| {
                    let prop_name = player.get_datum(&args[0]).string_value()?;
                    let prop_datum = Self::get_prop(player, datum, &prop_name)?;
                    let count = match &prop_datum {
                        Datum::Vector(_) => 3,
                        Datum::List(_, items, _) => items.len() as i32,
                        _ => 1,
                    };
                    Ok(player.alloc_datum(Datum::Int(count)))
                })
            }
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

/// Euler angles to column-major rotation matrix (R = Rz * Ry * Rx)
pub fn euler_to_matrix(rx_deg: f64, ry_deg: f64, rz_deg: f64) -> [f64; 16] {
    // Guard against NaN/infinity — use 0 for any invalid input
    let rx = if rx_deg.is_finite() { rx_deg } else { 0.0 }.to_radians();
    let ry = if ry_deg.is_finite() { ry_deg } else { 0.0 }.to_radians();
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

    let sy = (-n[2]).clamp(-1.0, 1.0);
    let ry = sy.asin();
    let cy = ry.cos();

    let (rx, rz);
    if cy.abs() > 1e-6 {
        rx = (n[6] / cy).atan2(n[10] / cy);
        rz = (n[1] / cy).atan2(n[0] / cy);
    } else {
        rx = 0.0;
        rz = n[4].atan2(n[5]);
    }

    (rx.to_degrees(), ry.to_degrees(), rz.to_degrees())
}

/// Extract axis-angle representation from the rotation part of a 4x4 matrix.
/// Returns (axis [f64; 3], angle_degrees f64).
fn matrix_to_axis_angle(m: &[f64; 16]) -> ([f64; 3], f64) {
    // Normalize rotation columns to remove scale
    let s0 = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt().max(1e-10);
    let s1 = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt().max(1e-10);
    let s2 = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt().max(1e-10);
    let r00 = m[0]/s0; let r01 = m[1]/s0; let r02 = m[2]/s0;
    let r10 = m[4]/s1; let r11 = m[5]/s1; let r12 = m[6]/s1;
    let r20 = m[8]/s2; let r21 = m[9]/s2; let r22 = m[10]/s2;

    // trace = 1 + 2*cos(angle)
    let trace = r00 + r11 + r22;
    let cos_a = ((trace - 1.0) / 2.0).clamp(-1.0, 1.0);
    let angle = cos_a.acos(); // radians

    if angle.abs() < 1e-10 {
        // No rotation
        return ([1.0, 0.0, 0.0], 0.0);
    }

    let sin_a = angle.sin();
    if sin_a.abs() > 1e-10 {
        let k = 1.0 / (2.0 * sin_a);
        let axis = [
            (r21 - r12) * k,
            (r02 - r20) * k,
            (r10 - r01) * k,
        ];
        (axis, angle.to_degrees())
    } else {
        // angle ≈ 180°, need to extract axis from the matrix diagonal
        let (ax, ay, az) = if r00 >= r11 && r00 >= r22 {
            let x = ((r00 + 1.0) / 2.0).sqrt();
            (x, r01 / (2.0 * x), r02 / (2.0 * x))
        } else if r11 >= r22 {
            let y = ((r11 + 1.0) / 2.0).sqrt();
            (r01 / (2.0 * y), y, r12 / (2.0 * y))
        } else {
            let z = ((r22 + 1.0) / 2.0).sqrt();
            (r02 / (2.0 * z), r12 / (2.0 * z), z)
        };
        ([ax, ay, az], angle.to_degrees())
    }
}

/// Build a 4x4 rotation matrix from axis-angle (angle in degrees).
fn axis_angle_to_matrix(axis: &[f64; 3], angle_deg: f64) -> [f64; 16] {
    let len = (axis[0]*axis[0] + axis[1]*axis[1] + axis[2]*axis[2]).sqrt();
    if len < 1e-10 {
        return [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
    }
    let (x, y, z) = (axis[0]/len, axis[1]/len, axis[2]/len);
    let a = angle_deg.to_radians();
    let c = a.cos();
    let s = a.sin();
    let t = 1.0 - c;
    [
        t*x*x + c,    t*x*y + s*z,  t*x*z - s*y,  0.0,
        t*x*y - s*z,  t*y*y + c,    t*y*z + s*x,  0.0,
        t*x*z + s*y,  t*y*z - s*x,  t*z*z + c,    0.0,
        0.0,          0.0,          0.0,           1.0,
    ]
}
