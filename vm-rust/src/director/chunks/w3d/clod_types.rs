/// Internal state types for CLOD (Continuous Level of Detail) mesh decoder.

use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct UpdateHeader {
    pub num_new_verts: u32,
    pub num_face_corner_updates: u32,
    pub num_patch_records: u32,
    pub num_sorted_faces: u32,
}

#[derive(Clone, Debug)]
pub struct StepRecord {
    pub vertex_delta: u32,
    pub face_delta: u32,
    pub patch_count: u32,
}

#[derive(Clone, Debug)]
pub struct PatchRecord {
    pub face_index: u32,
    pub corner_index: u32,
    pub new_vertex_index: u32,
    pub old_vertex_index: u32,
}

#[derive(Clone, Debug)]
pub struct NeighborFaceState {
    pub neighbor_mesh: [u32; 3],
    pub neighbor_face: [u32; 3],
    pub corner_flags: [u8; 3],
    pub face_flags_raw: u8,
    pub encoded_face_flags: [u8; 3],
}

impl Default for NeighborFaceState {
    fn default() -> Self {
        Self {
            neighbor_mesh: [0; 3],
            neighbor_face: [0; 3],
            corner_flags: [0; 3],
            face_flags_raw: 0,
            encoded_face_flags: [0; 3],
        }
    }
}

#[derive(Clone, Debug)]
pub struct MeshState {
    pub vertex_attributes: u32,
    pub expected_vertex_count: u32,
    pub expected_face_count: u32,
    pub max_updates: u32,
    pub update_data_count: u32,
    pub inverse_sync_bias: f32,
    pub current_res_counter: u32,
    pub vertex_count: u32,
    pub face_count: u32,
    pub new_vert_count: u32,
    pub mesh_index: usize,
    pub pending_face_record_surplus: i32,

    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tex_coords: Vec<Vec<[f32; 2]>>,    // one vec per texcoord layer
    pub bone_indices: Vec<Vec<u32>>,
    pub bone_weights: Vec<Vec<f32>>,
    pub faces: Vec<[u32; 3]>,
    pub neighbor_faces: Vec<Option<NeighborFaceState>>,

    pub step_records: Vec<StepRecord>,
    pub patch_records: Vec<PatchRecord>,

    // Per-(face, corner) value history vs global resolution: list of (global_res, value).
    // Lets a prediction read a reference corner at the encoder's SetResolution(resCounter-1)
    // collapsed state (global_res <= global_res_N - 2) instead of the live mesh — the general
    // fix that resolves the warehouse "broken left wall" with no oracle. See clod-director-decoder-trace.
    pub corner_history: HashMap<(u32, u8), Vec<(u32, u32)>>,

    // Sorted faces for current update
    pub sorted_faces: Vec<u32>,
}

impl MeshState {
    pub fn new(
        mesh_index: usize,
        vertex_attributes: u32,
        expected_vertex_count: u32,
        expected_face_count: u32,
        max_updates: u32,
        update_data_count: u32,
        inverse_sync_bias: f32,
    ) -> Self {
        let tc_layer_count = (vertex_attributes >> 4) as usize;
        let tex_coords = vec![Vec::new(); tc_layer_count.max(1)];
        Self {
            vertex_attributes,
            expected_vertex_count,
            expected_face_count,
            max_updates,
            update_data_count,
            inverse_sync_bias,
            current_res_counter: 0,
            vertex_count: 0,
            face_count: 0,
            new_vert_count: 0,
            mesh_index,
            pending_face_record_surplus: 0,
            positions: Vec::with_capacity(expected_vertex_count as usize),
            normals: Vec::with_capacity(expected_vertex_count as usize),
            tex_coords,
            bone_indices: Vec::new(),
            bone_weights: Vec::new(),
            faces: Vec::with_capacity(expected_face_count as usize),
            neighbor_faces: Vec::new(),
            step_records: Vec::with_capacity(max_updates as usize),
            patch_records: Vec::with_capacity(update_data_count as usize),
            corner_history: HashMap::new(),
            sorted_faces: Vec::new(),
        }
    }

    /// Read reference corner `(face, corner)` as of resolution boundary `max_gres`
    /// (all patches with global_res <= max_gres applied). Falls back to the live face value.
    pub fn corner_value_as_of(&self, face: u32, corner: u8, max_gres: i64) -> u32 {
        if let Some(hist) = self.corner_history.get(&(face, corner)) {
            if !hist.is_empty() {
                let mut val = hist[0].1;
                for &(g, v) in hist {
                    if (g as i64) <= max_gres { val = v; } else { break; }
                }
                return val;
            }
        }
        let fi = face as usize;
        if fi < self.faces.len() && (corner as usize) < 3 {
            self.faces[fi][corner as usize]
        } else {
            0
        }
    }

    pub fn tex_coord_layer_count(&self) -> usize {
        (self.vertex_attributes >> 4) as usize
    }

    pub fn has_bones(&self) -> bool {
        (self.vertex_attributes & 0x100) != 0
    }
}

#[derive(Clone, Debug)]
pub struct ResManagerState {
    pub resolution_cursor: u32,
    pub patch_cursor: u32,
    pub face_write_cursor: u32,
}
