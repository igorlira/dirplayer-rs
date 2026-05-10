//! Convex hull narrowphase — verbatim Rust ports of PhysX 3.4's
//!
//! Source files this is built from:
//! - `GeomUtils/src/convex/GuConvexMeshData.h`     — `HullPolygonData`
//! - `GeomUtils/src/convex/GuShapeConvex.h`         — `PolygonalData` / `PolygonalBox`
//! - `GeomUtils/src/convex/GuShapeConvex.cpp`       — `PolygonalBox` ctor / box vertex layout
//! - `GeomUtils/src/contact/GuContactConvexConvex.cpp:189-217` — `testSeparatingAxis`
//! - `GeomUtils/src/contact/GuContactConvexConvex.cpp:256-329` — `PxcFindSeparatingAxes`
//!
//! Pipeline:
//!   1. Class I face axes (hull A) + Class II face axes (hull B) — pure SAT
//!   2. Class III edge-cross axes (with a small bias so face axes win ties)
//!   3. Reference-face clipping (Sutherland-Hodgman) for manifold generation
//!
//! PhysX engine convention preserved: contact normal points B → A
//! (shape1 → shape0); separation < 0 ⇒ penetrating. The dispatch in
//! [`super::physx_native`] flips at the boundary into our solver.

#![allow(dead_code)]

use super::physx_gu::{v_add, v_cross, v_dot, v_len_sq, v_mul, v_neg, v_sub, GuContactBuffer, q_rotate, q_rotate_inv};
use super::physx_gu_capsule_box::GuBox;

/// Source: `GuConvexMeshData.h:47-71` — per-face plane equation + vertex
/// reference range + min-projection vertex index for fast SAT.
#[derive(Default, Clone, Copy, Debug)]
pub struct HullPolygonData {
    pub plane_n: [f64; 3],
    pub plane_d: f64,
    pub vref8: u16,    // offset into `polygon_vertex_refs`
    pub nb_verts: u8,
    pub min_index: u8, // argmin of plane.n · v over hull verts
}

/// Source: `GuShapeConvex.h:45-73`. The `mProjectHull` C++ function pointer
/// is replaced here by an `is_box` flag — boxes use the optimized path that
/// projects via `|R^T·axis|·halfExtents`, generic hulls walk every vertex.
#[derive(Default, Clone, Debug)]
pub struct PolygonalData {
    pub center: [f64; 3],
    pub verts: Vec<[f64; 3]>,
    pub polygons: Vec<HullPolygonData>,
    pub polygon_vertex_refs: Vec<u8>,
    /// Inscribed-sphere radius (optional, used for SAT early-out).
    pub internal_radius: f64,
    /// Set on box hulls — drives the optimized projection path.
    pub half_side: Option<[f64; 3]>,
}

/// Source: `GuShapeConvex.cpp:290-297` — `gPxcBoxPolygonData`.
const BOX_POLYGON_VERTEX_REFS: [u8; 24] = [
    0, 1, 2, 3,
    1, 5, 6, 2,
    5, 4, 7, 6,
    4, 0, 3, 7,
    3, 2, 6, 7,
    4, 5, 1, 0,
];

/// Build a `PolygonalData` from a box's half-extents. Source:
/// `GuShapeConvex.cpp:422-516` — 8 corners + 6 face polygons.
pub fn polygonal_box(half_side: [f64; 3]) -> PolygonalData {
    let lo = [-half_side[0], -half_side[1], -half_side[2]];
    let hi = half_side;
    let verts = vec![
        [lo[0], lo[1], lo[2]],   // 0 = ---
        [hi[0], lo[1], lo[2]],   // 1 = +--
        [hi[0], hi[1], lo[2]],   // 2 = ++-
        [lo[0], hi[1], lo[2]],   // 3 = -+-
        [lo[0], lo[1], hi[2]],   // 4 = --+
        [hi[0], lo[1], hi[2]],   // 5 = +-+
        [hi[0], hi[1], hi[2]],   // 6 = +++
        [lo[0], hi[1], hi[2]],   // 7 = -++
    ];
    let mut polys = [HullPolygonData::default(); 6];
    for i in 0..6 {
        polys[i].nb_verts = 4;
        polys[i].vref8 = (i * 4) as u16;
    }
    // X axis (lines 455-458, 460-461).
    polys[1].plane_n = [ 1.0, 0.0, 0.0]; polys[1].plane_d = -half_side[0]; polys[1].min_index = 0;
    polys[3].plane_n = [-1.0, 0.0, 0.0]; polys[3].plane_d = -half_side[0]; polys[3].min_index = 1;
    // Y axis (lines 471-474, 476-477).
    polys[4].plane_n = [0.0,  1.0, 0.0]; polys[4].plane_d = -half_side[1]; polys[4].min_index = 0;
    polys[5].plane_n = [0.0, -1.0, 0.0]; polys[5].plane_d = -half_side[1]; polys[5].min_index = 2;
    // Z axis (lines 485-488, 490-491).
    polys[2].plane_n = [0.0, 0.0,  1.0]; polys[2].plane_d = -half_side[2]; polys[2].min_index = 0;
    polys[0].plane_n = [0.0, 0.0, -1.0]; polys[0].plane_d = -half_side[2]; polys[0].min_index = 4;

    PolygonalData {
        center: [0.0; 3],
        verts,
        polygons: polys.to_vec(),
        polygon_vertex_refs: BOX_POLYGON_VERTEX_REFS.to_vec(),
        internal_radius: half_side[0].min(half_side[1]).min(half_side[2]),
        half_side: Some(half_side),
    }
}

/// Build a `PolygonalData` from arbitrary vertices + face vertex indices.
/// Vertex winding convention matches PhysX's `gPxcBoxPolygonData`: CW from
/// outside (= e2 × e1 yields the OUTWARD normal).
///
/// Returns `None` if the input is malformed (no verts/faces, face has fewer
/// than 3 verts or more than 255, vertex index out of range or > 255, or a
/// degenerate face). The caller (typically `setConvexHull` from a Lingo
/// script) is expected to fall back to an AABB box hull when this fails —
/// runtime code never panics on bad input.
pub fn polygonal_convex(verts: Vec<[f64; 3]>, faces: &[Vec<usize>]) -> Option<PolygonalData> {
    if verts.is_empty() || faces.is_empty() { return None; }

    let mut refs: Vec<u8> = Vec::new();
    let mut polys: Vec<HullPolygonData> = Vec::with_capacity(faces.len());

    for f in faces.iter() {
        if f.len() < 3 || f.len() > 255 { return None; }
        let vref8 = refs.len() as u16;
        for &k in f {
            if k >= verts.len() || k > 255 { return None; }
            refs.push(k as u8);
        }

        // Plane normal via e2 × e1 (CW-from-outside winding ⇒ OUTWARD).
        let v0 = verts[f[0]];
        let v1 = verts[f[1]];
        let v2 = verts[f[2]];
        let e1 = v_sub(v1, v0);
        let e2 = v_sub(v2, v0);
        let n = v_cross(e2, e1);
        let len = v_len_sq(n).sqrt();
        if len <= 1e-10 { return None; } // degenerate face (3 collinear verts)
        let n = [n[0] / len, n[1] / len, n[2] / len];
        let d = -(n[0] * v0[0] + n[1] * v0[1] + n[2] * v0[2]);

        // min_index: argmin of n · v over ALL hull verts.
        let mut min_idx = 0usize;
        let mut min_proj = f64::MAX;
        for (k, vk) in verts.iter().enumerate() {
            let p = n[0] * vk[0] + n[1] * vk[1] + n[2] * vk[2];
            if p < min_proj { min_proj = p; min_idx = k; }
        }

        polys.push(HullPolygonData {
            plane_n: n,
            plane_d: d,
            vref8,
            nb_verts: f.len() as u8,
            min_index: min_idx as u8,
        });
    }

    // Centroid.
    let inv = 1.0 / verts.len() as f64;
    let mut centroid = [0.0; 3];
    for v in &verts {
        centroid = v_add(centroid, *v);
    }
    centroid = v_mul(centroid, inv);

    // Inscribed-sphere radius — smallest distance from centroid to any face.
    let mut radius = f64::MAX;
    for p in &polys {
        let dist = (p.plane_n[0] * centroid[0] + p.plane_n[1] * centroid[1] + p.plane_n[2] * centroid[2] + p.plane_d).abs();
        if dist < radius { radius = dist; }
    }

    Some(PolygonalData {
        center: centroid,
        verts,
        polygons: polys,
        polygon_vertex_refs: refs,
        internal_radius: radius,
        half_side: None,
    })
}

/// Project a hull onto `axis` (in world space). Returns (min, max).
/// Box specialization uses the half-extents; generic walks every vertex.
fn project_hull(poly: &PolygonalData, axis: [f64; 3], world_box: &GuBox, world_p: [f64; 3]) -> (f64, f64) {
    if let Some(hs) = poly.half_side {
        // Box: center along axis ± |R^T·axis|·halfExtents.
        let center = v_dot(world_p, axis);
        let ex = v_dot(world_box.col0, axis).abs() * hs[0]
               + v_dot(world_box.col1, axis).abs() * hs[1]
               + v_dot(world_box.col2, axis).abs() * hs[2];
        return (center - ex, center + ex);
    }
    // General convex: walk every vert in world.
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for &v in &poly.verts {
        let w = v_add(world_box.rotate(v), world_p);
        let p = v_dot(w, axis);
        if p < lo { lo = p; }
        if p > hi { hi = p; }
    }
    (lo, hi)
}

/// Source: GuContactConvexConvex.cpp:189-217. Returns Some(depth) on overlap.
fn test_separating_axis(
    poly0: &PolygonalData, poly1: &PolygonalData,
    world0: &GuBox, world0_p: [f64; 3],
    world1: &GuBox, world1_p: [f64; 3],
    axis: [f64; 3], contact_distance: f64,
) -> Option<f64> {
    let (min0, max0) = project_hull(poly0, axis, world0, world0_p);
    let (min1, max1) = project_hull(poly1, axis, world1, world1_p);
    if max0 + contact_distance < min1 || max1 + contact_distance < min0 { return None; }
    let d0 = max0 - min1;
    let d1 = max1 - min0;
    Some(d0.min(d1))
}

/// Collect unique world-space edge directions of a hull, dedupe by
/// canonical (min, max) vertex index ordering.
fn collect_edges(poly: &PolygonalData, world: &GuBox, world_p: [f64; 3]) -> Vec<[f64; 3]> {
    let mut out: Vec<[f64; 3]> = Vec::new();
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for pg in &poly.polygons {
        let base = pg.vref8 as usize;
        let n = pg.nb_verts as usize;
        for e in 0..n {
            let va = poly.polygon_vertex_refs[base + e] as u32;
            let vb = poly.polygon_vertex_refs[base + (e + 1) % n] as u32;
            let lo = va.min(vb); let hi = va.max(vb);
            let key = (lo << 16) | hi;
            if !seen.insert(key) { continue; }
            let w0 = v_add(world.rotate(poly.verts[va as usize]), world_p);
            let w1 = v_add(world.rotate(poly.verts[vb as usize]), world_p);
            out.push(v_sub(w1, w0));
        }
    }
    out
}

/// Sutherland-Hodgman: keep vertices on the negative side of `n · v + d ≤ 0`.
fn clip_polygon_against_plane(
    polygon: &[[f64; 3]], plane_n: [f64; 3], plane_d: f64,
) -> Vec<[f64; 3]> {
    let mut result: Vec<[f64; 3]> = Vec::with_capacity(polygon.len() + 1);
    if polygon.is_empty() { return result; }
    let mut prev = polygon[polygon.len() - 1];
    let mut prev_dist = plane_n[0] * prev[0] + plane_n[1] * prev[1] + plane_n[2] * prev[2] + plane_d;
    for &cur in polygon {
        let cur_dist = plane_n[0] * cur[0] + plane_n[1] * cur[1] + plane_n[2] * cur[2] + plane_d;
        if cur_dist <= 0.0 {
            if prev_dist > 0.0 {
                let t = prev_dist / (prev_dist - cur_dist);
                result.push([
                    prev[0] + (cur[0] - prev[0]) * t,
                    prev[1] + (cur[1] - prev[1]) * t,
                    prev[2] + (cur[2] - prev[2]) * t,
                ]);
            }
            result.push(cur);
        } else if prev_dist <= 0.0 {
            let t = prev_dist / (prev_dist - cur_dist);
            result.push([
                prev[0] + (cur[0] - prev[0]) * t,
                prev[1] + (cur[1] - prev[1]) * t,
                prev[2] + (cur[2] - prev[2]) * t,
            ]);
        }
        prev = cur;
        prev_dist = cur_dist;
    }
    result
}

/// Manifold generation via reference-face clipping. Mirrors the algorithm
/// in `GuContactPolygonPolygon.cpp` but in standard Sutherland-Hodgman form
/// (PhysX uses an axis-aligned 2D projection optimization that we skip).
/// `contact_normal` is in PhysX convention (B → A); `overlap_depth` is the
/// positive overlap distance from SAT.
fn generate_manifold(
    buffer: &mut GuContactBuffer,
    poly_a: &PolygonalData, poly_b: &PolygonalData,
    world_a: &GuBox, world_a_p: [f64; 3],
    world_b: &GuBox, world_b_p: [f64; 3],
    contact_normal: [f64; 3],
    _overlap_depth: f64,
) -> i32 {
    // Reference face: face on A whose outward normal points TOWARD B
    // (most opposite to contactNormal=B→A, so MIN dot product).
    let mut ref_idx: i32 = -1;
    let mut best_align = f64::MAX;
    for (i, p) in poly_a.polygons.iter().enumerate() {
        let face_n = world_a.rotate(p.plane_n);
        let al = v_dot(face_n, contact_normal);
        if al < best_align { best_align = al; ref_idx = i as i32; }
    }
    if ref_idx < 0 { return 0; }

    // Incident face: face on B whose outward normal points TOWARD A
    // (MAX dot with contactNormal).
    let mut inc_idx: i32 = -1;
    let mut worst_align = f64::MIN;
    for (j, p) in poly_b.polygons.iter().enumerate() {
        let face_n = world_b.rotate(p.plane_n);
        let al = v_dot(face_n, contact_normal);
        if al > worst_align { worst_align = al; inc_idx = j as i32; }
    }
    if inc_idx < 0 { return 0; }

    let ref_polygon = &poly_a.polygons[ref_idx as usize];
    let ref_normal_world = world_a.rotate(ref_polygon.plane_n);
    let ref_v0 = v_add(
        world_a.rotate(poly_a.verts[poly_a.polygon_vertex_refs[ref_polygon.vref8 as usize] as usize]),
        world_a_p,
    );
    let ref_plane_d = -v_dot(ref_normal_world, ref_v0);

    // Gather incident face verts in world.
    let inc_polygon = &poly_b.polygons[inc_idx as usize];
    let inc_n = inc_polygon.nb_verts as usize;
    let mut inc_verts: Vec<[f64; 3]> = Vec::with_capacity(inc_n);
    for j in 0..inc_n {
        let vi = poly_b.polygon_vertex_refs[inc_polygon.vref8 as usize + j] as usize;
        inc_verts.push(v_add(world_b.rotate(poly_b.verts[vi]), world_b_p));
    }

    // Reference face verts (for clip planes).
    let ref_n = ref_polygon.nb_verts as usize;
    let mut ref_verts: Vec<[f64; 3]> = Vec::with_capacity(ref_n);
    for i in 0..ref_n {
        let vi = poly_a.polygon_vertex_refs[ref_polygon.vref8 as usize + i] as usize;
        ref_verts.push(v_add(world_a.rotate(poly_a.verts[vi]), world_a_p));
    }

    // Clip the incident polygon against each side plane of the reference face.
    let mut clipped = inc_verts;
    for i in 0..ref_n {
        if clipped.is_empty() { break; }
        let v0 = ref_verts[i];
        let v1 = ref_verts[(i + 1) % ref_n];
        let edge = v_sub(v1, v0);
        // Side normal: refNormal × edge ⇒ OUTWARD (vertices CCW from outside).
        let side_n = v_cross(ref_normal_world, edge);
        let side_len_sq = v_len_sq(side_n);
        if side_len_sq < 1e-12 { continue; }
        let side_len = side_len_sq.sqrt();
        let side_n = v_mul(side_n, 1.0 / side_len);
        let side_d = -v_dot(side_n, v0);
        clipped = clip_polygon_against_plane(&clipped, side_n, side_d);
    }

    // For each surviving vertex below the reference plane, emit a contact.
    let count_before = buffer.count();
    for v in clipped {
        let dist = v_dot(ref_normal_world, v) + ref_plane_d;
        if dist >= 0.0 { continue; }
        let contact_pt = [
            v[0] - ref_normal_world[0] * dist,
            v[1] - ref_normal_world[1] * dist,
            v[2] - ref_normal_world[2] * dist,
        ];
        buffer.add(contact_pt, contact_normal, dist);
    }
    if buffer.count() == count_before { return 0; }
    (buffer.count() - count_before) as i32
}

/// Hull-vs-hull SAT + manifold generation. Source:
/// `GuContactConvexConvex.cpp:806-1033` (`GuContactHullHull`).
pub fn contact_hull_hull(
    buffer: &mut GuContactBuffer,
    poly0: &PolygonalData, poly1: &PolygonalData,
    rot0: [f64; 4], pos0: [f64; 3],
    rot1: [f64; 4], pos1: [f64; 3],
    contact_distance: f64,
) -> i32 {
    let count_before = buffer.count();
    let world0 = GuBox::from_transform(rot0, pos0, [0.0; 3]); // extents unused for projection
    let world1 = GuBox::from_transform(rot1, pos1, [0.0; 3]);

    let mut best_depth = f64::MAX;
    let mut best_normal = [1.0, 0.0, 0.0];

    // Class I: face axes from hull A.
    for p in &poly0.polygons {
        let axis_world = world0.rotate(p.plane_n);
        let d = match test_separating_axis(poly0, poly1, &world0, pos0, &world1, pos1, axis_world, contact_distance) {
            Some(v) => v,
            None => return 0,
        };
        if d < best_depth { best_depth = d; best_normal = axis_world; }
    }
    // Class II: face axes from hull B.
    for p in &poly1.polygons {
        let axis_world = world1.rotate(p.plane_n);
        let d = match test_separating_axis(poly0, poly1, &world0, pos0, &world1, pos1, axis_world, contact_distance) {
            Some(v) => v,
            None => return 0,
        };
        if d < best_depth { best_depth = d; best_normal = axis_world; }
    }
    // Class III: edge crosses (face axes win ties via 1.005 bias).
    let edges_a = collect_edges(poly0, &world0, pos0);
    let edges_b = collect_edges(poly1, &world1, pos1);
    const EDGE_BIAS: f64 = 1.005;
    for &ea in &edges_a {
        for &eb in &edges_b {
            let axis = v_cross(ea, eb);
            let len_sq = v_len_sq(axis);
            if len_sq < 1e-6 { continue; }
            let inv_len = 1.0 / len_sq.sqrt();
            let axis = v_mul(axis, inv_len);
            let d = match test_separating_axis(poly0, poly1, &world0, pos0, &world1, pos1, axis, contact_distance) {
                Some(v) => v,
                None => return 0,
            };
            if d * EDGE_BIAS < best_depth { best_depth = d; best_normal = axis; }
        }
    }

    // Orient bestNormal so it points B → A (PhysX convention).
    let center = v_sub(pos1, pos0);
    if v_dot(best_normal, center) > 0.0 {
        best_normal = v_neg(best_normal);
    }

    // Manifold via reference-face clipping.
    let n = generate_manifold(buffer, poly0, poly1, &world0, pos0, &world1, pos1, best_normal, best_depth);
    if n == 0 {
        // Fallback: deepest vertex of B along -bestNormal.
        let support_local_dir = q_rotate_inv(rot1, v_neg(best_normal));
        let mut best_v = 0usize;
        let mut best_proj = f64::MIN;
        for (k, v) in poly1.verts.iter().enumerate() {
            let p = v_dot(*v, support_local_dir);
            if p > best_proj { best_proj = p; best_v = k; }
        }
        let support_world = v_add(q_rotate(rot1, poly1.verts[best_v]), pos1);
        buffer.add(support_world, best_normal, -best_depth);
    }
    (buffer.count() - count_before) as i32
}
