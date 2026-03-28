/// CLOD (Continuous Level of Detail) mesh decoder.
/// Ported from CLODMeshDecoder.cs.
///
/// Decodes compressed progressive meshes from IFX CompressedGeom (0xFFFFFF49) blocks.

use super::bitstream::IFXBitStreamCompressed;
use super::clod_types::*;
use super::types::*;

#[derive(Clone, Debug)]
pub struct ClodMeshDecoder {
    meshes: Vec<MeshState>,
    res_managers: Vec<ResManagerState>,

    // Inverse quantization factors (from ModelBlock2)
    position_iq: f32,
    normal_iq: f32,
    normal_crease_iq: f32,
    texcoord_iq: f32,
    diffuse_color_iq: f32,
    #[allow(dead_code)]
    specular_color_iq: f32,

    has_neighbor_mesh: bool,
    shading_count: u32,

    // Sync table for multi-mesh models
    sync_table: Option<Vec<Vec<u32>>>,
    distal_edge_merges: Option<Vec<Vec<DistalEdgeMergeRecord>>>,

    // Global resolution counter (persists across CLOD blocks)
    global_update_counter: u32,

    // Per-mesh cursors (persist across CLOD blocks for multi-mesh)
    mesh_cursors: Option<Vec<usize>>,
}

impl ClodMeshDecoder {
    pub fn new() -> Self {
        Self {
            meshes: Vec::new(),
            res_managers: Vec::new(),
            position_iq: 1.0,
            normal_iq: 1.0,
            normal_crease_iq: 1.0,
            texcoord_iq: 1.0,
            diffuse_color_iq: 1.0,
            specular_color_iq: 1.0,
            has_neighbor_mesh: false,
            shading_count: 0,
            sync_table: None,
            distal_edge_merges: None,
            global_update_counter: 0,
            mesh_cursors: None,
        }
    }

    /// Initialize decoder with mesh infos from ModelBlock2.
    pub fn set_mesh_infos(&mut self, res_info: &ModelResourceInfo) {
        self.position_iq = res_info.pos_iq;
        self.normal_iq = res_info.norm_iq;
        self.normal_crease_iq = res_info.normal_crease;
        self.texcoord_iq = res_info.tc_iq;
        self.diffuse_color_iq = res_info.diff_iq;
        self.specular_color_iq = res_info.spec_iq;
        self.has_neighbor_mesh = res_info.has_neighbor_mesh;
        self.shading_count = res_info.shading_count;
        self.sync_table = res_info.sync_table.clone();
        self.distal_edge_merges = res_info.distal_edge_merges.clone();

        self.meshes.clear();
        self.res_managers.clear();

        for (i, info) in res_info.mesh_infos.iter().enumerate() {
            self.meshes.push(MeshState::new(
                i,
                info.vertex_attributes,
                info.num_vertices,
                info.num_faces,
                info.num_updates,
                info.update_data_count,
                info.inverse_sync_bias,
            ));
            self.res_managers.push(ResManagerState {
                resolution_cursor: info.num_updates,
                patch_cursor: info.update_data_count,
                face_write_cursor: 0,
            });
        }

        self.global_update_counter = 0;
        self.mesh_cursors = None;
    }

    /// Decode a CompressedGeom (0xFFFFFF49) block.
    pub fn decode_block(&mut self, block_data: &[u8]) -> Result<(), String> {
        let mut bs = IFXBitStreamCompressed::new(block_data);

        let name = bs.read_ifx_string();
        let update_count = bs.read_u32();

        // Log mesh info for debugging
        if !self.meshes.is_empty() {
            let attrs: Vec<String> = self.meshes.iter().map(|m| format!("0x{:X}", m.vertex_attributes)).collect();
            web_sys::console::log_1(&format!(
                "[W3D CLOD] \"{}\" updates={} meshes={} vertAttrs=[{}] nbr={}",
                name, update_count, self.meshes.len(), attrs.join(","), self.has_neighbor_mesh
            ).into());
        }

        // Safety: update_count should never be huge
        if update_count > 100000 {
            return Err(format!("CLOD decode_block: unreasonable update_count={}", update_count));
        }

        if self.meshes.len() <= 1 {
            // Single mesh path
            for u in 0..update_count {
                if let Err(e) = self.decode_mesh_update(&mut bs, 0) {
                    web_sys::console::log_1(&format!(
                        "[W3D CLOD] decode failed \"{}\" update {}/{}: {} ({} verts, {} faces so far)",
                        name, u, update_count, e,
                        self.meshes.get(0).map(|m| m.positions.len()).unwrap_or(0),
                        self.meshes.get(0).map(|m| m.faces.len()).unwrap_or(0),
                    ).into());
                    // Use whatever geometry was decoded so far instead of failing completely
                    break;
                }
            }
        } else {
            // Multi-mesh path with sync table
            if self.mesh_cursors.is_none() {
                self.mesh_cursors = Some(vec![0usize; self.meshes.len()]);
            }

            // Take cursors and sync table out to avoid borrow conflicts
            let mut cursors = self.mesh_cursors.take().unwrap();
            let sync_table = self.sync_table.clone();

            for u in 0..update_count {
                let global_res = self.global_update_counter + u + 1;

                // Apply distal edge merges at this resolution
                self.apply_distal_edge_merges_at_resolution(global_res);

                // Dispatch updates to meshes according to sync table
                if let Some(ref sync_table) = sync_table {
                    let mesh_count = self.meshes.len();
                    for mi in 0..mesh_count {
                        if mi >= sync_table.len() {
                            continue;
                        }
                        let entries = &sync_table[mi];
                        while cursors[mi] < entries.len() && entries[cursors[mi]] <= global_res {
                            if let Err(e) = self.decode_mesh_update(&mut bs, mi) {
                                web_sys::console::log_1(&format!(
                                    "[W3D CLOD] multi-mesh decode failed mesh={} cursor={}: {} (using partial geometry)",
                                    mi, cursors[mi], e
                                ).into());
                                // Put cursors back before returning
                                self.mesh_cursors = Some(cursors);
                                self.global_update_counter += update_count;
                                return Ok(());
                            }
                            cursors[mi] += 1;
                        }
                    }
                }
            }

            // Put cursors back
            self.mesh_cursors = Some(cursors);
        }

        self.global_update_counter += update_count;
        Ok(())
    }

    /// Decode a single mesh update (position, normal, texcoord, face data).
    fn decode_mesh_update(&mut self, bs: &mut IFXBitStreamCompressed, mesh_idx: usize) -> Result<(), String> {
        // Read header
        let num_new_verts = bs.read_compressed_u32(1);
        let num_patch_records = bs.read_compressed_u32(3);
        let num_face_corner_updates = bs.read_compressed_u32(2);
        let num_sorted_faces = bs.read_compressed_u32(4);

        // Safety bounds
        if num_new_verts > 50000 || num_patch_records > 50000 || num_face_corner_updates > 50000 || num_sorted_faces > 50000 {
            return Err(format!(
                "CLOD update header out of range: verts={} patches={} faces={} sorted={}",
                num_new_verts, num_patch_records, num_face_corner_updates, num_sorted_faces
            ));
        }

        // Read sorted face indices
        let face_ctx_basis = self.get_face_context_basis(mesh_idx);

        let base_ctx = face_ctx_basis + 1024;
        let mut sorted_faces = Vec::with_capacity(num_sorted_faces as usize);

        if num_sorted_faces > 0 {
            sorted_faces.push(bs.read_compressed_u32(base_ctx));
            for s in 1..num_sorted_faces as usize {
                let prev = sorted_faces[s - 1];
                let diff_ctx = if base_ctx > prev { base_ctx - prev } else { 1 };
                let diff_ctx = diff_ctx.max(1);
                let delta = bs.read_compressed_u32(diff_ctx);
                sorted_faces.push(prev + delta);
            }
        }

        self.meshes[mesh_idx].sorted_faces = sorted_faces;
        self.meshes[mesh_idx].new_vert_count = num_new_verts;

        // Loop 1: Decode new vertices
        for _ in 0..num_new_verts {
            self.decode_new_vertex(bs, mesh_idx)?;
        }

        // Loop 2: Decode patch records
        for _ in 0..num_patch_records {
            self.decode_new_face_record(bs, mesh_idx)?;
        }

        // Loop 3: Decode face corner updates (new faces)
        for _ in 0..num_face_corner_updates {
            self.decode_face_corner_update(bs, mesh_idx)?;
            self.meshes[mesh_idx].face_count += 1;
        }

        // Write step record
        self.meshes[mesh_idx].step_records.push(StepRecord {
            vertex_delta: num_new_verts,
            face_delta: num_face_corner_updates,
            patch_count: num_patch_records,
        });

        self.meshes[mesh_idx].pending_face_record_surplus +=
            num_patch_records as i32 - num_face_corner_updates as i32;

        // Apply this update's patch records
        let mesh = &mut self.meshes[mesh_idx];
        let patch_end = mesh.patch_records.len();
        let patch_start = patch_end - num_patch_records as usize;
        for pi in patch_start..patch_end {
            let fi = mesh.patch_records[pi].face_index as usize;
            let ci = mesh.patch_records[pi].corner_index as usize;
            let nvi = mesh.patch_records[pi].new_vertex_index;
            if fi < mesh.faces.len() && ci < 3 {
                mesh.faces[fi][ci] = nvi;
            }
        }

        mesh.current_res_counter += 1;
        self.res_managers[mesh_idx].face_write_cursor = self.meshes[mesh_idx].face_count;

        Ok(())
    }

    /// Decode a new vertex (position, normal, texcoord, bones).
    fn decode_new_vertex(&mut self, bs: &mut IFXBitStreamCompressed, mesh_idx: usize) -> Result<(), String> {
        // ─── Position ───
        let pred_mode = bs.read_compressed_u8(6);
        let (mut pred_x, mut pred_y, mut pred_z) = (0.0f32, 0.0f32, 0.0f32);

        if pred_mode != 4 {
            let pred_idx = bs.read_compressed_u32(5) as usize;
            let mesh = &self.meshes[mesh_idx];
            if pred_idx < mesh.sorted_faces.len() {
                let face_idx = mesh.sorted_faces[pred_idx] as usize;
                if face_idx < mesh.faces.len() && (pred_mode as usize) < 3 {
                    let corner_vert = mesh.faces[face_idx][pred_mode as usize] as usize;
                    if corner_vert < mesh.positions.len() {
                        let p = mesh.positions[corner_vert];
                        pred_x = p[0];
                        pred_y = p[1];
                        pred_z = p[2];
                    }
                }
            }
        }

        let pos_sign = bs.read_compressed_u8(7);
        let mag_x = bs.read_compressed_u32(8);
        let mag_y = bs.read_compressed_u32(8);
        let mag_z = bs.read_compressed_u32(8);

        let iq = self.position_iq;
        let sx = if (pos_sign & 1) != 0 { -1.0 } else { 1.0 };
        let sy = if (pos_sign & 2) != 0 { -1.0 } else { 1.0 };
        let sz = if (pos_sign & 4) != 0 { -1.0 } else { 1.0 };
        let px = pred_x + sx * mag_x as f32 * iq;
        let py = pred_y + sy * mag_y as f32 * iq;
        let pz = pred_z + sz * mag_z as f32 * iq;

        self.meshes[mesh_idx].positions.push([px, py, pz]);

        // ─── Normal ───
        let norm_pred_mode = bs.read_compressed_u8(10);
        let (mut pred_nx, mut pred_ny, mut pred_nz) = (0.0f32, 0.0f32, 1.0f32);

        if norm_pred_mode != 4 {
            let norm_pred_idx = bs.read_compressed_u32(9) as usize;
            let mesh = &self.meshes[mesh_idx];
            if norm_pred_idx < mesh.sorted_faces.len() {
                let face_idx = mesh.sorted_faces[norm_pred_idx] as usize;
                if face_idx < mesh.faces.len() && (norm_pred_mode as usize) < 3 {
                    let corner_vert = mesh.faces[face_idx][norm_pred_mode as usize] as usize;
                    if corner_vert < mesh.normals.len() {
                        let n = mesh.normals[corner_vert];
                        pred_nx = n[0];
                        pred_ny = n[1];
                        pred_nz = n[2];
                    }
                }
            }
        }

        let norm_sign = bs.read_compressed_u8(11);
        let theta_mag = bs.read_compressed_u32(12);

        // Use f64 for azimuth context computation to match C# double precision.
        // The azCtx integer depends on sinTheta / NormalIQ which amplifies rounding errors.
        let theta = (theta_mag as f64 * self.normal_iq as f64).min(1.0);

        let sin_theta = ((1.0f64 - theta) * (theta + 1.0)).sqrt();
        let az_ctx = (sin_theta * std::f64::consts::FRAC_PI_2 / self.normal_iq as f64 + 0.5) as u32 + 1025;
        let az_mag = bs.read_compressed_u32(az_ctx);

        let (mut local_x, mut local_y, mut local_z): (f64, f64, f64);
        local_z = theta;

        if theta.abs() < 1.0 {
            let sin_t = ((1.0f64 - theta) * (theta + 1.0)).sqrt();
            let phi = if sin_t > 0.0 {
                az_mag as f64 * self.normal_crease_iq as f64 / sin_t
            } else {
                0.0
            };
            local_x = phi.cos() * sin_t;
            local_y = phi.sin() * sin_t;
        } else {
            local_x = 0.0;
            local_y = 0.0;
        }

        // Apply signs
        if (norm_sign & 1) != 0 { local_x = -local_x; }
        if (norm_sign & 2) != 0 { local_y = -local_y; }
        if (norm_sign & 4) != 0 { local_z = -local_z; }

        // Rotate from tangent space to world space (using f64 like C#)
        let (nx, ny, nz);
        let pnz = pred_nz as f64;
        if pnz.abs() >= 1.0 {
            nx = local_x as f32;
            ny = local_y as f32;
            nz = local_z as f32;
        } else {
            let pnx = pred_nx as f64;
            let pny = pred_ny as f64;
            let sin_pred = (1.0f64 - pnz * pnz).sqrt();
            let t00 = pnx * pnz / sin_pred;
            let t01 = -pny / sin_pred;
            let t10 = pnz * pny / sin_pred;
            let t11 = pnx / sin_pred;
            let t20 = -sin_pred;

            nx = (t00 * local_x + local_y * t01 + pnx * local_z) as f32;
            ny = (pny * local_z + local_y * t11 + t10 * local_x) as f32;
            nz = (pnz * local_z + t20 * local_x) as f32;
        }

        // Normalize
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        let (nx, ny, nz) = if len > 0.0 {
            (nx / len, ny / len, nz / len)
        } else {
            (0.0, 0.0, 1.0)
        };

        self.meshes[mesh_idx].normals.push([nx, ny, nz]);

        // ─── TexCoords ───
        let tc_layers = self.meshes[mesh_idx].tex_coord_layer_count();
        for layer in 0..tc_layers {
            let tc_pred_mode = bs.read_compressed_u8(14);
            let (mut pred_u, mut pred_v) = (0.0f32, 0.0f32);

            if tc_pred_mode != 4 {
                let tc_pred_idx = bs.read_compressed_u32(15) as usize;
                let mesh = &self.meshes[mesh_idx];
                if tc_pred_idx < mesh.sorted_faces.len() {
                    let face_idx = mesh.sorted_faces[tc_pred_idx] as usize;
                    if face_idx < mesh.faces.len() && (tc_pred_mode as usize) < 3 {
                        let corner_vert = mesh.faces[face_idx][tc_pred_mode as usize] as usize;
                        if layer < mesh.tex_coords.len() && corner_vert < mesh.tex_coords[layer].len() {
                            let tc = mesh.tex_coords[layer][corner_vert];
                            pred_u = tc[0];
                            pred_v = tc[1];
                        }
                    }
                }
            }

            let tc_sign = bs.read_compressed_u8(16);
            let tc_mag_u = bs.read_compressed_u32(17);
            let tc_mag_v = bs.read_compressed_u32(17);

            let tc_iq = self.texcoord_iq;
            let su = if (tc_sign & 1) != 0 { -1.0 } else { 1.0 };
            let sv = if (tc_sign & 2) != 0 { -1.0 } else { 1.0 };
            let tu = pred_u + su * tc_mag_u as f32 * tc_iq;
            let tv = pred_v + sv * tc_mag_v as f32 * tc_iq;

            let mesh = &mut self.meshes[mesh_idx];
            while mesh.tex_coords.len() <= layer {
                mesh.tex_coords.push(Vec::new());
            }
            mesh.tex_coords[layer].push([tu, tv]);
        }

        // ─── Bone weights ───
        // Always read bone count from ctx=18, even if vertex_attributes doesn't indicate bones.
        // The C# reference reads this unconditionally; the bone_count being 0 means no further reads.
        {
            let bone_count = bs.read_compressed_u32(18);
            if bone_count > 256 {
                return Err(format!("CLOD: unreasonable bone_count={}", bone_count));
            }
            let mut indices = Vec::with_capacity(bone_count as usize);
            let mut weights = Vec::with_capacity(bone_count as usize);

            if bone_count > 0 {
                weights.push(1.0f32);
                for n in 0..bone_count {
                    indices.push(bs.read_compressed_u32(19));
                    if n > 0 {
                        let weight_mag = bs.read_compressed_u32(20);
                        let w = (weight_mag as f32 * self.diffuse_color_iq).clamp(0.0, 1.0);
                        weights.push(w);
                        weights[0] -= w;
                    }
                }
            }

            self.meshes[mesh_idx].bone_indices.push(indices);
            self.meshes[mesh_idx].bone_weights.push(weights);
        }

        self.meshes[mesh_idx].vertex_count += 1;
        Ok(())
    }

    /// Decode a patch record (face corner replacement).
    fn decode_new_face_record(&mut self, bs: &mut IFXBitStreamCompressed, mesh_idx: usize) -> Result<(), String> {
        let match_idx = bs.read_compressed_u32(21) as usize;
        let mesh = &self.meshes[mesh_idx];
        let match_face_index = if match_idx < mesh.sorted_faces.len() {
            mesh.sorted_faces[match_idx]
        } else {
            0
        };

        let type_flag = bs.read_compressed_u8(22) as u32; // corner index 0/1/2

        let mut param = bs.read_compressed_u32(23);
        if param == 0 {
            param = bs.read_u32();
        } else {
            let mesh = &self.meshes[mesh_idx];
            param = mesh.vertex_count - mesh.new_vert_count + param - 1;
        }

        let corner_flag = bs.read_compressed_u8(24);
        let corner_vertex = match corner_flag {
            0 | 1 | 2 => {
                let mesh = &self.meshes[mesh_idx];
                if (match_face_index as usize) < mesh.faces.len() {
                    mesh.faces[match_face_index as usize][corner_flag as usize]
                } else {
                    0
                }
            }
            3 => bs.read_u32(),
            _ => 0,
        };

        self.meshes[mesh_idx].patch_records.push(PatchRecord {
            face_index: match_face_index,
            corner_index: type_flag,
            new_vertex_index: param,
            old_vertex_index: corner_vertex,
        });

        Ok(())
    }

    /// Decode a face corner update (new triangle).
    fn decode_face_corner_update(&mut self, bs: &mut IFXBitStreamCompressed, mesh_idx: usize) -> Result<(), String> {
        let mut corners = [0u32; 3];

        for c in 0..3 {
            let corner_type = bs.read_compressed_u8(25);
            match corner_type {
                0 => {
                    // Raw vertex index
                    corners[c] = bs.read_u32();
                }
                1 => {
                    // Relative to new vertices
                    let rel_idx = bs.read_compressed_u32(26);
                    let mesh = &self.meshes[mesh_idx];
                    corners[c] = rel_idx + mesh.vertex_count - mesh.new_vert_count;
                }
                2 | 3 | 4 => {
                    // Split from existing face corner
                    let split_idx = bs.read_compressed_u32(27) as usize;
                    let offset = bs.read_compressed_u32(28);
                    let mesh = &self.meshes[mesh_idx];
                    if split_idx < mesh.sorted_faces.len() {
                        let face_idx = mesh.sorted_faces[split_idx] as usize;
                        let corner = (corner_type - 2) as usize;
                        if face_idx < mesh.faces.len() && corner < 3 {
                            let base_vertex = mesh.faces[face_idx][corner];
                            corners[c] = offset + base_vertex;
                        }
                    }
                }
                _ => {}
            }
        }

        self.meshes[mesh_idx].faces.push(corners);

        // Update face_write_cursor immediately after writing face (before neighbor mesh reads).
        // The C# does this at CLODMeshDecoder.cs:3043 — critical because neighbor mesh
        // faceFlag==4 uses FaceWriteCursor+1024 as a dynamic arithmetic context.
        self.res_managers[mesh_idx].face_write_cursor = self.meshes[mesh_idx].face_count;

        // If has neighbor mesh, read neighbor data
        if self.has_neighbor_mesh {
            self.read_neighbor_mesh_data(bs, mesh_idx)?;
        }

        Ok(())
    }

    /// Read neighbor mesh data for a newly created face.
    fn read_neighbor_mesh_data(&mut self, bs: &mut IFXBitStreamCompressed, mesh_idx: usize) -> Result<(), String> {
        let mut nbr = NeighborFaceState::default();
        nbr.face_flags_raw = bs.read_compressed_u8(29);

        for c in 0..3 {
            nbr.corner_flags[c] = bs.read_compressed_u8(30);
        }

        for c in 0..3 {
            let face_flag = bs.read_compressed_u8(31);
            nbr.encoded_face_flags[c] = face_flag;

            match face_flag {
                0 | 1 | 2 => {
                    // Traversal-based neighbor resolution
                    let _nbr_val1 = bs.read_compressed_u32(34);
                    let _nbr_val2 = bs.read_compressed_u32(35);
                }
                4 => {
                    // Direct mesh + face from context basis
                    let nbr_mesh = bs.read_compressed_u32(32);
                    let face_ctx = self.get_face_context_basis(mesh_idx) + 1024;
                    let nbr_face = bs.read_compressed_u32(face_ctx);
                    nbr.neighbor_mesh[c] = nbr_mesh;
                    nbr.neighbor_face[c] = nbr_face;
                }
                5 => {
                    // Direct mesh + relative face
                    let nbr_mesh = bs.read_compressed_u32(32);
                    let rel_face = bs.read_compressed_u32(33);
                    nbr.neighbor_mesh[c] = nbr_mesh;
                    nbr.neighbor_face[c] = self.get_face_context_basis(mesh_idx) + rel_face;
                }
                _ => {
                    // faceFlag == 3 or >= 6: mesh index + face offset
                    let nbr_mesh = bs.read_compressed_u32(32);
                    let offset = bs.read_compressed_u32(36);
                    nbr.neighbor_mesh[c] = nbr_mesh;
                    if face_flag >= 6 {
                        let mesh = &self.meshes[mesh_idx];
                        let sorted_idx = (face_flag - 6) as usize;
                        if sorted_idx < mesh.sorted_faces.len() {
                            nbr.neighbor_face[c] = mesh.sorted_faces[sorted_idx] + offset;
                        }
                    } else {
                        nbr.neighbor_face[c] = offset;
                    }
                }
            }
        }

        self.meshes[mesh_idx].neighbor_faces.push(Some(nbr));
        Ok(())
    }

    fn get_face_context_basis(&self, mesh_idx: usize) -> u32 {
        if mesh_idx < self.res_managers.len() {
            self.res_managers[mesh_idx].face_write_cursor
        } else {
            0
        }
    }

    fn apply_distal_edge_merges_at_resolution(&mut self, _resolution: u32) {
        // Distal edge merges fix topology at boundaries between meshes
        // at different resolution levels. For Phase 1, just skip - the
        // decoded geometry is still correct, only the neighbor mesh
        // topology data would be affected.
    }

    /// Get combined decoded meshes as ClodDecodedMesh objects.
    pub fn get_decoded_meshes(&self) -> Vec<ClodDecodedMesh> {
        self.meshes
            .iter()
            .enumerate()
            .map(|(i, mesh)| {
                ClodDecodedMesh {
                    name: format!("mesh_{}", i),
                    positions: mesh.positions.clone(),
                    normals: mesh.normals.clone(),
                    tex_coords: mesh.tex_coords.clone(),
                    faces: mesh.faces.clone(),
                    diffuse_colors: Vec::new(),
                    specular_colors: Vec::new(),
                    bone_indices: mesh.bone_indices.clone(),
                    bone_weights: mesh.bone_weights.clone(),
                }
            })
            .collect()
    }

    /// Get decoded meshes at a specific LOD level (0.0 = lowest, 1.0 = highest)
    pub fn get_decoded_meshes_at_lod(&self, lod: f32) -> Vec<ClodDecodedMesh> {
        self.meshes
            .iter()
            .enumerate()
            .map(|(i, mesh)| {
                let total_steps = mesh.step_records.len();
                if total_steps == 0 {
                    return ClodDecodedMesh {
                        name: format!("mesh_{}", i),
                        positions: mesh.positions.clone(),
                        normals: mesh.normals.clone(),
                        tex_coords: mesh.tex_coords.clone(),
                        faces: mesh.faces.clone(),
                        diffuse_colors: Vec::new(),
                        specular_colors: Vec::new(),
                        bone_indices: mesh.bone_indices.clone(),
                        bone_weights: mesh.bone_weights.clone(),
                    };
                }

                // Calculate how many steps to apply for this LOD level
                let target_steps = ((lod * total_steps as f32) as usize).min(total_steps);

                // Count vertices and faces at target LOD
                let mut vert_count = 0u32;
                let mut face_count = 0u32;
                for step_idx in 0..target_steps {
                    let step = &mesh.step_records[step_idx];
                    vert_count += step.vertex_delta;
                    face_count += step.face_delta;
                }

                let vert_count = (vert_count as usize).min(mesh.positions.len());
                let face_count = (face_count as usize).min(mesh.faces.len());

                // mesh.faces is in its final (fully-patched) state. To get the
                // face state at target_steps, start from the full faces and
                // reverse-apply patches from the end back to target_steps,
                // restoring old_vertex_index for each undone patch.
                let mut faces = mesh.faces.clone();

                // Walk patches in reverse, undoing steps from total_steps-1 down to target_steps
                let mut patch_cursor = mesh.patch_records.len();
                for step_idx in (target_steps..total_steps).rev() {
                    let step = &mesh.step_records[step_idx];
                    let step_start = patch_cursor - step.patch_count as usize;
                    for pi in (step_start..patch_cursor).rev() {
                        let patch = &mesh.patch_records[pi];
                        let fi = patch.face_index as usize;
                        let ci = patch.corner_index as usize;
                        if fi < faces.len() && ci < 3 {
                            faces[fi][ci] = patch.old_vertex_index;
                        }
                    }
                    patch_cursor = step_start;
                }

                // Truncate to target face count
                faces.truncate(face_count);

                let tc = mesh.tex_coords.iter().map(|layer| {
                    layer[..vert_count.min(layer.len())].to_vec()
                }).collect();

                ClodDecodedMesh {
                    name: format!("mesh_{}", i),
                    positions: mesh.positions[..vert_count].to_vec(),
                    normals: mesh.normals[..vert_count.min(mesh.normals.len())].to_vec(),
                    tex_coords: tc,
                    faces,
                    diffuse_colors: Vec::new(),
                    specular_colors: Vec::new(),
                    bone_indices: mesh.bone_indices[..vert_count.min(mesh.bone_indices.len())].to_vec(),
                    bone_weights: mesh.bone_weights[..vert_count.min(mesh.bone_weights.len())].to_vec(),
                }
            })
            .collect()
    }

    /// Get decoded meshes at full resolution (all patches applied)
    pub fn get_decoded_meshes_full_resolution(&self) -> Vec<ClodDecodedMesh> {
        self.meshes
            .iter()
            .enumerate()
            .map(|(i, mesh)| {
                // Start with all faces, then apply ALL patch records
                let mut faces = mesh.faces.clone();
                for patch in &mesh.patch_records {
                    let fi = patch.face_index as usize;
                    let ci = patch.corner_index as usize;
                    if fi < faces.len() && ci < 3 {
                        faces[fi][ci] = patch.new_vertex_index;
                    }
                }

                ClodDecodedMesh {
                    name: format!("mesh_{}", i),
                    positions: mesh.positions.clone(),
                    normals: mesh.normals.clone(),
                    tex_coords: mesh.tex_coords.clone(),
                    faces,
                    diffuse_colors: Vec::new(),
                    specular_colors: Vec::new(),
                    bone_indices: mesh.bone_indices.clone(),
                    bone_weights: mesh.bone_weights.clone(),
                }
            })
            .collect()
    }

    /// Get the maximum number of resolution steps across all meshes
    pub fn max_resolution_steps(&self) -> usize {
        self.meshes.iter().map(|m| m.step_records.len()).max().unwrap_or(0)
    }
}
