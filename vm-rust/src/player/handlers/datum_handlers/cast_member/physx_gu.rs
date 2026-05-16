//! PhysX 3.4 narrowphase — verbatim Rust ports of the `Gu::contact*` family.
//!
//! Each function in this file is a 1:1 translation of the corresponding C++
//! at `E:\gitrepos\chameleonxxl\PhysX-3.4\PhysX_3.4\Source\GeomUtils\src\contact\`,
//! with the source path + line range cited above the Rust version.
//!
//! **PhysX engine convention** (preserved by these functions):
//! - `normal` points from shape1 → shape0 (B → A).
//! - `separation` is NEGATIVE when penetrating, positive when separated.
//! - `delta = transform0.p - transform1.p`.
//!
//! The boundary code in [`super::physx_native`] flips one to the other so
//! the rest of our solver sees a consistent A→B / positive-pen convention.
//!
//! All vectors are `[f64; 3]` (matching the rest of `dirplayer-rs`),
//! quaternions are `[f64; 4]` in `(x, y, z, w)` order. The PhysX SDK uses
//! `(x, y, z, w)` too — see `PxQuat.h`.

#![allow(dead_code)]

// ============================================================================
//  Vector + quaternion math (mirrors the bits of PxVec3 / PxQuat the
//  narrowphase actually touches; the engine has many more methods).
// ============================================================================

#[inline] pub fn v_add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [a[0]+b[0], a[1]+b[1], a[2]+b[2]] }
#[inline] pub fn v_sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [a[0]-b[0], a[1]-b[1], a[2]-b[2]] }
#[inline] pub fn v_mul(a: [f64; 3], s: f64) -> [f64; 3] { [a[0]*s, a[1]*s, a[2]*s] }
#[inline] pub fn v_neg(a: [f64; 3]) -> [f64; 3] { [-a[0], -a[1], -a[2]] }
#[inline] pub fn v_dot(a: [f64; 3], b: [f64; 3]) -> f64 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }
#[inline] pub fn v_len_sq(a: [f64; 3]) -> f64 { v_dot(a, a) }
#[inline] pub fn v_len(a: [f64; 3]) -> f64 { v_len_sq(a).sqrt() }
#[inline]
pub fn v_cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1]*b[2] - a[2]*b[1],
     a[2]*b[0] - a[0]*b[2],
     a[0]*b[1] - a[1]*b[0]]
}

/// Apply a quaternion `q = (x, y, z, w)` to vector `v`. Mirrors `PxQuat::rotate`.
#[inline]
pub fn q_rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    // Rodrigues shortcut: v + 2*q.w*(q.imag×v) + 2*q.imag×(q.imag×v).
    let vx = 2.0 * (q[1]*v[2] - q[2]*v[1]);
    let vy = 2.0 * (q[2]*v[0] - q[0]*v[2]);
    let vz = 2.0 * (q[0]*v[1] - q[1]*v[0]);
    [v[0] + q[3]*vx + (q[1]*vz - q[2]*vy),
     v[1] + q[3]*vy + (q[2]*vx - q[0]*vz),
     v[2] + q[3]*vz + (q[0]*vy - q[1]*vx)]
}

/// Apply the conjugate of `q` to `v` (`PxTransform::rotateInv`).
#[inline]
pub fn q_rotate_inv(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    q_rotate([-q[0], -q[1], -q[2], q[3]], v)
}

/// Compose orientation update: `q_new = (q + 0.5*ω*q*dt).normalize()`.
/// Used by the integrator. Mirrors PhysX's small-step integrator (PhysX
/// uses an exponential map for accuracy at large ω·dt; we linearize).
#[inline]
pub fn q_integrate(q: [f64; 4], omega: [f64; 3], dt: f64) -> [f64; 4] {
    let hx = omega[0] * dt * 0.5;
    let hy = omega[1] * dt * 0.5;
    let hz = omega[2] * dt * 0.5;
    // dq = (0, hx, hy, hz) (pure quat) → q_new = q + dq * q
    let dx =  hx*q[3] + hy*q[2] - hz*q[1];
    let dy = -hx*q[2] + hy*q[3] + hz*q[0];
    let dz =  hx*q[1] - hy*q[0] + hz*q[3];
    let dw = -hx*q[0] - hy*q[1] - hz*q[2];
    let nx = q[0] + dx; let ny = q[1] + dy; let nz = q[2] + dz; let nw = q[3] + dw;
    let inv_len = 1.0 / (nx*nx + ny*ny + nz*nz + nw*nw).sqrt();
    [nx * inv_len, ny * inv_len, nz * inv_len, nw * inv_len]
}

// ============================================================================
//  Contact buffer — mirrors `Gu::ContactBuffer` (PhysX uses fixed 64-slot;
//  Vec is functionally equivalent).
// ============================================================================

#[derive(Debug, Clone, Copy)]
pub struct GuContact {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub separation: f64,
}

#[derive(Debug, Default, Clone)]
pub struct GuContactBuffer {
    pub contacts: Vec<GuContact>,
}

impl GuContactBuffer {
    pub fn new() -> Self { Self { contacts: Vec::new() } }
    pub fn reset(&mut self) { self.contacts.clear(); }
    pub fn add(&mut self, point: [f64; 3], normal: [f64; 3], separation: f64) {
        self.contacts.push(GuContact { point, normal, separation });
    }
    pub fn count(&self) -> usize { self.contacts.len() }
}

// ============================================================================
//  GuContactSphereSphere — Source: GeomUtils/src/contact/GuContactSphereSphere.cpp:38-66
// ============================================================================

/// Returns Some((point, normal, separation)) on overlap. Normal points
/// shape1→shape0; separation < 0 ⇒ penetrating.
pub fn contact_sphere_sphere(
    transform0_p: [f64; 3], radius0: f64,
    transform1_p: [f64; 3], radius1: f64,
    contact_distance: f64,
) -> Option<GuContact> {
    let mut delta = v_sub(transform0_p, transform1_p);
    let distance_sq = v_len_sq(delta);
    let radius_sum = radius0 + radius1;
    let inflated = radius_sum + contact_distance;
    if distance_sq >= inflated * inflated { return None; }
    let magn = distance_sq.sqrt();
    if magn <= 1e-5 {
        delta = [1.0, 0.0, 0.0];
    } else {
        delta = v_mul(delta, 1.0 / magn);
    }
    // Source line 62: "PT: TODO: why is this formula different from the original code?"
    let scale = (radius0 + magn - radius1) * -0.5;
    let point = [delta[0]*scale + transform0_p[0],
                 delta[1]*scale + transform0_p[1],
                 delta[2]*scale + transform0_p[2]];
    Some(GuContact {
        point,
        normal: delta,
        separation: magn - radius_sum, // negative ⇒ penetrating
    })
}

// ============================================================================
//  GuContactSphereBox — Source: GeomUtils/src/contact/GuContactSphereBox.cpp:37-157
//  shape0 = sphere, shape1 = box. Output normal points box→sphere.
// ============================================================================
pub fn contact_sphere_box(
    sphere_origin: [f64; 3], sphere_radius: f64,
    box_extents: [f64; 3], box_rot: [f64; 4], box_pos: [f64; 3],
    contact_distance: f64,
) -> Option<GuContact> {
    // delta = sphere center relative to box center, then into box-local.
    let delta = v_sub(sphere_origin, box_pos);
    let mut d_rot = q_rotate_inv(box_rot, delta);

    // Clamp dRot against ±boxExtents.
    let mut outside = false;
    if d_rot[0] < -box_extents[0] { outside = true; d_rot[0] = -box_extents[0]; }
    else if d_rot[0] > box_extents[0] { outside = true; d_rot[0] =  box_extents[0]; }
    if d_rot[1] < -box_extents[1] { outside = true; d_rot[1] = -box_extents[1]; }
    else if d_rot[1] > box_extents[1] { outside = true; d_rot[1] =  box_extents[1]; }
    if d_rot[2] < -box_extents[2] { outside = true; d_rot[2] = -box_extents[2]; }
    else if d_rot[2] > box_extents[2] { outside = true; d_rot[2] =  box_extents[2]; }

    if outside {
        let mut point = q_rotate(box_rot, d_rot);
        let norm = v_sub(delta, point);
        let len_sq = v_len_sq(norm);
        let inflated = sphere_radius + contact_distance;
        if len_sq > inflated * inflated { return None; }
        let inv_len = if len_sq > 0.0 { 1.0 / len_sq.sqrt() } else { 0.0 };
        let normal = v_mul(norm, inv_len);
        let mut separation = inv_len * len_sq; // = sqrt(len_sq)
        point = v_add(point, box_pos);
        separation -= sphere_radius;
        Some(GuContact { point, normal, separation })
    } else {
        // Sphere center is inside the box.
        let abs_dr = [d_rot[0].abs(), d_rot[1].abs(), d_rot[2].abs()];
        let dist_to_surface = [
            box_extents[0] - abs_dr[0],
            box_extents[1] - abs_dr[1],
            box_extents[2] - abs_dr[2],
        ];

        let (loc_norm, mut separation) =
            if dist_to_surface[1] < dist_to_surface[0] {
                if dist_to_surface[1] < dist_to_surface[2] {
                    ([0.0, if d_rot[1] > 0.0 { 1.0 } else { -1.0 }, 0.0], -dist_to_surface[1])
                } else {
                    ([0.0, 0.0, if d_rot[2] > 0.0 { 1.0 } else { -1.0 }], -dist_to_surface[2])
                }
            } else if dist_to_surface[0] < dist_to_surface[2] {
                ([if d_rot[0] > 0.0 { 1.0 } else { -1.0 }, 0.0, 0.0], -dist_to_surface[0])
            } else {
                ([0.0, 0.0, if d_rot[2] > 0.0 { 1.0 } else { -1.0 }], -dist_to_surface[2])
            };
        let normal = q_rotate(box_rot, loc_norm);
        separation -= sphere_radius;
        Some(GuContact { point: sphere_origin, normal, separation })
    }
}

// ============================================================================
//  Cm::Matrix34 + box-vertex helpers — needed by GuContactBoxBox below.
// ============================================================================

/// `Cm::Matrix34` — 3×3 rotation columns + translation. Source:
/// `Source/Common/src/CmMatrix34.h`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CmMat34 {
    pub col0: [f64; 3],
    pub col1: [f64; 3],
    pub col2: [f64; 3],
    pub p: [f64; 3],
}

impl CmMat34 {
    pub fn from_transform(q: [f64; 4], p: [f64; 3]) -> Self {
        Self {
            col0: q_rotate(q, [1.0, 0.0, 0.0]),
            col1: q_rotate(q, [0.0, 1.0, 0.0]),
            col2: q_rotate(q, [0.0, 0.0, 1.0]),
            p,
        }
    }
    pub fn rotate_vec(&self, v: [f64; 3]) -> [f64; 3] {
        [self.col0[0]*v[0] + self.col1[0]*v[1] + self.col2[0]*v[2],
         self.col0[1]*v[0] + self.col1[1]*v[1] + self.col2[1]*v[2],
         self.col0[2]*v[0] + self.col1[2]*v[1] + self.col2[2]*v[2]]
    }
    pub fn transform(&self, v: [f64; 3]) -> [f64; 3] {
        v_add(self.rotate_vec(v), self.p)
    }
    /// `Cm::Matrix34::getInverseRT` — for rigid (R, p) the inverse is (Rᵀ, -Rᵀ·p).
    pub fn inverse_rt(&self) -> Self {
        let inv = Self {
            col0: [self.col0[0], self.col1[0], self.col2[0]],
            col1: [self.col0[1], self.col1[1], self.col2[1]],
            col2: [self.col0[2], self.col1[2], self.col2[2]],
            p: [0.0; 3],
        };
        let rp = inv.rotate_vec(self.p);
        Self { p: [-rp[0], -rp[1], -rp[2]], ..inv }
    }
}

impl std::ops::Mul for CmMat34 {
    type Output = CmMat34;
    fn mul(self, other: CmMat34) -> CmMat34 {
        CmMat34 {
            col0: self.rotate_vec(other.col0),
            col1: self.rotate_vec(other.col1),
            col2: self.rotate_vec(other.col2),
            p: v_add(self.rotate_vec(other.p), self.p),
        }
    }
}

// ============================================================================
//  GuContactBoxBox — Source: GeomUtils/src/contact/GuContactBoxBox.cpp:42-703
//
//  Three pieces:
//    1. doBoxBoxContactGeneration — 15-axis SAT (3 face A, 3 face B, 9 edges)
//    2. generateContacts — manifold via clip-other-box-against-reference-face
//    3. is_in_yz — quad-membership helper
// ============================================================================

const AXIS_A0: i32 = 0;
const AXIS_A1: i32 = 1;
const AXIS_A2: i32 = 2;
const AXIS_B0: i32 = 3;
const AXIS_B1: i32 = 4;
const AXIS_B2: i32 = 5;

#[derive(Default, Clone, Copy)]
struct VertexInfo {
    pos: [f64; 3],
    penetrate: bool,
    area: bool,
}

/// Source: GuContactBoxBox.cpp:120-161.
fn is_in_yz(y: f64, z: f64, face: &[VertexInfo; 4]) -> f64 {
    // 3+------+2
    //  |   *  |
    //  | (y,z)|
    // 0+------+1
    let mut prev_y = face[3].pos[1];
    let mut prev_z = face[3].pos[2];
    for i in 0..4 {
        let cur_y = face[i].pos[1];
        let cur_z = face[i].pos[2];
        if (cur_y - prev_y) * (z - prev_z) - (cur_z - prev_z) * (y - prev_y) >= 0.0 {
            return -1.0;
        }
        prev_y = cur_y;
        prev_z = cur_z;
    }
    let mut x = face[0].pos[0];
    let ay = y - face[0].pos[1];
    let az = z - face[0].pos[2];
    let mut b = v_sub(face[1].pos, face[0].pos);
    x += b[0] * (ay*b[1] + az*b[2]) / v_len_sq(b);
    b = v_sub(face[3].pos, face[0].pos);
    x += b[0] * (ay*b[1] + az*b[2]) / v_len_sq(b);
    x
}

/// Source: GuContactBoxBox.cpp:169-410.
#[allow(clippy::too_many_arguments)]
fn generate_contacts_box_box(
    contact_buffer: &mut GuContactBuffer,
    contact_normal: [f64; 3],
    mut y1: f64, mut z1: f64,
    box2: [f64; 3],
    transform0: CmMat34,
    transform1: CmMat34,
    contact_distance: f64,
) -> i32 {
    contact_buffer.reset();
    y1 += contact_distance;
    z1 += contact_distance;

    let trans1to0 = transform0.inverse_rt() * transform1;

    let mut vtx = [VertexInfo::default(); 8];
    let ex = v_mul(trans1to0.col0, box2[0]);
    let ey = v_mul(trans1to0.col1, box2[1]);
    let ez = v_mul(trans1to0.col2, box2[2]);

    let p_minus_ex = v_sub(trans1to0.p, ex);
    let p_plus_ex = v_add(trans1to0.p, ex);
    vtx[0].pos = p_minus_ex; vtx[2].pos = p_minus_ex; vtx[4].pos = p_minus_ex; vtx[6].pos = p_minus_ex;
    vtx[1].pos = p_plus_ex;  vtx[3].pos = p_plus_ex;  vtx[5].pos = p_plus_ex;  vtx[7].pos = p_plus_ex;

    let mut e = v_add(ey, ez);
    vtx[0].pos = v_sub(vtx[0].pos, e);
    vtx[1].pos = v_sub(vtx[1].pos, e);
    vtx[6].pos = v_add(vtx[6].pos, e);
    vtx[7].pos = v_add(vtx[7].pos, e);

    e = v_sub(ey, ez);
    vtx[2].pos = v_add(vtx[2].pos, e);
    vtx[3].pos = v_add(vtx[3].pos, e);
    vtx[4].pos = v_sub(vtx[4].pos, e);
    vtx[5].pos = v_sub(vtx[5].pos, e);

    // Vertex info — lines 227-262.
    for i in 0..8 {
        if vtx[i].pos[0] < -contact_distance {
            vtx[i].area = false;
            vtx[i].penetrate = false;
            continue;
        }
        vtx[i].penetrate = true;
        if vtx[i].pos[1].abs() <= y1 && vtx[i].pos[2].abs() <= z1 {
            vtx[i].area = true;
            contact_buffer.add(vtx[i].pos, contact_normal, -vtx[i].pos[0]);
        } else {
            vtx[i].area = false;
        }
    }

    // 12 edges — lines 264-379.
    let indices: [usize; 24] = [0,1, 1,3, 3,2, 2,0, 4,5, 5,7, 7,6, 6,4, 0,4, 1,5, 2,6, 3,7];
    let mut idx = 0;
    while idx < 24 {
        let i1 = indices[idx]; let i2 = indices[idx + 1];
        idx += 2;
        let mut p1 = vtx[i1];
        let mut p2 = vtx[i2];
        if !(p1.penetrate || p2.penetrate) { continue; }

        if !p1.area || !p2.area {
            // Test y
            if p1.pos[1] > p2.pos[1] { std::mem::swap(&mut p1, &mut p2); }
            if p1.pos[1] < y1 && p2.pos[1] >= y1 {
                let a = (y1 - p1.pos[1]) / (p2.pos[1] - p1.pos[1]);
                let z = p1.pos[2] + (p2.pos[2] - p1.pos[2]) * a;
                if z.abs() <= z1 {
                    let x = p1.pos[0] + (p2.pos[0] - p1.pos[0]) * a;
                    if x + contact_distance >= 0.0 {
                        contact_buffer.add([x, y1, z], contact_normal, -x);
                    }
                }
            }
            if p1.pos[1] < -y1 && p2.pos[1] >= -y1 {
                let a = (-y1 - p1.pos[1]) / (p2.pos[1] - p1.pos[1]);
                let z = p1.pos[2] + (p2.pos[2] - p1.pos[2]) * a;
                if z.abs() <= z1 {
                    let x = p1.pos[0] + (p2.pos[0] - p1.pos[0]) * a;
                    if x + contact_distance >= 0.0 {
                        contact_buffer.add([x, -y1, z], contact_normal, -x);
                    }
                }
            }
            // Test z
            if p1.pos[2] > p2.pos[2] { std::mem::swap(&mut p1, &mut p2); }
            if p1.pos[2] < z1 && p2.pos[2] >= z1 {
                let a = (z1 - p1.pos[2]) / (p2.pos[2] - p1.pos[2]);
                let y = p1.pos[1] + (p2.pos[1] - p1.pos[1]) * a;
                if y.abs() <= y1 {
                    let x = p1.pos[0] + (p2.pos[0] - p1.pos[0]) * a;
                    if x + contact_distance >= 0.0 {
                        contact_buffer.add([x, y, z1], contact_normal, -x);
                    }
                }
            }
            if p1.pos[2] < -z1 && p2.pos[2] >= -z1 {
                let a = (-z1 - p1.pos[2]) / (p2.pos[2] - p1.pos[2]);
                let y = p1.pos[1] + (p2.pos[1] - p1.pos[1]) * a;
                if y.abs() <= y1 {
                    let x = p1.pos[0] + (p2.pos[0] - p1.pos[0]) * a;
                    if x + contact_distance >= 0.0 {
                        contact_buffer.add([x, y, -z1], contact_normal, -x);
                    }
                }
            }
        }

        // Plane-crossing case (lines 360-377). Use original (unswapped) p1/p2.
        let pp1 = vtx[i1]; let pp2 = vtx[i2];
        if (!pp1.penetrate && !pp2.area) || (!pp2.penetrate && !pp1.area) {
            let a = -pp1.pos[0] / (pp2.pos[0] - pp1.pos[0]);
            let y = pp1.pos[1] + (pp2.pos[1] - pp1.pos[1]) * a;
            if y.abs() <= y1 {
                let z = pp1.pos[2] + (pp2.pos[2] - pp1.pos[2]) * a;
                if z.abs() <= z1 {
                    contact_buffer.add([0.0, y, z], contact_normal, 0.0);
                }
            }
        }
    }

    // 6 quads — lines 381-400.
    let face: [[usize; 4]; 6] = [
        [0,1,3,2], [1,5,7,3], [5,4,6,7],
        [4,0,2,6], [2,3,7,6], [0,4,5,1],
    ];
    let mut addflg = 0u32;
    for face_idx in 0..6 {
        if addflg == 0x0f { break; }
        let p = face[face_idx];
        let q = [vtx[p[0]], vtx[p[1]], vtx[p[2]], vtx[p[3]]];
        if !(q[0].penetrate && q[1].penetrate && q[2].penetrate && q[3].penetrate) { continue; }
        if !(!q[0].area || !q[1].area || !q[2].area || !q[3].area) { continue; }
        if (addflg & 1) == 0 {
            let x = is_in_yz(-y1, -z1, &q);
            if x >= 0.0 { addflg |= 1; contact_buffer.add([x, -y1, -z1], contact_normal, -x); }
        }
        if (addflg & 2) == 0 {
            let x = is_in_yz( y1, -z1, &q);
            if x >= 0.0 { addflg |= 2; contact_buffer.add([x,  y1, -z1], contact_normal, -x); }
        }
        if (addflg & 4) == 0 {
            let x = is_in_yz(-y1,  z1, &q);
            if x >= 0.0 { addflg |= 4; contact_buffer.add([x, -y1,  z1], contact_normal, -x); }
        }
        if (addflg & 8) == 0 {
            let x = is_in_yz( y1,  z1, &q);
            if x >= 0.0 { addflg |= 8; contact_buffer.add([x,  y1,  z1], contact_normal, -x); }
        }
    }

    // Local→world (line 405).
    for c in contact_buffer.contacts.iter_mut() {
        c.point = transform0.transform(c.point);
    }
    contact_buffer.contacts.len() as i32
}

/// Source: GuContactBoxBox.cpp:412-703 — 15-axis SAT.
#[allow(clippy::too_many_arguments)]
pub fn contact_box_box(
    buffer: &mut GuContactBuffer,
    extents0: [f64; 3], rot0: [f64; 4], pos0: [f64; 3],
    extents1: [f64; 3], rot1: [f64; 4], pos1: [f64; 3],
    pair_data: &mut u8,
    contact_distance: f64,
) -> i32 {
    let transform0 = CmMat34::from_transform(rot0, pos0);
    let transform1 = CmMat34::from_transform(rot1, pos1);

    let mut aaf_c = [[0.0f64; 3]; 3];
    let mut aaf_abs_c = [[0.0f64; 3]; 3];
    let mut af_ad = [0.0f64; 3];
    let mut d1 = [0.0f64; 6];
    let mut overlap = [0.0f64; 6];

    let k_d = v_sub(transform1.p, transform0.p);
    let axis00 = transform0.col0; let axis01 = transform0.col1; let axis02 = transform0.col2;
    let axis10 = transform1.col0; let axis11 = transform1.col1; let axis12 = transform1.col2;

    // Class I — face A.
    aaf_c[0][0] = v_dot(axis00, axis10); aaf_c[0][1] = v_dot(axis00, axis11); aaf_c[0][2] = v_dot(axis00, axis12);
    af_ad[0] = v_dot(axis00, k_d);
    aaf_abs_c[0][0] = 1e-6 + aaf_c[0][0].abs();
    aaf_abs_c[0][1] = 1e-6 + aaf_c[0][1].abs();
    aaf_abs_c[0][2] = 1e-6 + aaf_c[0][2].abs();
    d1[AXIS_A0 as usize] = af_ad[0];
    let mut d0 = extents0[0] + extents1[0]*aaf_abs_c[0][0] + extents1[1]*aaf_abs_c[0][1] + extents1[2]*aaf_abs_c[0][2];
    overlap[AXIS_A0 as usize] = d0 - d1[AXIS_A0 as usize].abs() + contact_distance;
    if overlap[AXIS_A0 as usize] < 0.0 { return 0; }

    aaf_c[1][0] = v_dot(axis01, axis10); aaf_c[1][1] = v_dot(axis01, axis11); aaf_c[1][2] = v_dot(axis01, axis12);
    af_ad[1] = v_dot(axis01, k_d);
    aaf_abs_c[1][0] = 1e-6 + aaf_c[1][0].abs();
    aaf_abs_c[1][1] = 1e-6 + aaf_c[1][1].abs();
    aaf_abs_c[1][2] = 1e-6 + aaf_c[1][2].abs();
    d1[AXIS_A1 as usize] = af_ad[1];
    d0 = extents0[1] + extents1[0]*aaf_abs_c[1][0] + extents1[1]*aaf_abs_c[1][1] + extents1[2]*aaf_abs_c[1][2];
    overlap[AXIS_A1 as usize] = d0 - d1[AXIS_A1 as usize].abs() + contact_distance;
    if overlap[AXIS_A1 as usize] < 0.0 { return 0; }

    aaf_c[2][0] = v_dot(axis02, axis10); aaf_c[2][1] = v_dot(axis02, axis11); aaf_c[2][2] = v_dot(axis02, axis12);
    af_ad[2] = v_dot(axis02, k_d);
    aaf_abs_c[2][0] = 1e-6 + aaf_c[2][0].abs();
    aaf_abs_c[2][1] = 1e-6 + aaf_c[2][1].abs();
    aaf_abs_c[2][2] = 1e-6 + aaf_c[2][2].abs();
    d1[AXIS_A2 as usize] = af_ad[2];
    d0 = extents0[2] + extents1[0]*aaf_abs_c[2][0] + extents1[1]*aaf_abs_c[2][1] + extents1[2]*aaf_abs_c[2][2];
    overlap[AXIS_A2 as usize] = d0 - d1[AXIS_A2 as usize].abs() + contact_distance;
    if overlap[AXIS_A2 as usize] < 0.0 { return 0; }

    // Class II — face B.
    d1[AXIS_B0 as usize] = v_dot(axis10, k_d);
    d0 = extents1[0] + extents0[0]*aaf_abs_c[0][0] + extents0[1]*aaf_abs_c[1][0] + extents0[2]*aaf_abs_c[2][0];
    overlap[AXIS_B0 as usize] = d0 - d1[AXIS_B0 as usize].abs() + contact_distance;
    if overlap[AXIS_B0 as usize] < 0.0 { return 0; }

    d1[AXIS_B1 as usize] = v_dot(axis11, k_d);
    d0 = extents1[1] + extents0[0]*aaf_abs_c[0][1] + extents0[1]*aaf_abs_c[1][1] + extents0[2]*aaf_abs_c[2][1];
    overlap[AXIS_B1 as usize] = d0 - d1[AXIS_B1 as usize].abs() + contact_distance;
    if overlap[AXIS_B1 as usize] < 0.0 { return 0; }

    d1[AXIS_B2 as usize] = v_dot(axis12, k_d);
    d0 = extents1[2] + extents0[0]*aaf_abs_c[0][2] + extents0[1]*aaf_abs_c[1][2] + extents0[2]*aaf_abs_c[2][2];
    overlap[AXIS_B2 as usize] = d0 - d1[AXIS_B2 as usize].abs() + contact_distance;
    if overlap[AXIS_B2 as usize] < 0.0 { return 0; }

    // Class III — 9 edge crosses (only when previously separated).
    if *pair_data == 0 {
        let tests = [
            ( af_ad[2]*aaf_c[1][0] - af_ad[1]*aaf_c[2][0],
              extents0[1]*aaf_abs_c[2][0] + extents0[2]*aaf_abs_c[1][0] + extents1[1]*aaf_abs_c[0][2] + extents1[2]*aaf_abs_c[0][1]),
            ( af_ad[2]*aaf_c[1][1] - af_ad[1]*aaf_c[2][1],
              extents0[1]*aaf_abs_c[2][1] + extents0[2]*aaf_abs_c[1][1] + extents1[0]*aaf_abs_c[0][2] + extents1[2]*aaf_abs_c[0][0]),
            ( af_ad[2]*aaf_c[1][2] - af_ad[1]*aaf_c[2][2],
              extents0[1]*aaf_abs_c[2][2] + extents0[2]*aaf_abs_c[1][2] + extents1[0]*aaf_abs_c[0][1] + extents1[1]*aaf_abs_c[0][0]),
            ( af_ad[0]*aaf_c[2][0] - af_ad[2]*aaf_c[0][0],
              extents0[0]*aaf_abs_c[2][0] + extents0[2]*aaf_abs_c[0][0] + extents1[1]*aaf_abs_c[1][2] + extents1[2]*aaf_abs_c[1][1]),
            ( af_ad[0]*aaf_c[2][1] - af_ad[2]*aaf_c[0][1],
              extents0[0]*aaf_abs_c[2][1] + extents0[2]*aaf_abs_c[0][1] + extents1[0]*aaf_abs_c[1][2] + extents1[2]*aaf_abs_c[1][0]),
            ( af_ad[0]*aaf_c[2][2] - af_ad[2]*aaf_c[0][2],
              extents0[0]*aaf_abs_c[2][2] + extents0[2]*aaf_abs_c[0][2] + extents1[0]*aaf_abs_c[1][1] + extents1[1]*aaf_abs_c[1][0]),
            ( af_ad[1]*aaf_c[0][0] - af_ad[0]*aaf_c[1][0],
              extents0[0]*aaf_abs_c[1][0] + extents0[1]*aaf_abs_c[0][0] + extents1[1]*aaf_abs_c[2][2] + extents1[2]*aaf_abs_c[2][1]),
            ( af_ad[1]*aaf_c[0][1] - af_ad[0]*aaf_c[1][1],
              extents0[0]*aaf_abs_c[1][1] + extents0[1]*aaf_abs_c[0][1] + extents1[0]*aaf_abs_c[2][2] + extents1[2]*aaf_abs_c[2][0]),
            ( af_ad[1]*aaf_c[0][2] - af_ad[0]*aaf_c[1][2],
              extents0[0]*aaf_abs_c[1][2] + extents0[1]*aaf_abs_c[0][2] + extents1[0]*aaf_abs_c[2][1] + extents1[1]*aaf_abs_c[2][0]),
        ];
        for (d, base) in tests.iter() {
            if d.abs() > contact_distance + base { return 0; }
        }
    }

    // Warm-start bias.
    if *pair_data != 0 {
        overlap[(*pair_data - 1) as usize] *= 0.999;
    }

    let mut minimum = f64::MAX;
    let mut min_index = 0i32;
    for i in (AXIS_A0 as usize)..6 {
        let v = overlap[i];
        if v >= 0.0 && v < minimum { minimum = v; min_index = i as i32; }
    }
    *pair_data = (min_index + 1) as u8;

    let sign = d1[min_index as usize] < 0.0;
    let mut trs = CmMat34::default();
    let ctc_nrm: [f64; 3];

    match min_index {
        x if x == AXIS_A0 => {
            if sign {
                ctc_nrm = axis00;
                trs.col0 = axis00; trs.col1 = axis01; trs.col2 = axis02;
                trs.p = v_sub(transform0.p, v_mul(axis00, extents0[0]));
            } else {
                ctc_nrm = v_neg(axis00);
                trs.col0 = v_neg(axis00); trs.col1 = v_neg(axis01); trs.col2 = axis02;
                trs.p = v_add(transform0.p, v_mul(axis00, extents0[0]));
            }
            generate_contacts_box_box(buffer, ctc_nrm, extents0[1], extents0[2], extents1, trs, transform1, contact_distance)
        }
        x if x == AXIS_A1 => {
            trs.col2 = axis00;
            if sign {
                ctc_nrm = axis01;
                trs.col0 = axis01; trs.col1 = axis02;
                trs.p = v_sub(transform0.p, v_mul(axis01, extents0[1]));
            } else {
                ctc_nrm = v_neg(axis01);
                trs.col0 = v_neg(axis01); trs.col1 = v_neg(axis02);
                trs.p = v_add(transform0.p, v_mul(axis01, extents0[1]));
            }
            generate_contacts_box_box(buffer, ctc_nrm, extents0[2], extents0[0], extents1, trs, transform1, contact_distance)
        }
        x if x == AXIS_A2 => {
            trs.col2 = axis01;
            if sign {
                ctc_nrm = axis02;
                trs.col0 = axis02; trs.col1 = axis00;
                trs.p = v_sub(transform0.p, v_mul(axis02, extents0[2]));
            } else {
                ctc_nrm = v_neg(axis02);
                trs.col0 = v_neg(axis02); trs.col1 = v_neg(axis00);
                trs.p = v_add(transform0.p, v_mul(axis02, extents0[2]));
            }
            generate_contacts_box_box(buffer, ctc_nrm, extents0[0], extents0[1], extents1, trs, transform1, contact_distance)
        }
        x if x == AXIS_B0 => {
            if sign {
                ctc_nrm = axis10;
                trs.col0 = v_neg(axis10); trs.col1 = v_neg(axis11); trs.col2 = axis12;
                trs.p = v_add(transform1.p, v_mul(axis10, extents1[0]));
            } else {
                ctc_nrm = v_neg(axis10);
                trs.col0 = axis10; trs.col1 = axis11; trs.col2 = axis12;
                trs.p = v_sub(transform1.p, v_mul(axis10, extents1[0]));
            }
            generate_contacts_box_box(buffer, ctc_nrm, extents1[1], extents1[2], extents0, trs, transform0, contact_distance)
        }
        x if x == AXIS_B1 => {
            trs.col2 = axis10;
            if sign {
                ctc_nrm = axis11;
                trs.col0 = v_neg(axis11); trs.col1 = v_neg(axis12);
                trs.p = v_add(transform1.p, v_mul(axis11, extents1[1]));
            } else {
                ctc_nrm = v_neg(axis11);
                trs.col0 = axis11; trs.col1 = axis12; trs.col2 = axis10;
                trs.p = v_sub(transform1.p, v_mul(axis11, extents1[1]));
            }
            generate_contacts_box_box(buffer, ctc_nrm, extents1[2], extents1[0], extents0, trs, transform0, contact_distance)
        }
        x if x == AXIS_B2 => {
            trs.col2 = axis11;
            if sign {
                ctc_nrm = axis12;
                trs.col0 = v_neg(axis12); trs.col1 = v_neg(axis10);
                trs.p = v_add(transform1.p, v_mul(axis12, extents1[2]));
            } else {
                ctc_nrm = v_neg(axis12);
                trs.col0 = axis12; trs.col1 = axis10;
                trs.p = v_sub(transform1.p, v_mul(axis12, extents1[2]));
            }
            generate_contacts_box_box(buffer, ctc_nrm, extents1[0], extents1[1], extents0, trs, transform0, contact_distance)
        }
        _ => 0,
    }
}

// ============================================================================
//  Capsule helpers
//  Source: GeomUtils/src/GuInternal.h:60-76 (getCapsuleHalfHeightVector)
//          GeomUtils/src/distance/GuDistancePointSegment.h:41-69
//          GeomUtils/src/distance/GuDistanceSegmentSegment.cpp:41-339
// ============================================================================

/// PhysX 3.x convention: capsule axis is body local +X.
#[inline]
pub fn get_capsule_half_height_vector(rot: [f64; 4], half_height: f64) -> [f64; 3] {
    v_mul(q_rotate(rot, [1.0, 0.0, 0.0]), half_height)
}

/// distancePointSegmentSquaredInternal — Source: GuDistancePointSegment.h:41-69.
pub fn distance_point_segment_sq_internal(p0: [f64; 3], dir: [f64; 3], point: [f64; 3]) -> (f64, f64) {
    let mut diff = v_sub(point, p0);
    let mut f_t = v_dot(diff, dir);
    if f_t <= 0.0 {
        f_t = 0.0;
    } else {
        let sqr_len = v_dot(dir, dir);
        if f_t >= sqr_len {
            f_t = 1.0;
            diff = v_sub(diff, dir);
        } else {
            f_t /= sqr_len;
            diff = v_sub(diff, v_mul(dir, f_t));
        }
    }
    (v_len_sq(diff), f_t)
}

#[inline]
pub fn distance_point_segment_sq(p0: [f64; 3], p1: [f64; 3], point: [f64; 3]) -> (f64, f64) {
    distance_point_segment_sq_internal(p0, v_sub(p1, p0), point)
}

const DSS_ZERO_TOLERANCE: f64 = 1e-6;

/// Source: GuDistanceSegmentSegment.cpp:41-339 (Wild Magic 9-region).
/// Inputs: each segment as (origin = midpoint, dir = unit, extent = halfLen).
/// Outputs: param0/param1 in [-extent, +extent] each.
#[allow(clippy::too_many_arguments)]
pub fn distance_segment_segment_squared_centered(
    origin0: [f64; 3], dir0: [f64; 3], extent0: f64,
    origin1: [f64; 3], dir1: [f64; 3], extent1: f64,
) -> (f64, f64, f64) {
    let k_diff = v_sub(origin0, origin1);
    let f_a01 = -v_dot(dir0, dir1);
    let f_b0 = v_dot(k_diff, dir0);
    let f_b1 = -v_dot(k_diff, dir1);
    let f_c = v_dot(k_diff, k_diff);
    let f_det = (1.0 - f_a01 * f_a01).abs();
    let mut f_s0;
    let mut f_s1;
    let f_sqr_dist;

    if f_det >= DSS_ZERO_TOLERANCE {
        f_s0 = f_a01 * f_b1 - f_b0;
        f_s1 = f_a01 * f_b0 - f_b1;
        let f_ext_det0 = extent0 * f_det;
        let f_ext_det1 = extent1 * f_det;

        if f_s0 >= -f_ext_det0 {
            if f_s0 <= f_ext_det0 {
                if f_s1 >= -f_ext_det1 {
                    if f_s1 <= f_ext_det1 {
                        // region 0 (interior)
                        let f_inv_det = 1.0 / f_det;
                        f_s0 *= f_inv_det;
                        f_s1 *= f_inv_det;
                        f_sqr_dist = f_s0 * (f_s0 + f_a01 * f_s1 + 2.0 * f_b0)
                            + f_s1 * (f_a01 * f_s0 + f_s1 + 2.0 * f_b1) + f_c;
                    } else {
                        // region 3
                        f_s1 = extent1;
                        let f_tmp_s0 = -(f_a01 * f_s1 + f_b0);
                        if f_tmp_s0 < -extent0 {
                            f_s0 = -extent0;
                            f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                        } else if f_tmp_s0 <= extent0 {
                            f_s0 = f_tmp_s0;
                            f_sqr_dist = -f_s0 * f_s0 + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                        } else {
                            f_s0 = extent0;
                            f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                        }
                    }
                } else {
                    // region 7
                    f_s1 = -extent1;
                    let f_tmp_s0 = -(f_a01 * f_s1 + f_b0);
                    if f_tmp_s0 < -extent0 {
                        f_s0 = -extent0;
                        f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                    } else if f_tmp_s0 <= extent0 {
                        f_s0 = f_tmp_s0;
                        f_sqr_dist = -f_s0 * f_s0 + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                    } else {
                        f_s0 = extent0;
                        f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                    }
                }
            } else if f_s1 >= -f_ext_det1 {
                if f_s1 <= f_ext_det1 {
                    // region 1
                    f_s0 = extent0;
                    let f_tmp_s1 = -(f_a01 * f_s0 + f_b1);
                    if f_tmp_s1 < -extent1 {
                        f_s1 = -extent1;
                        f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    } else if f_tmp_s1 <= extent1 {
                        f_s1 = f_tmp_s1;
                        f_sqr_dist = -f_s1 * f_s1 + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    } else {
                        f_s1 = extent1;
                        f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    }
                } else {
                    // region 2
                    f_s1 = extent1;
                    let f_tmp_s0 = -(f_a01 * f_s1 + f_b0);
                    if f_tmp_s0 < -extent0 {
                        f_s0 = -extent0;
                        f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                    } else if f_tmp_s0 <= extent0 {
                        f_s0 = f_tmp_s0;
                        f_sqr_dist = -f_s0 * f_s0 + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                    } else {
                        f_s0 = extent0;
                        let f_tmp_s1 = -(f_a01 * f_s0 + f_b1);
                        if f_tmp_s1 < -extent1 {
                            f_s1 = -extent1;
                            f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                        } else if f_tmp_s1 <= extent1 {
                            f_s1 = f_tmp_s1;
                            f_sqr_dist = -f_s1 * f_s1 + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                        } else {
                            f_s1 = extent1;
                            f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                        }
                    }
                }
            } else {
                // region 8
                f_s1 = -extent1;
                let f_tmp_s0 = -(f_a01 * f_s1 + f_b0);
                if f_tmp_s0 < -extent0 {
                    f_s0 = -extent0;
                    f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                } else if f_tmp_s0 <= extent0 {
                    f_s0 = f_tmp_s0;
                    f_sqr_dist = -f_s0 * f_s0 + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                } else {
                    f_s0 = extent0;
                    let f_tmp_s1 = -(f_a01 * f_s0 + f_b1);
                    if f_tmp_s1 > extent1 {
                        f_s1 = extent1;
                        f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    } else if f_tmp_s1 >= -extent1 {
                        f_s1 = f_tmp_s1;
                        f_sqr_dist = -f_s1 * f_s1 + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    } else {
                        f_s1 = -extent1;
                        f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    }
                }
            }
        } else if f_s1 >= -f_ext_det1 {
            if f_s1 <= f_ext_det1 {
                // region 5
                f_s0 = -extent0;
                let f_tmp_s1 = -(f_a01 * f_s0 + f_b1);
                if f_tmp_s1 < -extent1 {
                    f_s1 = -extent1;
                    f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                } else if f_tmp_s1 <= extent1 {
                    f_s1 = f_tmp_s1;
                    f_sqr_dist = -f_s1 * f_s1 + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                } else {
                    f_s1 = extent1;
                    f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                }
            } else {
                // region 4
                f_s1 = extent1;
                let f_tmp_s0 = -(f_a01 * f_s1 + f_b0);
                if f_tmp_s0 > extent0 {
                    f_s0 = extent0;
                    f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                } else if f_tmp_s0 >= -extent0 {
                    f_s0 = f_tmp_s0;
                    f_sqr_dist = -f_s0 * f_s0 + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
                } else {
                    f_s0 = -extent0;
                    let f_tmp_s1 = -(f_a01 * f_s0 + f_b1);
                    if f_tmp_s1 < -extent1 {
                        f_s1 = -extent1;
                        f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    } else if f_tmp_s1 <= extent1 {
                        f_s1 = f_tmp_s1;
                        f_sqr_dist = -f_s1 * f_s1 + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    } else {
                        f_s1 = extent1;
                        f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                    }
                }
            }
        } else {
            // region 6
            f_s1 = -extent1;
            let f_tmp_s0 = -(f_a01 * f_s1 + f_b0);
            if f_tmp_s0 > extent0 {
                f_s0 = extent0;
                f_sqr_dist = f_s0 * (f_s0 - 2.0 * f_tmp_s0) + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
            } else if f_tmp_s0 >= -extent0 {
                f_s0 = f_tmp_s0;
                f_sqr_dist = -f_s0 * f_s0 + f_s1 * (f_s1 + 2.0 * f_b1) + f_c;
            } else {
                f_s0 = -extent0;
                let f_tmp_s1 = -(f_a01 * f_s0 + f_b1);
                if f_tmp_s1 < -extent1 {
                    f_s1 = -extent1;
                    f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                } else if f_tmp_s1 <= extent1 {
                    f_s1 = f_tmp_s1;
                    f_sqr_dist = -f_s1 * f_s1 + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                } else {
                    f_s1 = extent1;
                    f_sqr_dist = f_s1 * (f_s1 - 2.0 * f_tmp_s1) + f_s0 * (f_s0 + 2.0 * f_b0) + f_c;
                }
            }
        }
    } else {
        // Parallel.
        let f_e0p_e1 = extent0 + extent1;
        let f_sign = if f_a01 > 0.0 { -1.0 } else { 1.0 };
        let b0_avr = 0.5 * (f_b0 - f_sign * f_b1);
        let mut f_lambda = -b0_avr;
        if f_lambda < -f_e0p_e1 { f_lambda = -f_e0p_e1; }
        else if f_lambda > f_e0p_e1 { f_lambda = f_e0p_e1; }
        f_s1 = -f_sign * f_lambda * extent1 / f_e0p_e1;
        f_s0 = f_lambda + f_sign * f_s1;
        f_sqr_dist = f_lambda * (f_lambda + 2.0 * b0_avr) + f_c;
    }
    (f_sqr_dist.max(0.0), f_s0, f_s1)
}

/// Wrapper from the (origin = p0, extent_vec = p1 - p0) form to centered form.
/// Returns (sqrDist, param0_in_0_1, param1_in_0_1).
pub fn distance_segment_segment_squared(
    origin0: [f64; 3], extent_vec0: [f64; 3],
    origin1: [f64; 3], extent_vec1: [f64; 3],
) -> (f64, f64, f64) {
    let mut dir0 = extent_vec0;
    let center0 = v_add(origin0, v_mul(extent_vec0, 0.5));
    let mut length0 = v_len(extent_vec0);
    let b0 = length0 != 0.0;
    let one_over_l0 = if b0 { 1.0 / length0 } else { 0.0 };
    if b0 { dir0 = v_mul(dir0, one_over_l0); length0 *= 0.5; }

    let mut dir1 = extent_vec1;
    let center1 = v_add(origin1, v_mul(extent_vec1, 0.5));
    let mut length1 = v_len(extent_vec1);
    let b1 = length1 != 0.0;
    let one_over_l1 = if b1 { 1.0 / length1 } else { 0.0 };
    if b1 { dir1 = v_mul(dir1, one_over_l1); length1 *= 0.5; }

    let (d2, p0, p1) = distance_segment_segment_squared_centered(
        center0, dir0, length0, center1, dir1, length1,
    );
    let r0 = if b0 { (length0 + p0) * one_over_l0 } else { 0.0 };
    let r1 = if b1 { (length1 + p1) * one_over_l1 } else { 0.0 };
    (d2, r0, r1)
}

// ============================================================================
//  GuContactSphereCapsule — Source: GuContactSphereCapsule.cpp:40-80
//  shape0 = sphere, shape1 = capsule.
// ============================================================================
pub fn contact_sphere_capsule(
    sphere_center: [f64; 3], sphere_radius: f64,
    capsule_pos: [f64; 3], capsule_rot: [f64; 4], capsule_half_height: f64, capsule_radius: f64,
    contact_distance: f64,
) -> Option<GuContact> {
    let hh = get_capsule_half_height_vector(capsule_rot, capsule_half_height);
    let seg_p0 = hh;
    let seg_p1 = v_neg(hh);
    let sphere_in_caps = v_sub(sphere_center, capsule_pos);

    let radius_sum = sphere_radius + capsule_radius;
    let inflated = radius_sum + contact_distance;
    let (sq_dist, u) = distance_point_segment_sq(seg_p0, seg_p1, sphere_in_caps);
    if sq_dist >= inflated * inflated { return None; }

    let closest = [
        seg_p0[0] + u * (seg_p1[0] - seg_p0[0]),
        seg_p0[1] + u * (seg_p1[1] - seg_p0[1]),
        seg_p0[2] + u * (seg_p1[2] - seg_p0[2]),
    ];
    let n = v_sub(sphere_in_caps, closest);
    let len_sq = v_len_sq(n);
    let normal = if len_sq == 0.0 {
        [1.0, 0.0, 0.0]
    } else {
        v_mul(n, 1.0 / len_sq.sqrt())
    };
    let point = [
        sphere_in_caps[0] + capsule_pos[0] - normal[0] * sphere_radius,
        sphere_in_caps[1] + capsule_pos[1] - normal[1] * sphere_radius,
        sphere_in_caps[2] + capsule_pos[2] - normal[2] * sphere_radius,
    ];
    Some(GuContact { point, normal, separation: sq_dist.sqrt() - radius_sum })
}

// ============================================================================
//  GuContactCapsuleCapsule — Source: GuContactCapsuleCapsule.cpp:42-153
//  Emits 1 or 2 contact points (parallel-axis case generates 2).
// ============================================================================
#[allow(clippy::too_many_arguments)]
pub fn contact_capsule_capsule(
    buffer: &mut GuContactBuffer,
    pos0: [f64; 3], rot0: [f64; 4], half_height0: f64, radius0: f64,
    pos1: [f64; 3], rot1: [f64; 4], half_height1: f64, radius1: f64,
    contact_distance: f64,
) -> i32 {
    let hh0 = get_capsule_half_height_vector(rot0, half_height0);
    let hh1 = get_capsule_half_height_vector(rot1, half_height1);
    let delta = v_sub(pos1, pos0);
    let seg_p0_a = hh0;
    let seg_p1_a = v_neg(hh0);
    let seg_p0_b = v_add(hh1, delta);
    let seg_p1_b = v_add(v_neg(hh1), delta);
    let dir0 = v_mul(hh0, -2.0);
    let dir1 = v_mul(hh1, -2.0);

    let (sq_dist, s, t) = distance_segment_segment_squared(
        seg_p0_a, v_sub(seg_p1_a, seg_p0_a),
        seg_p0_b, v_sub(seg_p1_b, seg_p0_b),
    );
    let radius_sum = radius0 + radius1;
    let inflated = radius_sum + contact_distance;
    let inflated_sq = inflated * inflated;
    if sq_dist >= inflated_sq { return 0; }

    let mut seg_len = [v_len(dir0), v_len(dir1)];
    let mut dir_norm = [dir0, dir1];
    if seg_len[0] != 0.0 { dir_norm[0] = v_mul(dir_norm[0], 1.0 / seg_len[0]); }
    if seg_len[1] != 0.0 { dir_norm[1] = v_mul(dir_norm[1], 1.0 / seg_len[1]); }
    let _ = &mut seg_len; // silence unused-mut

    let count_before = buffer.count();
    let segp0 = [seg_p0_a, seg_p0_b];
    let segp1 = [seg_p1_a, seg_p1_b];
    let radii = [radius0, radius1];

    if v_dot(dir_norm[0], dir_norm[1]).abs() > 0.9998 {
        let mut num_cons = 0;
        let seg_len_eps = [seg_len[0] * 0.001, seg_len[1] * 0.001];
        for dest in 0..2 {
            for start_end in 0..2 {
                let src = 1 - dest;
                let mut pos = [[0.0f64; 3]; 2];
                pos[dest] = if start_end == 1 { segp1[src] } else { segp0[src] };
                let p = v_dot(dir_norm[dest], v_sub(pos[dest], segp0[dest]));
                if p >= -seg_len_eps[dest] && p <= seg_len[dest] + seg_len_eps[dest] {
                    pos[src] = [
                        p * dir_norm[dest][0] + segp0[dest][0],
                        p * dir_norm[dest][1] + segp0[dest][1],
                        p * dir_norm[dest][2] + segp0[dest][2],
                    ];
                    let normal = v_sub(pos[1], pos[0]);
                    let normal_len_sq = v_len_sq(normal);
                    if normal_len_sq > 1e-6 && normal_len_sq < inflated_sq {
                        let distance = normal_len_sq.sqrt();
                        let normal = v_mul(normal, 1.0 / distance);
                        let src_radius = radii[src];
                        let mut point = v_sub(pos[1], v_mul(normal, src_radius));
                        point = v_add(point, pos0);
                        buffer.add(point, normal, distance - radius_sum);
                        num_cons += 1;
                    }
                }
            }
        }
        if num_cons > 0 { return (buffer.count() - count_before) as i32; }
    }

    // Single-contact path.
    let pos1_pt = [
        seg_p0_a[0] + s * (seg_p1_a[0] - seg_p0_a[0]),
        seg_p0_a[1] + s * (seg_p1_a[1] - seg_p0_a[1]),
        seg_p0_a[2] + s * (seg_p1_a[2] - seg_p0_a[2]),
    ];
    let pos2_pt = [
        seg_p0_b[0] + t * (seg_p1_b[0] - seg_p0_b[0]),
        seg_p0_b[1] + t * (seg_p1_b[1] - seg_p0_b[1]),
        seg_p0_b[2] + t * (seg_p1_b[2] - seg_p0_b[2]),
    ];
    let nrm = v_sub(pos1_pt, pos2_pt);
    let n_len_sq = v_len_sq(nrm);
    let normal = if n_len_sq < 1e-6 {
        if seg_len[0] > 1e-6 { dir_norm[0] } else { [1.0, 0.0, 0.0] }
    } else {
        v_mul(nrm, 1.0 / n_len_sq.sqrt())
    };
    let pt = v_sub(v_add(pos1_pt, pos0), v_mul(normal, radius0));
    buffer.add(pt, normal, sq_dist.sqrt() - radius_sum);
    (buffer.count() - count_before) as i32
}
