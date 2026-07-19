//! Native Havok physics engine — complete port from C# reference implementation.
//! Reverse-engineered from PPC/x86 decompilation of Havok Xtra 10.1.

use crate::player::cast_member::HavokPhysicsState;

// ============================================================
// TYPE ALIASES
// ============================================================
pub type V3 = [f64; 3];
/// Quaternion [w, x, y, z]
pub type Quat = [f64; 4];
/// 3x3 matrix, row-major: [M00,M01,M02, M10,M11,M12, M20,M21,M22]
pub type Mat3 = [f64; 9];

/// True if body `body_idx`, placed at (`pos`, `orient`), overlaps any static mesh or other
/// body beyond `tolerance`. Used by `attemptMoveTo` to reject a blocked move. Temporarily
/// moves the body to test the pose, then restores it.
pub fn body_blocked_at(state: &mut HavokPhysicsState, body_idx: usize, pos: V3, orient: Quat) -> bool {
    if body_idx >= state.rigid_bodies.len() { return false; }
    let saved_pos = state.rigid_bodies[body_idx].position;
    let saved_orient = state.rigid_bodies[body_idx].orientation;
    state.rigid_bodies[body_idx].position = pos;
    state.rigid_bodies[body_idx].orientation = orient;
    let tol = state.tolerance;
    let involves = |c: &CollisionContact| c.body_a == body_idx || c.body_b == Some(body_idx);
    let blocked = {
        let s: &HavokPhysicsState = state;
        detect_all_collisions(s).iter().any(|c| involves(c) && c.depth > tol)
            || detect_body_body_collisions(s).iter().any(|c| involves(c) && c.depth > tol)
    };
    state.rigid_bodies[body_idx].position = saved_pos;
    state.rigid_bodies[body_idx].orientation = saved_orient;
    blocked
}

pub const QUAT_IDENTITY: Quat = [1.0, 0.0, 0.0, 0.0];
pub const MAT3_IDENTITY: Mat3 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
pub const MAT3_ZERO: Mat3 = [0.0; 9];

// ============================================================
// Vec3 math helpers (f64)
// ============================================================
#[inline] pub fn v3_add(a: V3, b: V3) -> V3 { [a[0]+b[0], a[1]+b[1], a[2]+b[2]] }
#[inline] pub fn v3_sub(a: V3, b: V3) -> V3 { [a[0]-b[0], a[1]-b[1], a[2]-b[2]] }
#[inline] pub fn v3_scale(a: V3, s: f64) -> V3 { [a[0]*s, a[1]*s, a[2]*s] }
#[inline] pub fn v3_neg(a: V3) -> V3 { [-a[0], -a[1], -a[2]] }
#[inline] pub fn v3_dot(a: V3, b: V3) -> f64 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }
#[inline] pub fn v3_cross(a: V3, b: V3) -> V3 {
    [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]
}
#[inline] pub fn v3_len_sq(a: V3) -> f64 { v3_dot(a, a) }
#[inline] pub fn v3_len(a: V3) -> f64 { v3_len_sq(a).sqrt() }
#[inline] pub fn v3_normalized(a: V3) -> V3 {
    let l = v3_len(a); if l > 1e-10 { v3_scale(a, 1.0/l) } else { [0.0;3] }
}

// ============================================================
// Quaternion operations
// From C# HavokMath.cs — Hamilton product, Shepperd conversion, etc.
// ============================================================

/// Hamilton product: a * b.
/// From PPC: .__ml__Q25Havok10QuaternionCFRCQ25Havok10Quaternion (0x18c0)
#[inline]
pub fn quat_mul(a: Quat, b: Quat) -> Quat {
    [
        a[0]*b[0] - a[1]*b[1] - a[2]*b[2] - a[3]*b[3],
        a[0]*b[1] + a[1]*b[0] + a[2]*b[3] - a[3]*b[2],
        a[0]*b[2] - a[1]*b[3] + a[2]*b[0] + a[3]*b[1],
        a[0]*b[3] + a[1]*b[2] - a[2]*b[1] + a[3]*b[0],
    ]
}

#[inline]
pub fn quat_conjugate(q: Quat) -> Quat { [q[0], -q[1], -q[2], -q[3]] }

pub fn quat_normalize(q: Quat) -> Quat {
    let len = (q[0]*q[0] + q[1]*q[1] + q[2]*q[2] + q[3]*q[3]).sqrt();
    if len < 1e-10 { return QUAT_IDENTITY; }
    let inv = 1.0 / len;
    [q[0]*inv, q[1]*inv, q[2]*inv, q[3]*inv]
}

/// Rotate vector by quaternion: result = q * v * q_conjugate.
/// From PPC: getRotatedPos (0x4c640), optimized form.
pub fn quat_rotate_v(q: Quat, v: V3) -> V3 {
    let ww2m1 = 2.0*q[0]*q[0] - 1.0;
    let dot = q[1]*v[0] + q[2]*v[1] + q[3]*v[2];
    let cx = q[2]*v[2] - q[3]*v[1];
    let cy = q[3]*v[0] - q[1]*v[2];
    let cz = q[1]*v[1] - q[2]*v[0];
    [
        ww2m1*v[0] + 2.0*(dot*q[1] + q[0]*cx),
        ww2m1*v[1] + 2.0*(dot*q[2] + q[0]*cy),
        ww2m1*v[2] + 2.0*(dot*q[3] + q[0]*cz),
    ]
}

/// Create quaternion from axis-angle (angle in radians).
pub fn quat_from_axis_angle(axis: V3, angle: f64) -> Quat {
    let half = angle * 0.5;
    let s = half.sin();
    let n = v3_normalized(axis);
    [half.cos(), n[0]*s, n[1]*s, n[2]*s]
}

/// Convert quaternion to axis-angle. Returns (axis, angle_radians).
/// From x86: sub_10001ACB
pub fn quat_to_axis_angle(q: Quat) -> (V3, f64) {
    let w_clamped = q[0].clamp(-1.0, 1.0);
    let angle = 2.0 * w_clamped.acos();
    let sin_half = (1.0 - q[0]*q[0]).sqrt();
    if sin_half < 0.001 {
        ([0.0, 0.0, 1.0], angle)
    } else {
        ([q[1]/sin_half, q[2]/sin_half, q[3]/sin_half], angle)
    }
}

/// Convert quaternion to 3x3 rotation matrix (row-major).
/// From PPC: setRotation__Q25Havok9TransformFRCQ25Havok10Quaternion (0x9a940)
pub fn quat_to_mat3(q: Quat) -> Mat3 {
    let xx = 2.0*q[1]*q[1]; let yy = 2.0*q[2]*q[2]; let zz = 2.0*q[3]*q[3];
    let xy = 2.0*q[1]*q[2]; let xz = 2.0*q[1]*q[3]; let yz = 2.0*q[2]*q[3];
    let wx = 2.0*q[0]*q[1]; let wy = 2.0*q[0]*q[2]; let wz = 2.0*q[0]*q[3];
    [
        1.0-yy-zz, xy-wz,     xz+wy,
        xy+wz,     1.0-xx-zz, yz-wx,
        xz-wy,     yz+wx,     1.0-xx-yy,
    ]
}

/// Convert rotation matrix to quaternion (Shepperd's method).
/// From PPC: set__Q25Havok10QuaternionFRCQ25Havok7Matrix3 (0x984b0)
pub fn quat_from_mat3(m: Mat3) -> Quat {
    let trace = m[0] + m[4] + m[8];
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [0.25*s, (m[7]-m[5])/s, (m[2]-m[6])/s, (m[3]-m[1])/s]
    } else if m[0] > m[4] && m[0] > m[8] {
        let s = (1.0 + m[0] - m[4] - m[8]).sqrt() * 2.0;
        [(m[7]-m[5])/s, 0.25*s, (m[1]+m[3])/s, (m[2]+m[6])/s]
    } else if m[4] > m[8] {
        let s = (1.0 + m[4] - m[0] - m[8]).sqrt() * 2.0;
        [(m[2]-m[6])/s, (m[1]+m[3])/s, 0.25*s, (m[5]+m[7])/s]
    } else {
        let s = (1.0 + m[8] - m[0] - m[4]).sqrt() * 2.0;
        [(m[3]-m[1])/s, (m[2]+m[6])/s, (m[5]+m[7])/s, 0.25*s]
    }
}

/// Create quaternion from axis-angle in degrees (Director/Lingo convention).
pub fn quat_from_axis_angle_degrees(axis: V3, angle_deg: f64) -> Quat {
    quat_from_axis_angle(axis, angle_deg * std::f64::consts::PI / 180.0)
}

// ============================================================
// Matrix3 operations (row-major 3x3)
// From C# HavokMath.cs
// ============================================================

/// Matrix * vector: result[i] = sum_j(M[i,j] * v[j])
#[inline]
pub fn mat3_transform(m: Mat3, v: V3) -> V3 {
    [
        m[0]*v[0] + m[1]*v[1] + m[2]*v[2],
        m[3]*v[0] + m[4]*v[1] + m[5]*v[2],
        m[6]*v[0] + m[7]*v[1] + m[8]*v[2],
    ]
}

/// Matrix * matrix
pub fn mat3_mul(a: Mat3, b: Mat3) -> Mat3 {
    [
        a[0]*b[0]+a[1]*b[3]+a[2]*b[6], a[0]*b[1]+a[1]*b[4]+a[2]*b[7], a[0]*b[2]+a[1]*b[5]+a[2]*b[8],
        a[3]*b[0]+a[4]*b[3]+a[5]*b[6], a[3]*b[1]+a[4]*b[4]+a[5]*b[7], a[3]*b[2]+a[4]*b[5]+a[5]*b[8],
        a[6]*b[0]+a[7]*b[3]+a[8]*b[6], a[6]*b[1]+a[7]*b[4]+a[8]*b[7], a[6]*b[2]+a[7]*b[5]+a[8]*b[8],
    ]
}

/// Matrix inverse (3x3 Cramer's rule).
/// From PPC: makeInverse__Q25Havok7Matrix3Fv
pub fn mat3_inverse(m: Mat3) -> Mat3 {
    let det = m[0]*(m[4]*m[8]-m[5]*m[7]) - m[1]*(m[3]*m[8]-m[5]*m[6]) + m[2]*(m[3]*m[7]-m[4]*m[6]);
    if det.abs() < 1e-20 { return MAT3_IDENTITY; }
    let inv_det = 1.0 / det;
    [
        (m[4]*m[8]-m[5]*m[7])*inv_det, (m[2]*m[7]-m[1]*m[8])*inv_det, (m[1]*m[5]-m[2]*m[4])*inv_det,
        (m[5]*m[6]-m[3]*m[8])*inv_det, (m[0]*m[8]-m[2]*m[6])*inv_det, (m[2]*m[3]-m[0]*m[5])*inv_det,
        (m[3]*m[7]-m[4]*m[6])*inv_det, (m[1]*m[6]-m[0]*m[7])*inv_det, (m[0]*m[4]-m[1]*m[3])*inv_det,
    ]
}

pub fn mat3_transpose(m: Mat3) -> Mat3 {
    [m[0],m[3],m[6], m[1],m[4],m[7], m[2],m[5],m[8]]
}

pub fn mat3_scale_f(m: Mat3, s: f64) -> Mat3 {
    [m[0]*s,m[1]*s,m[2]*s, m[3]*s,m[4]*s,m[5]*s, m[6]*s,m[7]*s,m[8]*s]
}

pub fn mat3_add(a: Mat3, b: Mat3) -> Mat3 {
    [a[0]+b[0],a[1]+b[1],a[2]+b[2], a[3]+b[3],a[4]+b[4],a[5]+b[5], a[6]+b[6],a[7]+b[7],a[8]+b[8]]
}

// ============================================================
// Inertia computation
// From C# InertiaComputation.cs
// ============================================================

/// Box inertia: I_xx = m/12*(dy²+dz²), I_yy = m/12*(dx²+dz²), I_zz = m/12*(dx²+dy²)
pub fn box_inertia(mass: f64, dx: f64, dy: f64, dz: f64) -> Mat3 {
    let f = mass / 12.0;
    [f*(dy*dy+dz*dz), 0.0, 0.0, 0.0, f*(dx*dx+dz*dz), 0.0, 0.0, 0.0, f*(dx*dx+dy*dy)]
}

/// Sphere inertia: I_diag = 2/5 * m * r²
pub fn sphere_inertia(mass: f64, radius: f64) -> Mat3 {
    let d = 0.4 * mass * radius * radius;
    [d, 0.0, 0.0, 0.0, d, 0.0, 0.0, 0.0, d]
}

/// Parallel-axis theorem: shift inertia by offset from center of mass.
pub fn parallel_axis(inertia: Mat3, mass: f64, offset: V3) -> Mat3 {
    let dx = offset[0]; let dy = offset[1]; let dz = offset[2];
    let dx2 = dx*dx; let dy2 = dy*dy; let dz2 = dz*dz;
    [
        inertia[0]+mass*(dy2+dz2), inertia[1]-mass*dx*dy, inertia[2]-mass*dx*dz,
        inertia[3]-mass*dx*dy, inertia[4]+mass*(dx2+dz2), inertia[5]-mass*dy*dz,
        inertia[6]-mass*dx*dz, inertia[7]-mass*dy*dz, inertia[8]+mass*(dx2+dy2),
    ]
}

/// Recompute inertia tensor and inverse from a precomputed unit inertia + mass.
/// From PPC setMass__Q25Havok9RigidBodyFf (0x4c930):
///   mass      → 0xB8
///   inverseMass = 1/mass → 0xBC
///   inertia   = unit_inertia * mass → 0xC0..0xEC
///   inverseInertia = inertia^-1     → 0xF0..0x11C
/// The unit inertia tensor lives on the body at 0x128 and is populated
/// at body-creation time by InertialTensorComputer (see compute_polyhedron_unit_inertia).
pub fn recompute_body_inertia(
    mass: f64,
    unit_inertia: [f64; 9],
    inertia: &mut [f64; 9],
    inverse_inertia: &mut [f64; 9],
    inverse_mass: &mut f64,
) {
    if mass <= 0.0 {
        *inverse_mass = 0.0;
        *inertia = MAT3_ZERO;
        *inverse_inertia = MAT3_ZERO;
        return;
    }
    *inverse_mass = 1.0 / mass;
    *inertia = mat3_scale_f(unit_inertia, mass);
    *inverse_inertia = mat3_inverse(*inertia);
}

/// Build a default isotropic unit inertia tensor from an AABB half-extents,
/// used as a fallback when no mesh is available.
/// Matches the box formula: I_xx = (dy²+dz²)/12, I_yy = (dx²+dz²)/12, I_zz = (dx²+dy²)/12.
pub fn box_unit_inertia(half_extents: [f64; 3]) -> [f64; 9] {
    let dx = 2.0 * half_extents[0];
    let dy = 2.0 * half_extents[1];
    let dz = 2.0 * half_extents[2];
    let f = 1.0 / 12.0;
    [
        f*(dy*dy + dz*dz), 0.0, 0.0,
        0.0, f*(dx*dx + dz*dz), 0.0,
        0.0, 0.0, f*(dx*dx + dy*dy),
    ]
}

/// Compute unit inertia tensor, center-of-mass, and volume from a closed
/// triangle mesh via Mirtich's divergence-theorem moment integration.
///
/// Ports Havok's InertialTensorComputer from the PPC disassembly:
///   - computeInertialTensorM (0x5d3c0): primitive → polyhedron → moments → tensor
///   - compVolumeIntegrals    (0x5d6f0): accumulates signed volume + 1st/2nd moments
///   - compFaceIntegrals      (0x5da30): per-face Green's-theorem polygon integration
///   - computeInertialTensor  (0x5d500): converts moments → symmetric inertia tensor
///
/// The returned tensor is mass-independent: `inertia = unit_inertia * mass` at
/// `setMass` time (PPC 0x4c930). Matches the reference algorithm in
/// `HavokReference/InertiaComputation.cs::PolyhedronInertia`.
///
/// Handles reversed winding by using `|vol|` and propagating sign into the moments.
/// Returns `None` for degenerate/empty/zero-volume meshes.
pub fn compute_polyhedron_unit_inertia(
    positions: &[[f64; 3]],
    faces: &[[u32; 3]],
) -> Option<([f64; 9], [f64; 3], f64)> {
    if positions.len() < 3 || faces.is_empty() {
        return None;
    }

    // Pre-center on AABB for numerical stability — keeps the final
    // parallel-axis cancellation in single-digit magnitudes instead of millions.
    let mut mn = [f64::MAX; 3];
    let mut mx = [f64::MIN; 3];
    for p in positions {
        for i in 0..3 {
            if p[i] < mn[i] { mn[i] = p[i]; }
            if p[i] > mx[i] { mx[i] = p[i]; }
        }
    }
    let offset = [
        0.5 * (mn[0] + mx[0]),
        0.5 * (mn[1] + mx[1]),
        0.5 * (mn[2] + mx[2]),
    ];

    // Tetrahedron decomposition to origin. For each triangle (a, b, c), form the
    // signed tetrahedron (origin, a, b, c). Signed volumes and moment integrals
    // are summed across the whole mesh; because neighbouring faces' tets share
    // origin edges, the contributions telescope to give the exact volume integral
    // over the enclosed solid.
    //
    // Per-tet formulas (Lien & Kajiya 1984 / Tonon 2004, with v0=origin):
    //   V_tet            = (1/6) * a · (b × c)
    //   ∫x² dV over tet  = (V_tet / 10) * (a_x² + b_x² + c_x² + a_x*b_x + a_x*c_x + b_x*c_x)
    //   ∫xy dV over tet  = (V_tet / 20) *
    //         (2*(a_x*a_y + b_x*b_y + c_x*c_y)
    //          + a_x*b_y + b_x*a_y + a_x*c_y + c_x*a_y + b_x*c_y + c_x*b_y)
    //
    // NOTE: an earlier attempt mixed Mirtich's surface-integral scaling
    // (cross-product area weights with a degree-3 polynomial) with Tonon's
    // degree-2 polynomial, producing zero for axis-aligned boxes because
    // cross[i] is zero on faces perpendicular to axis i. The C# reference
    // `InertiaComputation.PolyhedronInertia` has the same latent bug — it is
    // never exercised by `Program.cs`, which hardcodes `UnitInertiaTensor`.
    let mut vol = 0.0f64;
    let mut fx = 0.0f64; let mut fy = 0.0f64; let mut fz = 0.0f64;
    let mut sxx = 0.0f64; let mut syy = 0.0f64; let mut szz = 0.0f64;
    let mut sxy = 0.0f64; let mut sxz = 0.0f64; let mut syz = 0.0f64;

    for face in faces {
        let i0 = face[0] as usize;
        let i1 = face[1] as usize;
        let i2 = face[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let a = [positions[i0][0]-offset[0], positions[i0][1]-offset[1], positions[i0][2]-offset[2]];
        let b = [positions[i1][0]-offset[0], positions[i1][1]-offset[1], positions[i1][2]-offset[2]];
        let c = [positions[i2][0]-offset[0], positions[i2][1]-offset[1], positions[i2][2]-offset[2]];

        // Signed tetrahedron volume: (1/6) * a · (b × c)
        let v_tet = (
            a[0] * (b[1]*c[2] - b[2]*c[1])
          + a[1] * (b[2]*c[0] - b[0]*c[2])
          + a[2] * (b[0]*c[1] - b[1]*c[0])
        ) / 6.0;
        vol += v_tet;

        // First moment contribution: V_tet * centroid, with origin vertex contributing 0.
        let cx4 = (a[0] + b[0] + c[0]) * 0.25;
        let cy4 = (a[1] + b[1] + c[1]) * 0.25;
        let cz4 = (a[2] + b[2] + c[2]) * 0.25;
        fx += v_tet * cx4;
        fy += v_tet * cy4;
        fz += v_tet * cz4;

        let ax=a[0]; let ay=a[1]; let az=a[2];
        let bx=b[0]; let by=b[1]; let bz=b[2];
        let cx=c[0]; let cy=c[1]; let cz=c[2];

        // Second moments — diagonal (∫x² dV, etc.), coefficient V_tet / 10.
        let xx_p = ax*ax + bx*bx + cx*cx + ax*bx + ax*cx + bx*cx;
        let yy_p = ay*ay + by*by + cy*cy + ay*by + ay*cy + by*cy;
        let zz_p = az*az + bz*bz + cz*cz + az*bz + az*cz + bz*cz;
        sxx += v_tet * xx_p * 0.1;
        syy += v_tet * yy_p * 0.1;
        szz += v_tet * zz_p * 0.1;

        // Products of inertia — ∫xy dV, etc., coefficient V_tet / 20.
        let xy_p = 2.0*(ax*ay + bx*by + cx*cy)
                 + ax*by + bx*ay + ax*cy + cx*ay + bx*cy + cx*by;
        let xz_p = 2.0*(ax*az + bx*bz + cx*cz)
                 + ax*bz + bx*az + ax*cz + cx*az + bx*cz + cx*bz;
        let yz_p = 2.0*(ay*az + by*bz + cy*cz)
                 + ay*bz + by*az + ay*cz + cy*az + by*cz + cy*bz;
        sxy += v_tet * xy_p * 0.05;
        sxz += v_tet * xz_p * 0.05;
        syz += v_tet * yz_p * 0.05;
    }

    let vol_abs = vol.abs();
    if vol_abs < 1e-10 {
        return None;
    }
    // If the mesh winding is reversed (vol<0), flip every accumulated moment too
    // so the signs stay consistent. This also lets us keep vol positive below.
    let sign = if vol > 0.0 { 1.0 } else { -1.0 };
    let vol = vol_abs;
    let (fx, fy, fz) = (fx*sign, fy*sign, fz*sign);
    let (sxx, syy, szz) = (sxx*sign, syy*sign, szz*sign);
    let (sxy, sxz, syz) = (sxy*sign, sxz*sign, syz*sign);

    // COM in the centered frame (small, so parallel-axis is numerically stable).
    let cm_x = fx / vol;
    let cm_y = fy / vol;
    let cm_z = fz / vol;

    // Per-unit-mass inertia tensor about the centered origin. For density = 1/vol:
    //   I_xx = ∫(y²+z²) dV / M = (syy + szz) / vol
    //   I_xy = -∫xy dV / M     = -sxy / vol
    let inv_vol = 1.0 / vol;
    let mut ixx = (syy + szz) * inv_vol;
    let mut iyy = (sxx + szz) * inv_vol;
    let mut izz = (sxx + syy) * inv_vol;
    let mut ixy = -sxy * inv_vol;
    let mut ixz = -sxz * inv_vol;
    let mut iyz = -syz * inv_vol;

    // Parallel-axis shift from the centered origin to the mesh COM.
    ixx -= cm_y*cm_y + cm_z*cm_z;
    iyy -= cm_x*cm_x + cm_z*cm_z;
    izz -= cm_x*cm_x + cm_y*cm_y;
    ixy += cm_x * cm_y;
    ixz += cm_x * cm_z;
    iyz += cm_y * cm_z;

    // Return COM back in the caller's coordinate frame.
    let com = [cm_x + offset[0], cm_y + offset[1], cm_z + offset[2]];

    let unit_inertia = [
        ixx, ixy, ixz,
        ixy, iyy, iyz,
        ixz, iyz, izz,
    ];
    Some((unit_inertia, com, vol))
}

// ============================================================
// Collision mesh
// ============================================================
pub struct CollisionMesh {
    pub name: String,
    pub vertices: Vec<V3>,
    pub triangles: Vec<[u32; 3]>,
    pub aabb_min: V3,
    pub aabb_max: V3,
    /// Index of the rigid body that owns this mesh (for friction/restitution).
    /// None = anonymous static scenery (uses default ground material).
    pub body_index: Option<usize>,
}

impl CollisionMesh {
    pub fn compute_aabb(&mut self) {
        let (mut mn, mut mx) = ([f64::MAX;3], [f64::MIN;3]);
        for v in &self.vertices {
            for i in 0..3 { mn[i] = mn[i].min(v[i]); mx[i] = mx[i].max(v[i]); }
        }
        self.aabb_min = mn; self.aabb_max = mx;
    }
}

// ============================================================
// Collision contact
// From C# Collision.cs — CollisionDetails
// ============================================================

#[derive(Clone)]
pub struct CollisionContact {
    /// Index of dynamic body A in rigid_bodies.
    pub body_a: usize,
    /// Index of body B (None = static mesh / ground).
    pub body_b: Option<usize>,
    /// World-space contact point.
    pub point: V3,
    /// Contact normal (pointing from B toward A).
    pub normal: V3,
    /// Penetration depth (positive = interpenetrating).
    pub depth: f64,
    /// Index into `state.collision_meshes` of the static mesh this contact hit
    /// (None for body-vs-body contacts). Lets the collision callback report the
    /// real surface model name (e.g. "GraficaStradaZona01") instead of a generic
    /// fallback when `body_b` is None.
    pub mesh_index: Option<usize>,
    /// Closing speed (|normal relative velocity|) captured PRE-resolution. The
    /// Havok Xtra passes this to collision callbacks as the 5th argument (the
    /// impact speed used for damage/sound) and gates the callback on the
    /// registerInterest threshold.
    pub normal_rel_vel: f64,
}

// ============================================================
// Collision detection
// ============================================================

/// Find ground Z under position by testing mesh triangles (ray down).
pub fn find_ground_z(meshes: &[CollisionMesh], x: f64, y: f64, max_z: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for mesh in meshes {
        if x < mesh.aabb_min[0] || x > mesh.aabb_max[0] { continue; }
        if y < mesh.aabb_min[1] || y > mesh.aabb_max[1] { continue; }
        for tri in &mesh.triangles {
            let v0 = mesh.vertices[tri[0] as usize];
            let v1 = mesh.vertices[tri[1] as usize];
            let v2 = mesh.vertices[tri[2] as usize];
            if !pt_in_tri_xy(x, y, v0, v1, v2) { continue; }
            let z = interp_z(x, y, v0, v1, v2);
            if z < max_z { best = Some(best.map_or(z, |b: f64| b.max(z))); }
        }
    }
    best
}

/// World-space velocity of a point rigidly attached to `rb`:
/// `v = v_linear + omega × (point - position)`. Used to capture the true impact
/// (closing) speed at a contact point for the collision callback's 5th argument.
#[inline]
pub fn body_point_velocity(rb: &crate::player::cast_member::HavokRigidBody, point: V3) -> V3 {
    let r = [point[0] - rb.position[0], point[1] - rb.position[1], point[2] - rb.position[2]];
    let w = rb.angular_velocity;
    [
        rb.linear_velocity[0] + w[1] * r[2] - w[2] * r[1],
        rb.linear_velocity[1] + w[2] * r[0] - w[0] * r[2],
        rb.linear_velocity[2] + w[0] * r[1] - w[1] * r[0],
    ]
}

/// Detect contacts for a body (sphere approximation) against static meshes.
/// Returns contacts with normals pointing AWAY from the triangle surface.
/// `tolerance` expands the effective collision distance so Havok detects
/// contacts before actual surface penetration (matching the Xtra behaviour).
pub fn detect_body_contacts(
    meshes: &[CollisionMesh], pos: V3, half_extents: V3, body_idx: usize, tolerance: f64, passive: bool,
    bodies: &[crate::player::cast_member::HavokRigidBody],
) -> Vec<CollisionContact> {
    // Broad-phase cull uses the largest half-extent. NON-passive scenes (the
    // tuned SuperSonic car) keep the original sphere-radius narrow phase with no
    // margin, since that's what their handling was calibrated against. Passive
    // scenes use the box-support narrow phase + contact margin so resting boxes
    // sit flush and a stack stays coupled.
    let max_r = half_extents[0].max(half_extents[1]).max(half_extents[2]) + tolerance;
    let margin = if passive { CONTACT_MARGIN } else { 0.0 };
    let mut contacts = Vec::new();
    for (mesh_idx, mesh) in meshes.iter().enumerate() {
        // Skip meshes owned by a MOVABLE body: those are baked at the body's
        // initial position and go stale once it moves (bogus deep collisions;
        // dynamic bodies interact via box-vs-box instead). A FIXED body's mesh
        // never moves, so it stays a valid static collider — and its body_index
        // lets the resolver use that surface's restitution/friction (the
        // Properties demo's bouncy/slippery floors).
        if let Some(owner) = mesh.body_index {
            if !bodies[owner].pinned { continue; }
        }
        if pos[0]+max_r < mesh.aabb_min[0] || pos[0]-max_r > mesh.aabb_max[0] { continue; }
        if pos[1]+max_r < mesh.aabb_min[1] || pos[1]-max_r > mesh.aabb_max[1] { continue; }
        if pos[2]+max_r < mesh.aabb_min[2] || pos[2]-max_r > mesh.aabb_max[2] { continue; }
        for tri in &mesh.triangles {
            let v0 = mesh.vertices[tri[0] as usize];
            let v1 = mesh.vertices[tri[1] as usize];
            let v2 = mesh.vertices[tri[2] as usize];
            let normal = v3_normalized(v3_cross(v3_sub(v1, v0), v3_sub(v2, v0)));
            if v3_len_sq(normal) < 1e-10 { continue; }
            // Passive: exact box support along the triangle normal. Non-passive:
            // original isotropic sphere radius.
            let eff_radius = if passive {
                half_extents[0]*normal[0].abs()
              + half_extents[1]*normal[1].abs()
              + half_extents[2]*normal[2].abs()
              + tolerance
            } else {
                max_r
            };
            let dist = v3_dot(v3_sub(pos, v0), normal);
            if dist.abs() > eff_radius { continue; }
            if dist < -eff_radius * 2.0 { continue; }
            let proj = v3_sub(pos, v3_scale(normal, dist));
            if pt_in_tri_3d(proj, v0, v1, v2) {
                let depth = eff_radius - dist;
                if depth > -margin {
                    // Impact speed at the contact point, captured before the
                    // resolver cancels the normal velocity. The mesh's owner (if
                    // any) is pinned here, so body_b's velocity is zero.
                    let va = body_point_velocity(&bodies[body_idx], proj);
                    let nrv = (va[0]*normal[0] + va[1]*normal[1] + va[2]*normal[2]).abs();
                    contacts.push(CollisionContact {
                        body_a: body_idx,
                        body_b: mesh.body_index,
                        point: proj,
                        normal,
                        depth,
                        mesh_index: Some(mesh_idx),
                        normal_rel_vel: nrv,
                    });
                }
            } else if passive && normal[2].abs() > 0.85 {
                // Edge/vertex contact on a near-horizontal FLOOR triangle: the body's
                // footprint still overlaps the triangle near an edge even though its
                // CENTRE projects outside the face. This is the seam between adjacent
                // road box colliders — On the Run paves the road with separate boxes,
                // and a car whose centre sits over the gap/edge between two of them
                // finds no face underneath and falls through. The face-only
                // `pt_in_tri_3d` test (a simplification of the C# reference's full
                // triangle-triangle closest-point) misses it.
                //
                // Restricted to up-facing floor triangles (|n.z|>0.85, Havok is
                // Z-up): applying it to ramp/wall faces turned their base edges into a
                // back-stop that blocked the car from climbing small 45° ramps and
                // whipped it out of the world. Sloped/vertical faces keep the
                // face-only test. Passive-only so SuperSonic's tuned narrow phase is
                // unchanged.
                let closest = closest_pt_on_tri(pos, v0, v1, v2);
                let to_pos = v3_sub(pos, closest);
                let d = v3_len(to_pos);
                if d > 1e-6 && d <= eff_radius {
                    let depth = eff_radius - d;
                    // Keep the surface normal (oriented toward the body) so a flat
                    // seam pushes the car straight up, not sideways toward the edge.
                    let n = if v3_dot(to_pos, normal) >= 0.0 { normal } else { v3_scale(normal, -1.0) };
                    if depth > -margin && n[2] > 0.85 {
                        let va = body_point_velocity(&bodies[body_idx], closest);
                        let nrv = (va[0]*n[0] + va[1]*n[1] + va[2]*n[2]).abs();
                        contacts.push(CollisionContact {
                            body_a: body_idx,
                            body_b: mesh.body_index,
                            point: closest,
                            normal: n,
                            depth,
                            mesh_index: Some(mesh_idx),
                            normal_rel_vel: nrv,
                        });
                    }
                }
            }
        }
    }
    contacts
}

/// Segment-segment closest points (for triangle-triangle distance).
/// From C# CollisionDetection.cs — SegmentSegmentClosest
fn segment_segment_closest(p0: V3, p1: V3, q0: V3, q1: V3) -> (V3, V3) {
    let d1 = v3_sub(p1, p0);
    let d2 = v3_sub(q1, q0);
    let r = v3_sub(p0, q0);
    let a = v3_dot(d1, d1);
    let e = v3_dot(d2, d2);
    let f = v3_dot(d2, r);
    let (s, t);
    if a <= 1e-10 && e <= 1e-10 {
        s = 0.0; t = 0.0;
    } else if a <= 1e-10 {
        s = 0.0; t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = v3_dot(d1, r);
        if e <= 1e-10 {
            t = 0.0; s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = v3_dot(d1, d2);
            let denom = a * e - b * b;
            let mut ss = if denom != 0.0 { ((b*f - c*e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut tt = (b * ss + f) / e;
            if tt < 0.0 { tt = 0.0; ss = (-c / a).clamp(0.0, 1.0); }
            else if tt > 1.0 { tt = 1.0; ss = ((b - c) / a).clamp(0.0, 1.0); }
            s = ss; t = tt;
        }
    }
    (v3_add(p0, v3_scale(d1, s)), v3_add(q0, v3_scale(d2, t)))
}

/// Triangle-triangle closest points.
/// From C# CollisionDetection.cs — TriangleTriangleDistance
/// Tests 9 edge-edge pairs + 2 face projections.
pub fn triangle_triangle_closest(
    a0: V3, a1: V3, a2: V3,
    b0: V3, b1: V3, b2: V3,
) -> (V3, V3, f64) {
    let mut best_sq = f64::MAX;
    let mut closest_a = a0;
    let mut closest_b = b0;
    let a_verts = [a0, a1, a2];
    let b_verts = [b0, b1, b2];
    // 9 edge-edge
    for i in 0..3 {
        let i1 = (i + 1) % 3;
        for j in 0..3 {
            let j1 = (j + 1) % 3;
            let (pa, pb) = segment_segment_closest(a_verts[i], a_verts[i1], b_verts[j], b_verts[j1]);
            let d_sq = v3_len_sq(v3_sub(pa, pb));
            if d_sq < best_sq { best_sq = d_sq; closest_a = pa; closest_b = pb; }
        }
    }
    // Face A → project B verts
    let na = v3_cross(v3_sub(a1, a0), v3_sub(a2, a0));
    let area_sq_a = v3_len_sq(na);
    if area_sq_a > 1e-10 {
        for &bv in &b_verts {
            let d = v3_dot(v3_sub(bv, a0), na);
            let proj = v3_sub(bv, v3_scale(na, d / area_sq_a));
            if pt_in_tri_3d(proj, a0, a1, a2) {
                let d_sq = v3_len_sq(v3_sub(proj, bv));
                if d_sq < best_sq { best_sq = d_sq; closest_a = proj; closest_b = bv; }
            }
        }
    }
    // Face B → project A verts
    let nb = v3_cross(v3_sub(b1, b0), v3_sub(b2, b0));
    let area_sq_b = v3_len_sq(nb);
    if area_sq_b > 1e-10 {
        for &av in &a_verts {
            let d = v3_dot(v3_sub(av, b0), nb);
            let proj = v3_sub(av, v3_scale(nb, d / area_sq_b));
            if pt_in_tri_3d(proj, b0, b1, b2) {
                let d_sq = v3_len_sq(v3_sub(av, proj));
                if d_sq < best_sq { best_sq = d_sq; closest_a = av; closest_b = proj; }
            }
        }
    }
    (closest_a, closest_b, best_sq)
}

// ============================================================
// Triangle geometry helpers
// ============================================================

fn pt_in_tri_xy(px: f64, py: f64, v0: V3, v1: V3, v2: V3) -> bool {
    let d00 = (v1[0]-v0[0])*(v1[0]-v0[0]) + (v1[1]-v0[1])*(v1[1]-v0[1]);
    let d01 = (v1[0]-v0[0])*(v2[0]-v0[0]) + (v1[1]-v0[1])*(v2[1]-v0[1]);
    let d02 = (v1[0]-v0[0])*(px-v0[0]) + (v1[1]-v0[1])*(py-v0[1]);
    let d11 = (v2[0]-v0[0])*(v2[0]-v0[0]) + (v2[1]-v0[1])*(v2[1]-v0[1]);
    let d12 = (v2[0]-v0[0])*(px-v0[0]) + (v2[1]-v0[1])*(py-v0[1]);
    let denom = d00*d11 - d01*d01;
    if denom.abs() < 1e-12 { return false; }
    let u = (d11*d02 - d01*d12) / denom;
    let v = (d00*d12 - d01*d02) / denom;
    u >= -1e-6 && v >= -1e-6 && (u + v) <= 1.0 + 1e-6
}

fn pt_in_tri_3d(p: V3, v0: V3, v1: V3, v2: V3) -> bool {
    let e0 = v3_sub(v1, v0); let e1 = v3_sub(v2, v0); let e2 = v3_sub(p, v0);
    let d00 = v3_dot(e0, e0); let d01 = v3_dot(e0, e1); let d02 = v3_dot(e0, e2);
    let d11 = v3_dot(e1, e1); let d12 = v3_dot(e1, e2);
    let denom = d00*d11 - d01*d01;
    if denom.abs() < 1e-12 { return false; }
    let u = (d11*d02 - d01*d12) / denom;
    let v = (d00*d12 - d01*d02) / denom;
    u >= -0.01 && v >= -0.01 && (u + v) <= 1.01
}

/// Closest point on triangle (a,b,c) to point p — Ericson, Real-Time Collision
/// Detection (handles the face, the three edges, and the three vertex regions).
/// Used by the box-support narrow phase to rest a body on a triangle edge when
/// its centre projects just outside the face (road box seams in On the Run).
fn closest_pt_on_tri(p: V3, a: V3, b: V3, c: V3) -> V3 {
    let ab = v3_sub(b, a);
    let ac = v3_sub(c, a);
    let ap = v3_sub(p, a);
    let d1 = v3_dot(ab, ap);
    let d2 = v3_dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 { return a; }            // vertex region A
    let bp = v3_sub(p, b);
    let d3 = v3_dot(ab, bp);
    let d4 = v3_dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 { return b; }             // vertex region B
    let vc = d1*d4 - d3*d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {           // edge region AB
        let v = d1 / (d1 - d3);
        return v3_add(a, v3_scale(ab, v));
    }
    let cp = v3_sub(p, c);
    let d5 = v3_dot(ab, cp);
    let d6 = v3_dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 { return c; }             // vertex region C
    let vb = d5*d2 - d1*d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {           // edge region AC
        let w = d2 / (d2 - d6);
        return v3_add(a, v3_scale(ac, w));
    }
    let va = d3*d6 - d5*d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {   // edge region BC
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return v3_add(b, v3_scale(v3_sub(c, b), w));
    }
    // Inside face region — barycentric combination.
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    v3_add(a, v3_add(v3_scale(ab, v), v3_scale(ac, w)))
}

fn interp_z(px: f64, py: f64, v0: V3, v1: V3, v2: V3) -> f64 {
    let d00 = (v1[0]-v0[0])*(v1[0]-v0[0]) + (v1[1]-v0[1])*(v1[1]-v0[1]);
    let d01 = (v1[0]-v0[0])*(v2[0]-v0[0]) + (v1[1]-v0[1])*(v2[1]-v0[1]);
    let d02 = (v1[0]-v0[0])*(px-v0[0]) + (v1[1]-v0[1])*(py-v0[1]);
    let d11 = (v2[0]-v0[0])*(v2[0]-v0[0]) + (v2[1]-v0[1])*(v2[1]-v0[1]);
    let d12 = (v2[0]-v0[0])*(px-v0[0]) + (v2[1]-v0[1])*(py-v0[1]);
    let denom = d00*d11 - d01*d01;
    if denom.abs() < 1e-12 { return v0[2]; }
    let u = (d11*d02 - d01*d12) / denom;
    let v = (d00*d12 - d01*d02) / denom;
    (1.0 - u - v) * v0[2] + u * v1[2] + v * v2[2]
}

// ============================================================
// Collision resolution: PointRRResolver (400-iteration impulse solver)
// From C# Collision.cs
// ============================================================

const MAX_RESOLVER_ITERATIONS: usize = 400;
/// Contact margin (Director units): generate contacts slightly before bodies
/// actually overlap so a resting stack stays *coupled* (the resolver can carry
/// friction + impulses between touching boxes and the floor) instead of each
/// box flickering in and out of contact. Near-contacts (depth <= 0) only affect
/// velocity — depenetration still requires real penetration.
const CONTACT_MARGIN: f64 = 2.0;
/// Default ground material: friction=0.5, restitution=0.3
const GROUND_FRICTION: f64 = 0.5;
const GROUND_RESTITUTION: f64 = 0.3;
/// Below this closing speed (Director units/s) a contact is treated as "resting":
/// restitution is suppressed and the penetration-correction *driving impulse* is
/// skipped, so a body sitting on a surface doesn't get pumped into a bounce.
const REST_VEL_THRESHOLD: f64 = 10.0;

/// Resolve collision contacts using iterative impulses.
/// From PPC: resolveWithImpulses (0x5AAB0)
fn resolve_contacts(state: &mut HavokPhysicsState, contacts: &[CollisionContact]) {
    if contacts.is_empty() { return; }

    let tolerance = state.tolerance;

    for _iteration in 0..MAX_RESOLVER_ITERATIONS {
        let mut all_converged = true;

        for contact in contacts {
            // Compute impulse
            let impulse = resolve_single_contact(state, contact);

            // Apply impulse pair
            apply_impulse_pair(state, impulse, contact);

            // Apply driving impulse (penetration correction) only for
            // anonymous terrain meshes.  For rigid-body-owned meshes the
            // position correction after the resolver handles depenetration;
            // applying driving impulse here injects energy into low-velocity
            // bounces and causes runaway velocity growth.
            //
            // Also skip it for low-velocity (resting) terrain contacts: the
            // post-resolver position correction already depenetrates them, and
            // the driving impulse — which is strongest for the shallow contacts a
            // resting body produces every substep — otherwise pumps energy in and
            // makes a body sitting on the ground bounce (On the Run's car on the
            // road). Genuine high-speed penetrations still get it.
            if contact.body_b.is_none() && contact.normal_rel_vel >= REST_VEL_THRESHOLD {
                apply_driving_impulse(state, contact, tolerance);
            }

            // Check convergence
            let vn = compute_normal_rel_velocity(state, contact);
            if vn < 0.0 { all_converged = false; }
        }

        if all_converged { break; }
    }
}

/// Compute impulse for a single contact.
/// From PPC: resolveSingleContact (ComplexFrictionRRResolver, 0x58C30)
fn resolve_single_contact(state: &HavokPhysicsState, contact: &CollisionContact) -> V3 {
    let rel_vel = compute_rel_velocity(state, contact);
    let vn = v3_dot(rel_vel, contact.normal);
    if vn >= 0.0 { return [0.0; 3]; }

    let rb_a = &state.rigid_bodies[contact.body_a];
    let (friction, restitution) = if let Some(b_idx) = contact.body_b {
        let rb_b = &state.rigid_bodies[b_idx];
        // Product restitution and friction (matches Director's observed behaviour
        // for the Properties demo: Ball2 bounce ratio ~0.70 with rest=0.75 * 1.0)
        ((rb_a.friction * rb_b.friction), (rb_a.restitution * rb_b.restitution))
    } else {
        ((rb_a.friction * GROUND_FRICTION), (rb_a.restitution * GROUND_RESTITUTION))
    };

    // Havok's bisection collision resolver loses a small amount of kinetic
    // energy each bounce due to numerical integration.  Apply a 0.93× damping
    // factor to match Director's observed bounce ratios (Properties demo Ball2:
    // Director ratio ≈ 0.70, product restitution = 0.75, 0.75 × 0.93 ≈ 0.70).
    const COLLISION_ENERGY_LOSS: f64 = 0.93;
    let restitution = restitution * COLLISION_ENERGY_LOSS;

    // Suppress restitution for low-speed contacts to prevent jitter at rest.
    let is_resting = vn.abs() < REST_VEL_THRESHOLD;
    let eff_restitution = if is_resting { 0.0 } else { restitution };
    let target_vn = -eff_restitution * vn;
    let eff_inv_mass = compute_effective_inverse_mass(state, contact);
    if eff_inv_mass < 1e-10 { return [0.0; 3]; }

    let normal_impulse_mag = (target_vn - vn) / eff_inv_mass;
    let mut impulse = v3_scale(contact.normal, normal_impulse_mag);

    // Friction (Coulomb cone). For SuperSonic-style driven scenes we skip it on
    // resting contacts so the car can slide on tilted surfaces under gravity.
    // For passive scenes we MUST apply it on resting contacts too, otherwise a
    // crate that slides flat across the floor never slows down or stops. A
    // driven body (the car) keeps the original skip-on-resting behaviour.
    let passive = state.springs.is_empty()
        && state.linear_dashpots.is_empty()
        && state.angular_dashpots.is_empty()
        && !state.rigid_bodies[contact.body_a].driven;
    if friction > 0.0 && (!is_resting || passive) {
        let tangent_vel = v3_sub(rel_vel, v3_scale(contact.normal, vn));
        let tangent_speed = v3_len(tangent_vel);
        if tangent_speed > 1e-6 {
            let tangent_dir = v3_scale(tangent_vel, 1.0 / tangent_speed);
            let max_friction = friction * normal_impulse_mag.abs();
            let friction_mag = (tangent_speed / eff_inv_mass).min(max_friction);
            impulse = v3_sub(impulse, v3_scale(tangent_dir, friction_mag));
        }
    }
    impulse
}

/// Apply impulse to both bodies. Body A gets +impulse, Body B gets -impulse.
/// From PPC: applyImpulsePair (0x5B540)
fn apply_impulse_pair(state: &mut HavokPhysicsState, impulse: V3, contact: &CollisionContact) {
    if v3_len_sq(impulse) < 1e-20 { return; }

    // Body A
    {
        let rb = &mut state.rigid_bodies[contact.body_a];
        if !rb.pinned && rb.inverse_mass > 0.0 {
            rb.linear_velocity = v3_add(rb.linear_velocity, v3_scale(impulse, rb.inverse_mass));
            let r = v3_sub(contact.point, rb.position);
            let t = v3_cross(r, impulse);
            let ang_impulse = mat3_transform(world_inv_inertia(rb), t);
            rb.angular_velocity = v3_add(rb.angular_velocity, ang_impulse);
        }
    }
    // Body B (if dynamic)
    if let Some(b_idx) = contact.body_b {
        let rb = &mut state.rigid_bodies[b_idx];
        if !rb.pinned && rb.inverse_mass > 0.0 {
            let neg = v3_neg(impulse);
            rb.linear_velocity = v3_add(rb.linear_velocity, v3_scale(neg, rb.inverse_mass));
            let r = v3_sub(contact.point, rb.position);
            let t = v3_cross(r, neg);
            let ang_impulse = mat3_transform(world_inv_inertia(rb), t);
            rb.angular_velocity = v3_add(rb.angular_velocity, ang_impulse);
        }
    }
}

/// Apply penetration-correction driving impulse.
/// From PPC: applyDrivingImpulse (0x5AD90)
fn apply_driving_impulse(state: &mut HavokPhysicsState, contact: &CollisionContact, driving_scale: f64) {
    if driving_scale <= 0.0 { return; }
    let scaled_factor = 2.0 * driving_scale;
    let t = (scaled_factor - contact.depth) / scaled_factor;
    if t <= 0.0 || t >= 1.0 { return; }
    let correction = driving_scale / 4.0;
    let vn = compute_normal_rel_velocity(state, contact);
    if vn <= 0.0 { return; }
    let driving_mag = t * t * correction;
    let driving_impulse = v3_scale(contact.normal, driving_mag);

    // Body A
    {
        let rb = &mut state.rigid_bodies[contact.body_a];
        if !rb.pinned && rb.inverse_mass > 0.0 {
            let scaled = v3_scale(driving_impulse, rb.mass);
            rb.linear_velocity = v3_add(rb.linear_velocity, v3_scale(scaled, rb.inverse_mass));
            let r = v3_sub(contact.point, rb.position);
            rb.angular_velocity = v3_add(rb.angular_velocity, mat3_transform(world_inv_inertia(rb), v3_cross(r, scaled)));
        }
    }
    // Body B
    if let Some(b_idx) = contact.body_b {
        let rb = &mut state.rigid_bodies[b_idx];
        if !rb.pinned && rb.inverse_mass > 0.0 {
            let neg_scaled = v3_neg(v3_scale(driving_impulse, rb.mass));
            rb.linear_velocity = v3_add(rb.linear_velocity, v3_scale(neg_scaled, rb.inverse_mass));
            let r = v3_sub(contact.point, rb.position);
            rb.angular_velocity = v3_add(rb.angular_velocity, mat3_transform(world_inv_inertia(rb), v3_cross(r, neg_scaled)));
        }
    }
}

/// Compute relative velocity at contact point (velA - velB).
fn compute_rel_velocity(state: &HavokPhysicsState, contact: &CollisionContact) -> V3 {
    let vel_a = if !state.rigid_bodies[contact.body_a].pinned {
        get_point_velocity(&state.rigid_bodies[contact.body_a], contact.point)
    } else { [0.0; 3] };
    let vel_b = if let Some(b_idx) = contact.body_b {
        if !state.rigid_bodies[b_idx].pinned {
            get_point_velocity(&state.rigid_bodies[b_idx], contact.point)
        } else { [0.0; 3] }
    } else { [0.0; 3] };
    v3_sub(vel_a, vel_b)
}

fn compute_normal_rel_velocity(state: &HavokPhysicsState, contact: &CollisionContact) -> f64 {
    v3_dot(compute_rel_velocity(state, contact), contact.normal)
}

/// Effective inverse mass at contact along normal.
/// From C# Collision.cs — ComputeEffectiveInverseMass
fn compute_effective_inverse_mass(state: &HavokPhysicsState, contact: &CollisionContact) -> f64 {
    let n = contact.normal;
    let mut result = 0.0;
    // Body A
    let rb_a = &state.rigid_bodies[contact.body_a];
    if !rb_a.pinned {
        result += rb_a.inverse_mass;
        let r = v3_sub(contact.point, rb_a.position);
        let rxn = v3_cross(r, n);
        let ang_contrib = v3_cross(mat3_transform(world_inv_inertia(rb_a), rxn), r);
        result += v3_dot(n, ang_contrib);
    }
    // Body B
    if let Some(b_idx) = contact.body_b {
        let rb_b = &state.rigid_bodies[b_idx];
        if !rb_b.pinned {
            result += rb_b.inverse_mass;
            let r = v3_sub(contact.point, rb_b.position);
            let rxn = v3_cross(r, n);
            let ang_contrib = v3_cross(mat3_transform(world_inv_inertia(rb_b), rxn), r);
            result += v3_dot(n, ang_contrib);
        }
    }
    result
}

/// World-frame inverse inertia: R · Iinv_body · Rᵀ.
///
/// Havok keeps TWO inertia representations and uses them in different places:
///   * the BODY-frame constant, used by the angular integration step
///     (`angVel += dt · (Iinv_BODY · torque)`, no rotation), and
///   * a WORLD matrix rebuilt from it each step, used ONLY by the contact solver.
///
/// So anything that combines the tensor with world-space quantities — contact
/// lever arms, normals, impulses, constraint torques — must use THIS one, while
/// the integrator must not. Mixing them up is frame-inconsistent and only looks
/// right while a body sits near its spawn orientation; it diverges badly once
/// the body rotates (e.g. a car going round a loop).
pub fn world_inv_inertia(rb: &crate::player::cast_member::HavokRigidBody) -> [f64; 9] {
    let r = quat_to_mat3(rb.orientation);
    mat3_mul(mat3_mul(r, rb.inverse_inertia_tensor), mat3_transpose(r))
}

/// Point velocity: v_linear + omega × (point - position)
/// From PPC: getPointVelocity (0x4cf90)
fn get_point_velocity(rb: &crate::player::cast_member::HavokRigidBody, world_point: V3) -> V3 {
    let r = v3_sub(world_point, rb.position);
    v3_add(rb.linear_velocity, v3_cross(rb.angular_velocity, r))
}

// ============================================================
// Actions: forces applied each substep
// From C# Actions.cs
// ============================================================

/// Apply drag forces to all bodies.
/// From x86: sub_10075C30 (DragAction::apply)
fn apply_drag(state: &mut HavokPhysicsState) {
    let linear_drag = state.drag_params[0];
    let angular_drag = state.drag_params[1];
    if linear_drag == 0.0 && angular_drag == 0.0 { return; }
    for rb in &mut state.rigid_bodies {
        if rb.pinned || !rb.active || rb.inverse_mass <= 0.0 { continue; }
        if linear_drag != 0.0 {
            for i in 0..3 { rb.force[i] -= linear_drag * rb.linear_velocity[i]; }
        }
        if angular_drag != 0.0 {
            for i in 0..3 { rb.torque[i] -= angular_drag * rb.angular_velocity[i]; }
        }
    }
}

/// Apply gravity to all bodies.
fn apply_gravity(state: &mut HavokPhysicsState) {
    let g = state.gravity;
    for rb in &mut state.rigid_bodies {
        if rb.pinned || !rb.active || rb.inverse_mass <= 0.0 { continue; }
        for i in 0..3 { rb.force[i] += g[i] * rb.mass; }
    }
}

/// Apply spring forces.
/// From C# Actions.cs — Spring::Apply (Hooke's law + damping)
fn apply_springs(state: &mut HavokPhysicsState, _dt: f64) {
    for si in 0..state.springs.len() {
        let spring = &state.springs[si];
        if spring.elasticity == 0.0 { continue; }
        let rb_a_name = match &spring.rigid_body_a { Some(n) => n.clone(), None => continue };
        let rb_b_name = spring.rigid_body_b.clone();
        let point_a_local = spring.point_a;
        let point_b_local = spring.point_b;
        let rest_length = spring.rest_length;
        let elasticity = spring.elasticity;
        let damping = spring.damping;
        let on_compression = spring.on_compression;
        let on_extension = spring.on_extension;

        let idx_a = match find_body_idx(state, &rb_a_name) { Some(i) => i, None => continue };
        let idx_b = rb_b_name.as_ref().and_then(|n| find_body_idx(state, n));

        // Transform points to world space
        let world_a = body_transform_point(&state.rigid_bodies[idx_a], point_a_local);
        let world_b = if let Some(ib) = idx_b {
            body_transform_point(&state.rigid_bodies[ib], point_b_local)
        } else {
            point_b_local // world space if no body B
        };

        let delta = v3_sub(world_b, world_a);
        let distance = v3_len(delta);
        if distance < 1e-10 { continue; }
        let direction = v3_scale(delta, 1.0 / distance);

        if !on_compression && distance < rest_length { continue; }
        if !on_extension && distance > rest_length { continue; }

        // Velocities at attachment points
        let vel_a = get_point_velocity(&state.rigid_bodies[idx_a], world_a);
        let vel_b = if let Some(ib) = idx_b {
            get_point_velocity(&state.rigid_bodies[ib], world_b)
        } else { [0.0; 3] };
        let rel_vel = v3_sub(vel_a, vel_b);

        // Hooke's law + damping
        let force_mag = elasticity * (distance - rest_length) - damping * v3_dot(rel_vel, direction);
        let force = v3_scale(direction, force_mag);

        // Apply to body A
        {
            let rb = &mut state.rigid_bodies[idx_a];
            if !rb.pinned {
                rb.force = v3_add(rb.force, force);
                let r = v3_sub(world_a, rb.position);
                rb.torque = v3_add(rb.torque, v3_cross(r, force));
            }
        }
        // Apply -force to body B
        if let Some(ib) = idx_b {
            let rb = &mut state.rigid_bodies[ib];
            if !rb.pinned {
                let neg = v3_neg(force);
                rb.force = v3_add(rb.force, neg);
                let r = v3_sub(world_b, rb.position);
                rb.torque = v3_add(rb.torque, v3_cross(r, neg));
            }
        }
    }
}

/// Apply linear dashpot forces.
/// From x86: sub_1001CAF0 — timeScale = dt * 151.0, direct impulse application.
fn apply_linear_dashpots(state: &mut HavokPhysicsState, dt: f64) {
    let time_scale = dt * 151.0;
    let post_damping = 0.001;
    for di in 0..state.linear_dashpots.len() {
        let dashpot = &state.linear_dashpots[di];
        if dashpot.strength == 0.0 { continue; }
        let rb_a_name = match &dashpot.rigid_body_a { Some(n) => n.clone(), None => continue };
        let rb_b_name = dashpot.rigid_body_b.clone();
        let point_a_local = dashpot.point_a;
        let point_b_local = dashpot.point_b;
        let strength = dashpot.strength;
        let damping_coeff = dashpot.damping;

        let idx_a = match find_body_idx(state, &rb_a_name) { Some(i) => i, None => continue };
        let idx_b = rb_b_name.as_ref().and_then(|n| find_body_idx(state, n));

        let world_a = body_transform_point(&state.rigid_bodies[idx_a], point_a_local);
        let (world_b, vel_b) = if let Some(ib) = idx_b {
            (body_transform_point(&state.rigid_bodies[ib], point_b_local), state.rigid_bodies[ib].linear_velocity)
        } else {
            (point_b_local, [0.0; 3])
        };

        let pos_diff = v3_scale(v3_sub(world_a, world_b), time_scale * strength);
        let vel_diff = v3_scale(v3_sub(state.rigid_bodies[idx_a].linear_velocity, vel_b), time_scale * damping_coeff);
        let force = v3_add(pos_diff, vel_diff);

        // Apply -force to body A (impulse-like, through inv mass)
        {
            let rb = &mut state.rigid_bodies[idx_a];
            if !rb.pinned && rb.inverse_mass > 0.0 {
                let neg = v3_neg(force);
                let impulse = v3_scale(neg, rb.inverse_mass);
                // ApplyImpulseAtPoint
                rb.linear_velocity = v3_add(rb.linear_velocity, v3_scale(impulse, rb.inverse_mass));
                let r = v3_sub(world_a, rb.position);
                let t = v3_cross(r, impulse);
                rb.angular_velocity = v3_add(rb.angular_velocity, mat3_transform(world_inv_inertia(rb), t));
                // Post damping: vel *= (1 - 0.001)
                let factor = 1.0 - post_damping;
                rb.linear_velocity = v3_scale(rb.linear_velocity, factor);
                rb.angular_velocity = v3_scale(rb.angular_velocity, factor);
            }
        }
        // Apply +force to body B
        if let Some(ib) = idx_b {
            let rb = &mut state.rigid_bodies[ib];
            if !rb.pinned && rb.inverse_mass > 0.0 {
                let impulse = v3_scale(force, rb.inverse_mass);
                rb.linear_velocity = v3_add(rb.linear_velocity, v3_scale(impulse, rb.inverse_mass));
                let r = v3_sub(world_b, rb.position);
                let t = v3_cross(r, impulse);
                rb.angular_velocity = v3_add(rb.angular_velocity, mat3_transform(world_inv_inertia(rb), t));
                let factor = 1.0 - post_damping;
                rb.linear_velocity = v3_scale(rb.linear_velocity, factor);
                rb.angular_velocity = v3_scale(rb.angular_velocity, factor);
            }
        }
    }
}

/// Apply angular dashpot torques.
/// From x86: sub_1001D2D0 — timeScale = dt * 200.0
fn apply_angular_dashpots(state: &mut HavokPhysicsState, dt: f64) {
    let time_scale = dt * 200.0;
    for di in 0..state.angular_dashpots.len() {
        let dashpot = &state.angular_dashpots[di];
        if dashpot.strength == 0.0 { continue; }
        let rb_a_name = match &dashpot.rigid_body_a { Some(n) => n.clone(), None => continue };
        let rb_b_name = dashpot.rigid_body_b.clone();
        let target_axis = dashpot.rotation_axis;
        let target_angle = dashpot.rotation_angle;
        let strength = dashpot.strength;
        let damping_coeff = dashpot.damping;

        let idx_a = match find_body_idx(state, &rb_a_name) { Some(i) => i, None => continue };
        let idx_b = rb_b_name.as_ref().and_then(|n| find_body_idx(state, n));

        // Target quaternion from axis-angle degrees
        let target_quat = quat_from_axis_angle_degrees(target_axis, target_angle);

        let (error_quat, ang_vel_diff);
        if let Some(ib) = idx_b {
            let desired = quat_mul(state.rigid_bodies[ib].orientation, target_quat);
            error_quat = quat_mul(quat_conjugate(desired), state.rigid_bodies[idx_a].orientation);
            ang_vel_diff = v3_sub(state.rigid_bodies[idx_a].angular_velocity, state.rigid_bodies[ib].angular_velocity);
        } else {
            error_quat = quat_mul(quat_conjugate(state.rigid_bodies[idx_a].orientation), target_quat);
            ang_vel_diff = state.rigid_bodies[idx_a].angular_velocity;
        };

        let (axis, angle) = quat_to_axis_angle(error_quat);
        let axis_angle = if angle.abs() <= 0.001 { [0.0; 3] } else { v3_scale(axis, angle) };

        let strength_contrib = v3_scale(axis_angle, time_scale * strength);
        let damp_contrib = v3_scale(ang_vel_diff, time_scale * damping_coeff);
        let total_torque = v3_add(strength_contrib, damp_contrib);

        // Apply -torque to body A
        {
            let rb = &mut state.rigid_bodies[idx_a];
            if !rb.pinned {
                let neg = v3_neg(total_torque);
                rb.angular_velocity = v3_add(rb.angular_velocity, mat3_transform(world_inv_inertia(rb), neg));
            }
        }
        // Apply +torque to body B
        if let Some(ib) = idx_b {
            let rb = &mut state.rigid_bodies[ib];
            if !rb.pinned {
                rb.angular_velocity = v3_add(rb.angular_velocity, mat3_transform(world_inv_inertia(rb), total_torque));
            }
        }
    }
}

// ============================================================
// Integration
// From C# RigidBody.cs — IntegrateEuler, IntegrateQuaternion
// ============================================================

/// Forward Euler integration for a single body.
/// From x86: sub_10014DA0 — confirmed by disassembly to do:
///   pos += lin_vel * dt
///   q_new = q + 0.5 * [0, omega] * q * dt  (normalized)
///   lin_vel += force * invMass * dt
///   omega += I_inv * torque * dt
fn integrate_body(rb: &mut crate::player::cast_member::HavokRigidBody, dt: f64) {
    if rb.pinned || !rb.active || rb.inverse_mass <= 0.0 { return; }

    // Phase 1: Position: pos += vel * dt
    for i in 0..3 { rb.position[i] += rb.linear_velocity[i] * dt; }

    // Phase 2: Quaternion integration with Baumgarte drift-correction.
    //
    // Plain `q += 0.5·ω·q·dt` + post-normalize lets |q| drift slightly during
    // integration and only pulls it back at the very end. Over many substeps
    // this can couple small rotational errors into the next substep's angular
    // velocity update via the body transform and produce slow divergence.
    //
    // Havok's convertDerivativeToArray (PPC 0x4b870, around 0x4ba84-0x4babc)
    // adds a Baumgarte-style `k·(1-|q|²)·q` correction term into the quaternion
    // derivative itself, so each substep's integration drives |q| back toward
    // unity in addition to the post-normalize. Port that here.
    //
    // The correction coefficient `k` is a free parameter; k=1 gives critical
    // correction at the substep rate, which is what Havok appears to use.
    let ox = rb.angular_velocity[0];
    let oy = rb.angular_velocity[1];
    let oz = rb.angular_velocity[2];
    let omega_q: Quat = [0.0, ox, oy, oz];
    let qdot = quat_mul(omega_q, rb.orientation);
    let q = rb.orientation;
    let q_norm_sq = q[0]*q[0] + q[1]*q[1] + q[2]*q[2] + q[3]*q[3];
    let drift_k: f64 = 1.0;
    let drift = drift_k * (1.0 - q_norm_sq);
    rb.orientation = quat_normalize([
        q[0] + (qdot[0] * 0.5 + drift * q[0]) * dt,
        q[1] + (qdot[1] * 0.5 + drift * q[1]) * dt,
        q[2] + (qdot[2] * 0.5 + drift * q[2]) * dt,
        q[3] + (qdot[3] * 0.5 + drift * q[3]) * dt,
    ]);

    // Phase 3: Linear velocity: vel += (F/m) * dt
    if rb.inverse_mass > 0.0 {
        for i in 0..3 { rb.linear_velocity[i] += rb.force[i] * rb.inverse_mass * dt; }
    }

    // Phase 4: Angular velocity: omega += I_inv * torque * dt
    let ang_accel = mat3_transform(rb.inverse_inertia_tensor, rb.torque);
    for i in 0..3 { rb.angular_velocity[i] += ang_accel[i] * dt; }
}

/// Save body state for rollback (bisection).
/// From x86: sub_100154C0
fn save_body_state(rb: &mut crate::player::cast_member::HavokRigidBody) {
    rb.saved_position = rb.position;
    rb.saved_orientation = rb.orientation;
    rb.saved_linear_velocity = rb.linear_velocity;
    rb.saved_angular_velocity = rb.angular_velocity;
}

/// Restore saved body state (rollback).
fn restore_body_state(rb: &mut crate::player::cast_member::HavokRigidBody) {
    rb.position = rb.saved_position;
    rb.orientation = rb.saved_orientation;
    rb.linear_velocity = rb.saved_linear_velocity;
    rb.angular_velocity = rb.saved_angular_velocity;
}

// ============================================================
// Helpers
// ============================================================

fn find_body_idx(state: &HavokPhysicsState, name: &str) -> Option<usize> {
    state.rigid_bodies.iter().position(|rb| rb.name.eq_ignore_ascii_case(name))
}

/// Transform a body-local point to world space.
fn body_transform_point(rb: &crate::player::cast_member::HavokRigidBody, local_point: V3) -> V3 {
    v3_add(quat_rotate_v(rb.orientation, local_point), rb.position)
}

/// Update rotation_axis/rotation_angle from orientation quaternion (for Lingo readback).
fn update_axis_angle_from_orientation(rb: &mut crate::player::cast_member::HavokRigidBody) {
    let (axis, angle_rad) = quat_to_axis_angle(rb.orientation);
    let angle_deg = angle_rad * (180.0 / std::f64::consts::PI);
    if angle_rad.abs() <= 0.001 {
        rb.rotation_axis = [1.0, 0.0, 0.0];
        rb.rotation_angle = 0.0;
    } else {
        rb.rotation_axis = v3_normalized(axis);
        rb.rotation_angle = angle_deg;
    }
}

// ============================================================
// Main physics step
// From C# World.cs — Step + StepSingle (bisection)
// ============================================================

/// Minimum timestep for bisection. Below this, resolve with impulses.
const MIN_BISECTION_DT: f64 = 0.00001;
/// Maximum bisection retries.
const MAX_BISECTION_RETRIES: usize = 30;

pub fn step_native(state: &mut HavokPhysicsState, time_increment: f64, num_sub_steps: i32) {
    let n_subs = num_sub_steps.max(1) as usize;
    let sub_dt = time_increment / n_subs as f64;

    // Game forces (from applyForceAtPoint) are applied with per-axis dividers
    // calibrated against Director's observed behaviour. The torque dividers
    // differ between pitch/roll and yaw, matching the empirical asymmetry we
    // see when comparing log parity against Director.
    //
    //   * Havok's integrator (sub_10014DA0) clears rb.force and rb.torque at
    //     the end of every inner integrate step. SuperSonic runs the adaptive
    //     integrator path that splits each substep into many inner micro-steps
    //     so game forces only persist for the very first micro-step. The
    //     effective attenuation is N² × (N_inner/N) with N=7 substeps.
    //   * LINEAR dynamics match Director at force_scale=49 (/N²). Verified by
    //     spring equilibrium and drive velocity.
    //   * PITCH/ROLL torque matches Director at torque_scale=434 (= N²×62/7).
    //     Verified by first-drive-frame ang_x = -0.0243 matching Director.
    //   * YAW torque matches Director at torque_scale≈159 (= N²×3.24). This
    //     is 2.73× less attenuation than pitch/roll. Verified by matching
    //     Director's turn radius during a hard left turn; larger yaw dividers
    //     collapse the turn circle to unrealistic sizes.
    //
    // The per-axis asymmetry means yaw-driving forces (sideways wheel friction
    // torque) and pitch-driving forces (drive force × lever) have different
    // effective timescales in Havok. We can't explain WHY this is without
    // deeper RE on the integrator's force handling per axis, but empirically
    // these values give a tight Director-parity fit.
    //
    // World-frame approximation: for SuperSonic the body's axes stay near world
    // alignment (small pitch/roll angles during driving), so applying the
    // per-axis divider in world frame is equivalent to body frame. If we
    // ever needed to simulate a car that flips upside down the correct thing
    // would be to rotate torque to body frame, divide per axis, rotate back.
    // LINEAR force attenuation: 6·N, measured directly against Director.
    //
    // A Lingo probe applied a known force at the centre of mass (no torque) and
    // read back the resulting velocity over step(0.025, N) for N = 1, 7, 20:
    //
    //     N        1         7        20
    //     Director 1.6667    0.2381   0.0833     → N·Δv  = 1.6667  (∝ 1/N)
    //     dirplayer 10.0000  0.2041   0.0250     → N²·Δv = 10.0    (∝ 1/N²)
    //
    // Exact to every digit printed. Director scales as 1/N — the force survives
    // exactly one substep of dt/N, because the force accumulator is cleared on
    // every integration step. (F/m)·dt = 10.0 here, so Director's response is
    // (F/m)·dt/(6N). The extra factor of 6 is measured but unexplained: it is not
    // in the substep structure, and not in the Lingo force entry points (their
    // worldScale conversion cancels against the velocity readback's).
    //
    // The previous value was N². It survived because at the N=7 these movies
    // use, N²=49 and 6N=42 are only 17% apart — and the two prior experiments
    // both tested force_scale=N (=7), which is 6× too strong, hence "way worse"
    // / uncontrollable. 6N matches Director at N=1, 7 AND 20, not just at 7.
    let force_scale: f64 = 6.0 * n_subs as f64;         // 42 at N=7 (was 49)
    // Torque is left on the OLD N² basis deliberately: the probe measured only
    // the linear response (angV stayed 0), so there is no measurement of
    // Director's torque law to justify moving it. These keep their previous
    // absolute values (434 / 158.76 at N=7) so this change isolates linear force.
    let n_sq = (n_subs * n_subs) as f64;                // 49 at N=7
    let torque_scale_pitch_roll: f64 = n_sq * (62.0 / 7.0);  // 434
    let torque_scale_yaw: f64 = n_sq * 3.24;                  // 158.76
    let saved_forces: Vec<([f64;3],[f64;3])> = state.rigid_bodies.iter()
        .map(|rb| (rb.force, rb.torque)).collect();

    for _sub in 0..n_subs {
        // Reset forces to game values each substep (gravity/drag added in
        // step_single). The per-axis force/torque dividers are a SuperSonic-
        // specific calibration; applying them to OTHER hover cars crushes their
        // suspension's levelling torque (~0.2%) and tips them over on tilted
        // roads. Only driven (makeMovableRigidBody) bodies use the calibration;
        // every other body gets its applied force/torque verbatim.
        for (i, rb) in state.rigid_bodies.iter_mut().enumerate() {
            if i < saved_forces.len() {
                let (fs, tsp, tsy) = if rb.driven {
                    (force_scale, torque_scale_pitch_roll, torque_scale_yaw)
                } else {
                    // Non-SuperSonic hover cars: scale torque the SAME as force
                    // (physically consistent) instead of the SuperSonic-only
                    // pitch/roll/yaw asymmetry that crushed levelling torque.
                    (force_scale, force_scale, force_scale)
                };
                rb.force = [
                    saved_forces[i].0[0]/fs,
                    saved_forces[i].0[1]/fs,
                    saved_forces[i].0[2]/fs,
                ];
                rb.torque = [
                    saved_forces[i].1[0]/tsp,   // world X ≈ body pitch
                    saved_forces[i].1[1]/tsp,   // world Y ≈ body roll
                    saved_forces[i].1[2]/tsy,   // world Z ≈ body yaw
                ];
            }
        }

        step_single(state, sub_dt);
    }

    // Clear forces after all substeps
    for rb in &mut state.rigid_bodies {
        rb.force = [0.0; 3];
        rb.torque = [0.0; 3];
    }

    // Numerical angular damping — not a clamp.
    //
    // Simple isotropic per-frame decay. Attempts at per-axis body-frame
    // damping (pitch/roll/yaw tuned individually) made turning worse in
    // user testing — probably because killing roll response during a turn
    // breaks the natural lean-into-the-turn dynamic the springs rely on.
    //
    // Numerical angular damping — not a clamp.
    //
    // Simple isotropic per-frame decay. Attempts at per-axis body-frame
    // damping (pitch/roll/yaw tuned individually) made turning worse in
    // user testing — probably because killing roll response during a turn
    // breaks the natural lean-into-the-turn dynamic the springs rely on.
    //
    // NOTE: this does NOT correspond to anything in the engine, despite what this
    // comment used to claim. The engine damps velocity only through dashpot actions
    // — a linear dashpot by 0.1%, an angular one by 0% — and it scales linear and
    // angular together. There is no global per-frame damping in the engine's step
    // loop, so a body driven purely by forces (a RaycastCar's hover springs, with no
    // dashpots) should receive none. Our 10%/frame is 100x the largest real value
    // and unconditional; it absorbs spurious spin our contact solver leaks.
    //
    // TRIED AND REJECTED, do not redo without new information:
    //   * setting it to 0.0 (faithful) — improved FinalDrive four-wheel contact
    //     45% -> 56%, but regressed other games;
    //   * skipping it for bodies that received a Lingo force this frame (derived from
    //     the `saved_forces` snapshot above) — FinalDrive 45% -> 66.5%, still rejected
    //     in testing.
    // The real fix is to make the contact solver dissipate like Havok's (friction,
    // resting contacts, deactivator) so the crutch can go entirely; see the abandoned
    // faithful solver in Desktop\havok\"havok_physics copy.rs".
    const ANGULAR_DRIFT_DAMP: f64 = 0.1; //0.05;
    let ang_factor = 1.0 - ANGULAR_DRIFT_DAMP;
    for rb in &mut state.rigid_bodies {
        if rb.pinned || !rb.active { continue; }
        rb.angular_velocity[0] *= ang_factor;
        rb.angular_velocity[1] *= ang_factor;
        rb.angular_velocity[2] *= ang_factor;
    }

    // Update derived state (rotation_axis/angle from quaternion for Lingo readback)
    for rb in &mut state.rigid_bodies {
        update_axis_angle_from_orientation(rb);
    }

    // --- Auto-sleep (deactivation) ---
    // A non-driven body that stays still — low linear+angular speed AND no Lingo
    // disturbance this step — for SLEEP_DELAY seconds deactivates, so resting
    // stacks stop jittering and stop loading the solver. Driven/forced bodies
    // (the SuperSonic car, On the Run's impulse-driven cars) are disturbed every
    // frame and never reach the countdown, so they never sleep mid-play. Waking
    // is unchanged (collision at resolve time; Lingo apply*/setters set
    // `active=true`). The reference RigidBody.ShouldDeactivate exists but is
    // unwired; this is a velocity-based equivalent of its settle test.
    {
        const SLEEP_DELAY: f64 = 0.5;
        let inv_s = if state.scale.abs() > 1e-10 { 1.0 / state.scale } else { 1.0 };
        let lin_thr2 = (0.1 * inv_s) * (0.1 * inv_s); // ~0.1 m/s, in display units
        let ang_thr2 = 0.05 * 0.05;                   // ~0.05 rad/s
        for rb in &mut state.rigid_bodies {
            if !rb.active || rb.pinned || rb.driven || rb.inverse_mass <= 0.0 {
                rb.lingo_disturbed = false;
                continue;
            }
            let lv = rb.linear_velocity;
            let av = rb.angular_velocity;
            let lin2 = lv[0]*lv[0] + lv[1]*lv[1] + lv[2]*lv[2];
            let ang2 = av[0]*av[0] + av[1]*av[1] + av[2]*av[2];
            if rb.lingo_disturbed || lin2 > lin_thr2 || ang2 > ang_thr2 {
                rb.sleep_countdown = SLEEP_DELAY;
            } else {
                rb.sleep_countdown -= time_increment;
                if rb.sleep_countdown <= 0.0 {
                    rb.active = false;
                    rb.linear_velocity = [0.0; 3];
                    rb.angular_velocity = [0.0; 3];
                }
            }
            rb.lingo_disturbed = false;
        }
    }

    state.sim_time += time_increment;
}

/// Single substep with bisection collision handling.
/// From x86: sub_100175C0
fn step_single(state: &mut HavokPhysicsState, dt: f64) {
    let mut remaining = dt;
    let mut retries = 0;

    // Passive scenes (standard Havok behaviour: no Lingo springs/dashpots and no
    // hover/drive-forced body) use discrete collision: integrate the full step,
    // then resolve + depenetrate. The bisection rollback path rolls back ALL
    // bodies on ANY collision, which for a pile of resting bodies starves the
    // simulation of time and freezes it. Bisection stays on for driven scenes
    // (the SuperSonic car was tuned against it).
    let passive = state.springs.is_empty()
        && state.linear_dashpots.is_empty()
        && state.angular_dashpots.is_empty()
        && !state.rigid_bodies.iter().any(|rb| rb.driven);

    while remaining > MIN_BISECTION_DT * 0.1 {
        // Phase 1: Save state for rollback
        for rb in &mut state.rigid_bodies {
            if !rb.pinned && rb.active {
                save_body_state(rb);
            }
        }

        // Phase 2: Apply gravity
        apply_gravity(state);

        // Phase 3: Apply actions (drag, springs, dashpots)
        apply_drag(state);
        apply_springs(state, remaining);
        apply_linear_dashpots(state, remaining);
        apply_angular_dashpots(state, remaining);

        // Phase 4: Integrate all bodies (Forward Euler with quaternion)
        for rb in &mut state.rigid_bodies {
            integrate_body(rb, remaining);
        }

        // Phase 4a: Cable / point-to-point constraints (pendulum hang+swing).
        apply_cables(state);

        // Phase 4b: Ground constraint (resting contacts)
        apply_ground_constraints(state);

        // Phase 4c: Surface contact constraint — keep bodies resting on
        // rigid-body-owned collision meshes (tilted surfaces, ground planes).
        // Without this, gravity's normal component pulls the body into the
        // surface each substep, triggering bisection+impulse repeatedly
        // instead of smooth sliding.
        apply_surface_contacts(state, remaining);

        // Phase 5: Detect transient collisions (body vs static mesh)
        let contacts = detect_all_collisions(state);
        let has_collisions = !contacts.is_empty();

        if !passive && has_collisions && remaining > MIN_BISECTION_DT && retries < MAX_BISECTION_RETRIES {
            // Collision detected — rollback and bisect
            for rb in &mut state.rigid_bodies {
                if !rb.pinned && rb.active {
                    restore_body_state(rb);
                }
            }
            remaining *= 0.5;
            if remaining < MIN_BISECTION_DT { remaining = MIN_BISECTION_DT; }
            retries += 1;
        } else {
            if has_collisions {
                // Can't bisect further — resolve with impulses (PointRRResolver)
                resolve_contacts(state, &contacts);

                // Position correction: push bodies out of penetration so they
                // don't immediately re-collide on the next substep (which would
                // trigger a global bisection rollback including bodies that are
                // already bouncing away).
                for c in &contacts {
                    if c.depth <= 0.0 { continue; }

                    // Is body_b another DYNAMIC body (box-vs-box)? Then split the
                    // depenetration and the relative normal-velocity removal
                    // between the two by inverse mass, so a stack settles without
                    // either box being shoved through its neighbour or the floor.
                    let b_dynamic = match c.body_b {
                        Some(j) => !state.rigid_bodies[j].pinned && state.rigid_bodies[j].inverse_mass > 0.0,
                        None => false,
                    };

                    if b_dynamic {
                        let j = c.body_b.unwrap();
                        // A moving body that penetrates a sleeping one wakes it,
                        // so the impact propagates through the stack.
                        state.rigid_bodies[c.body_a].active = true;
                        state.rigid_bodies[j].active = true;
                        let ia = state.rigid_bodies[c.body_a].inverse_mass;
                        let ib = state.rigid_bodies[j].inverse_mass;
                        let isum = ia + ib;
                        if isum <= 0.0 { continue; }
                        let fa = ia / isum;
                        let fb = ib / isum;
                        for k in 0..3 {
                            state.rigid_bodies[c.body_a].position[k] += c.normal[k] * c.depth * fa;
                            state.rigid_bodies[j].position[k]        -= c.normal[k] * c.depth * fb;
                        }
                        let va = state.rigid_bodies[c.body_a].linear_velocity;
                        let vb = state.rigid_bodies[j].linear_velocity;
                        let rvn = (va[0]-vb[0])*c.normal[0] + (va[1]-vb[1])*c.normal[1] + (va[2]-vb[2])*c.normal[2];
                        if rvn < 0.0 {
                            for k in 0..3 {
                                state.rigid_bodies[c.body_a].linear_velocity[k] -= rvn * fa * c.normal[k];
                                state.rigid_bodies[j].linear_velocity[k]        += rvn * fb * c.normal[k];
                            }
                        }
                        continue;
                    }

                    // Static / fixed-body contact: push body_a out fully.
                    let ground_friction = c.body_b
                        .map(|idx| state.rigid_bodies[idx].friction)
                        .unwrap_or(0.5);
                    let rb = &mut state.rigid_bodies[c.body_a];
                    if !rb.pinned && rb.inverse_mass > 0.0 {
                        let vn = rb.linear_velocity[0] * c.normal[0]
                               + rb.linear_velocity[1] * c.normal[1]
                               + rb.linear_velocity[2] * c.normal[2];
                        let push = c.depth;
                        rb.position[0] += c.normal[0] * push;
                        rb.position[1] += c.normal[1] * push;
                        rb.position[2] += c.normal[2] * push;
                        if vn.abs() < 10.0 {
                            rb.linear_velocity[0] -= vn * c.normal[0];
                            rb.linear_velocity[1] -= vn * c.normal[1];
                            rb.linear_velocity[2] -= vn * c.normal[2];
                        }
                        // Resting contact only for a body resting on another
                        // body's OWNED static mesh (SuperSonic car on terrain).
                        if c.body_b.is_some() && vn.abs() < 10.0 {
                            if let Some(m) = state.collision_meshes.iter().find(|m| m.body_index == c.body_b) {
                                rb.resting_normal = Some(crate::player::cast_member::RestingContact {
                                    normal: c.normal,
                                    plane_point: c.point,
                                    ground_friction,
                                    aabb_min: m.aabb_min,
                                    aabb_max: m.aabb_max,
                                });
                            }
                        }
                    }
                }

                // Record collision list for Lingo readback
                state.collision_list_cache.clear();
                for c in &contacts {
                    let body_a_name = state.rigid_bodies[c.body_a].name.clone();
                    // Name the contact partner. For a body-vs-body contact use the
                    // other rigid body's name; for a static-mesh contact fall back
                    // to the collision mesh's own name (the real surface model,
                    // e.g. "GraficaStradaZona01") so Lingo callbacks that classify
                    // the ground by name (cd[2] contains "strada"/"marciapiede"/…)
                    // work. Only when neither is known do we emit "ground".
                    let body_b_name = c.body_b
                        .map(|i| state.rigid_bodies[i].name.clone())
                        .or_else(|| c.mesh_index
                            .and_then(|mi| state.collision_meshes.get(mi))
                            .map(|m| m.name.clone()))
                        .unwrap_or_else(|| "ground".to_string());
                    state.collision_list_cache.push(crate::player::cast_member::HavokCollisionInfo {
                        body_a: body_a_name,
                        body_b: body_b_name,
                        point: c.point,
                        normal: c.normal,
                        normal_rel_vel: c.normal_rel_vel,
                    });
                }
            }
            break;
        }
    }
}

/// Apply cable / point-to-point constraints as rigid distance constraints
/// (a pendulum): keep each bob's attach point at `length` from its fixed
/// anchor, removing radial velocity so it swings under gravity instead of
/// falling. Position-based so it's stable for a light body like the lamp.
fn apply_cables(state: &mut HavokPhysicsState) {
    if state.cable_constraints.is_empty() { return; }
    let cables = state.cable_constraints.clone();
    for cable in &cables {
        let rb = &mut state.rigid_bodies[cable.body_index];
        if rb.pinned || !rb.active || rb.inverse_mass <= 0.0 { continue; }

        let attach_world = v3_add(rb.position, quat_rotate_v(rb.orientation, cable.attach_local));
        let d = v3_sub(attach_world, cable.anchor);
        let dist = v3_len(d);
        if dist < 1e-6 { continue; }
        let dir = v3_scale(d, 1.0 / dist);

        // Pull the body so its attach point sits exactly `length` from anchor.
        let correction = dist - cable.length;
        rb.position = v3_sub(rb.position, v3_scale(dir, correction));

        // Remove radial velocity (rigid rod) — keep only the tangential (swing).
        let vr = v3_dot(rb.linear_velocity, dir);
        rb.linear_velocity = v3_sub(rb.linear_velocity, v3_scale(dir, vr));
    }
}

/// Ground safety net — prevents bodies from falling through the collision mesh.
/// The game's Lingo spring forces handle normal suspension (oscillation around equilibrium).
/// This constraint only activates when the body penetrates below the mesh surface,
/// acting as a hard floor to prevent fall-through.
fn apply_ground_constraints(state: &mut HavokPhysicsState) {
    if !state.use_ground_constraint { return; }
    if state.collision_meshes.is_empty() && state.ground_z <= -1e10 { return; }

    let half_z = state.ground_body_half_z;

    for bi in 0..state.rigid_bodies.len() {
        if state.rigid_bodies[bi].pinned || !state.rigid_bodies[bi].active || state.rigid_bodies[bi].inverse_mass <= 0.0 {
            continue;
        }
        let pos = state.rigid_bodies[bi].position;

        if !state.collision_meshes.is_empty() {
            if let Some(ground_z) = find_ground_z(&state.collision_meshes, pos[0], pos[1], pos[2] + 100.0) {
                let body_bottom = pos[2] - half_z;
                // Safety net: only clamp if body penetrates MORE than 5 units below ground.
                // Normal suspension is handled by the game's Lingo spring forces.
                // The car needs to be free to oscillate above the ground mesh.
                if body_bottom < ground_z - 5.0 {
                    state.rigid_bodies[bi].position[2] = ground_z + half_z - 5.0;
                    let vz = state.rigid_bodies[bi].linear_velocity[2];
                    if vz > 0.0 {
                        state.rigid_bodies[bi].linear_velocity[2] *= 0.5;
                    }
                }
            }
        } else if state.ground_z > -1e10 {
            // Flat ground fallback (no mesh)
            let body_bottom = pos[2] - half_z;
            if body_bottom < state.ground_z + 0.5 {
                state.rigid_bodies[bi].position[2] = state.ground_z + half_z;
                let vz = state.rigid_bodies[bi].linear_velocity[2];
                if vz > 0.0 { state.rigid_bodies[bi].linear_velocity[2] = 0.0; }
                else if vz < -0.01 { state.rigid_bodies[bi].linear_velocity[2] *= -0.05; }
            }
        }
    }
}

/// Surface contact constraint using analytical plane projection.
/// No per-triangle detection — uses stored plane normal + point for stable sliding.
/// Clears resting contact when ball leaves the mesh AABB (edge of platform).
fn apply_surface_contacts(state: &mut HavokPhysicsState, dt: f64) {
    let g = state.gravity;

    for bi in 0..state.rigid_bodies.len() {
        let rc = match &state.rigid_bodies[bi].resting_normal {
            Some(rc) => rc.clone(),
            None => continue,
        };
        if state.rigid_bodies[bi].pinned || !state.rigid_bodies[bi].active
            || state.rigid_bodies[bi].inverse_mass <= 0.0 { continue; }

        let he = state.rigid_bodies[bi].inertia_half_extents;
        let body_radius = he[0].max(he[1]).max(he[2]);
        let eff_radius = body_radius + state.tolerance;
        let pos = state.rigid_bodies[bi].position;
        let n = rc.normal;

        // Check if ball is still within the mesh AABB (on the platform)
        if pos[0] < rc.aabb_min[0] || pos[0] > rc.aabb_max[0]
            || pos[1] < rc.aabb_min[1] || pos[1] > rc.aabb_max[1] {
            // Left the platform edge — free fall
            state.rigid_bodies[bi].resting_normal = None;
            continue;
        }

        // Compute the Z position that keeps the ball at eff_radius from the
        // tilted plane.  Adjusting only Z avoids the X-jitter that occurs when
        // projecting along the tilted normal (which has an X component).
        // Plane eq: dot(pos - pp, n) = eff_radius
        //   (px-ppx)*nx + (py-ppy)*ny + (pz-ppz)*nz = eff_radius
        //   pz = ppz + (eff_radius - (px-ppx)*nx - (py-ppy)*ny) / nz
        let rb = &mut state.rigid_bodies[bi];
        if n[2].abs() > 0.01 {
            let target_z = rc.plane_point[2]
                + (eff_radius - (rb.position[0]-rc.plane_point[0])*n[0]
                              - (rb.position[1]-rc.plane_point[1])*n[1]) / n[2];
            rb.position[2] = target_z;
        }

        // Cancel normal velocity
        let vn = rb.linear_velocity[0]*n[0] + rb.linear_velocity[1]*n[1] + rb.linear_velocity[2]*n[2];
        if vn < 0.0 {
            rb.linear_velocity[0] -= vn * n[0];
            rb.linear_velocity[1] -= vn * n[1];
            rb.linear_velocity[2] -= vn * n[2];
        }
        // Cancel normal force
        let fn_ = rb.force[0]*n[0] + rb.force[1]*n[1] + rb.force[2]*n[2];
        if fn_ < 0.0 {
            rb.force[0] -= fn_ * n[0];
            rb.force[1] -= fn_ * n[1];
            rb.force[2] -= fn_ * n[2];
        }

        // Tangential velocity
        let tv = [
            rb.linear_velocity[0] - vn.max(0.0)*n[0],
            rb.linear_velocity[1] - vn.max(0.0)*n[1],
            rb.linear_velocity[2] - vn.max(0.0)*n[2],
        ];
        let t_speed = v3_len(tv);

        // Sliding friction with rolling cap
        let g_n = g[0]*n[0] + g[1]*n[1] + g[2]*n[2];
        let g_tan = v3_len([g[0]-g_n*n[0], g[1]-g_n*n[1], g[2]-g_n*n[2]]);
        let mu = rb.friction * rc.ground_friction;
        if mu > 0.0 && t_speed > 1e-6 {
            let max_rolling = (2.0/7.0) * g_tan;
            let eff_friction = (mu * g_n.abs()).min(max_rolling);
            let friction_decel = eff_friction * dt;
            let factor = if friction_decel >= t_speed { 0.0 } else { 1.0 - friction_decel / t_speed };
            rb.linear_velocity[0] = n[0]*vn.max(0.0) + tv[0]*factor;
            rb.linear_velocity[1] = n[1]*vn.max(0.0) + tv[1]*factor;
            rb.linear_velocity[2] = n[2]*vn.max(0.0) + tv[2]*factor;
        }

        // Rolling angular velocity for ALL resting balls: ω = v/r
        if t_speed > 1e-6 && body_radius > 0.01 {
            let cur_tv = [
                rb.linear_velocity[0] - vn.max(0.0)*n[0],
                rb.linear_velocity[1] - vn.max(0.0)*n[1],
                rb.linear_velocity[2] - vn.max(0.0)*n[2],
            ];
            let cur_speed = v3_len(cur_tv);
            let vd = [tv[0]/t_speed, tv[1]/t_speed, tv[2]/t_speed];
            rb.angular_velocity = v3_scale(v3_cross(n, vd), cur_speed / body_radius);
        }
    }
}

/// Dynamic-vs-dynamic collision using an axis-aligned box approximation around
/// each body's COM. Resolves along the axis of least penetration (minimum
/// translation vector), which is stable for the near-axis-aligned crates in the
/// warehouse demo. Returns one contact per overlapping pair.
fn detect_body_body_collisions(state: &HavokPhysicsState) -> Vec<CollisionContact> {
    let mut out = Vec::new();
    let n = state.rigid_bodies.len();
    for i in 0..n {
        let a = &state.rigid_bodies[i];
        if a.pinned || a.inverse_mass <= 0.0 || a.driven { continue; }
        let ca = v3_add(a.position, quat_rotate_v(a.orientation, a.center_of_mass));
        for j in (i+1)..n {
            let b = &state.rigid_bodies[j];
            if b.pinned || b.inverse_mass <= 0.0 || b.driven { continue; }
            // At least one body must be awake — two sleeping bodies in resting
            // contact must NOT interact, or the whole stack would wake itself.
            if !a.active && !b.active { continue; }
            let cb = v3_add(b.position, quat_rotate_v(b.orientation, b.center_of_mass));

            // Per-axis overlap of the two AABBs (half-extents summed).
            let mut min_overlap = f64::MAX;
            let mut axis = 0usize;
            let mut sep = false;
            for k in 0..3 {
                let sum = a.inertia_half_extents[k] + b.inertia_half_extents[k];
                let d = ca[k] - cb[k];
                let ov = sum - d.abs();
                if ov <= -CONTACT_MARGIN { sep = true; break; }
                if ov < min_overlap { min_overlap = ov; axis = k; }
            }
            if sep { continue; }

            // Normal points from B toward A along the least-penetrated axis.
            let mut normal = [0.0; 3];
            normal[axis] = if ca[axis] - cb[axis] >= 0.0 { 1.0 } else { -1.0 };
            let point = v3_scale(v3_add(ca, cb), 0.5);
            // Closing speed between the two bodies at the contact point, captured
            // before resolution (impact speed for the collision callback).
            let va = body_point_velocity(a, point);
            let vb = body_point_velocity(b, point);
            let nrv = ((va[0]-vb[0])*normal[0] + (va[1]-vb[1])*normal[1] + (va[2]-vb[2])*normal[2]).abs();
            out.push(CollisionContact {
                body_a: i,
                body_b: Some(j),
                point,
                normal,
                depth: min_overlap,
                mesh_index: None,
                normal_rel_vel: nrv,
            });
        }
    }
    out
}

/// Detect all transient collisions (body vs static mesh walls/obstacles).
fn detect_all_collisions(state: &HavokPhysicsState) -> Vec<CollisionContact> {
    if state.collision_meshes.is_empty() { return Vec::new(); }

    let mut all_contacts = Vec::new();

    // A "passive" scene (no Lingo springs / dashpots) relies entirely on the
    // engine to keep bodies on the static ground mesh — this is the standard
    // Havok behaviour library demo (e.g. the warehouse falling-crates scene).
    // SuperSonic-style scenes register dashpots/springs and instead drive
    // ground contact from Lingo hover forces, so for those we keep skipping the
    // upward-facing static-ground contacts (handled by the ground constraint).
    let passive_scene = state.springs.is_empty()
        && state.linear_dashpots.is_empty()
        && state.angular_dashpots.is_empty();

    for bi in 0..state.rigid_bodies.len() {
        let rb = &state.rigid_bodies[bi];
        if rb.pinned || !rb.active || rb.inverse_mass <= 0.0 { continue; }
        // Skip bodies in resting contact — handled by apply_surface_contacts
        if rb.resting_normal.is_some() { continue; }

        // Per-body decision: a DRIVEN body (the SuperSonic / car-demo car, which
        // receives hover/drive forces) keeps the original collision path it was
        // tuned against — even in an otherwise passive scene. Force-free objects
        // (crates, blocks) use the box-stacking path.
        let body_passive = passive_scene && !rb.driven;

        // Use the body's actual half-extents (box support) for collision.
        let he = rb.inertia_half_extents;
        // Box-stacking objects test against the COM-offset box centre (so boxes
        // rest flush). Driven bodies keep the original origin-centred test.
        let center = if body_passive {
            v3_add(rb.position, quat_rotate_v(rb.orientation, rb.center_of_mass))
        } else {
            rb.position
        };

        let contacts = detect_body_contacts(&state.collision_meshes, center, he, bi, state.tolerance, body_passive, &state.rigid_bodies);
        // Keep only the deepest contact per body to avoid duplicate impulses
        // from coplanar triangles (e.g. two triangles forming a box face).
        let mut best: Option<CollisionContact> = None;
        for c in contacts {
            // Skip upward-facing ground contacts for unowned scenery meshes when
            // a driven body handles ground via hover forces. Box-stacking
            // objects must keep these or they fall through the floor.
            if c.normal[2] > 0.7 && c.body_b.is_none() && !body_passive { continue; }
            if best.as_ref().map_or(true, |b| c.depth > b.depth) {
                best = Some(c);
            }
        }
        if let Some(c) = best {
            all_contacts.push(c);
        }
    }

    // Dynamic body vs dynamic body (axis-aligned box approximation) — passive
    // scenes only. This is what keeps a stack of crates standing. SuperSonic-
    // style scenes have no dynamic-vs-dynamic stacks and were tuned without it.
    if passive_scene {
        all_contacts.extend(detect_body_body_collisions(state));
    }

    all_contacts
}

// ============================================================
// W3D sync transform
// Builds a 4x4 column-major matrix from position + quaternion orientation.
// Accounts for center-of-mass offset (display origin ≠ physics COM).
// From C# RigidBody.cs — GetDisplayToWorldTransform
// ============================================================

/// Extract yaw angle (rotation about Z) from quaternion.
/// The LEGO game's Lingo scripts rely on the car's W3D transform being a
/// pure yaw rotation (no pitch/roll). The old code used `yaw_angle` directly.
/// We extract yaw from the quaternion to match this expected behavior.
pub fn quat_to_yaw(q: Quat) -> f64 {
    // For a quaternion representing rotation about Z only:
    // yaw = 2 * atan2(q.z, q.w)
    // For general quaternions, extract the Z-rotation component:
    let siny_cosp = 2.0 * (q[0] * q[3] + q[1] * q[2]);
    let cosy_cosp = 1.0 - 2.0 * (q[2] * q[2] + q[3] * q[3]);
    siny_cosp.atan2(cosy_cosp)
}

pub fn build_sync_transform(pos: V3, orientation: Quat, com_local: V3, scale: V3) -> [f32; 16] {
    // Rotation around the center of mass, not around the visual origin.
    //
    // Convention: `pos` is the "reference position" — the visual origin's world
    // location under NO rotation. The physical COM world position is
    // `pos + com_local` (with com_local stored in body-local space). When the
    // body rotates, the COM should stay fixed (pure rotation preserves COM) and
    // the visual origin should orbit around it.
    //
    // For a wheel at local position v, the correct world position is:
    //   world_wheel = R * (v - com_local) + com_world
    //               = R*v - R*com_local + pos + com_local
    //               = R*v + pos + (I - R) * com_local
    //
    // So the 4x4 transform used by the W3D scene graph has the rotation R and
    // translation `pos + (I - R) * com_local`. Without this correction, wheel
    // child nodes rotate around the visual origin instead of the COM, the game's
    // actAsSpring damping sees wrong wheel velocities, and pitch/roll grow
    // unbounded during driving.
    let m = quat_to_mat3(orientation);

    // R * com_local
    let rx = m[0]*com_local[0] + m[1]*com_local[1] + m[2]*com_local[2];
    let ry = m[3]*com_local[0] + m[4]*com_local[1] + m[5]*com_local[2];
    let rz = m[6]*com_local[0] + m[7]*com_local[1] + m[8]*com_local[2];

    // translation = pos + com_local - R*com_local
    let tx = pos[0] + com_local[0] - rx;
    let ty = pos[1] + com_local[1] - ry;
    let tz = pos[2] + com_local[2] - rz;

    // Scale the rotation columns by the authored per-axis model scale so the
    // rendered model keeps its size (RaycastCar wheel/hover points are derived
    // from this transform).
    let (s0, s1, s2) = (scale[0], scale[1], scale[2]);
    [
        (m[0]*s0) as f32, (m[3]*s0) as f32, (m[6]*s0) as f32, 0.0,
        (m[1]*s1) as f32, (m[4]*s1) as f32, (m[7]*s1) as f32, 0.0,
        (m[2]*s2) as f32, (m[5]*s2) as f32, (m[8]*s2) as f32, 0.0,
        tx as f32, ty as f32, tz as f32, 1.0,
    ]
}
