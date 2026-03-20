//! glTF 2.0 exporter for W3D scenes.
//!
//! Exports meshes, materials, textures, skeleton, and animations as a GLB (binary glTF) file.
//! This format supports bones, weights, and keyframe animations unlike OBJ.

use super::types::*;

/// Export a W3dScene as a GLB (binary glTF) byte buffer.
pub fn export_glb(scene: &W3dScene) -> Vec<u8> {
    let mut bin = BinaryBuffer::new();
    let mut gltf = GltfRoot::new();

    // Add a single scene
    gltf.scenes.push(GltfScene { nodes: vec![] });

    // Export meshes and materials
    let mut mesh_node_indices = vec![];
    for (resource_name, meshes) in &scene.clod_meshes {
        let world_transform = scene.find_transform_for_resource_pub(resource_name);
        for (mi, mesh) in meshes.iter().enumerate() {
            if mesh.positions.is_empty() || mesh.faces.is_empty() { continue; }

            let mesh_name = if meshes.len() > 1 {
                format!("{}_{}", resource_name, mi)
            } else {
                resource_name.clone()
            };

            let mesh_idx = export_mesh(&mut gltf, &mut bin, mesh, &mesh_name, scene);
            let node_idx = gltf.nodes.len();
            let mut node = GltfNode {
                name: mesh_name,
                mesh: Some(mesh_idx),
                ..Default::default()
            };
            if let Some(m) = world_transform {
                node.matrix = Some(m);
            }
            gltf.nodes.push(node);
            gltf.scenes[0].nodes.push(node_idx);
            mesh_node_indices.push(node_idx);
        }
    }

    // Export raw meshes
    for mesh in &scene.raw_meshes {
        if mesh.positions.is_empty() || mesh.faces.is_empty() { continue; }
        let decoded = ClodDecodedMesh {
            name: mesh.name.clone(),
            positions: mesh.positions.clone(),
            normals: mesh.normals.clone(),
            tex_coords: vec![mesh.tex_coords.clone()],
            faces: mesh.faces.clone(),
            diffuse_colors: mesh.vertex_colors.clone(),
            specular_colors: vec![],
            bone_indices: vec![],
            bone_weights: vec![],
        };
        let mesh_idx = export_mesh(&mut gltf, &mut bin, &decoded, &mesh.name, scene);
        let node_idx = gltf.nodes.len();
        gltf.nodes.push(GltfNode {
            name: mesh.name.clone(),
            mesh: Some(mesh_idx),
            ..Default::default()
        });
        gltf.scenes[0].nodes.push(node_idx);
    }

    // Export skeleton + skin if available
    if let Some(skeleton) = scene.skeletons.first() {
        if skeleton.bones.len() > 1 {
            export_skeleton(&mut gltf, &mut bin, skeleton, scene, &mesh_node_indices);
        }
    }

    // Export textures
    for (name, data) in &scene.texture_images {
        if data.is_empty() { continue; }
        let is_png = data.len() >= 2 && data[0] == 0x89 && data[1] == 0x50;
        let mime = if is_png { "image/png" } else { "image/jpeg" };

        let buf_view_idx = add_buffer_view(&mut gltf, &mut bin, data);
        let image_idx = gltf.images.len();
        gltf.images.push(GltfImage {
            buffer_view: buf_view_idx,
            mime_type: mime.to_string(),
            name: name.clone(),
        });
        let tex_idx = gltf.textures.len();
        gltf.textures.push(GltfTexture { source: image_idx, name: name.clone() });

        // Link to materials by matching texture name in shader layers
        for (mat_idx, mat) in gltf.materials.iter_mut().enumerate() {
            if let Some(shader) = scene.shaders.get(mat_idx) {
                if shader.texture_layers.iter().any(|l| l.name == *name && (l.tex_mode == 0 || l.tex_mode == 5)) {
                    mat.base_color_texture = Some(tex_idx);
                }
            }
        }
    }

    // Serialize to GLB
    let json_str = serialize_gltf(&gltf, bin.data.len());
    encode_glb(&json_str, &bin.data)
}

fn export_mesh(
    gltf: &mut GltfRoot,
    bin: &mut BinaryBuffer,
    mesh: &ClodDecodedMesh,
    name: &str,
    scene: &W3dScene,
) -> usize {
    // Positions
    let (pos_min, pos_max) = compute_bounds(&mesh.positions);
    let pos_view = add_buffer_view(gltf, bin, &flatten_vec3(&mesh.positions));
    let pos_acc = gltf.accessors.len();
    gltf.accessors.push(GltfAccessor {
        buffer_view: pos_view,
        component_type: 5126, // FLOAT
        count: mesh.positions.len(),
        acc_type: "VEC3".into(),
        min: Some(pos_min),
        max: Some(pos_max),
    });

    // Normals
    let norm_view = add_buffer_view(gltf, bin, &flatten_vec3(&mesh.normals));
    let norm_acc = gltf.accessors.len();
    gltf.accessors.push(GltfAccessor {
        buffer_view: norm_view,
        component_type: 5126,
        count: mesh.normals.len(),
        acc_type: "VEC3".into(),
        min: None, max: None,
    });

    // Texcoords
    let tc_acc = if !mesh.tex_coords.is_empty() && !mesh.tex_coords[0].is_empty() {
        let tc_data: Vec<[f32; 2]> = mesh.tex_coords[0].iter().map(|tc| {
            // Remap W3D [-0.5, 0.5] → [0, 1], V-flip
            [tc[0] + 0.5, 0.5 - tc[1]]
        }).collect();
        let tc_view = add_buffer_view(gltf, bin, &flatten_vec2(&tc_data));
        let idx = gltf.accessors.len();
        gltf.accessors.push(GltfAccessor {
            buffer_view: tc_view,
            component_type: 5126,
            count: tc_data.len(),
            acc_type: "VEC2".into(),
            min: None, max: None,
        });
        Some(idx)
    } else { None };

    // Indices (reverse winding for glTF CCW)
    let idx_data: Vec<u32> = mesh.faces.iter().flat_map(|f| [f[0], f[2], f[1]]).collect();
    let idx_view = add_buffer_view(gltf, bin, &to_bytes_u32(&idx_data));
    let idx_acc = gltf.accessors.len();
    gltf.accessors.push(GltfAccessor {
        buffer_view: idx_view,
        component_type: 5125, // UNSIGNED_INT
        count: idx_data.len(),
        acc_type: "SCALAR".into(),
        min: None, max: None,
    });

    // Bone indices + weights (for skinning)
    let joints_acc = if !mesh.bone_indices.is_empty() && mesh.bone_indices.len() == mesh.positions.len() {
        let joints: Vec<[u16; 4]> = mesh.bone_indices.iter().map(|v| {
            let mut out = [0u16; 4];
            for (i, &idx) in v.iter().take(4).enumerate() { out[i] = idx as u16; }
            out
        }).collect();
        let jv = add_buffer_view(gltf, bin, &to_bytes_u16_4(&joints));
        let idx = gltf.accessors.len();
        gltf.accessors.push(GltfAccessor {
            buffer_view: jv,
            component_type: 5123, // UNSIGNED_SHORT
            count: joints.len(),
            acc_type: "VEC4".into(),
            min: None, max: None,
        });
        Some(idx)
    } else { None };

    let weights_acc = if !mesh.bone_weights.is_empty() && mesh.bone_weights.len() == mesh.positions.len() {
        let weights: Vec<[f32; 4]> = mesh.bone_weights.iter().map(|v| {
            let mut out = [0.0f32; 4];
            let mut sum = 0.0f32;
            for (i, &w) in v.iter().take(4).enumerate() { out[i] = w; sum += w; }
            if sum > 0.0 { for w in out.iter_mut() { *w /= sum; } } else { out[0] = 1.0; }
            out
        }).collect();
        let wv = add_buffer_view(gltf, bin, &flatten_vec4(&weights));
        let idx = gltf.accessors.len();
        gltf.accessors.push(GltfAccessor {
            buffer_view: wv,
            component_type: 5126,
            count: weights.len(),
            acc_type: "VEC4".into(),
            min: None, max: None,
        });
        Some(idx)
    } else { None };

    // Material
    let mat_idx = resolve_material_index(gltf, scene, name);

    let mesh_idx = gltf.meshes.len();
    let mut prim = GltfPrimitive {
        attributes: vec![
            ("POSITION".into(), pos_acc),
            ("NORMAL".into(), norm_acc),
        ],
        indices: idx_acc,
        material: mat_idx,
    };
    if let Some(tc) = tc_acc {
        prim.attributes.push(("TEXCOORD_0".into(), tc));
    }
    if let Some(j) = joints_acc {
        prim.attributes.push(("JOINTS_0".into(), j));
    }
    if let Some(w) = weights_acc {
        prim.attributes.push(("WEIGHTS_0".into(), w));
    }

    gltf.meshes.push(GltfMesh {
        name: name.to_string(),
        primitives: vec![prim],
    });
    mesh_idx
}

fn export_skeleton(
    gltf: &mut GltfRoot,
    bin: &mut BinaryBuffer,
    skeleton: &W3dSkeleton,
    scene: &W3dScene,
    mesh_node_indices: &[usize],
) {
    let bone_node_start = gltf.nodes.len();

    // Create bone nodes
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let node = GltfNode {
            name: bone.name.clone(),
            translation: Some([bone.dir_x, bone.dir_y, bone.dir_z]),
            rotation: Some([bone.rot_x, bone.rot_y, bone.rot_z, bone.rot_w]),
            ..Default::default()
        };
        gltf.nodes.push(node);
    }

    // Set up parent-child relationships
    for (i, bone) in skeleton.bones.iter().enumerate() {
        if bone.parent_index >= 0 {
            let parent_idx = bone_node_start + bone.parent_index as usize;
            if parent_idx < gltf.nodes.len() {
                gltf.nodes[parent_idx].children.push(bone_node_start + i);
            }
        }
    }

    // Add root bones to scene
    for (i, bone) in skeleton.bones.iter().enumerate() {
        if bone.parent_index < 0 {
            gltf.scenes[0].nodes.push(bone_node_start + i);
        }
    }

    // Compute inverse bind matrices
    let inv_bind = super::skeleton::build_inverse_bind_matrices(skeleton);
    let ibm_data: Vec<u8> = inv_bind.iter().flat_map(|m| {
        m.iter().flat_map(|f| f.to_le_bytes())
    }).collect();
    let ibm_view = add_buffer_view(gltf, bin, &ibm_data);
    let ibm_acc = gltf.accessors.len();
    gltf.accessors.push(GltfAccessor {
        buffer_view: ibm_view,
        component_type: 5126,
        count: skeleton.bones.len(),
        acc_type: "MAT4".into(),
        min: None, max: None,
    });

    // Create skin
    let joint_indices: Vec<usize> = (0..skeleton.bones.len()).map(|i| bone_node_start + i).collect();
    let skin_idx = gltf.skins.len();
    gltf.skins.push(GltfSkin {
        name: skeleton.name.clone(),
        inverse_bind_matrices: ibm_acc,
        joints: joint_indices,
        skeleton_root: Some(bone_node_start),
    });

    // Assign skin to mesh nodes
    for &ni in mesh_node_indices {
        if ni < gltf.nodes.len() {
            gltf.nodes[ni].skin = Some(skin_idx);
        }
    }

    // Export animations
    for motion in &scene.motions {
        export_animation(gltf, bin, motion, skeleton, bone_node_start);
    }
}

fn export_animation(
    gltf: &mut GltfRoot,
    bin: &mut BinaryBuffer,
    motion: &W3dMotion,
    skeleton: &W3dSkeleton,
    bone_node_start: usize,
) {
    let mut channels = vec![];
    let mut samplers = vec![];

    for track in &motion.tracks {
        let bone_idx = match skeleton.find_bone_by_name(&track.bone_name) {
            Some(i) => i,
            None => continue,
        };
        let target_node = bone_node_start + bone_idx;
        if track.keyframes.is_empty() { continue; }

        // Time accessor (shared for translation + rotation)
        let times: Vec<f32> = track.keyframes.iter().map(|k| k.time).collect();
        let time_view = add_buffer_view(gltf, bin, &to_bytes_f32(&times));
        let time_acc = gltf.accessors.len();
        let t_min = times.first().copied().unwrap_or(0.0);
        let t_max = times.last().copied().unwrap_or(0.0);
        gltf.accessors.push(GltfAccessor {
            buffer_view: time_view,
            component_type: 5126,
            count: times.len(),
            acc_type: "SCALAR".into(),
            min: Some(vec![t_min]),
            max: Some(vec![t_max]),
        });

        // Translation
        let translations: Vec<f32> = track.keyframes.iter()
            .flat_map(|k| [k.pos_x, k.pos_y, k.pos_z])
            .collect();
        let trans_view = add_buffer_view(gltf, bin, &to_bytes_f32(&translations));
        let trans_acc = gltf.accessors.len();
        gltf.accessors.push(GltfAccessor {
            buffer_view: trans_view,
            component_type: 5126,
            count: track.keyframes.len(),
            acc_type: "VEC3".into(),
            min: None, max: None,
        });
        let trans_sampler = samplers.len();
        samplers.push(GltfAnimSampler { input: time_acc, output: trans_acc, interpolation: "LINEAR".into() });
        channels.push(GltfAnimChannel { sampler: trans_sampler, target_node, target_path: "translation".into() });

        // Rotation (quaternion)
        let rotations: Vec<f32> = track.keyframes.iter()
            .flat_map(|k| [k.rot_x, k.rot_y, k.rot_z, k.rot_w])
            .collect();
        let rot_view = add_buffer_view(gltf, bin, &to_bytes_f32(&rotations));
        let rot_acc = gltf.accessors.len();
        gltf.accessors.push(GltfAccessor {
            buffer_view: rot_view,
            component_type: 5126,
            count: track.keyframes.len(),
            acc_type: "VEC4".into(),
            min: None, max: None,
        });
        let rot_sampler = samplers.len();
        samplers.push(GltfAnimSampler { input: time_acc, output: rot_acc, interpolation: "LINEAR".into() });
        channels.push(GltfAnimChannel { sampler: rot_sampler, target_node, target_path: "rotation".into() });

        // Scale
        let has_scale = track.keyframes.iter().any(|k| {
            (k.scale_x - 1.0).abs() > 0.001 || (k.scale_y - 1.0).abs() > 0.001 || (k.scale_z - 1.0).abs() > 0.001
        });
        if has_scale {
            let scales: Vec<f32> = track.keyframes.iter()
                .flat_map(|k| [k.scale_x.max(0.001), k.scale_y.max(0.001), k.scale_z.max(0.001)])
                .collect();
            let scale_view = add_buffer_view(gltf, bin, &to_bytes_f32(&scales));
            let scale_acc = gltf.accessors.len();
            gltf.accessors.push(GltfAccessor {
                buffer_view: scale_view,
                component_type: 5126,
                count: track.keyframes.len(),
                acc_type: "VEC3".into(),
                min: None, max: None,
            });
            let scale_sampler = samplers.len();
            samplers.push(GltfAnimSampler { input: time_acc, output: scale_acc, interpolation: "LINEAR".into() });
            channels.push(GltfAnimChannel { sampler: scale_sampler, target_node, target_path: "scale".into() });
        }
    }

    if !channels.is_empty() {
        gltf.animations.push(GltfAnimation {
            name: motion.name.clone(),
            channels,
            samplers,
        });
    }
}

fn resolve_material_index(gltf: &mut GltfRoot, scene: &W3dScene, resource_name: &str) -> Option<usize> {
    let mat_name = scene.resolve_material_for_resource(resource_name)?;
    let mat = scene.materials.iter().find(|m| m.name == mat_name)?;

    // Check if already added
    if let Some(idx) = gltf.materials.iter().position(|m| m.name == mat_name) {
        return Some(idx);
    }

    let idx = gltf.materials.len();
    gltf.materials.push(GltfMaterial {
        name: mat_name,
        base_color: [mat.diffuse[0], mat.diffuse[1], mat.diffuse[2], mat.opacity],
        metallic: mat.reflectivity,
        roughness: 1.0 - (mat.shininess / 128.0).min(1.0),
        emissive: [mat.emissive[0], mat.emissive[1], mat.emissive[2]],
        base_color_texture: None,
    });
    Some(idx)
}

// ─── Binary buffer helpers ───

struct BinaryBuffer {
    data: Vec<u8>,
}

impl BinaryBuffer {
    fn new() -> Self { Self { data: Vec::new() } }

    fn write(&mut self, bytes: &[u8]) -> (usize, usize) {
        let offset = self.data.len();
        self.data.extend_from_slice(bytes);
        // Align to 4 bytes
        while self.data.len() % 4 != 0 { self.data.push(0); }
        (offset, bytes.len())
    }
}

fn add_buffer_view(gltf: &mut GltfRoot, bin: &mut BinaryBuffer, data: &[u8]) -> usize {
    let (offset, length) = bin.write(data);
    let idx = gltf.buffer_views.len();
    gltf.buffer_views.push(GltfBufferView { byte_offset: offset, byte_length: length });
    idx
}

fn flatten_vec3(data: &[[f32; 3]]) -> Vec<u8> {
    data.iter().flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes())).collect()
}
fn flatten_vec2(data: &[[f32; 2]]) -> Vec<u8> {
    data.iter().flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes())).collect()
}
fn flatten_vec4(data: &[[f32; 4]]) -> Vec<u8> {
    data.iter().flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes())).collect()
}
fn to_bytes_u32(data: &[u32]) -> Vec<u8> {
    data.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn to_bytes_f32(data: &[f32]) -> Vec<u8> {
    data.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn to_bytes_u16_4(data: &[[u16; 4]]) -> Vec<u8> {
    data.iter().flat_map(|v| v.iter().flat_map(|u| u.to_le_bytes())).collect()
}

fn compute_bounds(positions: &[[f32; 3]]) -> (Vec<f32>, Vec<f32>) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in positions {
        for i in 0..3 {
            if p[i] < min[i] { min[i] = p[i]; }
            if p[i] > max[i] { max[i] = p[i]; }
        }
    }
    (min.to_vec(), max.to_vec())
}

// ─── glTF data structures (minimal for serialization) ───

#[derive(Default)]
struct GltfRoot {
    scenes: Vec<GltfScene>,
    nodes: Vec<GltfNode>,
    meshes: Vec<GltfMesh>,
    accessors: Vec<GltfAccessor>,
    buffer_views: Vec<GltfBufferView>,
    materials: Vec<GltfMaterial>,
    textures: Vec<GltfTexture>,
    images: Vec<GltfImage>,
    skins: Vec<GltfSkin>,
    animations: Vec<GltfAnimation>,
}

impl GltfRoot {
    fn new() -> Self { Self::default() }
}

#[derive(Default)]
struct GltfScene { nodes: Vec<usize> }

#[derive(Default)]
struct GltfNode {
    name: String,
    mesh: Option<usize>,
    skin: Option<usize>,
    children: Vec<usize>,
    matrix: Option<[f32; 16]>,
    translation: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
}

struct GltfMesh {
    name: String,
    primitives: Vec<GltfPrimitive>,
}

struct GltfPrimitive {
    attributes: Vec<(String, usize)>,
    indices: usize,
    material: Option<usize>,
}

struct GltfAccessor {
    buffer_view: usize,
    component_type: u32,
    count: usize,
    acc_type: String,
    min: Option<Vec<f32>>,
    max: Option<Vec<f32>>,
}

struct GltfBufferView {
    byte_offset: usize,
    byte_length: usize,
}

struct GltfMaterial {
    name: String,
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    emissive: [f32; 3],
    base_color_texture: Option<usize>,
}

struct GltfTexture { source: usize, name: String }
struct GltfImage { buffer_view: usize, mime_type: String, name: String }

struct GltfSkin {
    name: String,
    inverse_bind_matrices: usize,
    joints: Vec<usize>,
    skeleton_root: Option<usize>,
}

struct GltfAnimation {
    name: String,
    channels: Vec<GltfAnimChannel>,
    samplers: Vec<GltfAnimSampler>,
}

struct GltfAnimChannel {
    sampler: usize,
    target_node: usize,
    target_path: String,
}

struct GltfAnimSampler {
    input: usize,
    output: usize,
    interpolation: String,
}

// ─── JSON serialization (manual, no serde dependency) ───

fn serialize_gltf(root: &GltfRoot, bin_size: usize) -> String {
    let mut j = String::from("{\n");
    j.push_str("  \"asset\": { \"version\": \"2.0\", \"generator\": \"dirplayer-rs W3D exporter\" },\n");

    // Scenes
    j.push_str("  \"scene\": 0,\n");
    j.push_str("  \"scenes\": [");
    for (i, s) in root.scenes.iter().enumerate() {
        if i > 0 { j.push(','); }
        j.push_str(&format!("{{ \"nodes\": {:?} }}", s.nodes));
    }
    j.push_str("],\n");

    // Nodes
    j.push_str("  \"nodes\": [");
    for (i, n) in root.nodes.iter().enumerate() {
        if i > 0 { j.push(','); }
        j.push_str(&format!("{{ \"name\": {:?}", n.name));
        if let Some(m) = n.mesh { j.push_str(&format!(", \"mesh\": {}", m)); }
        if let Some(s) = n.skin { j.push_str(&format!(", \"skin\": {}", s)); }
        if !n.children.is_empty() { j.push_str(&format!(", \"children\": {:?}", n.children)); }
        if let Some(ref m) = n.matrix {
            j.push_str(&format!(", \"matrix\": {:?}", m.to_vec()));
        }
        if let Some(ref t) = n.translation {
            j.push_str(&format!(", \"translation\": {:?}", t.to_vec()));
        }
        if let Some(ref r) = n.rotation {
            j.push_str(&format!(", \"rotation\": {:?}", r.to_vec()));
        }
        j.push('}');
    }
    j.push_str("],\n");

    // Meshes
    j.push_str("  \"meshes\": [");
    for (i, m) in root.meshes.iter().enumerate() {
        if i > 0 { j.push(','); }
        j.push_str(&format!("{{ \"name\": {:?}, \"primitives\": [", m.name));
        for (pi, p) in m.primitives.iter().enumerate() {
            if pi > 0 { j.push(','); }
            j.push_str("{ \"attributes\": {");
            for (ai, (name, idx)) in p.attributes.iter().enumerate() {
                if ai > 0 { j.push(','); }
                j.push_str(&format!(" {:?}: {}", name, idx));
            }
            j.push_str(&format!(" }}, \"indices\": {}", p.indices));
            if let Some(mat) = p.material { j.push_str(&format!(", \"material\": {}", mat)); }
            j.push('}');
        }
        j.push_str("] }");
    }
    j.push_str("],\n");

    // Accessors
    j.push_str("  \"accessors\": [");
    for (i, a) in root.accessors.iter().enumerate() {
        if i > 0 { j.push(','); }
        j.push_str(&format!(
            "{{ \"bufferView\": {}, \"componentType\": {}, \"count\": {}, \"type\": {:?}",
            a.buffer_view, a.component_type, a.count, a.acc_type
        ));
        if let Some(ref min) = a.min { j.push_str(&format!(", \"min\": {:?}", min)); }
        if let Some(ref max) = a.max { j.push_str(&format!(", \"max\": {:?}", max)); }
        j.push('}');
    }
    j.push_str("],\n");

    // Buffer views
    j.push_str("  \"bufferViews\": [");
    for (i, bv) in root.buffer_views.iter().enumerate() {
        if i > 0 { j.push(','); }
        j.push_str(&format!(
            "{{ \"buffer\": 0, \"byteOffset\": {}, \"byteLength\": {} }}",
            bv.byte_offset, bv.byte_length
        ));
    }
    j.push_str("],\n");

    // Buffers
    j.push_str(&format!("  \"buffers\": [{{ \"byteLength\": {} }}],\n", bin_size));

    // Materials
    if !root.materials.is_empty() {
        j.push_str("  \"materials\": [");
        for (i, m) in root.materials.iter().enumerate() {
            if i > 0 { j.push(','); }
            j.push_str(&format!("{{ \"name\": {:?}, \"pbrMetallicRoughness\": {{ \"baseColorFactor\": {:?}, \"metallicFactor\": {}, \"roughnessFactor\": {}",
                m.name, m.base_color.to_vec(), m.metallic, m.roughness));
            if let Some(tex) = m.base_color_texture {
                j.push_str(&format!(", \"baseColorTexture\": {{ \"index\": {} }}", tex));
            }
            j.push_str(" }");
            if m.emissive[0] > 0.0 || m.emissive[1] > 0.0 || m.emissive[2] > 0.0 {
                j.push_str(&format!(", \"emissiveFactor\": {:?}", m.emissive.to_vec()));
            }
            j.push_str(" }");
        }
        j.push_str("],\n");
    }

    // Textures
    if !root.textures.is_empty() {
        j.push_str("  \"textures\": [");
        for (i, t) in root.textures.iter().enumerate() {
            if i > 0 { j.push(','); }
            j.push_str(&format!("{{ \"source\": {} }}", t.source));
        }
        j.push_str("],\n");
    }

    // Images
    if !root.images.is_empty() {
        j.push_str("  \"images\": [");
        for (i, img) in root.images.iter().enumerate() {
            if i > 0 { j.push(','); }
            j.push_str(&format!("{{ \"bufferView\": {}, \"mimeType\": {:?}, \"name\": {:?} }}",
                img.buffer_view, img.mime_type, img.name));
        }
        j.push_str("],\n");
    }

    // Skins
    if !root.skins.is_empty() {
        j.push_str("  \"skins\": [");
        for (i, s) in root.skins.iter().enumerate() {
            if i > 0 { j.push(','); }
            j.push_str(&format!("{{ \"name\": {:?}, \"inverseBindMatrices\": {}, \"joints\": {:?}",
                s.name, s.inverse_bind_matrices, s.joints));
            if let Some(root) = s.skeleton_root {
                j.push_str(&format!(", \"skeleton\": {}", root));
            }
            j.push_str(" }");
        }
        j.push_str("],\n");
    }

    // Animations
    if !root.animations.is_empty() {
        j.push_str("  \"animations\": [");
        for (i, anim) in root.animations.iter().enumerate() {
            if i > 0 { j.push(','); }
            j.push_str(&format!("{{ \"name\": {:?}, \"channels\": [", anim.name));
            for (ci, ch) in anim.channels.iter().enumerate() {
                if ci > 0 { j.push(','); }
                j.push_str(&format!(
                    "{{ \"sampler\": {}, \"target\": {{ \"node\": {}, \"path\": {:?} }} }}",
                    ch.sampler, ch.target_node, ch.target_path
                ));
            }
            j.push_str("], \"samplers\": [");
            for (si, s) in anim.samplers.iter().enumerate() {
                if si > 0 { j.push(','); }
                j.push_str(&format!(
                    "{{ \"input\": {}, \"output\": {}, \"interpolation\": {:?} }}",
                    s.input, s.output, s.interpolation
                ));
            }
            j.push_str("] }");
        }
        j.push_str("],\n");
    }

    // Remove trailing comma+newline and close
    if j.ends_with(",\n") {
        j.truncate(j.len() - 2);
        j.push('\n');
    }
    j.push('}');
    j
}

/// Encode GLB binary container (magic + JSON chunk + BIN chunk)
fn encode_glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let json_bytes = json.as_bytes();
    // Pad JSON to 4-byte alignment with spaces
    let json_padded_len = (json_bytes.len() + 3) & !3;
    let bin_padded_len = (bin.len() + 3) & !3;

    let total_len = 12 + 8 + json_padded_len + 8 + bin_padded_len;
    let mut glb = Vec::with_capacity(total_len);

    // Header
    glb.extend_from_slice(b"glTF");                  // magic
    glb.extend_from_slice(&2u32.to_le_bytes());       // version
    glb.extend_from_slice(&(total_len as u32).to_le_bytes()); // total length

    // JSON chunk
    glb.extend_from_slice(&(json_padded_len as u32).to_le_bytes()); // chunk length
    glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes());            // chunk type: JSON
    glb.extend_from_slice(json_bytes);
    for _ in 0..(json_padded_len - json_bytes.len()) { glb.push(b' '); }

    // BIN chunk
    glb.extend_from_slice(&(bin_padded_len as u32).to_le_bytes()); // chunk length
    glb.extend_from_slice(&0x004E4942u32.to_le_bytes());            // chunk type: BIN
    glb.extend_from_slice(bin);
    for _ in 0..(bin_padded_len - bin.len()) { glb.push(0); }

    glb
}
