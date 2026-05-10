//! Triangle-mesh container + midphase + per-tri narrowphase.
//!
//! Sources cited in the C# files (PhysX 3.4):
//!   GeomUtils\src\mesh\GuMeshData.h, GuMidphaseInterface.h, GuMidphaseRTree.cpp
//!   GeomUtils\src\distance\GuDistancePointTriangle.cpp:37-111
//!   GeomUtils\src\contact\GuContactSphereMesh.cpp:288-355
//!   GeomUtils\src\distance\GuDistanceSegmentTriangle.cpp
//!
//! All inputs/outputs are in mesh-LOCAL space. The Lingo dispatch in
//! `physx_native.rs` transforms shape positions into mesh-local before invoking
//! the contact driver here.

use super::physx_gu_rtree::{LeafTriangles, RTree, RTreeAabbCallback, RTreeBuilder, RTreeRaycastCallback};

/// Source: GuTriangleMesh.h:42-110 (subset). Wraps vertex+index+RTree.
#[derive(Debug, Clone, Default)]
pub struct GuTriangleMesh {
    pub vertices: Vec<[f32; 3]>,
    /// 3 indices per triangle.
    pub triangles: Vec<u32>,
    pub tree: RTree,
    /// Local-space AABB.
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}

impl GuTriangleMesh {
    pub fn nb_triangles(&self) -> usize { self.triangles.len() / 3 }

    pub fn get_triangle(&self, tri_index: u32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let i = tri_index as usize * 3;
        (
            self.vertices[self.triangles[i    ] as usize],
            self.vertices[self.triangles[i + 1] as usize],
            self.vertices[self.triangles[i + 2] as usize],
        )
    }

    pub fn build(vertices: Vec<[f32; 3]>, triangles: Vec<u32>) -> Self {
        let tree = RTreeBuilder::build(&triangles, &vertices);
        let mut mesh = Self { vertices, triangles, tree, aabb_min: [0.0; 3], aabb_max: [0.0; 3] };
        if mesh.vertices.is_empty() {
            return mesh;
        }
        let mut mn = mesh.vertices[0];
        let mut mx = mesh.vertices[0];
        for v in mesh.vertices.iter().skip(1) {
            if v[0] < mn[0] { mn[0] = v[0]; } if v[0] > mx[0] { mx[0] = v[0]; }
            if v[1] < mn[1] { mn[1] = v[1]; } if v[1] > mx[1] { mx[1] = v[1]; }
            if v[2] < mn[2] { mn[2] = v[2]; } if v[2] > mx[2] { mx[2] = v[2]; }
        }
        mesh.aabb_min = mn; mesh.aabb_max = mx;
        mesh
    }
}

/// Source: GuMidphaseInterface.h:58-73 — `MeshHitCallback`.
pub trait MeshHitCallback {
    /// Returns true to continue traversal, false to abort.
    fn process(&mut self, tri_index: u32, v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> bool;
}

/// Source: GuMidphaseRTree.cpp pattern. AABB-overlap query.
pub fn midphase_intersect_aabb<C: MeshHitCallback + ?Sized>(
    mesh: &GuTriangleMesh,
    box_min: [f32; 3],
    box_max: [f32; 3],
    cb: &mut C,
) {
    if mesh.nb_triangles() == 0 { return; }
    let mut adapter = MidphaseAabbAdapter { mesh, cb, aborted: false };
    mesh.tree.traverse_aabb(box_min, box_max, &mut adapter);
}

struct MidphaseAabbAdapter<'a, C: MeshHitCallback + ?Sized> {
    mesh: &'a GuTriangleMesh,
    cb: &'a mut C,
    aborted: bool,
}

impl<'a, C: MeshHitCallback + ?Sized> RTreeAabbCallback for MidphaseAabbAdapter<'a, C> {
    fn process(&mut self, leaf_encoded: u32) -> bool {
        let lf = LeafTriangles { data: leaf_encoded | 1 };
        let first = lf.triangle_index();
        for k in 0..lf.nb_triangles() {
            let orig_tri = self.mesh.tree.tri_indices[(first + k) as usize];
            let (v0, v1, v2) = self.mesh.get_triangle(orig_tri);
            if !self.cb.process(orig_tri, v0, v1, v2) {
                self.aborted = true;
                return false;
            }
        }
        true
    }
}

// =============================================================================
//  Per-triangle contact gen
// =============================================================================

/// One contact between a query shape and a triangle. All in mesh-local space.
#[derive(Debug, Clone, Copy, Default)]
pub struct GuTriContact {
    pub point: [f32; 3],
    /// triangle → query shape (unit length).
    pub normal: [f32; 3],
    /// PhysX convention: < 0 ⇒ penetrating.
    pub separation: f32,
    pub triangle_index: u32,
}

#[inline] fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0]*b[0] + a[1]*b[1] + a[2]*b[2] }
#[inline] fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1]*b[2] - a[2]*b[1], a[2]*b[0] - a[0]*b[2], a[0]*b[1] - a[1]*b[0]]
}
#[inline] fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0]-b[0], a[1]-b[1], a[2]-b[2]] }
#[inline] fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0]+b[0], a[1]+b[1], a[2]+b[2]] }
#[inline] fn scale(v: [f32; 3], s: f32) -> [f32; 3] { [v[0]*s, v[1]*s, v[2]*s] }
#[inline] fn neg(v: [f32; 3]) -> [f32; 3] { [-v[0], -v[1], -v[2]] }
#[inline] fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = dot(v, v).sqrt();
    if l < 1e-12 { [0.0; 3] } else { [v[0]/l, v[1]/l, v[2]/l] }
}

/// Source: GuDistancePointTriangle.cpp:37-111 — Ericson's closest-pt-on-tri.
pub fn closest_pt_point_triangle(
    p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3],
) -> [f32; 3] {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 { return a; }

    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 { return b; }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return add(a, scale(ab, v));
    }

    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 { return c; }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return add(a, scale(ac, w));
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return add(b, scale(sub(c, b), w));
    }

    let denom = 1.0 / (va + vb + vc);
    let vv = vb * denom;
    let ww = vc * denom;
    add(add(a, scale(ab, vv)), scale(ac, ww))
}

/// Source: GuContactSphereMesh.cpp:288-355 — sphere-vs-triangle.
pub fn sphere_vs_triangle(
    sphere_center: [f32; 3], sphere_radius: f32, contact_dist: f32,
    v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], tri_index: u32,
) -> Option<GuTriContact> {
    let inflated = sphere_radius + contact_dist;
    let inflated2 = inflated * inflated;
    let cp = closest_pt_point_triangle(sphere_center, v0, v1, v2);
    let d = sub(cp, sphere_center);
    let sq_dist = dot(d, d);
    if sq_dist >= inflated2 { return None; }

    let e0 = sub(v1, v0);
    let e1 = sub(v2, v0);
    let plane_normal = cross(e0, e1);
    let plane_d = dot(plane_normal, v0);
    if dot(plane_normal, sphere_center) < plane_d { return None; }

    const K_EPS: f32 = 1e-12;
    let (dir, dist) = if sq_dist > K_EPS {
        let dist = sq_dist.sqrt();
        ([d[0]/dist, d[1]/dist, d[2]/dist], dist)
    } else {
        (neg(normalize(plane_normal)), 0.0)
    };
    Some(GuTriContact {
        normal: neg(dir),
        point: [
            sphere_center[0] + sphere_radius * dir[0],
            sphere_center[1] + sphere_radius * dir[1],
            sphere_center[2] + sphere_radius * dir[2],
        ],
        separation: dist - sphere_radius,
        triangle_index: tri_index,
    })
}

/// Source: Ericson §5.1.9 — segment-vs-segment closest pair.
fn closest_pt_segment_segment(
    p1: [f32; 3], q1: [f32; 3], p2: [f32; 3], q2: [f32; 3],
) -> ([f32; 3], [f32; 3], f32) {
    const K_EPS: f32 = 1e-10;
    let d1 = sub(q1, p1);
    let d2 = sub(q2, p2);
    let r  = sub(p1, p2);
    let a = dot(d1, d1);
    let e = dot(d2, d2);
    let f = dot(d2, r);

    if a <= K_EPS && e <= K_EPS {
        let dd = sub(p1, p2);
        return (p1, p2, dot(dd, dd));
    }

    let (s, t);
    if a <= K_EPS {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let cdot = dot(d1, r);
        if e <= K_EPS {
            t = 0.0;
            s = (-cdot / a).clamp(0.0, 1.0);
        } else {
            let bdot = dot(d1, d2);
            let denom = a * e - bdot * bdot;
            let mut s_calc = if denom != 0.0 { ((bdot * f - cdot * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t_calc = (bdot * s_calc + f) / e;
            if t_calc < 0.0 {
                t_calc = 0.0;
                s_calc = (-cdot / a).clamp(0.0, 1.0);
            } else if t_calc > 1.0 {
                t_calc = 1.0;
                s_calc = ((bdot - cdot) / a).clamp(0.0, 1.0);
            }
            s = s_calc; t = t_calc;
        }
    }
    let c1 = add(p1, scale(d1, s));
    let c2 = add(p2, scale(d2, t));
    let dd = sub(c1, c2);
    (c1, c2, dot(dd, dd))
}

/// Source: GuDistanceSegmentTriangle.cpp pattern. Closest pair between a
/// segment and a triangle. Returns (segPt, triPt, sqDist).
pub fn closest_pt_segment_triangle(
    p: [f32; 3], q: [f32; 3],
    a: [f32; 3], b: [f32; 3], c: [f32; 3],
) -> ([f32; 3], [f32; 3], f32) {
    let mut best_sq = f32::MAX;
    let mut seg_pt = p;
    let mut tri_pt = a;

    let trial = closest_pt_point_triangle(p, a, b, c);
    let dd = sub(p, trial); let sq = dot(dd, dd);
    if sq < best_sq { best_sq = sq; seg_pt = p; tri_pt = trial; }

    let trial = closest_pt_point_triangle(q, a, b, c);
    let dd = sub(q, trial); let sq = dot(dd, dd);
    if sq < best_sq { best_sq = sq; seg_pt = q; tri_pt = trial; }

    for &(ea, eb) in &[(a, b), (b, c), (c, a)] {
        let (sp, tp, sq3) = closest_pt_segment_segment(p, q, ea, eb);
        if sq3 < best_sq { best_sq = sq3; seg_pt = sp; tri_pt = tp; }
    }

    // Segment-vs-face: penetrating intersection ⇒ sqDist = 0.
    let pq = sub(q, p);
    let ab = sub(b, a);
    let ac = sub(c, a);
    let n = cross(ab, ac);
    let denom = dot(n, pq);
    if denom.abs() > 1e-12 {
        let u = dot(n, sub(a, p)) / denom;
        if (0.0..=1.0).contains(&u) {
            let hit = add(p, scale(pq, u));
            // Inside-tri barycentric test.
            let pmv0 = sub(hit, a);
            let dot00 = dot(ac, ac);
            let dot01 = dot(ac, ab);
            let dot02 = dot(ac, pmv0);
            let dot11 = dot(ab, ab);
            let dot12 = dot(ab, pmv0);
            let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01);
            let uu = (dot11 * dot02 - dot01 * dot12) * inv_denom;
            let vv = (dot00 * dot12 - dot01 * dot02) * inv_denom;
            if uu >= 0.0 && vv >= 0.0 && uu + vv <= 1.0 {
                best_sq = 0.0;
                seg_pt = hit;
                tri_pt = hit;
            }
        }
    }
    (seg_pt, tri_pt, best_sq)
}

pub fn capsule_vs_triangle(
    p0: [f32; 3], p1: [f32; 3], radius: f32, contact_dist: f32,
    v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], tri_index: u32,
) -> Option<GuTriContact> {
    let (seg_pt, tri_pt, sq_dist) = closest_pt_segment_triangle(p0, p1, v0, v1, v2);
    let inflated = radius + contact_dist;
    if sq_dist >= inflated * inflated { return None; }

    let e0 = sub(v1, v0);
    let e1 = sub(v2, v0);
    let plane_normal = cross(e0, e1);
    let plane_d = dot(plane_normal, v0);
    if dot(plane_normal, seg_pt) < plane_d { return None; }

    const K_EPS: f32 = 1e-12;
    let d = sub(tri_pt, seg_pt);
    let (dir, dist) = if sq_dist > K_EPS {
        let dist = sq_dist.sqrt();
        ([d[0]/dist, d[1]/dist, d[2]/dist], dist)
    } else {
        (neg(normalize(plane_normal)), 0.0)
    };
    Some(GuTriContact {
        normal: neg(dir),
        point: [
            seg_pt[0] + radius * dir[0],
            seg_pt[1] + radius * dir[1],
            seg_pt[2] + radius * dir[2],
        ],
        separation: dist - radius,
        triangle_index: tri_index,
    })
}

/// Source: GuPCMTriangleContactGen.cpp::SATSweep pattern (reduced — single
/// contact per (axis, triangle)). The dispatcher accumulates contacts across
/// all triangles for a multi-point manifold per body pair.
pub fn box_vs_triangle(
    box_center: [f32; 3], box_half_extents: [f32; 3],
    box_axis_x: [f32; 3], box_axis_y: [f32; 3], box_axis_z: [f32; 3],
    contact_dist: f32,
    v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], tri_index: u32,
) -> Option<GuTriContact> {
    let mut best_overlap = f32::MAX;
    let mut best_axis = [0.0f32; 3];
    let mut best_sign = 1.0f32;

    if !test_axis(box_axis_x, box_center, box_half_extents,
                  box_axis_x, box_axis_y, box_axis_z, v0, v1, v2,
                  &mut best_overlap, &mut best_axis, &mut best_sign, contact_dist) {
        return None;
    }
    if !test_axis(box_axis_y, box_center, box_half_extents,
                  box_axis_x, box_axis_y, box_axis_z, v0, v1, v2,
                  &mut best_overlap, &mut best_axis, &mut best_sign, contact_dist) {
        return None;
    }
    if !test_axis(box_axis_z, box_center, box_half_extents,
                  box_axis_x, box_axis_y, box_axis_z, v0, v1, v2,
                  &mut best_overlap, &mut best_axis, &mut best_sign, contact_dist) {
        return None;
    }

    let tri_e0 = sub(v1, v0);
    let tri_e1 = sub(v2, v0);
    let tri_normal_raw = cross(tri_e0, tri_e1);
    let tri_normal = if dot(tri_normal_raw, tri_normal_raw) > 1e-12 {
        let n = normalize(tri_normal_raw);
        if !test_axis(n, box_center, box_half_extents,
                      box_axis_x, box_axis_y, box_axis_z, v0, v1, v2,
                      &mut best_overlap, &mut best_axis, &mut best_sign, contact_dist) {
            return None;
        }
        n
    } else {
        [0.0; 3]
    };

    let tri_edges = [sub(v1, v0), sub(v2, v1), sub(v0, v2)];
    let box_edges = [box_axis_x, box_axis_y, box_axis_z];
    for be in box_edges.iter() {
        for te in tri_edges.iter() {
            let axis = cross(*be, *te);
            let ax2 = dot(axis, axis);
            if ax2 < 1e-8 { continue; }
            let axis = normalize(axis);
            if !test_axis(axis, box_center, box_half_extents,
                          box_axis_x, box_axis_y, box_axis_z, v0, v1, v2,
                          &mut best_overlap, &mut best_axis, &mut best_sign, contact_dist) {
                return None;
            }
        }
    }

    let n = scale(best_axis, best_sign);
    if dot(tri_normal, box_center) < dot(tri_normal, v0) - contact_dist {
        return None;
    }
    let sx = if dot(box_axis_x, n) >= 0.0 { -box_half_extents[0] } else { box_half_extents[0] };
    let sy = if dot(box_axis_y, n) >= 0.0 { -box_half_extents[1] } else { box_half_extents[1] };
    let sz = if dot(box_axis_z, n) >= 0.0 { -box_half_extents[2] } else { box_half_extents[2] };
    let deepest = [
        box_center[0] + box_axis_x[0] * sx + box_axis_y[0] * sy + box_axis_z[0] * sz,
        box_center[1] + box_axis_x[1] * sx + box_axis_y[1] * sy + box_axis_z[1] * sz,
        box_center[2] + box_axis_x[2] * sx + box_axis_y[2] * sy + box_axis_z[2] * sz,
    ];
    Some(GuTriContact {
        normal: n,
        point: deepest,
        separation: -best_overlap,
        triangle_index: tri_index,
    })
}

fn test_axis(
    axis: [f32; 3],
    box_center: [f32; 3], box_half_extents: [f32; 3],
    box_axis_x: [f32; 3], box_axis_y: [f32; 3], box_axis_z: [f32; 3],
    v0: [f32; 3], v1: [f32; 3], v2: [f32; 3],
    best_overlap: &mut f32, best_axis: &mut [f32; 3], best_sign: &mut f32,
    contact_dist: f32,
) -> bool {
    let r = box_half_extents[0] * dot(box_axis_x, axis).abs()
          + box_half_extents[1] * dot(box_axis_y, axis).abs()
          + box_half_extents[2] * dot(box_axis_z, axis).abs();
    let box_c = dot(box_center, axis);
    let box_min = box_c - r;
    let box_max = box_c + r;

    let p0 = dot(v0, axis); let p1 = dot(v1, axis); let p2 = dot(v2, axis);
    let tri_min = p0.min(p1).min(p2);
    let tri_max = p0.max(p1).max(p2);

    if box_max + contact_dist < tri_min { return false; }
    if tri_max + contact_dist < box_min { return false; }

    let depth_a = box_max - tri_min;
    let depth_b = tri_max - box_min;
    let (depth, sign) = if depth_a < depth_b { (depth_a, -1.0f32) } else { (depth_b, 1.0f32) };
    if depth < *best_overlap {
        *best_overlap = depth;
        *best_axis = axis;
        *best_sign = sign;
    }
    true
}

// =============================================================================
//  High-level shape-vs-mesh contact driver
// =============================================================================

pub fn sphere_vs_mesh(
    mesh: &GuTriangleMesh,
    sphere_center: [f32; 3], sphere_radius: f32, contact_dist: f32,
) -> Vec<GuTriContact> {
    let mut out = Vec::new();
    let r = sphere_radius + contact_dist;
    let mn = [sphere_center[0] - r, sphere_center[1] - r, sphere_center[2] - r];
    let mx = [sphere_center[0] + r, sphere_center[1] + r, sphere_center[2] + r];
    struct Cb<'a> { center: [f32; 3], radius: f32, contact_dist: f32, out: &'a mut Vec<GuTriContact> }
    impl<'a> MeshHitCallback for Cb<'a> {
        fn process(&mut self, ti: u32, v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> bool {
            if let Some(c) = sphere_vs_triangle(self.center, self.radius, self.contact_dist, v0, v1, v2, ti) {
                self.out.push(c);
            }
            true
        }
    }
    midphase_intersect_aabb(mesh, mn, mx, &mut Cb { center: sphere_center, radius: sphere_radius, contact_dist, out: &mut out });
    out
}

pub fn capsule_vs_mesh(
    mesh: &GuTriangleMesh,
    p0: [f32; 3], p1: [f32; 3], radius: f32, contact_dist: f32,
) -> Vec<GuTriContact> {
    let mut out = Vec::new();
    let r = radius + contact_dist;
    let mn = [p0[0].min(p1[0]) - r, p0[1].min(p1[1]) - r, p0[2].min(p1[2]) - r];
    let mx = [p0[0].max(p1[0]) + r, p0[1].max(p1[1]) + r, p0[2].max(p1[2]) + r];
    struct Cb<'a> { p0: [f32; 3], p1: [f32; 3], radius: f32, contact_dist: f32, out: &'a mut Vec<GuTriContact> }
    impl<'a> MeshHitCallback for Cb<'a> {
        fn process(&mut self, ti: u32, v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> bool {
            if let Some(c) = capsule_vs_triangle(self.p0, self.p1, self.radius, self.contact_dist, v0, v1, v2, ti) {
                self.out.push(c);
            }
            true
        }
    }
    midphase_intersect_aabb(mesh, mn, mx, &mut Cb { p0, p1, radius, contact_dist, out: &mut out });
    out
}

pub fn box_vs_mesh(
    mesh: &GuTriangleMesh,
    box_center: [f32; 3], box_half_extents: [f32; 3],
    box_axis_x: [f32; 3], box_axis_y: [f32; 3], box_axis_z: [f32; 3],
    contact_dist: f32,
) -> Vec<GuTriContact> {
    let mut out = Vec::new();
    let ex = box_half_extents[0] * box_axis_x[0].abs() + box_half_extents[1] * box_axis_y[0].abs() + box_half_extents[2] * box_axis_z[0].abs();
    let ey = box_half_extents[0] * box_axis_x[1].abs() + box_half_extents[1] * box_axis_y[1].abs() + box_half_extents[2] * box_axis_z[1].abs();
    let ez = box_half_extents[0] * box_axis_x[2].abs() + box_half_extents[1] * box_axis_y[2].abs() + box_half_extents[2] * box_axis_z[2].abs();
    let pad = contact_dist;
    let mn = [box_center[0] - ex - pad, box_center[1] - ey - pad, box_center[2] - ez - pad];
    let mx = [box_center[0] + ex + pad, box_center[1] + ey + pad, box_center[2] + ez + pad];
    struct Cb<'a> { c: [f32;3], he: [f32;3], ax: [f32;3], ay: [f32;3], az: [f32;3], cd: f32, out: &'a mut Vec<GuTriContact> }
    impl<'a> MeshHitCallback for Cb<'a> {
        fn process(&mut self, ti: u32, v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> bool {
            if let Some(c) = box_vs_triangle(self.c, self.he, self.ax, self.ay, self.az, self.cd, v0, v1, v2, ti) {
                self.out.push(c);
            }
            true
        }
    }
    midphase_intersect_aabb(mesh, mn, mx, &mut Cb { c: box_center, he: box_half_extents, ax: box_axis_x, ay: box_axis_y, az: box_axis_z, cd: contact_dist, out: &mut out });
    out
}

/// Raycast against a triangle mesh — used by Lingo `getRayCastClosestShape`
/// when a #concaveShape body is in the scene. Returns (tri_index, t, point,
/// normal) for the closest hit, or None if no hit.
pub fn raycast_mesh(
    mesh: &GuTriangleMesh,
    ray_origin: [f32; 3], ray_dir: [f32; 3], max_t: f32,
) -> Option<(u32, f32, [f32; 3], [f32; 3])> {
    if mesh.nb_triangles() == 0 { return None; }
    struct Cb<'a> {
        mesh: &'a GuTriangleMesh,
        ray_origin: [f32; 3],
        ray_dir: [f32; 3],
        best_t: f32,
        best: Option<(u32, [f32; 3], [f32; 3])>,
    }
    impl<'a> RTreeRaycastCallback for Cb<'a> {
        fn process(&mut self, leaf_encoded: u32, max_t: &mut f32) -> bool {
            let lf = LeafTriangles { data: leaf_encoded | 1 };
            let first = lf.triangle_index();
            for k in 0..lf.nb_triangles() {
                let orig_tri = self.mesh.tree.tri_indices[(first + k) as usize];
                let (v0, v1, v2) = self.mesh.get_triangle(orig_tri);
                if let Some((t, n)) = ray_vs_triangle(self.ray_origin, self.ray_dir, v0, v1, v2, *max_t) {
                    if t < self.best_t {
                        self.best_t = t;
                        let p = [
                            self.ray_origin[0] + self.ray_dir[0] * t,
                            self.ray_origin[1] + self.ray_dir[1] * t,
                            self.ray_origin[2] + self.ray_dir[2] * t,
                        ];
                        self.best = Some((orig_tri, p, n));
                        *max_t = t;
                    }
                }
            }
            true
        }
    }
    let mut cb = Cb { mesh, ray_origin, ray_dir, best_t: max_t, best: None };
    mesh.tree.traverse_ray(ray_origin, ray_dir, max_t, &mut cb, [0.0; 3]);
    cb.best.map(|(t, p, n)| (t, cb.best_t, p, n))
}

/// Möller–Trumbore ray-vs-triangle intersection. Returns (t, normal) if
/// the ray hits the triangle within [0, max_t] from the front side.
fn ray_vs_triangle(
    origin: [f32; 3], dir: [f32; 3],
    v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], max_t: f32,
) -> Option<(f32, [f32; 3])> {
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let p = cross(dir, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 { return None; }
    let inv_det = 1.0 / det;
    let s = sub(origin, v0);
    let u = dot(s, p) * inv_det;
    if !(0.0..=1.0).contains(&u) { return None; }
    let q = cross(s, e1);
    let v = dot(dir, q) * inv_det;
    if v < 0.0 || u + v > 1.0 { return None; }
    let t = dot(e2, q) * inv_det;
    if t <= 0.0 || t > max_t { return None; }
    let n = normalize(cross(e1, e2));
    Some((t, n))
}
