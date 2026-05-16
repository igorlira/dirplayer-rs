//! GuContactCapsuleBox — Rust port of PhysX 3.4's capsule-vs-OBB contact
//! generation.
//!
//! Source files:
//! - `GeomUtils/src/contact/GuContactCapsuleBox.cpp` (main + helpers)
//! - `GeomUtils/src/distance/GuDistanceSegmentBox.cpp` (segment-OBB)
//! - `GeomUtils/src/distance/GuDistancePointBox.cpp` (point-OBB)
//! - `GeomUtils/src/intersection/GuIntersectionRayBox.cpp` (ray-AABB)
//! - `GeomUtils/src/GuBox.cpp` (computeOBBPoints + getBoxEdges)
//! - `PxShared/src/foundation/include/PsMathUtils.h` (closestAxis + makeFatEdge)
//!
//! PhysX engine convention preserved: contact normal points box → capsule
//! (shape1 → shape0); separation < 0 ⇒ penetrating. The dispatch in
//! [`super::physx_native`] flips at the boundary.

#![allow(dead_code)]

use super::physx_gu::{
    v_add, v_cross, v_dot, v_len, v_len_sq, v_mul, v_neg, v_sub, GuContactBuffer,
};

// ----------------------------------------------------------------------------
//  Gu::Box — OBB with column-stored rotation. Source: GuBox.h.
// ----------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
pub struct GuBox {
    pub center: [f64; 3],
    pub extents: [f64; 3],
    pub col0: [f64; 3],
    pub col1: [f64; 3],
    pub col2: [f64; 3],
}

impl GuBox {
    pub fn from_transform(rot: [f64; 4], pos: [f64; 3], half_extents: [f64; 3]) -> Self {
        Self {
            center: pos,
            extents: half_extents,
            col0: super::physx_gu::q_rotate(rot, [1.0, 0.0, 0.0]),
            col1: super::physx_gu::q_rotate(rot, [0.0, 1.0, 0.0]),
            col2: super::physx_gu::q_rotate(rot, [0.0, 0.0, 1.0]),
        }
    }

    /// R · v.
    pub fn rotate(&self, v: [f64; 3]) -> [f64; 3] {
        [
            self.col0[0] * v[0] + self.col1[0] * v[1] + self.col2[0] * v[2],
            self.col0[1] * v[0] + self.col1[1] * v[1] + self.col2[1] * v[2],
            self.col0[2] * v[0] + self.col1[2] * v[1] + self.col2[2] * v[2],
        ]
    }

    /// R^T · v (PxMat33::transformTranspose).
    pub fn transform_transpose(&self, v: [f64; 3]) -> [f64; 3] {
        [
            self.col0[0] * v[0] + self.col0[1] * v[1] + self.col0[2] * v[2],
            self.col1[0] * v[0] + self.col1[1] * v[1] + self.col1[2] * v[2],
            self.col2[0] * v[0] + self.col2[1] * v[1] + self.col2[2] * v[2],
        ]
    }

    /// Source: GuBox.cpp:88-131 — `Gu::computeOBBPoints` (8 OBB vertices).
    pub fn compute_box_points(&self, pts: &mut [[f64; 3]; 8]) {
        let axis0 = v_mul(self.col0, self.extents[0]);
        let axis1 = v_mul(self.col1, self.extents[1]);
        let axis2 = v_mul(self.col2, self.extents[2]);

        let p_minus = v_sub(self.center, axis0);
        let p_plus = v_add(self.center, axis0);
        pts[0] = p_minus; pts[3] = p_minus; pts[4] = p_minus; pts[7] = p_minus;
        pts[1] = p_plus;  pts[2] = p_plus;  pts[5] = p_plus;  pts[6] = p_plus;

        let mut tmp = v_add(axis1, axis2);
        pts[0] = v_sub(pts[0], tmp);
        pts[1] = v_sub(pts[1], tmp);
        pts[6] = v_add(pts[6], tmp);
        pts[7] = v_add(pts[7], tmp);

        tmp = v_sub(axis1, axis2);
        pts[2] = v_add(pts[2], tmp);
        pts[3] = v_add(pts[3], tmp);
        pts[4] = v_sub(pts[4], tmp);
        pts[5] = v_sub(pts[5], tmp);
    }
}

/// 24 edge indices (12 edges × 2 verts each). Source: GuBox.cpp:67-85.
pub const BOX_EDGES: [u8; 24] = [
    0, 1,  1, 2,  2, 3,  3, 0,
    7, 6,  6, 5,  5, 4,  4, 7,
    1, 5,  6, 2,
    3, 7,  4, 0,
];

// ----------------------------------------------------------------------------
//  closestAxis with j/k outputs — Source: PsMathUtils.h:426-451.
// ----------------------------------------------------------------------------

pub fn closest_axis_jk(v: [f64; 3]) -> (usize, usize, usize) {
    let abs_x = v[0].abs();
    let abs_y = v[1].abs();
    let abs_z = v[2].abs();
    let mut m = 0usize;
    let mut j = 1usize;
    let mut k = 2usize;
    if abs_y > abs_x && abs_y > abs_z {
        j = 2; k = 0; m = 1;
    } else if abs_z > abs_x {
        j = 0; k = 1; m = 2;
    }
    (m, j, k)
}

/// Source: PsMathUtils.h:456-467.
pub fn make_fat_edge(p0: &mut [f64; 3], p1: &mut [f64; 3], fat_coeff: f64) {
    let delta = v_sub(*p1, *p0);
    let m = v_len(delta);
    if m > 0.0 {
        let s = fat_coeff / m;
        let scaled = v_mul(delta, s);
        *p0 = v_sub(*p0, scaled);
        *p1 = v_add(*p1, scaled);
    }
}

// ----------------------------------------------------------------------------
//  intersectRayAABB — Source: GuIntersectionRayBox.cpp:230-279.
// ----------------------------------------------------------------------------

pub fn intersect_ray_aabb(
    minimum: [f64; 3], maximum: [f64; 3],
    ro: [f64; 3], rd: [f64; 3],
) -> (i32, f64, f64) {
    const LOCAL_EPSILON: f64 = 1.192_092_9e-7; // PX_EPS_F32
    let mut ret: i32 = -1;
    let mut tnear = f64::MIN;
    let mut tfar = f64::MAX;

    for a in 0..3 {
        if rd[a] > -LOCAL_EPSILON && rd[a] < LOCAL_EPSILON {
            if ro[a] < minimum[a] || ro[a] > maximum[a] { return (-1, tnear, tfar); }
        } else {
            let one_over_dir = 1.0 / rd[a];
            let mut t1 = (minimum[a] - ro[a]) * one_over_dir;
            let mut t2 = (maximum[a] - ro[a]) * one_over_dir;
            let mut b = a as i32;
            if t1 > t2 { std::mem::swap(&mut t1, &mut t2); b += 3; }
            if t1 > tnear { tnear = t1; ret = b; }
            if t2 < tfar { tfar = t2; }
            if tnear > tfar || tfar < LOCAL_EPSILON { return (-1, tnear, tfar); }
        }
    }
    if tnear > tfar || tfar < LOCAL_EPSILON { return (-1, tnear, tfar); }
    (ret, tnear, tfar)
}

// ----------------------------------------------------------------------------
//  distancePointBoxSquared — Source: GuDistancePointBox.cpp:34-66.
// ----------------------------------------------------------------------------

pub fn distance_point_box_squared(
    point: [f64; 3], box_origin: [f64; 3], box_extent: [f64; 3], box_rot: &GuBox,
) -> (f64, [f64; 3]) {
    let diff = v_sub(point, box_origin);
    let mut closest = [
        box_rot.col0[0] * diff[0] + box_rot.col0[1] * diff[1] + box_rot.col0[2] * diff[2],
        box_rot.col1[0] * diff[0] + box_rot.col1[1] * diff[1] + box_rot.col1[2] * diff[2],
        box_rot.col2[0] * diff[0] + box_rot.col2[1] * diff[1] + box_rot.col2[2] * diff[2],
    ];

    let mut sqr = 0.0f64;
    let ex = box_extent;
    for ax in 0..3 {
        if closest[ax] < -ex[ax] {
            let d = closest[ax] + ex[ax];
            sqr += d * d;
            closest[ax] = -ex[ax];
        } else if closest[ax] > ex[ax] {
            let d = closest[ax] - ex[ax];
            sqr += d * d;
            closest[ax] = ex[ax];
        }
    }
    (sqr, closest)
}

// ----------------------------------------------------------------------------
//  Wild-Magic line-box distance with 5 region helpers.
//  Source: GuDistanceSegmentBox.cpp:38-513.
// ----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn face_region(
    i0: usize, i1: usize, i2: usize,
    rk_pnt: &mut [f64; 3], rk_dir: &[f64; 3], extents: &[f64; 3], rk_pme: &[f64; 3],
    pf_lparam: &mut f64, want_param: bool, rf_sqr_distance: &mut f64,
) {
    let mut k_ppe = [0.0f64; 3];
    let (mut f_lsqr, f_inv);
    let (mut f_tmp, mut f_param, mut f_t, mut f_delta);

    k_ppe[i1] = rk_pnt[i1] + extents[i1];
    k_ppe[i2] = rk_pnt[i2] + extents[i2];
    if rk_dir[i0] * k_ppe[i1] >= rk_dir[i1] * rk_pme[i0] {
        if rk_dir[i0] * k_ppe[i2] >= rk_dir[i2] * rk_pme[i0] {
            if want_param {
                rk_pnt[i0] = extents[i0];
                f_inv = 1.0 / rk_dir[i0];
                rk_pnt[i1] -= rk_dir[i1] * rk_pme[i0] * f_inv;
                rk_pnt[i2] -= rk_dir[i2] * rk_pme[i0] * f_inv;
                *pf_lparam = -rk_pme[i0] * f_inv;
            }
        } else {
            f_lsqr = rk_dir[i0] * rk_dir[i0] + rk_dir[i2] * rk_dir[i2];
            f_tmp = f_lsqr * k_ppe[i1] - rk_dir[i1] * (rk_dir[i0] * rk_pme[i0] + rk_dir[i2] * k_ppe[i2]);
            if f_tmp <= 2.0 * f_lsqr * extents[i1] {
                f_t = f_tmp / f_lsqr;
                f_lsqr += rk_dir[i1] * rk_dir[i1];
                f_tmp = k_ppe[i1] - f_t;
                f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * f_tmp + rk_dir[i2] * k_ppe[i2];
                f_param = -f_delta / f_lsqr;
                *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + f_tmp * f_tmp + k_ppe[i2] * k_ppe[i2] + f_delta * f_param;
                if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = f_t - extents[i1]; rk_pnt[i2] = -extents[i2]; }
            } else {
                f_lsqr += rk_dir[i1] * rk_dir[i1];
                f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * rk_pme[i1] + rk_dir[i2] * k_ppe[i2];
                f_param = -f_delta / f_lsqr;
                *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + rk_pme[i1] * rk_pme[i1] + k_ppe[i2] * k_ppe[i2] + f_delta * f_param;
                if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = extents[i1]; rk_pnt[i2] = -extents[i2]; }
            }
        }
    } else if rk_dir[i0] * k_ppe[i2] >= rk_dir[i2] * rk_pme[i0] {
        f_lsqr = rk_dir[i0] * rk_dir[i0] + rk_dir[i1] * rk_dir[i1];
        f_tmp = f_lsqr * k_ppe[i2] - rk_dir[i2] * (rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * k_ppe[i1]);
        if f_tmp <= 2.0 * f_lsqr * extents[i2] {
            f_t = f_tmp / f_lsqr;
            f_lsqr += rk_dir[i2] * rk_dir[i2];
            f_tmp = k_ppe[i2] - f_t;
            f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * k_ppe[i1] + rk_dir[i2] * f_tmp;
            f_param = -f_delta / f_lsqr;
            *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + k_ppe[i1] * k_ppe[i1] + f_tmp * f_tmp + f_delta * f_param;
            if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = -extents[i1]; rk_pnt[i2] = f_t - extents[i2]; }
        } else {
            f_lsqr += rk_dir[i2] * rk_dir[i2];
            f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * k_ppe[i1] + rk_dir[i2] * rk_pme[i2];
            f_param = -f_delta / f_lsqr;
            *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + k_ppe[i1] * k_ppe[i1] + rk_pme[i2] * rk_pme[i2] + f_delta * f_param;
            if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = -extents[i1]; rk_pnt[i2] = extents[i2]; }
        }
    } else {
        f_lsqr = rk_dir[i0] * rk_dir[i0] + rk_dir[i2] * rk_dir[i2];
        f_tmp = f_lsqr * k_ppe[i1] - rk_dir[i1] * (rk_dir[i0] * rk_pme[i0] + rk_dir[i2] * k_ppe[i2]);
        if f_tmp >= 0.0 {
            if f_tmp <= 2.0 * f_lsqr * extents[i1] {
                f_t = f_tmp / f_lsqr;
                f_lsqr += rk_dir[i1] * rk_dir[i1];
                f_tmp = k_ppe[i1] - f_t;
                f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * f_tmp + rk_dir[i2] * k_ppe[i2];
                f_param = -f_delta / f_lsqr;
                *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + f_tmp * f_tmp + k_ppe[i2] * k_ppe[i2] + f_delta * f_param;
                if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = f_t - extents[i1]; rk_pnt[i2] = -extents[i2]; }
            } else {
                f_lsqr += rk_dir[i1] * rk_dir[i1];
                f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * rk_pme[i1] + rk_dir[i2] * k_ppe[i2];
                f_param = -f_delta / f_lsqr;
                *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + rk_pme[i1] * rk_pme[i1] + k_ppe[i2] * k_ppe[i2] + f_delta * f_param;
                if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = extents[i1]; rk_pnt[i2] = -extents[i2]; }
            }
            return;
        }
        f_lsqr = rk_dir[i0] * rk_dir[i0] + rk_dir[i1] * rk_dir[i1];
        f_tmp = f_lsqr * k_ppe[i2] - rk_dir[i2] * (rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * k_ppe[i1]);
        if f_tmp >= 0.0 {
            if f_tmp <= 2.0 * f_lsqr * extents[i2] {
                f_t = f_tmp / f_lsqr;
                f_lsqr += rk_dir[i2] * rk_dir[i2];
                f_tmp = k_ppe[i2] - f_t;
                f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * k_ppe[i1] + rk_dir[i2] * f_tmp;
                f_param = -f_delta / f_lsqr;
                *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + k_ppe[i1] * k_ppe[i1] + f_tmp * f_tmp + f_delta * f_param;
                if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = -extents[i1]; rk_pnt[i2] = f_t - extents[i2]; }
            } else {
                f_lsqr += rk_dir[i2] * rk_dir[i2];
                f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * k_ppe[i1] + rk_dir[i2] * rk_pme[i2];
                f_param = -f_delta / f_lsqr;
                *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + k_ppe[i1] * k_ppe[i1] + rk_pme[i2] * rk_pme[i2] + f_delta * f_param;
                if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = -extents[i1]; rk_pnt[i2] = extents[i2]; }
            }
            return;
        }
        // (v[i1],v[i2])-corner is closest
        f_lsqr += rk_dir[i2] * rk_dir[i2];
        f_delta = rk_dir[i0] * rk_pme[i0] + rk_dir[i1] * k_ppe[i1] + rk_dir[i2] * k_ppe[i2];
        f_param = -f_delta / f_lsqr;
        *rf_sqr_distance += rk_pme[i0] * rk_pme[i0] + k_ppe[i1] * k_ppe[i1] + k_ppe[i2] * k_ppe[i2] + f_delta * f_param;
        if want_param { *pf_lparam = f_param; rk_pnt[i0] = extents[i0]; rk_pnt[i1] = -extents[i1]; rk_pnt[i2] = -extents[i2]; }
    }
}

fn case_no_zeros(
    rk_pnt: &mut [f64; 3], rk_dir: &[f64; 3], extents: &[f64; 3],
    pf_lparam: &mut f64, want_param: bool, rf_sqr_distance: &mut f64,
) {
    let k_pme = [rk_pnt[0] - extents[0], rk_pnt[1] - extents[1], rk_pnt[2] - extents[2]];
    let f_prod_dx_py = rk_dir[0] * k_pme[1];
    let f_prod_dy_px = rk_dir[1] * k_pme[0];
    if f_prod_dy_px >= f_prod_dx_py {
        let f_prod_dz_px = rk_dir[2] * k_pme[0];
        let f_prod_dx_pz = rk_dir[0] * k_pme[2];
        if f_prod_dz_px >= f_prod_dx_pz {
            face_region(0, 1, 2, rk_pnt, rk_dir, extents, &k_pme, pf_lparam, want_param, rf_sqr_distance);
        } else {
            face_region(2, 0, 1, rk_pnt, rk_dir, extents, &k_pme, pf_lparam, want_param, rf_sqr_distance);
        }
    } else {
        let f_prod_dz_py = rk_dir[2] * k_pme[1];
        let f_prod_dy_pz = rk_dir[1] * k_pme[2];
        if f_prod_dz_py >= f_prod_dy_pz {
            face_region(1, 2, 0, rk_pnt, rk_dir, extents, &k_pme, pf_lparam, want_param, rf_sqr_distance);
        } else {
            face_region(2, 0, 1, rk_pnt, rk_dir, extents, &k_pme, pf_lparam, want_param, rf_sqr_distance);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn case0(
    i0: usize, i1: usize, i2: usize,
    rk_pnt: &mut [f64; 3], rk_dir: &[f64; 3], extents: &[f64; 3],
    pf_lparam: &mut f64, want_param: bool, rf_sqr_distance: &mut f64,
) {
    let f_pme0 = rk_pnt[i0] - extents[i0];
    let f_pme1 = rk_pnt[i1] - extents[i1];
    let f_prod0 = rk_dir[i1] * f_pme0;
    let f_prod1 = rk_dir[i0] * f_pme1;
    let mut f_delta;
    if f_prod0 >= f_prod1 {
        rk_pnt[i0] = extents[i0];
        let f_ppe1 = rk_pnt[i1] + extents[i1];
        f_delta = f_prod0 - rk_dir[i0] * f_ppe1;
        if f_delta >= 0.0 {
            let f_inv_lsqr = 1.0 / (rk_dir[i0] * rk_dir[i0] + rk_dir[i1] * rk_dir[i1]);
            *rf_sqr_distance += f_delta * f_delta * f_inv_lsqr;
            if want_param { rk_pnt[i1] = -extents[i1]; *pf_lparam = -(rk_dir[i0] * f_pme0 + rk_dir[i1] * f_ppe1) * f_inv_lsqr; }
        } else if want_param {
            let f_inv = 1.0 / rk_dir[i0]; rk_pnt[i1] -= f_prod0 * f_inv; *pf_lparam = -f_pme0 * f_inv;
        }
    } else {
        rk_pnt[i1] = extents[i1];
        let f_ppe0 = rk_pnt[i0] + extents[i0];
        f_delta = f_prod1 - rk_dir[i1] * f_ppe0;
        if f_delta >= 0.0 {
            let f_inv_lsqr = 1.0 / (rk_dir[i0] * rk_dir[i0] + rk_dir[i1] * rk_dir[i1]);
            *rf_sqr_distance += f_delta * f_delta * f_inv_lsqr;
            if want_param { rk_pnt[i0] = -extents[i0]; *pf_lparam = -(rk_dir[i0] * f_ppe0 + rk_dir[i1] * f_pme1) * f_inv_lsqr; }
        } else if want_param {
            let f_inv = 1.0 / rk_dir[i1]; rk_pnt[i0] -= f_prod1 * f_inv; *pf_lparam = -f_pme1 * f_inv;
        }
    }
    if rk_pnt[i2] < -extents[i2] { f_delta = rk_pnt[i2] + extents[i2]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[i2] = -extents[i2]; }
    else if rk_pnt[i2] > extents[i2] { f_delta = rk_pnt[i2] - extents[i2]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[i2] = extents[i2]; }
}

#[allow(clippy::too_many_arguments)]
fn case00(
    i0: usize, i1: usize, i2: usize,
    rk_pnt: &mut [f64; 3], rk_dir: &[f64; 3], extents: &[f64; 3],
    pf_lparam: &mut f64, want_param: bool, rf_sqr_distance: &mut f64,
) {
    if want_param { *pf_lparam = (extents[i0] - rk_pnt[i0]) / rk_dir[i0]; }
    rk_pnt[i0] = extents[i0];
    let mut f_delta;
    if rk_pnt[i1] < -extents[i1] { f_delta = rk_pnt[i1] + extents[i1]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[i1] = -extents[i1]; }
    else if rk_pnt[i1] > extents[i1] { f_delta = rk_pnt[i1] - extents[i1]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[i1] = extents[i1]; }
    if rk_pnt[i2] < -extents[i2] { f_delta = rk_pnt[i2] + extents[i2]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[i2] = -extents[i2]; }
    else if rk_pnt[i2] > extents[i2] { f_delta = rk_pnt[i2] - extents[i2]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[i2] = extents[i2]; }
}

fn case000(rk_pnt: &mut [f64; 3], extents: &[f64; 3], rf_sqr_distance: &mut f64) {
    let mut f_delta;
    if rk_pnt[0] < -extents[0] { f_delta = rk_pnt[0] + extents[0]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[0] = -extents[0]; }
    else if rk_pnt[0] > extents[0] { f_delta = rk_pnt[0] - extents[0]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[0] = extents[0]; }
    if rk_pnt[1] < -extents[1] { f_delta = rk_pnt[1] + extents[1]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[1] = -extents[1]; }
    else if rk_pnt[1] > extents[1] { f_delta = rk_pnt[1] - extents[1]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[1] = extents[1]; }
    if rk_pnt[2] < -extents[2] { f_delta = rk_pnt[2] + extents[2]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[2] = -extents[2]; }
    else if rk_pnt[2] > extents[2] { f_delta = rk_pnt[2] - extents[2]; *rf_sqr_distance += f_delta * f_delta; rk_pnt[2] = extents[2]; }
}

/// Source: GuDistanceSegmentBox.cpp:436-513.
fn distance_line_box_squared(
    line_origin: [f64; 3], line_direction: [f64; 3],
    box_origin: [f64; 3], box_extent: [f64; 3], box_rot: &GuBox,
) -> (f64, f64, [f64; 3]) {
    let diff = v_sub(line_origin, box_origin);
    let mut pnt = [
        diff[0] * box_rot.col0[0] + diff[1] * box_rot.col0[1] + diff[2] * box_rot.col0[2],
        diff[0] * box_rot.col1[0] + diff[1] * box_rot.col1[1] + diff[2] * box_rot.col1[2],
        diff[0] * box_rot.col2[0] + diff[1] * box_rot.col2[1] + diff[2] * box_rot.col2[2],
    ];
    let mut dir = [
        line_direction[0] * box_rot.col0[0] + line_direction[1] * box_rot.col0[1] + line_direction[2] * box_rot.col0[2],
        line_direction[0] * box_rot.col1[0] + line_direction[1] * box_rot.col1[1] + line_direction[2] * box_rot.col1[2],
        line_direction[0] * box_rot.col2[0] + line_direction[1] * box_rot.col2[1] + line_direction[2] * box_rot.col2[2],
    ];

    let mut reflect = [false; 3];
    for i in 0..3 {
        if dir[i] < 0.0 {
            pnt[i] = -pnt[i];
            dir[i] = -dir[i];
            reflect[i] = true;
        }
    }

    let ext = box_extent;
    let mut sqr = 0.0f64;
    let mut lp = 0.0f64;
    let want_param = true;

    if dir[0] > 0.0 {
        if dir[1] > 0.0 {
            if dir[2] > 0.0 { case_no_zeros(&mut pnt, &dir, &ext, &mut lp, want_param, &mut sqr); }
            else { case0(0, 1, 2, &mut pnt, &dir, &ext, &mut lp, want_param, &mut sqr); }
        } else if dir[2] > 0.0 { case0(0, 2, 1, &mut pnt, &dir, &ext, &mut lp, want_param, &mut sqr); }
        else { case00(0, 1, 2, &mut pnt, &dir, &ext, &mut lp, want_param, &mut sqr); }
    } else if dir[1] > 0.0 {
        if dir[2] > 0.0 { case0(1, 2, 0, &mut pnt, &dir, &ext, &mut lp, want_param, &mut sqr); }
        else { case00(1, 0, 2, &mut pnt, &dir, &ext, &mut lp, want_param, &mut sqr); }
    } else if dir[2] > 0.0 { case00(2, 0, 1, &mut pnt, &dir, &ext, &mut lp, want_param, &mut sqr); }
    else { case000(&mut pnt, &ext, &mut sqr); lp = 0.0; }

    for i in 0..3 { if reflect[i] { pnt[i] = -pnt[i]; } }
    (sqr, lp, pnt)
}

/// Source: GuDistanceSegmentBox.cpp:516-549.
pub fn distance_segment_box_squared(
    seg_p0: [f64; 3], seg_p1: [f64; 3],
    box_origin: [f64; 3], box_extent: [f64; 3], box_rot: &GuBox,
) -> (f64, f64, [f64; 3]) {
    let (sqr, lp, bp) = distance_line_box_squared(seg_p0, v_sub(seg_p1, seg_p0), box_origin, box_extent, box_rot);
    if lp >= 0.0 {
        if lp <= 1.0 { (sqr, lp, bp) } else {
            let (s, p) = distance_point_box_squared(seg_p1, box_origin, box_extent, box_rot);
            (s, 1.0, p)
        }
    } else {
        let (s, p) = distance_point_box_squared(seg_p0, box_origin, box_extent, box_rot);
        (s, 0.0, p)
    }
}

// ----------------------------------------------------------------------------
//  intersectEdgeEdgePreca — Source: GuContactCapsuleBox.cpp:74-111.
// ----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn intersect_edge_edge_preca(
    p1: [f64; 3], p2: [f64; 3], v1: [f64; 3],
    plane_n: [f64; 3], plane_d: f64, i: usize, j: usize, coeff: f64,
    dir: [f64; 3], p3: [f64; 3], p4: [f64; 3],
) -> Option<(f64, [f64; 3])> {
    let d3 = v_dot(plane_n, p3) - plane_d;
    let d4 = v_dot(plane_n, p4) - plane_d;
    let mut temp = d3 * d4;
    if temp > 0.0 { return None; }

    let v2 = v_sub(p4, p3);
    temp = v_dot(plane_n, v2);
    if temp == 0.0 { return None; }
    let ratio = d3 / temp;
    let mut ip = [p3[0] - v2[0] * ratio, p3[1] - v2[1] * ratio, p3[2] - v2[2] * ratio];

    let dist = (v1[i] * (ip[j] - p1[j]) - v1[j] * (ip[i] - p1[i])) * coeff;
    if dist < 0.0 { return None; }

    ip = [ip[0] - dist * dir[0], ip[1] - dist * dir[1], ip[2] - dist * dir[2]];
    let dot = (p1[0] - ip[0]) * (p2[0] - ip[0])
            + (p1[1] - ip[1]) * (p2[1] - ip[1])
            + (p1[2] - ip[2]) * (p2[2] - ip[2]);
    if dot < 0.0 { Some((dist, ip)) } else { None }
}

// ----------------------------------------------------------------------------
//  GuTestAxis + GuCapsuleOBBOverlap3 — Source: GuContactCapsuleBox.cpp:114-200.
// ----------------------------------------------------------------------------

fn gu_test_axis(
    axis: [f64; 3], seg_p0: [f64; 3], seg_p1: [f64; 3], radius: f64, b: &GuBox,
) -> Option<f64> {
    let mut min0 = v_dot(seg_p0, axis);
    let mut max0 = v_dot(seg_p1, axis);
    if min0 > max0 { std::mem::swap(&mut min0, &mut max0); }
    min0 -= radius;
    max0 += radius;

    let box_cen = v_dot(b.center, axis);
    let box_ext =
        v_dot(b.col0, axis).abs() * b.extents[0]
      + v_dot(b.col1, axis).abs() * b.extents[1]
      + v_dot(b.col2, axis).abs() * b.extents[2];
    let min1 = box_cen - box_ext;
    let max1 = box_cen + box_ext;

    if max0 < min1 || max1 < min0 { return None; }
    let d0 = max0 - min1;
    let d1 = max1 - min0;
    Some(d0.min(d1))
}

fn gu_capsule_obb_overlap3(
    seg_p0: [f64; 3], seg_p1: [f64; 3], radius: f64, b: &GuBox,
) -> Option<(f64, [f64; 3])> {
    let mut sep_axis = [0.0f64; 3];
    let mut pen_depth = f64::MAX;

    for axis in [b.col0, b.col1, b.col2] {
        let d = gu_test_axis(axis, seg_p0, seg_p1, radius, b)?;
        if d < pen_depth { pen_depth = d; sep_axis = axis; }
    }

    let cap_axis = v_sub(seg_p1, seg_p0);
    let cap_len = v_len(cap_axis);
    if cap_len > 1e-6 {
        let cap_axis_n = v_mul(cap_axis, 1.0 / cap_len);
        for box_axis in [b.col0, b.col1, b.col2] {
            let cross = v_cross(cap_axis_n, box_axis);
            let cl = v_len(cross);
            if cl > 1e-6 {
                let cn = v_mul(cross, 1.0 / cl);
                let d = gu_test_axis(cn, seg_p0, seg_p1, radius, b)?;
                if d < pen_depth { pen_depth = d; sep_axis = cn; }
            }
        }
    }

    // Orient sep so it points BOX → CAPSULE.
    let witness = [
        (seg_p0[0] + seg_p1[0]) * 0.5 - b.center[0],
        (seg_p0[1] + seg_p1[1]) * 0.5 - b.center[1],
        (seg_p0[2] + seg_p1[2]) * 0.5 - b.center[2],
    ];
    let dot_w = v_dot(sep_axis, witness);
    if dot_w < 0.0 { sep_axis = v_neg(sep_axis); }
    Some((pen_depth, sep_axis))
}

// ----------------------------------------------------------------------------
//  Contact generators — Source: GuContactCapsuleBox.cpp:203-355.
// ----------------------------------------------------------------------------

const FAT_BOX_EDGE_COEFF: f64 = 0.01;

fn gu_generate_vf_contacts(
    contact_buffer: &mut GuContactBuffer,
    seg_p0: [f64; 3], seg_p1: [f64; 3], radius: f64, world_box: &GuBox,
    normal: [f64; 3], contact_distance: f64,
) {
    let max = world_box.extents;
    let min = v_neg(world_box.extents);
    let tmp2_neg = world_box.transform_transpose(normal);
    let tmp2 = v_neg(tmp2_neg);

    for i in 0..2 {
        let pos = if i == 0 { seg_p0 } else { seg_p1 };
        let tmp = world_box.transform_transpose(v_sub(pos, world_box.center));
        let (res, tnear, _) = intersect_ray_aabb(min, max, tmp, tmp2);
        if res != -1 && tnear < radius + contact_distance {
            contact_buffer.add(
                v_sub(pos, v_mul(normal, tnear)),
                normal,
                tnear - radius,
            );
        }
    }
}

fn gu_generate_ee_contacts_impl(
    contact_buffer: &mut GuContactBuffer,
    seg_p0: [f64; 3], seg_p1: [f64; 3], radius: f64, world_box: &GuBox,
    normal: [f64; 3], use_v2_negated: bool, contact_distance: f64, check_contact_distance: bool,
) {
    let mut pts = [[0.0f64; 3]; 8];
    world_box.compute_box_points(&mut pts);
    let mut s0 = seg_p0;
    let mut s1 = seg_p1;
    make_fat_edge(&mut s0, &mut s1, FAT_BOX_EDGE_COEFF);

    let v1 = v_sub(s1, s0);
    let plane_n = if !use_v2_negated {
        v_cross(v1, normal)
    } else {
        v_neg(v_cross(v1, normal))
    };
    let plane_d = -v_dot(plane_n, s0);

    let (_, ii, jj) = closest_axis_jk(plane_n);

    let coeff = if !use_v2_negated {
        1.0 / (v1[ii] * normal[jj] - v1[jj] * normal[ii])
    } else {
        1.0 / (v1[jj] * normal[ii] - v1[ii] * normal[jj])
    };

    let dir_arg = if use_v2_negated { v_neg(normal) } else { normal };

    for i in 0..12 {
        let p1 = pts[BOX_EDGES[i * 2] as usize];
        let p2 = pts[BOX_EDGES[i * 2 + 1] as usize];

        if let Some((dist, ip)) = intersect_edge_edge_preca(
            s0, s1, v1, plane_n, plane_d, ii, jj, coeff, dir_arg, p1, p2,
        ) {
            if !use_v2_negated {
                contact_buffer.add(
                    v_sub(ip, v_mul(normal, dist)),
                    normal,
                    -(radius + dist),
                );
            } else {
                if check_contact_distance && dist >= radius + contact_distance { continue; }
                contact_buffer.add(
                    v_sub(ip, v_mul(normal, dist)),
                    normal,
                    dist - radius,
                );
            }
        }
    }
}

fn gu_generate_ee_contacts(
    cb: &mut GuContactBuffer, seg_p0: [f64; 3], seg_p1: [f64; 3], radius: f64, b: &GuBox, normal: [f64; 3],
) {
    gu_generate_ee_contacts_impl(cb, seg_p0, seg_p1, radius, b, normal, false, 0.0, false);
}

fn gu_generate_ee_contacts2(
    cb: &mut GuContactBuffer, seg_p0: [f64; 3], seg_p1: [f64; 3], radius: f64, b: &GuBox, normal: [f64; 3],
    contact_distance: f64,
) {
    gu_generate_ee_contacts_impl(cb, seg_p0, seg_p1, radius, b, normal, true, contact_distance, true);
}

// ----------------------------------------------------------------------------
//  Main entry — Source: GuContactCapsuleBox.cpp:361-457.
// ----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn contact_capsule_box(
    contact_buffer: &mut GuContactBuffer,
    capsule_pos: [f64; 3], capsule_rot: [f64; 4], capsule_half_height: f64, capsule_radius: f64,
    box_pos: [f64; 3], box_rot: [f64; 4], box_half_extents: [f64; 3],
    contact_distance: f64,
) -> i32 {
    let count_before = contact_buffer.count();

    let hh = super::physx_gu::get_capsule_half_height_vector(capsule_rot, capsule_half_height);
    let seg_p0 = v_add(capsule_pos, hh);
    let seg_p1 = v_sub(capsule_pos, hh);
    let inflated_radius = capsule_radius + contact_distance;

    let world_box = GuBox::from_transform(box_rot, box_pos, box_half_extents);

    let (square_dist, t, on_box_local) = distance_segment_box_squared(
        seg_p0, seg_p1, world_box.center, world_box.extents, &world_box,
    );
    if square_dist >= inflated_radius * inflated_radius { return 0; }

    let mut penetration = square_dist == 0.0;
    if !penetration {
        let on_segment = [
            seg_p0[0] + t * (seg_p1[0] - seg_p0[0]),
            seg_p0[1] + t * (seg_p1[1] - seg_p0[1]),
            seg_p0[2] + t * (seg_p1[2] - seg_p0[2]),
        ];
        let on_box_world = v_add(world_box.center, world_box.rotate(on_box_local));
        let normal = v_sub(on_segment, on_box_world);
        let normal_len = v_len(normal);
        if normal_len > 0.0 {
            let normal = v_mul(normal, 1.0 / normal_len);
            gu_generate_vf_contacts(contact_buffer, seg_p0, seg_p1, capsule_radius, &world_box, normal, contact_distance);
            if contact_buffer.count() - count_before == 2 { return 2; }
            gu_generate_ee_contacts2(contact_buffer, seg_p0, seg_p1, capsule_radius, &world_box, normal, contact_distance);
            if contact_buffer.count() == count_before {
                contact_buffer.add(on_box_world, normal, square_dist.sqrt() - capsule_radius);
            }
            return (contact_buffer.count() - count_before) as i32;
        }
        // Linux-edge fallback per source comment.
        penetration = true;
    }

    if penetration {
        let Some((depth, sep_axis)) = gu_capsule_obb_overlap3(seg_p0, seg_p1, capsule_radius, &world_box) else {
            return 0;
        };
        gu_generate_vf_contacts(contact_buffer, seg_p0, seg_p1, capsule_radius, &world_box, sep_axis, contact_distance);
        if contact_buffer.count() - count_before == 2 { return 2; }
        gu_generate_ee_contacts(contact_buffer, seg_p0, seg_p1, capsule_radius, &world_box, sep_axis);
        if contact_buffer.count() == count_before {
            let center = [
                (seg_p0[0] + seg_p1[0]) * 0.5,
                (seg_p0[1] + seg_p1[1]) * 0.5,
                (seg_p0[2] + seg_p1[2]) * 0.5,
            ];
            contact_buffer.add(center, sep_axis, -(capsule_radius + depth));
        }
    }
    let _ = (v_len_sq([0.0; 3]),); // keep imports happy in case of dead-code warnings
    (contact_buffer.count() - count_before) as i32
}
