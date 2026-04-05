use std::collections::HashMap;
use super::skeleton::{build_node_world_matrices, export_basis_transform};

#[derive(Clone, Debug)]
pub struct W3dMaterial {
    pub name: String,
    pub ambient: [f32; 4],
    pub diffuse: [f32; 4],
    pub specular: [f32; 4],
    pub emissive: [f32; 4],
    pub reflectivity: f32,
    pub opacity: f32,
    pub shininess: f32,
}

impl Default for W3dMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            ambient: [63.0 / 255.0, 63.0 / 255.0, 63.0 / 255.0, 1.0],
            diffuse: [1.0, 1.0, 1.0, 1.0],
            specular: [1.0, 1.0, 1.0, 1.0],
            emissive: [0.0, 0.0, 0.0, 1.0],
            reflectivity: 0.0,
            opacity: 1.0,
            shininess: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct W3dTextureLayer {
    pub name: String,
    pub intensity: f32,
    pub blend_func: u8,
    pub blend_src: u8,
    pub blend_const: f32,
    pub tex_mode: u8,
    pub tex_transform: [f32; 16],
    pub wrap_transform: [f32; 16],
    pub repeat_s: u8,
    pub repeat_t: u8,
}

impl Default for W3dTextureLayer {
    fn default() -> Self {
        Self {
            name: String::new(),
            intensity: 1.0,
            blend_func: 0,
            blend_src: 0,
            blend_const: 0.5,
            tex_mode: 0,
            tex_transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
            wrap_transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
            repeat_s: 1,
            repeat_t: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum W3dShaderType {
    LitTexture,
    Painter,
    Inker,
    Engraver,
    Newsprint,
    Particle,
}

impl Default for W3dShaderType {
    fn default() -> Self { W3dShaderType::LitTexture }
}

/// W3dShader: Director shader properties.
/// `derive(Default)` works because all field types have correct defaults:
/// - `String` → empty, `u32/f32` → 0, `Vec` → empty
/// - `W3dShaderType` → LitTexture (#standard)
/// - Color/blend/transform defaults come from `W3dMaterial::default()` and
///   `W3dTextureLayer::default()` when accessed via `get_shader_prop`.
/// - Fallback values in `get_shader_prop` match Director's DefaultShader spec.
#[derive(Clone, Debug, Default)]
pub struct W3dShader {
    pub name: String,
    pub material_name: String,
    pub attrs: u32,
    pub render_pass: u32,
    pub texture_layers: Vec<W3dTextureLayer>,
    pub shader_type: W3dShaderType,
    /// When true, textured models use actual diffuse color for lighting.
    /// When false (default), textured models use white (1,1,1) for lighting.
    pub use_diffuse_with_texture: bool,
    // NPR-specific fields
    pub toon_steps: u32,      // ShaderPainter: number of quantization steps
    pub outline_width: f32,   // ShaderInker: outline thickness
    pub outline_color: [f32; 4], // ShaderInker: outline color (RGBA)
}

#[derive(Clone, Debug)]
pub struct W3dBone {
    pub name: String,
    pub parent_index: i32,
    pub length: f32,
    pub dir_x: f32,
    pub dir_y: f32,
    pub dir_z: f32,
    pub rot_x: f32,
    pub rot_y: f32,
    pub rot_z: f32,
    pub rot_w: f32,
    pub attributes: u32,
}

#[derive(Clone, Debug, Default)]
pub struct W3dSkeleton {
    pub name: String,
    pub bones: Vec<W3dBone>,
}

impl W3dSkeleton {
    pub fn find_bone_by_name(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct W3dKeyframe {
    pub time: f32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rot_x: f32,
    pub rot_y: f32,
    pub rot_z: f32,
    pub rot_w: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub scale_z: f32,
}

impl Default for W3dKeyframe {
    fn default() -> Self {
        Self {
            time: 0.0,
            pos_x: 0.0,
            pos_y: 0.0,
            pos_z: 0.0,
            rot_x: 0.0,
            rot_y: 0.0,
            rot_z: 0.0,
            rot_w: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
            scale_z: 1.0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct W3dMotionTrack {
    pub bone_name: String,
    pub keyframes: Vec<W3dKeyframe>,
}

impl W3dMotionTrack {
    /// Interpolate keyframe at time t. Linear for position/scale, SLERP for rotation.
    pub fn evaluate(&self, time: f32) -> W3dKeyframe {
        if self.keyframes.is_empty() {
            return W3dKeyframe::default();
        }
        if self.keyframes.len() == 1 || time <= self.keyframes[0].time {
            return self.keyframes[0].clone();
        }
        let last = &self.keyframes[self.keyframes.len() - 1];
        if time >= last.time {
            return last.clone();
        }

        let mut i = 0;
        while i < self.keyframes.len() - 1 && self.keyframes[i + 1].time < time {
            i += 1;
        }

        let k0 = &self.keyframes[i];
        let k1 = &self.keyframes[i + 1];
        let dt = k1.time - k0.time;
        let t = if dt > 0.0 { (time - k0.time) / dt } else { 0.0 };

        let (rx, ry, rz, rw) = slerp(
            k0.rot_x, k0.rot_y, k0.rot_z, k0.rot_w,
            k1.rot_x, k1.rot_y, k1.rot_z, k1.rot_w,
            t,
        );

        W3dKeyframe {
            time,
            pos_x: k0.pos_x + (k1.pos_x - k0.pos_x) * t,
            pos_y: k0.pos_y + (k1.pos_y - k0.pos_y) * t,
            pos_z: k0.pos_z + (k1.pos_z - k0.pos_z) * t,
            rot_x: rx,
            rot_y: ry,
            rot_z: rz,
            rot_w: rw,
            scale_x: k0.scale_x + (k1.scale_x - k0.scale_x) * t,
            scale_y: k0.scale_y + (k1.scale_y - k0.scale_y) * t,
            scale_z: k0.scale_z + (k1.scale_z - k0.scale_z) * t,
        }
    }
}

fn slerp(
    ax: f32, ay: f32, az: f32, aw: f32,
    mut bx: f32, mut by: f32, mut bz: f32, mut bw: f32,
    t: f32,
) -> (f32, f32, f32, f32) {
    let mut dot = ax * bx + ay * by + az * bz + aw * bw;
    if dot < 0.0 {
        bx = -bx;
        by = -by;
        bz = -bz;
        bw = -bw;
        dot = -dot;
    }

    let (s0, s1);
    if dot > 0.9995 {
        s0 = 1.0 - t;
        s1 = t;
    } else {
        let theta = dot.min(1.0).acos();
        let sin_theta = theta.sin();
        s0 = ((1.0 - t) * theta).sin() / sin_theta;
        s1 = (t * theta).sin() / sin_theta;
    }

    (
        s0 * ax + s1 * bx,
        s0 * ay + s1 * by,
        s0 * az + s1 * bz,
        s0 * aw + s1 * bw,
    )
}

#[derive(Clone, Debug, Default)]
pub struct W3dMotion {
    pub name: String,
    pub tracks: Vec<W3dMotionTrack>,
}

impl W3dMotion {
    pub fn find_track_by_bone(&self, bone_name: &str) -> Option<&W3dMotionTrack> {
        self.tracks.iter().find(|t| t.bone_name == bone_name)
    }

    pub fn duration(&self) -> f32 {
        self.tracks
            .iter()
            .filter_map(|t| t.keyframes.last().map(|k| k.time))
            .fold(0.0f32, f32::max)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum W3dNodeType {
    Group,
    Model,
    Light,
    View,
}

#[derive(Clone, Debug)]
pub struct W3dNode {
    pub name: String,
    pub parent_name: String,
    pub resource_name: String,
    pub model_resource_name: String,
    pub node_type: W3dNodeType,
    pub transform: [f32; 16],
    pub shader_name: String,
    pub near_plane: f32,
    pub far_plane: f32,
    pub fov: f32,
    pub screen_width: i32,
    pub screen_height: i32,
}

impl Default for W3dNode {
    fn default() -> Self {
        Self {
            name: String::new(),
            parent_name: String::new(),
            resource_name: String::new(),
            model_resource_name: String::new(),
            node_type: W3dNodeType::Group,
            transform: [0.0; 16],
            shader_name: String::new(),
            near_plane: 1.0,
            far_plane: 1000.0,
            fov: 30.0,
            screen_width: 640,
            screen_height: 480,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum W3dLightType {
    Ambient,
    Directional,
    Point,
    Spot,
}

#[derive(Clone, Debug)]
pub struct W3dLight {
    pub name: String,
    pub light_type: W3dLightType,
    pub color: [f32; 3],
    pub attenuation: [f32; 3],
    pub spot_angle: f32,
    pub enabled: bool,
}

impl Default for W3dLight {
    fn default() -> Self {
        Self {
            name: String::new(),
            light_type: W3dLightType::Ambient,
            color: [1.0, 1.0, 1.0],
            attenuation: [1.0, 0.0, 0.0],
            spot_angle: 90.0,
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClodMeshInfo {
    pub num_vertices: u32,
    pub num_faces: u32,
    pub num_updates: u32,
    pub update_data_count: u32,
    pub inverse_sync_bias: f32,
    pub vertex_attributes: u32,
}

#[derive(Clone, Debug, Default)]
pub struct DistalEdgeMergeRecord {
    pub mesh_a: u32,
    pub face_a: u32,
    pub corner_a: u32,
    pub mesh_b: u32,
    pub face_b: u32,
    pub corner_b: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ModelShaderBinding {
    pub name: String,
    pub mesh_bindings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ModelResourceInfo {
    pub name: String,
    pub mesh_infos: Vec<ClodMeshInfo>,
    pub max_resolution: u32,
    pub shading_count: u32,
    pub shader_bindings: Vec<ModelShaderBinding>,
    pub pos_iq: f32,
    pub norm_iq: f32,
    pub normal_crease: f32,
    pub tc_iq: f32,
    pub diff_iq: f32,
    pub spec_iq: f32,
    pub has_distal_edge_merge: bool,
    pub has_neighbor_mesh: bool,
    /// UV generator mode: 0=planar, 1=spherical, 2=cylindrical, 3=reflection
    pub uv_gen_mode: Option<u8>,
    pub sync_table: Option<Vec<Vec<u32>>>,
    pub distal_edge_merges: Option<Vec<Vec<DistalEdgeMergeRecord>>>,
}

#[derive(Clone, Debug, Default)]
pub struct ClodDecodedMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tex_coords: Vec<Vec<[f32; 2]>>,
    pub faces: Vec<[u32; 3]>,
    pub diffuse_colors: Vec<[f32; 4]>,
    pub specular_colors: Vec<[f32; 4]>,
    pub bone_indices: Vec<Vec<u32>>,
    pub bone_weights: Vec<Vec<f32>>,
}

#[derive(Clone, Debug)]
pub struct W3dRawMesh {
    pub name: String,
    pub chain_index: u32,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tex_coords: Vec<[f32; 2]>,
    pub vertex_colors: Vec<[f32; 4]>,
    pub faces: Vec<[u32; 3]>,
}

#[derive(Clone, Debug)]
pub struct W3dTextureInfo {
    pub name: String,
    pub render_format: u8,
    pub mip_mode: u8,
    pub mag_filter: u8,
    pub image_type: u8,
}

#[derive(Clone, Debug, Default)]
pub struct W3dScene {
    pub materials: Vec<W3dMaterial>,
    pub shaders: Vec<W3dShader>,
    pub nodes: Vec<W3dNode>,
    pub lights: Vec<W3dLight>,
    pub texture_images: HashMap<String, Vec<u8>>,
    pub texture_infos: Vec<W3dTextureInfo>,
    pub skeletons: Vec<W3dSkeleton>,
    pub motions: Vec<W3dMotion>,
    pub model_resources: HashMap<String, ModelResourceInfo>,
    pub clod_meshes: HashMap<String, Vec<ClodDecodedMesh>>,
    pub clod_decoders: HashMap<String, super::clod_decoder::ClodMeshDecoder>,
    pub raw_meshes: Vec<W3dRawMesh>,
    /// Monotonically increasing counter; bumped whenever mesh geometry changes
    pub mesh_content_version: u64,
    /// Monotonically increasing counter; bumped whenever texture_images is mutated
    pub texture_content_version: u64,
}

impl W3dScene {
    /// Export the scene to OBJ format with default mtl name.
    pub fn export_obj(&self) -> String {
        self.export_obj_with_mtl("scene.mtl")
    }

    /// Export the scene to OBJ format, matching the C# W3DParser assembled output.
    /// Ported from CLODMeshDecoder.WriteAssembledPart + ExportMesh.
    pub fn export_obj_with_mtl(&self, mtl_filename: &str) -> String {
        let mut obj = String::new();
        let basis_transform = export_basis_transform();
        let num_parts = self.clod_meshes.len() + self.raw_meshes.len();
        obj.push_str(&format!("# Assembled W3D model: {} parts\n", num_parts));
        obj.push_str(&format!("mtllib {}\n", mtl_filename));
        obj.push_str("s off\n");

        let mut total_verts: u32 = 0;
        let mut total_normals: u32 = 0;
        let mut total_texcoords: u32 = 0;

        // Export each CLOD resource as a part (like C# WriteAssembledPart)
        for (resource_name, meshes) in &self.clod_meshes {
            let safe_name = resource_name.replace(' ', "_");
            let world_transform = self.find_transform_for_resource(resource_name);
            let mat_name = self.resolve_material_for_resource(resource_name);

            // Collect all positions from all sub-meshes of this resource
            // Collect all positions, build per-mesh face groups
            let mut positions: Vec<[f32; 3]> = Vec::new();
            let mut texcoords: Vec<[f32; 2]> = Vec::new();
            // Per-mesh: (vertex_offset, faces)
            let mut mesh_face_groups: Vec<(u32, Vec<[u32; 3]>)> = Vec::new();

            let mut vertex_colors: Vec<[f32; 4]> = Vec::new();

            for mesh in meshes {
                let vert_off = positions.len() as u32;
                for pos in &mesh.positions {
                    positions.push(*pos);
                }
                if !mesh.tex_coords.is_empty() && !mesh.tex_coords[0].is_empty() {
                    for tc in &mesh.tex_coords[0] {
                        texcoords.push(*tc);
                    }
                }
                // Collect vertex colors (diffuse)
                for color in &mesh.diffuse_colors {
                    vertex_colors.push(*color);
                }
                let mut faces = Vec::new();
                for face in &mesh.faces {
                    let v0 = face[0] + vert_off;
                    let v1 = face[1] + vert_off;
                    let v2 = face[2] + vert_off;
                    if (v0 as usize) < positions.len() && (v1 as usize) < positions.len() && (v2 as usize) < positions.len() {
                        faces.push([v0, v1, v2]);
                    }
                }
                mesh_face_groups.push((vert_off, faces));
            }

            // Build combined face list for normal recalculation
            let all_faces: Vec<(usize, [u32; 3])> = mesh_face_groups.iter()
                .flat_map(|(_, faces)| faces.iter().map(|f| (0usize, *f)))
                .collect();

            // Recalculate smooth normals
            let normals = recalculate_smooth_normals(&positions, &all_faces);

            // Write vertex data first (all v/vn/vt before any groups/faces)
            let has_vcolors = vertex_colors.len() == positions.len();
            for (i, pos) in positions.iter().enumerate() {
                let (px, py, pz) = if let Some(ref m) = world_transform {
                    transform_point(m, pos[0], pos[1], pos[2])
                } else {
                    (pos[0], pos[1], pos[2])
                };
                let (px, py, pz) = transform_point(&basis_transform, px, py, pz);
                if has_vcolors {
                    let c = &vertex_colors[i];
                    // Extended OBJ: v x y z r g b (supported by MeshLab, Blender, etc.)
                    obj.push_str(&format!("v {:.6} {:.6} {:.6} {:.4} {:.4} {:.4}\n", px, py, pz, c[0], c[1], c[2]));
                } else {
                    obj.push_str(&format!("v {:.6} {:.6} {:.6}\n", px, py, pz));
                }
            }
            for n in &normals {
                let (nx, ny, nz) = if let Some(ref m) = world_transform {
                    transform_normal(m, n[0], n[1], n[2])
                } else {
                    (n[0], n[1], n[2])
                };
                let (nx, ny, nz) = transform_normal(&basis_transform, nx, ny, nz);
                obj.push_str(&format!("vn {:.6} {:.6} {:.6}\n", nx, ny, nz));
            }
            for tc in &texcoords {
                // W3D CLOD UVs are in [-0.5, 0.5] — remap to [0, 1] for OBJ
                obj.push_str(&format!("vt {:.6} {:.6}\n", 0.5 - tc[0], 0.5 - tc[1]));
            }

            // Write per-mesh groups with material assignments
            let has_tc = !texcoords.is_empty();
            let pos_count = positions.len() as u32;
            let tc_count = texcoords.len() as u32;
            let res_info = self.model_resources.get(resource_name);

            for (mesh_idx, (_, faces)) in mesh_face_groups.iter().enumerate() {
                obj.push_str(&format!("\no mesh_{}\ng mesh_{}\n", mesh_idx, mesh_idx));

                // Per-mesh material from shader bindings
                let mesh_mat = self.resolve_material_for_mesh(resource_name, mesh_idx);
                if let Some(ref mat) = mesh_mat {
                    obj.push_str(&format!("usemtl {}\n", mat.replace(' ', "_")));
                } else if let Some(ref mat) = mat_name {
                    obj.push_str(&format!("usemtl {}\n", mat.replace(' ', "_")));
                }

                for face in faces {
                    if face[0] >= pos_count || face[1] >= pos_count || face[2] >= pos_count { continue; }
                    let v0 = face[0] + total_verts + 1;
                    let v1 = face[1] + total_verts + 1;
                    let v2 = face[2] + total_verts + 1;
                    let n0 = face[0] + total_normals + 1;
                    let n1 = face[1] + total_normals + 1;
                    let n2 = face[2] + total_normals + 1;
                    if has_tc && face[0] < tc_count && face[1] < tc_count && face[2] < tc_count {
                        let t0 = face[0] + total_texcoords + 1;
                        let t1 = face[1] + total_texcoords + 1;
                        let t2 = face[2] + total_texcoords + 1;
                        // Reverse winding (v0,v2,v1) for correct OBJ CCW front faces
                        obj.push_str(&format!("f {}/{}/{} {}/{}/{} {}/{}/{}\n", v0, t0, n0, v2, t2, n2, v1, t1, n1));
                    } else {
                        obj.push_str(&format!("f {}//{} {}//{} {}//{}\n", v0, n0, v2, n2, v1, n1));
                    }
                }
            }

            total_verts += positions.len() as u32;
            total_normals += normals.len() as u32;
            total_texcoords += texcoords.len() as u32;
        }

        // Export raw meshes
        for mesh in &self.raw_meshes {
            let safe_name = mesh.name.replace(' ', "_");

            // Vertex data first (with optional vertex colors)
            let has_raw_vcolors = mesh.vertex_colors.len() == mesh.positions.len();
            for (i, pos) in mesh.positions.iter().enumerate() {
                let (px, py, pz) = transform_point(&basis_transform, pos[0], pos[1], pos[2]);
                if has_raw_vcolors {
                    let c = &mesh.vertex_colors[i];
                    obj.push_str(&format!("v {:.6} {:.6} {:.6} {:.4} {:.4} {:.4}\n", px, py, pz, c[0], c[1], c[2]));
                } else {
                    obj.push_str(&format!("v {:.6} {:.6} {:.6}\n", px, py, pz));
                }
            }
            for norm in &mesh.normals {
                let (nx, ny, nz) = transform_normal(&basis_transform, norm[0], norm[1], norm[2]);
                obj.push_str(&format!("vn {:.6} {:.6} {:.6}\n", nx, ny, nz));
            }
            let has_tc = !mesh.tex_coords.is_empty();
            if has_tc {
                for tc in &mesh.tex_coords {
                    // W3D CLOD UVs are in [-0.5, 0.5] — remap: U = u+0.5, V = 0.5-v
                    obj.push_str(&format!("vt {:.6} {:.6}\n", tc[0] + 0.5, 0.5 - tc[1]));
                }
            }

            // Group header after vertex data
            obj.push_str(&format!("\no {}\ng {}\n", safe_name, safe_name));

            // Try to resolve material for raw mesh by name
            let raw_mat = self.resolve_raw_mesh_material(&mesh.name);
            if let Some(ref mat) = raw_mat {
                obj.push_str(&format!("usemtl {}\n", mat.replace(' ', "_")));
            }

            let has_normals = !mesh.normals.is_empty();
            for face in &mesh.faces {
                let v0 = face[0] + total_verts + 1;
                let v1 = face[1] + total_verts + 1;
                let v2 = face[2] + total_verts + 1;
                if has_tc && has_normals {
                    let n0 = face[0] + total_normals + 1;
                    let n1 = face[1] + total_normals + 1;
                    let n2 = face[2] + total_normals + 1;
                    let t0 = face[0] + total_texcoords + 1;
                    let t1 = face[1] + total_texcoords + 1;
                    let t2 = face[2] + total_texcoords + 1;
                    obj.push_str(&format!("f {}/{}/{} {}/{}/{} {}/{}/{}\n", v0, t0, n0, v1, t1, n1, v2, t2, n2));
                } else if has_normals {
                    let n0 = face[0] + total_normals + 1;
                    let n1 = face[1] + total_normals + 1;
                    let n2 = face[2] + total_normals + 1;
                    obj.push_str(&format!("f {}//{} {}//{} {}//{}\n", v0, n0, v1, n1, v2, n2));
                } else {
                    obj.push_str(&format!("f {} {} {}\n", v0, v1, v2));
                }
            }

            total_verts += mesh.positions.len() as u32;
            total_normals += mesh.normals.len() as u32;
            if has_tc { total_texcoords += mesh.tex_coords.len() as u32; }
        }

        obj
    }

    /// Export MTL file matching C# W3DParser format.
    pub fn export_mtl(&self, mtl_name: &str) -> String {
        let mut mtl = String::new();
        mtl.push_str("# W3D materials\n");

        for mat in &self.materials {
            let safe_name = mat.name.replace(' ', "_");
            mtl.push_str(&format!("newmtl {}\n", safe_name));
            mtl.push_str(&format!("Ka {:.4} {:.4} {:.4}\n", mat.ambient[0], mat.ambient[1], mat.ambient[2]));
            mtl.push_str(&format!("Kd {:.4} {:.4} {:.4}\n", mat.diffuse[0], mat.diffuse[1], mat.diffuse[2]));
            mtl.push_str(&format!("Ks {:.4} {:.4} {:.4}\n", mat.specular[0], mat.specular[1], mat.specular[2]));
            mtl.push_str(&format!("Ke {:.4} {:.4} {:.4}\n", mat.emissive[0], mat.emissive[1], mat.emissive[2]));
            let ns = if mat.reflectivity > 0.0 { mat.reflectivity * 1000.0 } else { mat.shininess };
            if ns > 0.0 {
                mtl.push_str(&format!("Ns {:.4}\n", ns));
            }
            mtl.push_str(&format!("d {:.4}\n", mat.opacity));

            // Find texture maps from shaders
            if let Some(shader) = self.shaders.iter().find(|s| s.material_name.eq_ignore_ascii_case(&mat.name)) {
                for layer in &shader.texture_layers {
                    if layer.name.is_empty() { continue; }
                    let ext = self.get_texture_extension(&layer.name);
                    match layer.tex_mode {
                        0 | 5 => mtl.push_str(&format!("map_Kd {}.{}\n", layer.name, ext)),
                        6 => mtl.push_str(&format!("map_Ks {}.{}\n", layer.name, ext)),
                        _ => {}
                    }
                }
            }
            mtl.push('\n');
        }

        mtl
    }

    /// Export all texture images as a list of (filename, raw_bytes) pairs.
    /// The raw bytes are in their original format (JPEG/PNG) or raw RGBA.
    /// Raw RGBA textures (4-byte header: width_le16, height_le16, then RGBA pixels)
    /// are converted to a simple TGA format for broader tool compatibility.
    pub fn export_textures(&self) -> Vec<(String, Vec<u8>)> {
        let mut result = Vec::new();
        for (name, data) in &self.texture_images {
            if data.is_empty() { continue; }
            let ext = self.get_texture_extension(name);
            let filename = format!("{}.{}", name, ext);

            if ext == "jpg" || ext == "png" {
                // Already in a standard format — pass through
                result.push((filename, data.clone()));
            } else if data.len() >= 4 {
                // Raw RGBA: first 4 bytes are width(u16 LE) + height(u16 LE), rest is RGBA pixels
                let w = u16::from_le_bytes([data[0], data[1]]) as u32;
                let h = u16::from_le_bytes([data[2], data[3]]) as u32;
                let pixel_data = &data[4..];
                let expected = (w * h * 4) as usize;
                if pixel_data.len() >= expected {
                    // Convert to uncompressed TGA (type 2) for universal compatibility
                    let tga = encode_tga_rgba(w, h, &pixel_data[..expected]);
                    let tga_filename = format!("{}.tga", name);
                    result.push((tga_filename, tga));
                }
            }
        }
        result
    }

    fn get_texture_extension(&self, tex_name: &str) -> &str {
        if let Some(data) = self.texture_images.get(tex_name) {
            if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 { return "jpg"; }
            if data.len() >= 2 && data[0] == 0x89 && data[1] == 0x50 { return "png"; }
        }
        "jpg"
    }

    /// Resolve material name for a model resource via shader bindings and model nodes.
    pub fn resolve_material_for_resource(&self, resource_name: &str) -> Option<String> {
        // Try model node shader → material chain
        for node in &self.nodes {
            if node.node_type != W3dNodeType::Model { continue; }
            let res = if !node.model_resource_name.is_empty() { &node.model_resource_name } else { &node.resource_name };
            if res != resource_name { continue; }
            if !node.shader_name.is_empty() {
                if let Some(shader) = self.shaders.iter().find(|s| s.name.eq_ignore_ascii_case(&node.shader_name)) {
                    if !shader.material_name.is_empty() {
                        return Some(shader.material_name.clone());
                    }
                }
            }
        }
        // Try resource shader bindings
        if let Some(res) = self.model_resources.get(resource_name) {
            for binding in &res.shader_bindings {
                // Try binding name as shader name
                if let Some(shader) = self.shaders.iter().find(|s| s.name.eq_ignore_ascii_case(&binding.name)) {
                    if !shader.material_name.is_empty() {
                        return Some(shader.material_name.clone());
                    }
                }
                // Try binding name as direct material name
                if self.materials.iter().any(|m| m.name.eq_ignore_ascii_case(&binding.name)) {
                    return Some(binding.name.clone());
                }
                // Try mesh binding names
                for mesh_binding in &binding.mesh_bindings {
                    if mesh_binding.is_empty() { continue; }
                    if let Some(shader) = self.shaders.iter().find(|s| s.name.eq_ignore_ascii_case(mesh_binding)) {
                        if !shader.material_name.is_empty() {
                            return Some(shader.material_name.clone());
                        }
                    }
                    if self.materials.iter().any(|m| m.name.eq_ignore_ascii_case(mesh_binding)) {
                        return Some(mesh_binding.clone());
                    }
                }
            }
        }
        // Fallback: if there's only one non-default material, use it
        let non_default: Vec<_> = self.materials.iter()
            .filter(|m| !m.name.contains("Default"))
            .collect();
        if non_default.len() == 1 {
            return Some(non_default[0].name.clone());
        }
        None
    }

    /// Resolve material for a specific sub-mesh within a resource (via shader bindings).
    fn resolve_material_for_mesh(&self, resource_name: &str, mesh_idx: usize) -> Option<String> {
        if let Some(res) = self.model_resources.get(resource_name) {
            for binding in &res.shader_bindings {
                if mesh_idx < binding.mesh_bindings.len() {
                    let binding_name = &binding.mesh_bindings[mesh_idx];
                    if binding_name.is_empty() { continue; }
                    // Try as shader name → material
                    if let Some(shader) = self.shaders.iter().find(|s| s.name.eq_ignore_ascii_case(binding_name)) {
                        if !shader.material_name.is_empty() {
                            return Some(shader.material_name.clone());
                        }
                    }
                    // Try as direct material name
                    if self.materials.iter().any(|m| m.name.eq_ignore_ascii_case(binding_name)) {
                        return Some(binding_name.clone());
                    }
                }
                // Try binding.name as shader for all meshes
                if let Some(shader) = self.shaders.iter().find(|s| s.name.eq_ignore_ascii_case(&binding.name)) {
                    if !shader.material_name.is_empty() {
                        return Some(shader.material_name.clone());
                    }
                }
            }
        }
        None
    }

    /// Resolve material for a raw mesh by matching its name to model nodes and shader bindings.
    fn resolve_raw_mesh_material(&self, mesh_name: &str) -> Option<String> {
        // Try to find a model node that references this mesh
        for node in &self.nodes {
            if node.node_type != W3dNodeType::Model { continue; }
            let res = if !node.model_resource_name.is_empty() { &node.model_resource_name } else { &node.resource_name };
            if res != mesh_name { continue; }
            if !node.shader_name.is_empty() {
                if let Some(shader) = self.shaders.iter().find(|s| s.name.eq_ignore_ascii_case(&node.shader_name)) {
                    if !shader.material_name.is_empty() {
                        return Some(shader.material_name.clone());
                    }
                }
            }
        }
        // Try matching mesh name as a resource name in shader bindings
        self.resolve_material_for_resource(mesh_name)
    }

    /// Find world transform for a model resource from the scene graph (public).
    pub fn find_transform_for_resource_pub(&self, resource_name: &str) -> Option<[f32; 16]> {
        self.find_transform_for_resource(resource_name)
    }

    /// Find world transform for a model resource from the scene graph.
    fn find_transform_for_resource(&self, resource_name: &str) -> Option<[f32; 16]> {
        let world_transforms = build_node_world_matrices(&self.nodes);
        for node in &self.nodes {
            if node.node_type != W3dNodeType::Model { continue; }
            let res = if !node.model_resource_name.is_empty() { &node.model_resource_name } else { &node.resource_name };
            if res != resource_name { continue; }
            if let Some(world_transform) = world_transforms.get(&node.name) {
                if !is_identity(world_transform) {
                    return Some(*world_transform);
                }
            }
        }
        None
    }
}

/// Recalculate smooth normals from positions and faces.
/// Area-weighted face normals accumulated per vertex, then normalized.
/// Ported from C# CLODMeshDecoder.RecalculateSmoothNormals.
fn recalculate_smooth_normals(positions: &[[f32; 3]], faces: &[(usize, [u32; 3])]) -> Vec<[f32; 3]> {
    let mut accum = vec![[0.0f32; 3]; positions.len()];

    for (_, face) in faces {
        let i0 = face[0] as usize;
        let i1 = face[1] as usize;
        let i2 = face[2] as usize;

        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() { continue; }
        if i0 == i1 || i1 == i2 || i0 == i2 { continue; }

        let p0 = positions[i0];
        let p1 = positions[i1];
        let p2 = positions[i2];

        let e1x = p1[0] - p0[0]; let e1y = p1[1] - p0[1]; let e1z = p1[2] - p0[2];
        let e2x = p2[0] - p0[0]; let e2y = p2[1] - p0[1]; let e2z = p2[2] - p0[2];

        // Cross product = face normal * 2*area (area-weighted)
        let nx = e1y * e2z - e1z * e2y;
        let ny = e1z * e2x - e1x * e2z;
        let nz = e1x * e2y - e1y * e2x;

        if nx * nx + ny * ny + nz * nz < 1e-12 { continue; }

        accum[i0][0] += nx; accum[i0][1] += ny; accum[i0][2] += nz;
        accum[i1][0] += nx; accum[i1][1] += ny; accum[i1][2] += nz;
        accum[i2][0] += nx; accum[i2][1] += ny; accum[i2][2] += nz;
    }

    accum.iter().map(|n| {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-8 { [n[0] / len, n[1] / len, n[2] / len] } else { [0.0, 1.0, 0.0] }
    }).collect()
}

fn transform_point(m: &[f32; 16], x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    (
        m[0] * x + m[4] * y + m[8] * z + m[12],
        m[1] * x + m[5] * y + m[9] * z + m[13],
        m[2] * x + m[6] * y + m[10] * z + m[14],
    )
}

fn transform_normal(m: &[f32; 16], nx: f32, ny: f32, nz: f32) -> (f32, f32, f32) {
    let ox = m[0] * nx + m[4] * ny + m[8] * nz;
    let oy = m[1] * nx + m[5] * ny + m[9] * nz;
    let oz = m[2] * nx + m[6] * ny + m[10] * nz;
    let len = (ox * ox + oy * oy + oz * oz).sqrt();
    if len > 1e-8 { (ox / len, oy / len, oz / len) } else { (nx, ny, nz) }
}

fn is_identity(m: &[f32; 16]) -> bool {
    let id = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
    m.iter().zip(id.iter()).all(|(a, b)| (a - b).abs() < 1e-6)
}

/// Encode raw RGBA pixel data as an uncompressed TGA file (type 2, 32-bit BGRA).
fn encode_tga_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let w = width as u16;
    let h = height as u16;
    // TGA header: 18 bytes
    let mut tga = Vec::with_capacity(18 + rgba.len());
    tga.push(0);           // ID length
    tga.push(0);           // Color map type (none)
    tga.push(2);           // Image type (uncompressed true-color)
    tga.extend_from_slice(&[0, 0, 0, 0, 0]); // Color map spec (unused)
    tga.extend_from_slice(&[0, 0]); // X origin
    tga.extend_from_slice(&[0, 0]); // Y origin
    tga.extend_from_slice(&w.to_le_bytes()); // Width
    tga.extend_from_slice(&h.to_le_bytes()); // Height
    tga.push(32);          // Bits per pixel (BGRA)
    tga.push(0x28);        // Image descriptor: top-left origin + 8 alpha bits

    // Convert RGBA to BGRA (TGA native order)
    for pixel in rgba.chunks_exact(4) {
        tga.push(pixel[2]); // B
        tga.push(pixel[1]); // G
        tga.push(pixel[0]); // R
        tga.push(pixel[3]); // A
    }
    tga
}
