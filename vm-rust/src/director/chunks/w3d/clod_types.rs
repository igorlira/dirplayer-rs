/// Internal state types for CLOD (Continuous Level of Detail) mesh decoder.

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

    // v11.X-style face snapshot: captures mesh.faces AFTER loops but BEFORE batch.
    // Predictions and split corners read from this snapshot (patches 0..N-2 at step N).
    // Confirmed correct by Director 12 Lingo queries.
    pub face_basis_snapshot: Vec<[u32; 3]>,

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
            face_basis_snapshot: Vec::new(),
            sorted_faces: Vec::new(),
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
