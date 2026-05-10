//! Raycast narrowphase — verbatim Rust ports of PhysX 3.4's `Gu::raycast_*`
//! family.
//!
//! Source files:
//! - `GeomUtils\src\GuRaycastTests.cpp`                      (per-shape dispatch)
//! - `GeomUtils\src\intersection\GuIntersectionRayBox.cpp`   (`rayAABBIntersect2`)
//! - `GeomUtils\src\intersection\GuIntersectionRaySphere.cpp` (`intersectRaySphere`)
//! - `GeomUtils\src\intersection\GuIntersectionRayCapsule.cpp` (`intersectRayCapsuleInternal`)
//!
//! Convention: hit `distance` is the `t` along the ray (origin + dir*t = impact);
//! `position` is the world-space hit point; `normal` is the outward surface
//! normal at the hit. `dir` should be a unit vector for distances to be
//! meaningful — callers that take Lingo direction vectors must normalize first.

#![allow(dead_code)]

use super::physx_gu::{q_rotate, q_rotate_inv};

#[derive(Default, Clone, Copy, Debug)]
pub struct GuRaycastHit {
    pub distance: f64,
    pub position: [f64; 3],
    pub normal: [f64; 3],
    pub face_index: u32,
}

const GU_RAY_SURFACE_OFFSET: f64 = 0.1;

// ----------------------------------------------------------------------------
//  intersectRaySphere — Source: GuIntersectionRaySphere.cpp:38-105
// ----------------------------------------------------------------------------

pub fn intersect_ray_sphere_basic(
    origin: [f64; 3], dir: [f64; 3], length: f64,
    center: [f64; 3], radius: f64,
) -> Option<(f64, [f64; 3])> {
    let offset = [center[0] - origin[0], center[1] - origin[1], center[2] - origin[2]];
    let ray_dist = dir[0] * offset[0] + dir[1] * offset[1] + dir[2] * offset[2];
    let off2 = offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2];
    let rad2 = radius * radius;
    if off2 <= rad2 {
        return Some((0.0, origin));
    }
    if ray_dist <= 0.0 || (ray_dist - length) > radius {
        return None;
    }
    let d = rad2 - (off2 - ray_dist * ray_dist);
    if d < 0.0 { return None; }
    let dist = ray_dist - d.sqrt();
    if dist > length { return None; }
    let hit = [origin[0] + dir[0] * dist, origin[1] + dir[1] * dist, origin[2] + dir[2] * dist];
    Some((dist, hit))
}

pub fn intersect_ray_sphere(
    origin: [f64; 3], dir: [f64; 3], length: f64,
    center: [f64; 3], radius: f64,
) -> Option<(f64, [f64; 3])> {
    let x = [origin[0] - center[0], origin[1] - center[1], origin[2] - center[2]];
    let mut l = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() - radius - GU_RAY_SURFACE_OFFSET;
    if l < 0.0 { l = 0.0; }
    let origin2 = [origin[0] + l * dir[0], origin[1] + l * dir[1], origin[2] + l * dir[2]];
    let result = intersect_ray_sphere_basic(origin2, dir, length - l, center, radius);
    result.map(|(d, h)| (d + l, h))
}

// ----------------------------------------------------------------------------
//  rayAABBIntersect2 — Source: GuIntersectionRayBox.cpp:155-219.
//  Returns `(0, _, _)` on miss; `(1+axis, t, coord)` on hit (axis = 0/1/2).
// ----------------------------------------------------------------------------

pub fn ray_aabb_intersect2(
    minimum: [f64; 3], maximum: [f64; 3],
    ro: [f64; 3], rd: [f64; 3],
) -> (i32, f64, [f64; 3]) {
    let mut c = [0.0f64; 3];
    let mut max_t = [-1.0f64; 3];
    let mut inside = true;

    for i in 0..3 {
        if ro[i] < minimum[i] {
            c[i] = minimum[i];
            inside = false;
            if rd[i] != 0.0 { max_t[i] = (minimum[i] - ro[i]) / rd[i]; }
        } else if ro[i] > maximum[i] {
            c[i] = maximum[i];
            inside = false;
            if rd[i] != 0.0 { max_t[i] = (maximum[i] - ro[i]) / rd[i]; }
        }
    }

    if inside { return (1, 0.0, ro); }

    let mut which_plane = 0usize;
    if max_t[1] > max_t[which_plane] { which_plane = 1; }
    if max_t[2] > max_t[which_plane] { which_plane = 2; }
    if max_t[which_plane] < 0.0 { return (0, 0.0, [0.0; 3]); }

    const RAYAABB_EPSILON: f64 = 0.00001;
    for i in 0..3 {
        if i != which_plane {
            c[i] = ro[i] + max_t[which_plane] * rd[i];
            if c[i] < minimum[i] - RAYAABB_EPSILON || c[i] > maximum[i] + RAYAABB_EPSILON {
                return (0, 0.0, [0.0; 3]);
            }
        }
    }
    ((1 + which_plane) as i32, max_t[which_plane], c)
}

// ----------------------------------------------------------------------------
//  raycast_box — Source: GuRaycastTests.cpp:46-109
// ----------------------------------------------------------------------------

pub fn raycast_box(
    box_half_extents: [f64; 3], box_rot: [f64; 4], box_pos: [f64; 3],
    ray_origin: [f64; 3], ray_dir: [f64; 3], max_dist: f64,
) -> Option<GuRaycastHit> {
    // Bring ray into box-local space.
    let local_origin_rel = [ray_origin[0] - box_pos[0], ray_origin[1] - box_pos[1], ray_origin[2] - box_pos[2]];
    let local_origin = q_rotate_inv(box_rot, local_origin_rel);
    let local_dir = q_rotate_inv(box_rot, ray_dir);

    let neg = [-box_half_extents[0], -box_half_extents[1], -box_half_extents[2]];
    let (rval, t, local_impact) = ray_aabb_intersect2(neg, box_half_extents, local_origin, local_dir);
    if rval == 0 || t > max_dist { return None; }

    let mut hit = GuRaycastHit { distance: t, face_index: 0xffffffff, ..Default::default() };
    if t != 0.0 {
        let r = q_rotate(box_rot, local_impact);
        hit.position = [r[0] + box_pos[0], r[1] + box_pos[1], r[2] + box_pos[2]];
    } else {
        hit.position = ray_origin;
    }
    if t == 0.0 {
        hit.normal = [-ray_dir[0], -ray_dir[1], -ray_dir[2]];
    } else {
        let axis = (rval - 1) as usize;
        let sign = if local_impact[axis] > 0.0 { 1.0 } else { -1.0 };
        let mut n = [0.0; 3];
        n[axis] = sign;
        hit.normal = q_rotate(box_rot, n);
    }
    Some(hit)
}

// ----------------------------------------------------------------------------
//  raycast_sphere — Source: GuRaycastTests.cpp:111-156
// ----------------------------------------------------------------------------

pub fn raycast_sphere(
    sphere_center: [f64; 3], sphere_radius: f64,
    ray_origin: [f64; 3], ray_dir: [f64; 3], max_dist: f64,
) -> Option<GuRaycastHit> {
    let (dist, pos) = intersect_ray_sphere(ray_origin, ray_dir, max_dist, sphere_center, sphere_radius)?;
    let mut hit = GuRaycastHit { distance: dist, position: pos, face_index: 0xffffffff, ..Default::default() };
    if dist == 0.0 {
        hit.normal = [-ray_dir[0], -ray_dir[1], -ray_dir[2]];
    } else {
        let n = [pos[0] - sphere_center[0], pos[1] - sphere_center[1], pos[2] - sphere_center[2]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        hit.normal = if len > 1e-10 { [n[0] / len, n[1] / len, n[2] / len] } else { n };
    }
    Some(hit)
}

// ----------------------------------------------------------------------------
//  raycast_capsule — Source: GuRaycastTests.cpp:158-220 + GuIntersectionRayCapsule.cpp:36-245
// ----------------------------------------------------------------------------

pub fn raycast_capsule(
    cap_pos: [f64; 3], cap_rot: [f64; 4], cap_half_height: f64, cap_radius: f64,
    ray_origin: [f64; 3], ray_dir: [f64; 3], max_dist: f64,
) -> Option<GuRaycastHit> {
    // Capsule segment endpoints (world): center ± rotated half-height along local +X.
    let hh = q_rotate(cap_rot, [cap_half_height, 0.0, 0.0]);
    let p0 = [cap_pos[0] + hh[0], cap_pos[1] + hh[1], cap_pos[2] + hh[2]];
    let p1 = [cap_pos[0] - hh[0], cap_pos[1] - hh[1], cap_pos[2] - hh[2]];

    let t = intersect_ray_capsule(ray_origin, ray_dir, p0, p1, cap_radius)?;
    if t < 0.0 || t > max_dist { return None; }

    let position = [ray_origin[0] + ray_dir[0] * t, ray_origin[1] + ray_dir[1] * t, ray_origin[2] + ray_dir[2] * t];
    let mut hit = GuRaycastHit { distance: t, position, face_index: 0xffffffff, ..Default::default() };

    if t == 0.0 {
        hit.normal = [-ray_dir[0], -ray_dir[1], -ray_dir[2]];
    } else {
        let seg = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let seg_len_sq = seg[0] * seg[0] + seg[1] * seg[1] + seg[2] * seg[2];
        let mut u = 0.0;
        if seg_len_sq > 1e-12 {
            let d = [position[0] - p0[0], position[1] - p0[1], position[2] - p0[2]];
            u = ((d[0] * seg[0] + d[1] * seg[1] + d[2] * seg[2]) / seg_len_sq).clamp(0.0, 1.0);
        }
        let p_on_seg = [p0[0] + u * seg[0], p0[1] + u * seg[1], p0[2] + u * seg[2]];
        let n = [position[0] - p_on_seg[0], position[1] - p_on_seg[1], position[2] - p_on_seg[2]];
        let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        hit.normal = if nl > 1e-10 { [n[0] / nl, n[1] / nl, n[2] / nl] } else { n };
    }
    Some(hit)
}

/// Source: GuIntersectionRayCapsule.cpp:36-245 — full ray-capsule with
/// orthonormal basis build, infinite-cylinder test, and end-cap fallback.
fn intersect_ray_capsule(
    origin: [f64; 3], dir: [f64; 3],
    p0: [f64; 3], p1: [f64; 3], radius: f64,
) -> Option<f64> {
    // Build the kU/kV/kW basis where kW is along the capsule axis.
    let mut kw = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let f_w_length = (kw[0] * kw[0] + kw[1] * kw[1] + kw[2] * kw[2]).sqrt();
    if f_w_length != 0.0 { kw = [kw[0] / f_w_length, kw[1] / f_w_length, kw[2] / f_w_length]; }

    // Degenerate capsule (sphere) — switch to sphere path.
    if f_w_length <= 1e-6 {
        let d0 = (origin[0] - p0[0]).powi(2) + (origin[1] - p0[1]).powi(2) + (origin[2] - p0[2]).powi(2);
        let d1 = (origin[0] - p1[0]).powi(2) + (origin[1] - p1[1]).powi(2) + (origin[2] - p1[2]).powi(2);
        let approx_length = (d0.max(d1) + radius) * 2.0;
        return intersect_ray_sphere(origin, dir, approx_length, p0, radius).map(|(t, _)| t);
    }

    let ku = if kw[0].abs() >= kw[1].abs() {
        let f_inv_length = 1.0 / (kw[0] * kw[0] + kw[2] * kw[2]).sqrt();
        [-kw[2] * f_inv_length, 0.0, kw[0] * f_inv_length]
    } else {
        let f_inv_length = 1.0 / (kw[1] * kw[1] + kw[2] * kw[2]).sqrt();
        [0.0, kw[2] * f_inv_length, -kw[1] * f_inv_length]
    };
    let mut kv = [
        kw[1] * ku[2] - kw[2] * ku[1],
        kw[2] * ku[0] - kw[0] * ku[2],
        kw[0] * ku[1] - kw[1] * ku[0],
    ];
    {
        let l = (kv[0] * kv[0] + kv[1] * kv[1] + kv[2] * kv[2]).sqrt();
        if l > 0.0 { kv = [kv[0] / l, kv[1] / l, kv[2] / l]; }
    }

    // Transform ray into (kU, kV, kW) basis.
    let mut kd = [
        ku[0] * dir[0] + ku[1] * dir[1] + ku[2] * dir[2],
        kv[0] * dir[0] + kv[1] * dir[1] + kv[2] * dir[2],
        kw[0] * dir[0] + kw[1] * dir[1] + kw[2] * dir[2],
    ];
    let f_d_length = (kd[0] * kd[0] + kd[1] * kd[1] + kd[2] * kd[2]).sqrt();
    let f_inv_d_length = if f_d_length != 0.0 { 1.0 / f_d_length } else { 0.0 };
    kd = [kd[0] * f_inv_d_length, kd[1] * f_inv_d_length, kd[2] * f_inv_d_length];

    let k_diff = [origin[0] - p0[0], origin[1] - p0[1], origin[2] - p0[2]];
    let kp = [
        ku[0] * k_diff[0] + ku[1] * k_diff[1] + ku[2] * k_diff[2],
        kv[0] * k_diff[0] + kv[1] * k_diff[1] + kv[2] * k_diff[2],
        kw[0] * k_diff[0] + kw[1] * k_diff[1] + kw[2] * k_diff[2],
    ];
    let f_radius_sq = radius * radius;

    const PX_EPS_REAL: f64 = 1e-6;
    let mut s = [0.0f64; 2];
    let mut n = 0usize;

    if kd[2].abs() < 1.0 - PX_EPS_REAL && f_d_length >= PX_EPS_REAL {
        let f_a = kd[0] * kd[0] + kd[1] * kd[1];
        let f_b = kp[0] * kd[0] + kp[1] * kd[1];
        let f_c = kp[0] * kp[0] + kp[1] * kp[1] - f_radius_sq;
        let f_discr = f_b * f_b - f_a * f_c;
        if f_discr >= 0.0 {
            let f_root = f_discr.sqrt();
            if f_a != 0.0 {
                let f_inv_a = 1.0 / f_a;
                let f_t = (-f_b - f_root) * f_inv_a;
                let f_tmp = kp[2] + f_t * kd[2];
                const EPSILON: f64 = 1e-3;
                if f_tmp >= -EPSILON && f_tmp <= f_w_length + EPSILON {
                    s[n] = f_t * f_inv_d_length; n += 1;
                }
                let f_t = (-f_b + f_root) * f_inv_a;
                let f_tmp = kp[2] + f_t * kd[2];
                if n < 2 && f_tmp >= -EPSILON && f_tmp <= f_w_length + EPSILON {
                    s[n] = f_t * f_inv_d_length; n += 1;
                }
            }
        }
    }

    // End-cap fallback: try the two spheres at p0 and p1 if cylinder didn't
    // produce two hits.
    if n < 2 {
        if let Some((t0, _)) = intersect_ray_sphere(origin, dir, f64::MAX, p0, radius) {
            if n < 2 { s[n] = t0; n += 1; }
        }
    }
    if n < 2 {
        if let Some((t1, _)) = intersect_ray_sphere(origin, dir, f64::MAX, p1, radius) {
            if n < 2 { s[n] = t1; n += 1; }
        }
    }
    if n == 0 { return None; }

    let mut best = f64::MAX;
    for i in 0..n {
        if s[i] >= 0.0 && s[i] < best { best = s[i]; }
    }
    if best == f64::MAX { None } else { Some(best) }
}
