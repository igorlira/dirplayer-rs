//! HeightField terrain
//!
//! Sources cited:
//!   GeomUtils\src\hf\GuHeightFieldData.h, GuHeightField.h, GuHeightFieldUtil.h
//!   GeomUtils\src\hf\GuOverlapTestsHF.cpp::overlapAABBTriangles
//!
//! The per-triangle narrowphase from `physx_gu_mesh.rs` is reused — only the
//! broadphase differs (row/col cell walk instead of RTree traversal).
//!
//! Coordinate convention (matching PhysX HeightFieldGeometry):
//!   row indexes the X axis: world_x = row * row_scale + origin.x
//!   col indexes the Z axis: world_z = col * column_scale + origin.z
//!   height value           : world_y = sample.height * height_scale + origin.y

use super::physx_gu_mesh::{
    box_vs_triangle, capsule_vs_triangle, sphere_vs_triangle,
    GuTriContact, MeshHitCallback,
};

#[derive(Debug, Clone, Default)]
pub struct GuHeightField {
    pub rows: usize,
    pub columns: usize,
    /// Heights row-major: `heights[row * columns + col]`.
    pub heights: Vec<f32>,
    pub origin: [f32; 3],
    pub row_scale: f32,
    pub column_scale: f32,
    pub height_scale: f32,
    /// Local-space AABB.
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
}

impl GuHeightField {
    pub fn nb_cells(&self) -> usize {
        if self.rows == 0 || self.columns == 0 { 0 } else { (self.rows - 1) * (self.columns - 1) }
    }
    pub fn nb_triangles(&self) -> usize { self.nb_cells() * 2 }

    pub fn get_height(&self, row: usize, col: usize) -> f32 {
        self.heights[row * self.columns + col] * self.height_scale + self.origin[1]
    }

    pub fn get_sample_pos(&self, row: usize, col: usize) -> [f32; 3] {
        [
            row as f32 * self.row_scale + self.origin[0],
            self.get_height(row, col),
            col as f32 * self.column_scale + self.origin[2],
        ]
    }

    /// Source: HeightFieldUtil::getTriangle (subset).
    /// Triangulation:
    ///   subTri 0 : c00, c11, c10  (CCW from above ⇒ +Y normal)
    ///   subTri 1 : c00, c01, c11
    pub fn get_triangle(&self, tri_index: u32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let cell_index = (tri_index >> 1) as usize;
        let sub_tri = tri_index & 1;
        let cols_m1 = self.columns - 1;
        let row = cell_index / cols_m1;
        let col = cell_index - row * cols_m1;
        let c00 = self.get_sample_pos(row,     col);
        let c10 = self.get_sample_pos(row + 1, col);
        let c01 = self.get_sample_pos(row,     col + 1);
        let c11 = self.get_sample_pos(row + 1, col + 1);
        if sub_tri == 0 { (c00, c11, c10) } else { (c00, c01, c11) }
    }

    pub fn compute_aabb(&mut self) {
        if self.rows == 0 || self.columns == 0 {
            self.aabb_min = self.origin; self.aabb_max = self.origin; return;
        }
        let mut min_h = f32::MAX;
        let mut max_h = f32::MIN;
        for &h in &self.heights {
            let v = h * self.height_scale;
            if v < min_h { min_h = v; }
            if v > max_h { max_h = v; }
        }
        self.aabb_min = [self.origin[0], min_h + self.origin[1], self.origin[2]];
        self.aabb_max = [
            self.origin[0] + (self.rows - 1) as f32 * self.row_scale,
            max_h + self.origin[1],
            self.origin[2] + (self.columns - 1) as f32 * self.column_scale,
        ];
    }

    pub fn build(rows: usize, columns: usize, heights: Vec<f32>, row_scale: f32, column_scale: f32, height_scale: f32, origin: [f32; 3]) -> Self {
        debug_assert_eq!(heights.len(), rows * columns);
        let mut hf = Self {
            rows, columns, heights, origin,
            row_scale, column_scale, height_scale,
            aabb_min: [0.0; 3], aabb_max: [0.0; 3],
        };
        hf.compute_aabb();
        hf
    }

    /// Source: GuOverlapTestsHF.cpp::overlapAABBTriangles. Walk every cell
    /// whose row/col footprint overlaps the query AABB.
    pub fn intersect_aabb<C: MeshHitCallback + ?Sized>(&self, box_min: [f32; 3], box_max: [f32; 3], cb: &mut C) {
        if self.nb_cells() == 0 { return; }
        let mut row0_f = (box_min[0] - self.origin[0]) / self.row_scale;
        let mut row1_f = (box_max[0] - self.origin[0]) / self.row_scale;
        let mut col0_f = (box_min[2] - self.origin[2]) / self.column_scale;
        let mut col1_f = (box_max[2] - self.origin[2]) / self.column_scale;
        if row0_f > row1_f { std::mem::swap(&mut row0_f, &mut row1_f); }
        if col0_f > col1_f { std::mem::swap(&mut col0_f, &mut col1_f); }

        let row0 = (row0_f.floor() as i32).max(0) as usize;
        let row1 = (row1_f.ceil() as i32).min((self.rows - 2) as i32).max(0) as usize;
        let col0 = (col0_f.floor() as i32).max(0) as usize;
        let col1 = (col1_f.ceil() as i32).min((self.columns - 2) as i32).max(0) as usize;
        let cols_m1 = self.columns - 1;

        for row in row0..=row1 {
            if row >= self.rows - 1 { break; }
            for col in col0..=col1 {
                if col >= cols_m1 { break; }
                let cell_index = row * cols_m1 + col;
                for sub_tri in 0..2u32 {
                    let tri_idx = (cell_index as u32 * 2) + sub_tri;
                    let (v0, v1, v2) = self.get_triangle(tri_idx);
                    let min_y = v0[1].min(v1[1]).min(v2[1]);
                    let max_y = v0[1].max(v1[1]).max(v2[1]);
                    if min_y > box_max[1] || max_y < box_min[1] { continue; }
                    if !cb.process(tri_idx, v0, v1, v2) { return; }
                }
            }
        }
    }
}

// =============================================================================
//  Shape-vs-heightfield contact drivers — mirror physx_gu_mesh.rs surface.
// =============================================================================

pub fn sphere_vs_heightfield(
    hf: &GuHeightField,
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
    hf.intersect_aabb(mn, mx, &mut Cb { center: sphere_center, radius: sphere_radius, contact_dist, out: &mut out });
    out
}

pub fn capsule_vs_heightfield(
    hf: &GuHeightField,
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
    hf.intersect_aabb(mn, mx, &mut Cb { p0, p1, radius, contact_dist, out: &mut out });
    out
}

pub fn box_vs_heightfield(
    hf: &GuHeightField,
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
    hf.intersect_aabb(mn, mx, &mut Cb { c: box_center, he: box_half_extents, ax: box_axis_x, ay: box_axis_y, az: box_axis_z, cd: contact_dist, out: &mut out });
    out
}
