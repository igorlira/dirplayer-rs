use std::collections::VecDeque;
use log::debug;

use crate::{
    director::{
        chunks::w3d::types::*,
        lingo::datum::Datum,
    },
    player::{
        cast_lib::CastMemberRef,
        reserve_player_mut,
        DatumRef, ScriptError,
    },
};

const IDENTITY_MATRIX: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

pub struct Shockwave3dObjectDatumHandlers {}

impl Shockwave3dObjectDatumHandlers {
    pub fn get_prop(obj_ref: &DatumRef, prop_name: &String) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let s3d_ref = match player.get_datum(obj_ref) {
                Datum::Shockwave3dObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected Shockwave3dObjectRef".to_string())),
            };
            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
            let scene = {
                let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                    .ok_or_else(|| ScriptError::new("3D member not found".to_string()))?;
                let w3d = member.member_type.as_shockwave3d()
                    .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
                w3d.parsed_scene.clone()
                    .ok_or_else(|| ScriptError::new("No parsed 3D scene".to_string()))?
            };
            Self::get_prop_inner(player, &s3d_ref, &member_ref, &scene, prop_name)
        })
    }

    fn get_prop_inner(
        player: &mut crate::player::DirPlayer,
        s3d_ref: &crate::director::lingo::datum::Shockwave3dObjectRef,
        member_ref: &CastMemberRef,
        scene: &W3dScene,
        prop_name: &str,
    ) -> Result<DatumRef, ScriptError> {
        match s3d_ref.object_type.as_str() {
            "model" | "bonesPlayer" => Self::get_model_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "shader" => Self::get_shader_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "texture" => Self::get_texture_prop(player, scene, &s3d_ref.name, prop_name),
            "camera" => Self::get_camera_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "light" => Self::get_light_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "group" => Self::get_node_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "modelResource" => Self::get_model_resource_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "motion" => Self::get_motion_prop(player, scene, &s3d_ref.name, prop_name),
            "colorBuffer" => {
                // colorBuffer.clearAtRender property
                let cam_name = s3d_ref.name.clone();
                match prop_name {
                    "clearAtRender" => {
                        let val = {
                            let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                            member.and_then(|m| m.member_type.as_shockwave3d())
                                .and_then(|w3d| w3d.runtime_state.camera_clear_at_render.get(&cam_name))
                                .copied()
                                .unwrap_or(true)
                        };
                        Ok(player.alloc_datum(Datum::Int(if val { 1 } else { 0 })))
                    }
                    _ => Ok(player.alloc_datum(Datum::Void)),
                }
            }
            "meshDeform" => Self::get_mesh_deform_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "overlay" | "backdrop" => {
                // overlay/backdrop object: name format "cameraName:index"
                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                let cam_name = parts.get(0).unwrap_or(&"").to_string();
                let ov_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let is_overlay = s3d_ref.object_type == "overlay";
                let overlay = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| {
                            let map = if is_overlay { &w3d.runtime_state.camera_overlays } else { &w3d.runtime_state.camera_backdrops };
                            map.get(&cam_name).and_then(|v| v.get(ov_idx)).cloned()
                        })
                };
                match prop_name {
                    "source" => {
                        if let Some(ov) = &overlay {
                            if !ov.source_texture.is_empty() {
                                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(
                                    crate::director::lingo::datum::Shockwave3dObjectRef {
                                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                        object_type: "texture".to_string(), name: ov.source_texture.clone(),
                                    }
                                )))
                            } else { Ok(player.alloc_datum(Datum::Void)) }
                        } else { Ok(player.alloc_datum(Datum::Void)) }
                    }
                    "loc" => {
                        let ov = overlay.unwrap_or_default();
                        let x = player.alloc_datum(Datum::Float(ov.loc[0]));
                        let y = player.alloc_datum(Datum::Float(ov.loc[1]));
                        Ok(player.alloc_datum(Datum::Point([x, y])))
                    }
                    "blend" => Ok(player.alloc_datum(Datum::Float(overlay.map(|o| o.blend).unwrap_or(100.0)))),
                    "scale" => Ok(player.alloc_datum(Datum::Float(overlay.map(|o| o.scale).unwrap_or(1.0)))),
                    "rotation" => Ok(player.alloc_datum(Datum::Float(overlay.map(|o| o.rotation).unwrap_or(0.0)))),
                    "regPoint" => {
                        let ov = overlay.unwrap_or_default();
                        let x = player.alloc_datum(Datum::Float(ov.reg_point[0]));
                        let y = player.alloc_datum(Datum::Float(ov.reg_point[1]));
                        Ok(player.alloc_datum(Datum::Point([x, y])))
                    }
                    _ => Ok(player.alloc_datum(Datum::Void)),
                }
            }
            "emitter" => {
                // Get or create persistent emitter state
                let emitter = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.emitters.get(&s3d_ref.name))
                        .cloned()
                        .unwrap_or_default()
                };
                match prop_name {
                    "loop" => Ok(player.alloc_datum(Datum::Int(if emitter.is_loop { 1 } else { 0 }))),
                    "mode" => Ok(player.alloc_datum(Datum::Symbol(emitter.mode.clone()))),
                    "numParticles" => Ok(player.alloc_datum(Datum::Int(emitter.num_particles))),
                    "direction" => Ok(player.alloc_datum(Datum::Vector(emitter.direction))),
                    "region" => Ok(player.alloc_datum(Datum::Vector(emitter.region))),
                    "distribution" => Ok(player.alloc_datum(Datum::Symbol(emitter.distribution.clone()))),
                    "angle" => Ok(player.alloc_datum(Datum::Float(emitter.angle))),
                    "path" => Ok(player.alloc_datum(Datum::Void)),
                    "pathStrength" => Ok(player.alloc_datum(Datum::Float(emitter.path_strength))),
                    "minSpeed" => Ok(player.alloc_datum(Datum::Float(emitter.min_speed))),
                    "maxSpeed" => Ok(player.alloc_datum(Datum::Float(emitter.max_speed))),
                    _ => {
                        web_sys::console::log_1(&format!("[W3D] emitter(\"{}\").{} (stub)", s3d_ref.name, prop_name).into());
                        Ok(player.alloc_datum(Datum::Void))
                    }
                }
            }
            "sds" => {
                // Subdivision Surface modifier properties
                let sds = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.sds_state.get(&s3d_ref.name))
                        .cloned()
                        .unwrap_or_default()
                };
                match prop_name {
                    "depth" => Ok(player.alloc_datum(Datum::Int(sds.depth))),
                    "tension" => Ok(player.alloc_datum(Datum::Int(sds.tension as i32))),
                    "error" => Ok(player.alloc_datum(Datum::Int(sds.error as i32))),
                    "enabled" => Ok(player.alloc_datum(Datum::Int(if sds.enabled { 1 } else { 0 }))),
                    _ => {
                        web_sys::console::log_1(&format!("[W3D] sds(\"{}\").{} not implemented", s3d_ref.name, prop_name).into());
                        Ok(player.alloc_datum(Datum::Void))
                    }
                }
            }
            "bone" => {
                // name format is "modelName:boneIndex"
                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                let model_name = parts.get(0).unwrap_or(&"");
                let bone_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                match prop_name {
                    "transform" | "worldTransform" => {
                        // Evaluate skeleton and return actual bone world matrix
                        let bone_matrix = player.movie.cast_manager.find_member_by_ref(member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| {
                                let scene = w3d.parsed_scene.as_ref()?;
                                let skeleton = scene.skeletons.first()?;
                                if bone_idx >= skeleton.bones.len() { return None; }
                                let motion = w3d.runtime_state.current_motion.as_deref()
                                    .and_then(|name| scene.motions.iter().find(|m| m.name == name))
                                    .or_else(|| scene.motions.first());
                                let time = w3d.runtime_state.animation_time;
                                let duration = motion.map(|m| m.duration()).unwrap_or(0.0);
                                let t = if duration > 0.0 { time % duration } else { 0.0 };
                                let matrices = crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, motion, t);
                                matrices.get(bone_idx).copied()
                            });
                        if let Some(m) = bone_matrix {
                            // Return as Transform3d datum
                            let m64: [f64; 16] = [
                                m[0] as f64, m[1] as f64, m[2] as f64, m[3] as f64,
                                m[4] as f64, m[5] as f64, m[6] as f64, m[7] as f64,
                                m[8] as f64, m[9] as f64, m[10] as f64, m[11] as f64,
                                m[12] as f64, m[13] as f64, m[14] as f64, m[15] as f64,
                            ];
                            Ok(player.alloc_datum(Datum::Transform3d(m64)))
                        } else {
                            Ok(get_persistent_node_transform(player, member_ref, model_name))
                        }
                    }
                    "name" => {
                        // Return actual bone name from skeleton
                        let name = player.movie.cast_manager.find_member_by_ref(member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.parsed_scene.as_ref())
                            .and_then(|s| s.skeletons.first())
                            .and_then(|skel| skel.bones.get(bone_idx))
                            .map(|b| b.name.clone())
                            .unwrap_or_else(|| format!("bone_{}", bone_idx));
                        Ok(player.alloc_datum(Datum::String(name)))
                    }
                    _ => {
                        web_sys::console::log_1(&format!("[W3D] bone[{}].{} (stub)", bone_idx, prop_name).into());
                        Ok(player.alloc_datum(Datum::Void))
                    }
                }
            }
            "meshDeformMesh" => {
                // mesh[m].textureLayer — return persistent list from runtime state
                // name format is "modelName:meshIndex"
                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                let model_name = parts.get(0).unwrap_or(&"").to_string();
                let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                match prop_name {
                    "textureLayer" => {
                        // Get or create a persistent texture layer list DatumRef in runtime state
                        let member_ref_owned = member_ref.clone();

                        // Check if we already have a stored DatumRef for this mesh's textureLayer
                        let existing_ref = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref_owned);
                            member.and_then(|m| m.member_type.as_shockwave3d())
                                .and_then(|w3d| w3d.runtime_state.mesh_deform.get(&model_name))
                                .and_then(|md| md.meshes.get(mesh_idx))
                                .and_then(|mesh| mesh.texture_layer_datum_ref.clone())
                        };

                        if let Some(datum_ref) = existing_ref {
                            Ok(datum_ref)
                        } else {
                            // Create a new empty list and store the DatumRef
                            let list_ref = player.alloc_datum(Datum::List(
                                crate::director::lingo::datum::DatumType::List, VecDeque::new(), false,
                            ));
                            // Store it in runtime state
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref_owned) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    use crate::player::cast_member::{MeshDeformState, MeshDeformMesh};
                                    let md = w3d.runtime_state.mesh_deform
                                        .entry(model_name.clone())
                                        .or_insert_with(MeshDeformState::default);
                                    while md.meshes.len() <= mesh_idx {
                                        md.meshes.push(MeshDeformMesh::default());
                                    }
                                    md.meshes[mesh_idx].texture_layer_datum_ref = Some(list_ref.clone());
                                }
                            }
                            Ok(list_ref)
                        }
                    }
                    "face" => {
                        let face_count = scene.nodes.iter().find(|n| n.name == *model_name)
                            .and_then(|n| {
                                let rn = if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name };
                                scene.model_resources.get(rn.as_str())
                            })
                            .and_then(|res| res.mesh_infos.get(mesh_idx))
                            .map(|m| m.num_faces)
                            .unwrap_or(0);
                        // Return a PropList with #count
                        let count_key = player.alloc_datum(Datum::Symbol("count".to_string()));
                        let count_val = player.alloc_datum(Datum::Int(face_count as i32));
                        Ok(player.alloc_datum(Datum::PropList(VecDeque::from(vec![(count_key, count_val)]), false)))
                    }
                    "vertexList" => {
                        // Return a list of vertex vectors from clod_meshes or raw_meshes
                        let mut items = VecDeque::new();
                        let node = scene.nodes.iter().find(|n| n.name == *model_name);
                        let model_res_name = node.map(|n| n.model_resource_name.as_str()).unwrap_or("");
                        let res_name = node.map(|n| n.resource_name.as_str()).unwrap_or("");

                        // Try model_resource_name first, then resource_name for clod_meshes
                        let keys_to_try: Vec<&str> = [model_res_name, res_name].iter()
                            .filter(|k| !k.is_empty() && **k != ".")
                            .copied().collect();

                        for key in &keys_to_try {
                            if let Some(meshes) = scene.clod_meshes.get(*key) {
                                if let Some(mesh) = meshes.get(mesh_idx) {
                                    for pos in &mesh.positions {
                                        items.push_back(player.alloc_datum(Datum::Vector([pos[0] as f64, pos[1] as f64, pos[2] as f64])));
                                    }
                                }
                                if !items.is_empty() { break; }
                            }
                        }
                        // Fallback to raw_meshes with both keys
                        if items.is_empty() {
                            for key in &keys_to_try {
                                for raw in &scene.raw_meshes {
                                    if raw.name == *key && raw.chain_index as usize == mesh_idx {
                                        for pos in &raw.positions {
                                            items.push_back(player.alloc_datum(Datum::Vector([pos[0] as f64, pos[1] as f64, pos[2] as f64])));
                                        }
                                        break;
                                    }
                                }
                                if !items.is_empty() { break; }
                            }
                        }
                        Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, items, false,
                        )))
                    }
                    _ => Ok(player.alloc_datum(Datum::Void)),
                }
            }
            "meshDeformTexLayer" => {
                // textureLayer[n].textureCoordinateList — get from runtime state
                let parts: Vec<&str> = s3d_ref.name.splitn(3, ':').collect();
                let model_name = parts.get(0).unwrap_or(&"");
                let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let layer_idx: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                match prop_name {
                    "textureCoordinateList" => {
                        // Return the stored texture coordinates
                        let coords = {
                            let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                            member.and_then(|m| m.member_type.as_shockwave3d())
                                .and_then(|w3d| w3d.runtime_state.mesh_deform.get(*model_name))
                                .and_then(|md| md.meshes.get(mesh_idx))
                                .and_then(|mesh| mesh.texture_layers.get(layer_idx))
                                .map(|layer| layer.texture_coordinate_list.clone())
                                .unwrap_or_default()
                        };
                        // Convert to list of [u, v] lists
                        let mut items = VecDeque::new();
                        for uv in &coords {
                            let u = player.alloc_datum(Datum::Float(uv[0] as f64));
                            let v = player.alloc_datum(Datum::Float(uv[1] as f64));
                            items.push_back(player.alloc_datum(Datum::List(
                                crate::director::lingo::datum::DatumType::List, VecDeque::from(vec![u, v]), false,
                            )));
                        }
                        Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, items, false,
                        )))
                    }
                    _ => Ok(player.alloc_datum(Datum::Void)),
                }
            }
            _ => Err(ScriptError::new(format!("Unknown 3D object type '{}'", s3d_ref.object_type))),
        }
    }

    pub fn set_prop(obj_ref: &DatumRef, prop_name: &String, value: &Datum) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            let s3d_ref = match player.get_datum(obj_ref) {
                Datum::Shockwave3dObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected Shockwave3dObjectRef".to_string())),
            };

            let member_ref = crate::player::cast_lib::CastMemberRef {
                cast_lib: s3d_ref.cast_lib,
                cast_member: s3d_ref.cast_member,
            };

            match prop_name.as_str() {
                "transform" => {
                    if let Datum::Transform3d(m) = value {
                        let m32: [f32; 16] = m.map(|v| v as f32);
                        set_node_transform(player, &member_ref, &s3d_ref.name, m32);
                    }
                    Ok(())
                }
                "visible" => {
                    let visible = match value {
                        Datum::Int(v) => *v != 0,
                        _ => true,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.node_visibility.insert(s3d_ref.name.clone(), visible);
                        }
                    }
                    Ok(())
                }
                "visibility" => {
                    // #both, #front, #back, #none
                    let visible = match value {
                        Datum::Symbol(s) => s != "none",
                        _ => true,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.node_visibility.insert(s3d_ref.name.clone(), visible);
                        }
                    }
                    Ok(())
                }
                "shader" => {
                    let shader_name = value.string_value().unwrap_or_default();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let shader_map = w3d.runtime_state.node_shaders
                                .entry(s3d_ref.name.clone())
                                .or_insert_with(std::collections::HashMap::new);
                            shader_map.insert(0, shader_name);
                        }
                    }
                    Ok(())
                }
                "worldPosition" => {
                    if let Datum::Vector(v) = value {
                        let mut m = get_or_init_node_transform(player, &member_ref, &s3d_ref.name);
                        m[12] = v[0] as f32;
                        m[13] = v[1] as f32;
                        m[14] = v[2] as f32;
                        set_node_transform(player, &member_ref, &s3d_ref.name, m);
                    }
                    Ok(())
                }
                "playRate" => {
                    let rate = match value {
                        Datum::Float(f) => *f as f32,
                        Datum::Int(i) => *i as f32,
                        _ => 1.0,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.play_rate = rate;
                        }
                    }
                    Ok(())
                }
                "blendTime" => {
                    let ms = match value {
                        Datum::Float(f) => *f as f32,
                        Datum::Int(i) => *i as f32,
                        _ => 0.0,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.animation_blend_time = ms;
                        }
                    }
                    Ok(())
                }
                "currentTime" => {
                    let time = match value {
                        Datum::Int(i) => *i as f32 / 1000.0,
                        Datum::Float(f) => *f as f32 / 1000.0,
                        _ => 0.0,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.animation_time = time;
                        }
                    }
                    Ok(())
                }
                // Camera properties
                "fieldOfView" | "projectionAngle" => {
                    let fov = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 30.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == s3d_ref.name) {
                                    node.fov = fov;
                                }
                            }
                        }
                    }
                    Ok(())
                }
                "hither" | "nearClipPlane" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 1.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == s3d_ref.name) {
                                    node.near_plane = v;
                                }
                            }
                        }
                    }
                    Ok(())
                }
                "yon" | "farClipPlane" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 10000.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == s3d_ref.name) {
                                    node.far_plane = v;
                                }
                            }
                        }
                    }
                    Ok(())
                }
                "parent" => {
                    let is_detach = matches!(value, Datum::Void);
                    let parent_name = match value {
                        Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                        Datum::String(s) => s.clone(),
                        Datum::Void => String::new(), // VOID = detach from world
                        _ => "World".to_string(),
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            // Track detached nodes for renderer filtering
                            if is_detach {
                                w3d.runtime_state.detached_nodes.insert(s3d_ref.name.clone());
                            } else {
                                w3d.runtime_state.detached_nodes.remove(&s3d_ref.name);
                            }
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == s3d_ref.name) {
                                    node.parent_name = parent_name;
                                }
                            }
                        }
                    }
                    Ok(())
                }
                // Camera fog properties
                "fog.enabled" => {
                    let enabled = match value { Datum::Int(v) => *v != 0, _ => false };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.fog_enabled = enabled;
                        }
                    }
                    Ok(())
                }
                "fog.near" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 1.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.fog_near = v;
                        }
                    }
                    Ok(())
                }
                "fog.far" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 1000.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.fog_far = v;
                        }
                    }
                    Ok(())
                }
                // Shader texture properties
                "texture" | "textureList" => {
                    // Get texture name from the value (could be a Shockwave3dObjectRef or string)
                    let tex_name = match value {
                        Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                        Datum::String(s) => s.clone(),
                        _ => String::new(),
                    };
                    if !tex_name.is_empty() && s3d_ref.object_type == "shader" {
                        // Get persistent textureList ref if it exists (read before mutable borrow)
                        let list_ref = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                            member.and_then(|m| m.member_type.as_shockwave3d())
                                .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(&s3d_ref.name))
                                .cloned()
                        };
                        // Update persistent textureList first (prevents sync from overwriting)
                        if let Some(list_ref) = list_ref {
                            let new_val = player.alloc_datum(Datum::String(tex_name.clone()));
                            if let Datum::List(_, ref mut items, _) = player.get_datum_mut(&list_ref) {
                                if !items.is_empty() {
                                    items[0] = new_val;
                                }
                            }
                        }
                        // Update the shader's first texture layer in the parsed scene
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                        if shader.texture_layers.is_empty() {
                                            shader.texture_layers.push(crate::director::chunks::w3d::types::W3dTextureLayer {
                                                name: tex_name.clone(),
                                                ..Default::default()
                                            });
                                        } else {
                                            shader.texture_layers[0].name = tex_name.clone();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }
                // Shader material property setters
                "diffuse" | "ambient" | "emissive" | "specular" if s3d_ref.object_type == "shader" => {
                    debug!(
                        "[W3D-SET] shader(\"{}\").{}", s3d_ref.name, prop_name
                    );
                    let color = match value {
                        Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)) => {
                            [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0]
                        }
                        _ => [0.5, 0.5, 0.5, 1.0],
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                // Find the shader's material and update it
                                let mat_name = scene.shaders.iter()
                                    .find(|s| s.name == s3d_ref.name)
                                    .map(|s| s.material_name.clone())
                                    .unwrap_or_default();
                                // Find material by: 1) material_name, 2) shader name, 3) create with shader name
                                let mat = if !mat_name.is_empty() {
                                    if let Some(m) = scene.materials.iter_mut().find(|m| m.name == mat_name) {
                                        Some(m)
                                    } else { None }
                                } else { None };
                                let mat = if let Some(m) = mat { m }
                                else if let Some(m) = scene.materials.iter_mut().find(|m| m.name == s3d_ref.name) { m }
                                else {
                                    // Create material with the SHADER name so the renderer can find it
                                    let shader_name = s3d_ref.name.clone();
                                    scene.materials.push(crate::director::chunks::w3d::types::W3dMaterial {
                                        name: shader_name.clone(),
                                        ..Default::default()
                                    });
                                    if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == shader_name) {
                                        shader.material_name = shader_name;
                                    }
                                    scene.materials.last_mut().unwrap()
                                };
                                match prop_name.as_str() {
                                    "diffuse" => mat.diffuse = color,
                                    "ambient" => mat.ambient = color,
                                    "emissive" => mat.emissive = color,
                                    "specular" => mat.specular = color,
                                    _ => {}
                                }
                            }
                        }
                    }
                    Ok(())
                }
                "blend" if s3d_ref.object_type == "shader" => {
                    // blend = 0-100 → opacity 0.0-1.0
                    let blend_val = value.to_float().unwrap_or(100.0) as f32;
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                // Find the shader and get/create its material
                                let mat_name_to_update = {
                                    let shader = scene.shaders.iter_mut()
                                        .find(|s| s.name == s3d_ref.name);
                                    if let Some(shader) = shader {
                                        if shader.material_name.is_empty() {
                                            // Shader has no material — create one and link it
                                            let new_mat_name = format!("{}_mat", s3d_ref.name);
                                            shader.material_name = new_mat_name.clone();
                                            Some((new_mat_name, true))
                                        } else {
                                            Some((shader.material_name.clone(), false))
                                        }
                                    } else { None }
                                };
                                if let Some((mat_name, needs_create)) = mat_name_to_update {
                                    if needs_create {
                                        use crate::director::chunks::w3d::types::W3dMaterial;
                                        scene.materials.push(W3dMaterial {
                                            name: mat_name,
                                            opacity: blend_val / 100.0,
                                            ..Default::default()
                                        });
                                    } else {
                                        if let Some(mat) = scene.materials.iter_mut().find(|m| m.name == mat_name) {
                                            mat.opacity = blend_val / 100.0;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }
                "transparent" if s3d_ref.object_type == "shader" => {
                    // transparent = 1 means use alpha blending
                    // Just store in the material opacity if needed
                    Ok(())
                }
                "shininess" | "flat" | "renderStyle"
                | "useDiffuseWithTexture" if s3d_ref.object_type == "shader" => {
                    // Accept these shader properties silently
                    Ok(())
                }
                "rootNode" if s3d_ref.object_type == "camera" => {
                    let cam_name = s3d_ref.name.clone();
                    let root_name = match value {
                        Datum::Shockwave3dObjectRef(r) => Some(r.name.clone()),
                        Datum::Void => None,
                        _ => None,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(name) = root_name {
                                w3d.runtime_state.camera_root_nodes.insert(cam_name, name);
                            } else {
                                w3d.runtime_state.camera_root_nodes.remove(&cam_name);
                            }
                        }
                    }
                    Ok(())
                }
                "clearAtRender" if s3d_ref.object_type == "colorBuffer" => {
                    let cam_name = s3d_ref.name.clone();
                    let val = match value {
                        Datum::Int(v) => *v != 0,
                        _ => true,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.camera_clear_at_render.insert(cam_name, val);
                        }
                    }
                    Ok(())
                }
                _ if s3d_ref.object_type == "overlay" || s3d_ref.object_type == "backdrop" => {
                    // Set overlay/backdrop properties: source, loc, blend, scale, regPoint, rotation
                    let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                    let cam_name = parts.get(0).unwrap_or(&"").to_string();
                    let ov_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let is_overlay = s3d_ref.object_type == "overlay";

                    // Pre-extract values that need player borrows (for Point datums)
                    let (loc_vals, reg_vals) = match prop_name.as_str() {
                        "loc" => {
                            if let Datum::Point(p) = value {
                                let x = player.get_datum(&p[0]).to_float().unwrap_or(0.0);
                                let y = player.get_datum(&p[1]).to_float().unwrap_or(0.0);
                                (Some([x, y]), None)
                            } else { (None, None) }
                        }
                        "regPoint" => {
                            if let Datum::Point(p) = value {
                                let x = player.get_datum(&p[0]).to_float().unwrap_or(0.0);
                                let y = player.get_datum(&p[1]).to_float().unwrap_or(0.0);
                                (None, Some([x, y]))
                            } else { (None, None) }
                        }
                        _ => (None, None),
                    };

                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let map = if is_overlay { &mut w3d.runtime_state.camera_overlays } else { &mut w3d.runtime_state.camera_backdrops };
                            if let Some(list) = map.get_mut(&cam_name) {
                                if let Some(ov) = list.get_mut(ov_idx) {
                                    match prop_name.as_str() {
                                        "source" => {
                                            ov.source_texture = match value {
                                                Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                                                Datum::String(s) => s.clone(),
                                                _ => String::new(),
                                            };
                                            ov.source_texture_lower = ov.source_texture.to_lowercase();
                                        }
                                        "loc" => { if let Some(v) = loc_vals { ov.loc = v; } }
                                        "blend" => ov.blend = value.to_float().unwrap_or(100.0),
                                        "scale" => ov.scale = value.to_float().unwrap_or(1.0),
                                        "rotation" => ov.rotation = value.to_float().unwrap_or(0.0),
                                        "regPoint" => { if let Some(v) = reg_vals { ov.reg_point = v; } }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }
                _ if s3d_ref.object_type == "sds" => {
                    // Subdivision Surface modifier set properties
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let sds = w3d.runtime_state.sds_state
                                .entry(s3d_ref.name.clone())
                                .or_insert_with(crate::player::cast_member::SdsState::default);
                            match prop_name.as_str() {
                                "depth" => sds.depth = value.int_value().unwrap_or(1) as i32,
                                "tension" => sds.tension = value.to_float().unwrap_or(0.0) as f32,
                                "error" => sds.error = value.to_float().unwrap_or(0.0) as f32,
                                "enabled" => sds.enabled = value.int_value().unwrap_or(1) != 0,
                                _ => {}
                            }
                        }
                    }
                    Ok(())
                }
                "image" if s3d_ref.object_type == "texture" => {
                    // texture("name").image = bitmapObject
                    // Convert bitmap to RGBA and store in scene.texture_images
                    let bitmap_ref = match value {
                        Datum::BitmapRef(r) => Some(*r),
                        _ => None,
                    };
                    if let Some(bmp_ref) = bitmap_ref {
                        let rgba_data = if let Some(bmp) = player.bitmap_manager.get_bitmap(bmp_ref) {
                            let w = bmp.width;
                            let h = bmp.height;
                            let palettes = player.movie.cast_manager.palettes();
                            let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
                            for y in 0..h as usize {
                                for x in 0..w as usize {
                                    let (r, g, b, a) = bmp.get_pixel_color_with_alpha(&palettes, x as u16, y as u16);
                                    let idx = (y * w as usize + x) * 4;
                                    rgba[idx] = r;
                                    rgba[idx + 1] = g;
                                    rgba[idx + 2] = b;
                                    rgba[idx + 3] = a;
                                }
                            }
                            Some((w, h, rgba))
                        } else {
                            None
                        };
                        if let Some((w, h, mut rgba)) = rgba_data {
                            // When use_alpha bitmap has white opaque pixels (255,255,255,255),
                            // make them transparent. This handles the case where setAlpha(0)
                            // set background transparent but copyPixels overwrote alpha to 255.
                            // White background pixels should remain transparent for overlay compositing.
                            if let Some(bmp) = player.bitmap_manager.get_bitmap(bmp_ref) {
                                if bmp.use_alpha {
                                    let total = (w as usize) * (h as usize);
                                    for i in 0..total {
                                        let idx = i * 4;
                                        if rgba[idx] == 255 && rgba[idx+1] == 255 && rgba[idx+2] == 255 && rgba[idx+3] == 255 {
                                            rgba[idx+3] = 0; // Make white background transparent
                                        }
                                    }
                                }
                            }
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        let mut tex_data = Vec::with_capacity(8 + rgba.len());
                                        tex_data.extend_from_slice(&(w as u32).to_le_bytes());
                                        tex_data.extend_from_slice(&(h as u32).to_le_bytes());
                                        tex_data.extend_from_slice(&rgba);
                                        scene.texture_images.insert(s3d_ref.name.clone(), tex_data);
                                        scene.texture_content_version += 1;
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                }
                _ => {
                    // Handle meshDeformTexLayer.textureCoordinateList = data
                    if s3d_ref.object_type == "emitter" {
                        use crate::player::cast_member::EmitterState;
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                let em = w3d.runtime_state.emitters
                                    .entry(s3d_ref.name.clone())
                                    .or_insert_with(EmitterState::default);
                                match prop_name.as_str() {
                                    "loop" => em.is_loop = value.int_value().unwrap_or(1) != 0,
                                    "mode" => em.mode = value.symbol_value().unwrap_or_else(|_| value.string_value().unwrap_or_default()),
                                    "numParticles" => em.num_particles = value.int_value().unwrap_or(100),
                                    "direction" => if let Datum::Vector(v) = value { em.direction = *v; },
                                    "region" => if let Datum::Vector(v) = value { em.region = *v; },
                                    "distribution" => em.distribution = value.symbol_value().unwrap_or_else(|_| value.string_value().unwrap_or_default()),
                                    "angle" => em.angle = value.float_value().unwrap_or(30.0),
                                    "minSpeed" => em.min_speed = value.float_value().unwrap_or(1.0),
                                    "maxSpeed" => em.max_speed = value.float_value().unwrap_or(1.0),
                                    "pathStrength" => em.path_strength = value.float_value().unwrap_or(0.0),
                                    _ => {}
                                }
                            }
                        }
                        return Ok(());
                    }
                    if s3d_ref.object_type == "meshDeformTexLayer" && prop_name == "textureCoordinateList" {
                        let parts: Vec<&str> = s3d_ref.name.splitn(3, ':').collect();
                        let model_name = parts.get(0).unwrap_or(&"").to_string();
                        let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let layer_idx: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

                        // Parse UV coordinates from the value (list of [u, v] pairs)
                        let coords: Vec<[f32; 2]> = if let Datum::List(_, items, _) = value {
                            items.iter().map(|item_ref| {
                                let item = player.get_datum(item_ref);
                                if let Datum::List(_, uv_items, _) = item {
                                    if uv_items.len() >= 2 {
                                        let u = player.get_datum(&uv_items[0]).float_value().unwrap_or(0.0) as f32;
                                        let v = player.get_datum(&uv_items[1]).float_value().unwrap_or(0.0) as f32;
                                        [u, v]
                                    } else { [0.0, 0.0] }
                                } else { [0.0, 0.0] }
                            }).collect()
                        } else {
                            vec![]
                        };

                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                use crate::player::cast_member::{MeshDeformState, MeshDeformMesh, MeshDeformTextureLayer};
                                let md = w3d.runtime_state.mesh_deform
                                    .entry(model_name)
                                    .or_insert_with(|| MeshDeformState::default());
                                // Auto-extend meshes
                                while md.meshes.len() <= mesh_idx {
                                    md.meshes.push(MeshDeformMesh::default());
                                }
                                // Auto-extend texture layers
                                while md.meshes[mesh_idx].texture_layers.len() <= layer_idx {
                                    md.meshes[mesh_idx].texture_layers.push(MeshDeformTextureLayer::default());
                                }
                                md.meshes[mesh_idx].texture_layers[layer_idx].texture_coordinate_list = coords;
                            }
                        }
                        return Ok(());
                    }
                    Ok(())
                }
            }
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let s3d_ref = match player.get_datum(datum) {
                Datum::Shockwave3dObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected Shockwave3dObjectRef".to_string())),
            };

            let member_ref = crate::player::cast_lib::CastMemberRef {
                cast_lib: s3d_ref.cast_lib,
                cast_member: s3d_ref.cast_member,
            };

            match handler_name.as_str() {
                // ─── Node transform methods ───
                "translate" => {
                    let (dx, dy, dz) = read_xyz_args(player, args);
                    apply_translation(player, &member_ref, &s3d_ref.name, dx, dy, dz);
                    Ok(player.alloc_datum(Datum::Void))
                }
                "rotate" => {
                    let (rx, ry, rz) = read_xyz_args(player, args);
                    apply_rotation(player, &member_ref, &s3d_ref.name, rx, ry, rz);
                    Ok(player.alloc_datum(Datum::Void))
                }
                "scale" => {
                    let (sx, sy, sz) = read_xyz_args(player, args);
                    apply_scale(player, &member_ref, &s3d_ref.name, sx, sy, sz);
                    Ok(player.alloc_datum(Datum::Void))
                }
                "pointAt" => {
                    if !args.is_empty() {
                        if let Datum::Vector(target) = player.get_datum(&args[0]) {
                            let target = *target;
                            let (ux, uy, uz) = if args.len() > 1 {
                                if let Datum::Vector(up) = player.get_datum(&args[1]) {
                                    (up[0] as f32, up[1] as f32, up[2] as f32)
                                } else { (0.0f32, 1.0, 0.0) }
                            } else { (0.0f32, 1.0, 0.0) };
                            apply_point_at(player, &member_ref, &s3d_ref.name,
                                target[0] as f32, target[1] as f32, target[2] as f32,
                                ux, uy, uz);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }

                "getWorldTransform" => {
                    // Return world-relative transform (accumulated through parent chain)
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                    let world_t = if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                if let Some(node) = scene.nodes.iter().find(|n| n.name == s3d_ref.name) {
                                    // Get local transform (runtime override or static)
                                    let local = w3d.runtime_state.node_transforms.get(&s3d_ref.name)
                                        .copied().unwrap_or(node.transform);
                                    // Walk parent chain
                                    let mut result = local;
                                    let mut current_parent = &node.parent_name;
                                    for _ in 0..20 {
                                        if current_parent.is_empty() || current_parent == "World" { break; }
                                        if let Some(pn) = scene.nodes.iter().find(|n| n.name == *current_parent) {
                                            let pt = w3d.runtime_state.node_transforms.get(&pn.name)
                                                .copied().unwrap_or(pn.transform);
                                            result = mat4_mul_f32(&pt, &result);
                                            current_parent = &pn.parent_name;
                                        } else { break; }
                                    }
                                    result
                                } else {
                                    get_node_transform(player, &member_ref, &s3d_ref.name)
                                }
                            } else {
                                get_node_transform(player, &member_ref, &s3d_ref.name)
                            }
                        } else {
                            get_node_transform(player, &member_ref, &s3d_ref.name)
                        }
                    } else {
                        get_node_transform(player, &member_ref, &s3d_ref.name)
                    };
                    Ok(player.alloc_datum(Datum::Transform3d(world_t.map(|v| v as f64))))
                }

                // ─── Bones player / animation methods ───
                // play(motionName {, looped, startTime, endTime, scale, offset})
                // play() with no args = resume paused motion
                "play" => {
                    // Pre-read all args before mutable borrow of player
                    let play_args = if args.is_empty() {
                        None
                    } else {
                        let motion_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        let is_loop = args.get(1).map(|a| player.get_datum(a).int_value().unwrap_or(0) != 0).unwrap_or(false);
                        let start_time_ms = args.get(2).map(|a| player.get_datum(a).to_float().unwrap_or(0.0)).unwrap_or(0.0);
                        let end_time_ms = args.get(3).map(|a| player.get_datum(a).to_float().unwrap_or(-1.0)).unwrap_or(-1.0);
                        let scale = args.get(4).map(|a| player.get_datum(a).to_float().unwrap_or(1.0)).unwrap_or(1.0);
                        let offset_ms = args.get(5).map(|a| {
                            let d = player.get_datum(a);
                            match d {
                                Datum::Symbol(s) if s == "synchronized" => -1.0f64,
                                _ => d.to_float().unwrap_or(0.0),
                            }
                        }).unwrap_or(0.0);
                        Some((motion_name, is_loop, start_time_ms, end_time_ms, scale, offset_ms))
                    };

                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some((motion_name, is_loop, start_time_ms, end_time_ms, scale, offset_ms)) = play_args {
                                // Set up crossfade blending using stored blendTime
                                let blend_time = w3d.runtime_state.animation_blend_time;
                                if blend_time > 0.0 && w3d.runtime_state.current_motion.is_some() {
                                    w3d.runtime_state.previous_motion = w3d.runtime_state.current_motion.clone();
                                    w3d.runtime_state.blend_duration = blend_time / 1000.0;
                                    w3d.runtime_state.blend_elapsed = 0.0;
                                    w3d.runtime_state.blend_weight = 0.0;
                                } else {
                                    w3d.runtime_state.previous_motion = None;
                                    w3d.runtime_state.blend_weight = 1.0;
                                }

                                w3d.runtime_state.current_motion = Some(motion_name);
                                w3d.runtime_state.animation_playing = true;
                                w3d.runtime_state.animation_loop = is_loop;
                                w3d.runtime_state.animation_start_time = start_time_ms as f32 / 1000.0;
                                w3d.runtime_state.animation_end_time = end_time_ms as f32 / 1000.0;
                                w3d.runtime_state.animation_scale = scale as f32;
                                w3d.runtime_state.motion_ended = false;

                                // Determine initial animation time from offset
                                if offset_ms >= 0.0 {
                                    w3d.runtime_state.animation_time = offset_ms as f32 / 1000.0;
                                }
                                // else: #synchronized — keep current relative position
                            } else {
                                // No args: resume paused animation
                                w3d.runtime_state.animation_playing = true;
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "playNext" | "queue" => {
                    if !args.is_empty() {
                        let motion_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                w3d.runtime_state.motion_queue.push(motion_name);
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "pause" => {
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.animation_playing = false;
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "resume" => {
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.animation_playing = true;
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "stop" => {
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.animation_playing = false;
                            w3d.runtime_state.animation_time = 0.0;
                            w3d.runtime_state.current_motion = None;
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }

                // ─── Scene management ───
                "clone" | "cloneDeep" => {
                    // Return a new model ref with the cloned name and add node to scene
                    let clone_name = if !args.is_empty() {
                        player.get_datum(&args[0]).string_value().unwrap_or_default()
                    } else {
                        format!("{}_clone", s3d_ref.name)
                    };
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                let source_node = scene.nodes.iter().find(|n| n.name == s3d_ref.name).cloned();
                                if let Some(mut new_node) = source_node {
                                    // Per Director docs: clone shares the same parent as original
                                    // If name is empty, clone has no parent (temporary instance)
                                    if clone_name.is_empty() {
                                        new_node.parent_name = String::new();
                                    }
                                    // Otherwise keep original parent_name (already copied from source)
                                    new_node.name = clone_name.clone();
                                    scene.nodes.push(new_node);
                                } else {
                                    use crate::director::chunks::w3d::types::*;
                                    scene.nodes.push(W3dNode {
                                        name: clone_name.clone(), node_type: W3dNodeType::Model,
                                        parent_name: "World".to_string(),
                                        resource_name: String::new(), model_resource_name: String::new(),
                                        shader_name: String::new(),
                                        near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                        screen_width: 640, screen_height: 480,
                                        transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
                                    });
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(crate::director::lingo::datum::Shockwave3dObjectRef {
                        cast_lib: s3d_ref.cast_lib,
                        cast_member: s3d_ref.cast_member,
                        object_type: s3d_ref.object_type.clone(),
                        name: clone_name,
                    })))
                }
                "addChild" => Ok(player.alloc_datum(Datum::Void)),
                "addToWorld" => {
                    // Set model's parent to World and remove from detached nodes
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.detached_nodes.remove(&s3d_ref.name);
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == s3d_ref.name) {
                                    if node.parent_name.is_empty() {
                                        node.parent_name = "World".to_string();
                                    }
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "removeFromWorld" => {
                    // Detach model from world
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.detached_nodes.insert(s3d_ref.name.clone());
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == s3d_ref.name) {
                                    node.parent_name = String::new();
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "isInWorld" => {
                    // Check if node is in the world (not detached)
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                    let in_world = if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if w3d.runtime_state.detached_nodes.contains(&s3d_ref.name) {
                                false
                            } else if let Some(ref scene) = w3d.parsed_scene {
                                scene.nodes.iter().any(|n| n.name == s3d_ref.name && !n.parent_name.is_empty())
                            } else { false }
                        } else { false }
                    } else { false };
                    Ok(player.alloc_datum(Datum::Int(if in_world { 1 } else { 0 })))
                }
                "addModifier" => {
                    // Initialize meshDeform state when #meshDeform modifier is added
                    if !args.is_empty() {
                        let mod_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        if mod_name == "meshDeform" {
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let mesh_count = {
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.parsed_scene.as_ref())
                                    .map(|scene| {
                                        let node = scene.nodes.iter().find(|n| n.name == s3d_ref.name);
                                        let res_name = node.map(|n| if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name });
                                        res_name.and_then(|rn| scene.model_resources.get(rn.as_str()))
                                            .map(|res| res.mesh_infos.len())
                                            .unwrap_or(1)
                                    })
                                    .unwrap_or(1)
                            };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    use crate::player::cast_member::{MeshDeformState, MeshDeformMesh};
                                    let state = MeshDeformState {
                                        meshes: (0..mesh_count).map(|_| MeshDeformMesh::default()).collect(),
                                    };
                                    w3d.runtime_state.mesh_deform.insert(s3d_ref.name.clone(), state);
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "removeModifier" => Ok(player.alloc_datum(Datum::Void)),
                "registerScript" => Ok(player.alloc_datum(Datum::Void)),
                "isInWorld" => Ok(player.alloc_datum(Datum::Int(1))),

                // ─── Camera methods ───
                "modelUnderLoc" => {
                    if !args.is_empty() {
                        // Get screen point from argument
                        let (sx, sy) = match player.get_datum(&args[0]) {
                            Datum::Point(refs) => {
                                let x = player.get_datum(&refs[0]).int_value().unwrap_or(0) as f32;
                                let y = player.get_datum(&refs[1]).int_value().unwrap_or(0) as f32;
                                (x, y)
                            }
                            _ => (0.0, 0.0),
                        };

                        // Get scene for ray casting
                        let scene = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                                .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                            let w3d = member.member_type.as_shockwave3d()
                                .ok_or_else(|| ScriptError::new("Not 3D".to_string()))?;
                            w3d.parsed_scene.clone()
                        };

                        if let Some(scene) = scene {
                            use crate::director::chunks::w3d::raycast;

                            // Build view/projection from scene
                            let view_node = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View);
                            let (fov, near, far) = if let Some(n) = view_node {
                                (n.fov.to_radians(), n.near_plane, n.far_plane)
                            } else {
                                (30.0f32.to_radians(), 1.0, 10000.0)
                            };

                            let width = 320.0f32; // default viewport
                            let height = 240.0f32;
                            let aspect = width / height;

                            // Simple perspective + view matrices (column-major)
                            let f = 1.0 / (fov / 2.0).tan();
                            let nf = 1.0 / (near - far);
                            let proj = [
                                f/aspect, 0.0, 0.0, 0.0,
                                0.0, f, 0.0, 0.0,
                                0.0, 0.0, (far+near)*nf, -1.0,
                                0.0, 0.0, 2.0*far*near*nf, 0.0,
                            ];

                            let view = if let Some(n) = view_node {
                                // Simplified inverse of view node transform
                                let t = &n.transform;
                                let tx = t[12]; let ty = t[13]; let tz = t[14];
                                [
                                    t[0], t[4], t[8], 0.0,
                                    t[1], t[5], t[9], 0.0,
                                    t[2], t[6], t[10], 0.0,
                                    -(t[0]*tx+t[1]*ty+t[2]*tz), -(t[4]*tx+t[5]*ty+t[6]*tz), -(t[8]*tx+t[9]*ty+t[10]*tz), 1.0,
                                ]
                            } else {
                                [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,-500.0,1.0]
                            };

                            let ray = raycast::screen_to_ray(sx, sy, width, height, &view, &proj);
                            if let Some(hit) = raycast::raycast_scene(&ray, &scene, 100000.0) {
                                // Return the model name as a Shockwave3dObjectRef
                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: s3d_ref.cast_lib,
                                    cast_member: s3d_ref.cast_member,
                                    object_type: "model".to_string(),
                                    name: hit.model_name,
                                })));
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "modelsUnderLoc" => {
                    // Same as modelUnderLoc but returns a list (for now, just return 0-1 results)
                    Ok(player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List, VecDeque::new(), false,
                    )))
                }
                "screenToWorld" => {
                    if !args.is_empty() {
                        let (sx, sy) = match player.get_datum(&args[0]) {
                            Datum::Point(refs) => {
                                let x = player.get_datum(&refs[0]).int_value().unwrap_or(0) as f32;
                                let y = player.get_datum(&refs[1]).int_value().unwrap_or(0) as f32;
                                (x, y)
                            }
                            _ => (0.0, 0.0),
                        };

                        // Simplified: return ray origin and direction
                        let pos = player.alloc_datum(Datum::Vector([sx as f64, sy as f64, 0.0]));
                        let dir = player.alloc_datum(Datum::Vector([0.0, 0.0, -1.0]));
                        Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, VecDeque::from(vec![pos, dir]), false,
                        )))
                    } else {
                        Ok(player.alloc_datum(Datum::Void))
                    }
                }
                "worldToScreen" => {
                    // Project 3D world point to 2D screen coords via view-projection matrix
                    let world_pt = if !args.is_empty() {
                        match player.get_datum(&args[0]) {
                            Datum::Vector(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
                            _ => [0.0, 0.0, 0.0],
                        }
                    } else { [0.0, 0.0, 0.0] };

                    // Get camera view and projection from scene
                    let (sx, sy) = if let Some(member) = player.movie.cast_manager.find_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d() {
                            if let Some(ref _scene) = w3d.parsed_scene {
                                // Use the scene3d renderer to do the projection
                                // For now, use the camera transform from runtime state
                                let cam_t = w3d.runtime_state.node_transforms.get("DefaultView")
                                    .or_else(|| w3d.runtime_state.node_transforms.get("defaultview"));
                                if let Some(cam_t) = cam_t {
                                    // Simple projection: transform to camera space
                                    let vx = world_pt[0] - cam_t[12];
                                    let vy = world_pt[1] - cam_t[13];
                                    let vz = world_pt[2] - cam_t[14];
                                    // Perspective divide (simplified)
                                    let depth = (cam_t[8]*vx + cam_t[9]*vy + cam_t[10]*vz).abs().max(0.01);
                                    let sx = (cam_t[0]*vx + cam_t[1]*vy + cam_t[2]*vz) / depth;
                                    let sy = (cam_t[4]*vx + cam_t[5]*vy + cam_t[6]*vz) / depth;
                                    // NDC to screen (assume 320x240 center)
                                    (160.0 + sx * 160.0, 120.0 - sy * 120.0)
                                } else {
                                    (0.0, 0.0)
                                }
                            } else { (0.0, 0.0) }
                        } else { (0.0, 0.0) }
                    } else { (0.0, 0.0) };

                    let x = player.alloc_datum(Datum::Float(sx as f64));
                    let y = player.alloc_datum(Datum::Float(sy as f64));
                    Ok(player.alloc_datum(Datum::Point([x, y])))
                }
                "renderDirect" | "renderToTexture" => {
                    // camera.renderDirect(texture) / camera.renderToTexture(texture)
                    let target_tex_name = if !args.is_empty() {
                        match player.get_datum(&args[0]) {
                            Datum::Shockwave3dObjectRef(r) if r.object_type == "texture" => r.name.clone(),
                            Datum::String(s) => s.clone(),
                            _ => String::new(),
                        }
                    } else { String::new() };

                    if !target_tex_name.is_empty() {
                        let cam_name = s3d_ref.name.clone();
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                w3d.runtime_state.render_targets.insert(cam_name.clone(), target_tex_name.clone());
                                web_sys::console::log_1(&format!(
                                    "[W3D] camera(\"{}\").renderDirect(\"{}\") — render target set",
                                    cam_name, target_tex_name
                                ).into());
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "addOverlay" | "addBackdrop" => {
                    // addOverlay(texture, point, rotation)
                    let is_overlay = handler_name == "addOverlay";
                    let tex_name = if !args.is_empty() {
                        match player.get_datum(&args[0]) {
                            Datum::Shockwave3dObjectRef(r) if r.object_type == "texture" => r.name.clone(),
                            Datum::String(s) => s.clone(),
                            _ => String::new(),
                        }
                    } else { String::new() };
                    let loc = if args.len() > 1 {
                        match player.get_datum(&args[1]) {
                            Datum::Point(p) => {
                                let x = player.get_datum(&p[0]).to_float().unwrap_or(0.0);
                                let y = player.get_datum(&p[1]).to_float().unwrap_or(0.0);
                                [x, y]
                            }
                            _ => [0.0, 0.0],
                        }
                    } else { [0.0, 0.0] };
                    let rotation = if args.len() > 2 {
                        player.get_datum(&args[2]).to_float().unwrap_or(0.0)
                    } else { 0.0 };

                    let camera_name = s3d_ref.name.clone();
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };

                    // Find next OverlayShader-copyN number and create shader + overlay
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let shader_name = if let Some(scene) = w3d.scene_mut() {
                                let prefix = "OverlayShader-copy";
                                let max_n = scene.shaders.iter()
                                    .filter_map(|s| {
                                        if s.name.starts_with(prefix) {
                                            s.name[prefix.len()..].parse::<u32>().ok()
                                        } else { None }
                                    })
                                    .max().unwrap_or(0);
                                let shader_name = format!("{}{}", prefix, max_n + 1);
                                scene.shaders.push(crate::director::chunks::w3d::types::W3dShader {
                                    name: shader_name.clone(),
                                    ..Default::default()
                                });
                                shader_name
                            } else { String::new() };

                            let overlay = crate::player::cast_member::CameraOverlay {
                                source_texture_lower: tex_name.to_lowercase(),
                                source_texture: tex_name,
                                loc,
                                rotation,
                                shader_name,
                                ..Default::default()
                            };
                            let list = if is_overlay {
                                w3d.runtime_state.camera_overlays.entry(camera_name).or_insert_with(Vec::new)
                            } else {
                                w3d.runtime_state.camera_backdrops.entry(camera_name).or_insert_with(Vec::new)
                            };
                            list.push(overlay);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "removeOverlay" | "removeBackdrop" => {
                    // removeOverlay(index) — 1-based
                    let is_overlay = handler_name == "removeOverlay";
                    let index = if !args.is_empty() {
                        player.get_datum(&args[0]).int_value().unwrap_or(1) as usize
                    } else { 1 };
                    let camera_name = s3d_ref.name.clone();
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };

                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let list = if is_overlay {
                                w3d.runtime_state.camera_overlays.get_mut(&camera_name)
                            } else {
                                w3d.runtime_state.camera_backdrops.get_mut(&camera_name)
                            };
                            if let Some(list) = list {
                                let idx = index.saturating_sub(1);
                                if idx < list.len() {
                                    let removed = list.remove(idx);
                                    // Remove associated shader
                                    if let Some(scene) = w3d.scene_mut() {
                                        scene.shaders.retain(|s| s.name != removed.shader_name);
                                    }
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }

                // setProp/getProp/setAt/getAt — property access via method call
                "setProp" | "setaProp" => {
                    // setProp(#propName, value) or setProp(#propName, index, value)
                    if args.len() == 3 {
                        // Indexed set: setProp(#propName, index, value)
                        let prop = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        let index = player.get_datum(&args[1]).int_value()?;
                        let value_ref = args[2].clone();
                        let value_datum = player.get_datum(&value_ref).clone();

                        // For shaderList assignment, update node_shaders for the renderer
                        if (prop == "shaderList" || prop == "shader") && s3d_ref.object_type == "model" {
                            if let Datum::Shockwave3dObjectRef(shader_ref) = &value_datum {
                                let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                        let mesh_idx = if index > 0 { (index - 1) as usize } else { 0 };
                                        let shader_map = w3d.runtime_state.node_shaders
                                            .entry(s3d_ref.name.clone())
                                            .or_insert_with(std::collections::HashMap::new);
                                        shader_map.insert(mesh_idx, shader_ref.name.clone());
                                    }
                                }
                            }
                        }

                        // Also update the transient list
                        let list_ref = Self::get_prop(datum, &prop)?;
                        let list_datum = player.get_datum(&list_ref);
                        if let Datum::List(_, items, _) = list_datum {
                            let idx = (index as usize).saturating_sub(1); // 1-based to 0-based
                            // Auto-extend list if needed
                            let needs_extend = idx >= items.len();
                            if needs_extend {
                                let current_len = items.len();
                                let mut new_items = VecDeque::new();
                                for _ in current_len..idx {
                                    new_items.push_back(player.alloc_datum(Datum::Void));
                                }
                                new_items.push_back(value_ref);
                                if let Datum::List(_, ref mut items, _) = player.get_datum_mut(&list_ref) {
                                    items.extend(new_items);
                                }
                            } else {
                                if let Datum::List(_, ref mut items, _) = player.get_datum_mut(&list_ref) {
                                    items[idx] = value_ref;
                                }
                            }
                        }
                    } else if args.len() >= 2 {
                        let prop = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        let value = player.get_datum(&args[args.len() - 1]).clone();
                        Self::set_prop(datum, &prop, &value)?;
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "getaProp" => {
                    if !args.is_empty() {
                        let prop = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        Self::get_prop(datum, &prop)
                    } else {
                        Ok(player.alloc_datum(Datum::Void))
                    }
                }
                "setAt" | "setProp" => {
                    // setProp(#shaderList, I, shaderRef) — update model's shader at index
                    // setAt(I, value) — set a property by index
                    if args.len() >= 3 {
                        // setProp(#prop, index, value) pattern
                        let prop_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        let index = player.get_datum(&args[1]).int_value().unwrap_or(1);
                        let value = player.get_datum(&args[2]).clone();
                        if prop_name == "shaderList" || prop_name == "shader" {
                            if let Datum::Shockwave3dObjectRef(shader_ref) = &value {
                                let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                        // Store per-mesh shader override (1-based Lingo index → 0-based)
                                        let mesh_idx = if index > 0 { (index - 1) as usize } else { 0 };
                                        let shader_map = w3d.runtime_state.node_shaders
                                            .entry(s3d_ref.name.clone())
                                            .or_insert_with(std::collections::HashMap::new);
                                        shader_map.insert(mesh_idx, shader_ref.name.clone());
                                    }
                                }
                            }
                        } else if prop_name == "blendFunctionList" {
                            // Set blend function for a texture layer
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let blend_val = match &value {
                                Datum::Symbol(s) => match s.as_str() {
                                    "add" => 1u8,
                                    "replace" => 2,
                                    "blend" => 3,
                                    _ => 0, // multiply
                                },
                                _ => 0,
                            };
                            let idx = (index as usize).saturating_sub(1);
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                            while shader.texture_layers.len() <= idx {
                                                shader.texture_layers.push(crate::director::chunks::w3d::types::W3dTextureLayer::default());
                                            }
                                            shader.texture_layers[idx].blend_func = blend_val;
                                        }
                                    }
                                }
                            }
                        } else if prop_name == "blendSourceList" {
                            // Set blend source for a texture layer (#alpha or #constant)
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let src_val = match &value {
                                Datum::Symbol(s) if s == "alpha" => 1u8,
                                _ => 0, // constant
                            };
                            let idx = (index as usize).saturating_sub(1);
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                            while shader.texture_layers.len() <= idx {
                                                shader.texture_layers.push(crate::director::chunks::w3d::types::W3dTextureLayer::default());
                                            }
                                            shader.texture_layers[idx].blend_src = src_val;
                                        }
                                    }
                                }
                            }
                        } else if prop_name == "blendConstantList" {
                            // Set blend constant for a texture layer (0-100)
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let const_val = match &value {
                                Datum::Float(f) => (*f as f32) / 100.0,
                                Datum::Int(i) => (*i as f32) / 100.0,
                                _ => 0.5,
                            };
                            let idx = (index as usize).saturating_sub(1);
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                            while shader.texture_layers.len() <= idx {
                                                shader.texture_layers.push(crate::director::chunks::w3d::types::W3dTextureLayer::default());
                                            }
                                            shader.texture_layers[idx].blend_const = const_val;
                                        }
                                    }
                                }
                            }
                        } else if prop_name == "textureList" {
                            // Update the persistent textureList at the given index
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let list_ref = {
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(&s3d_ref.name))
                                    .cloned()
                            };
                            let list_ref = if let Some(lr) = list_ref {
                                lr
                            } else {
                                // Lazily create the persistent textureList from scene data
                                let scene = {
                                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                    member.and_then(|m| m.member_type.as_shockwave3d())
                                        .and_then(|w3d| w3d.parsed_scene.clone())
                                };
                                let mut items = VecDeque::new();
                                if let Some(ref scene) = scene {
                                    let shader = scene.shaders.iter().find(|s| s.name == s3d_ref.name);
                                    if let Some(s) = shader {
                                        for layer in &s.texture_layers {
                                            if !layer.name.is_empty() {
                                                items.push_back(player.alloc_datum(Datum::String(layer.name.clone())));
                                            } else {
                                                items.push_back(player.alloc_datum(Datum::Void));
                                            }
                                        }
                                    }
                                }
                                while items.len() < 8 {
                                    items.push_back(player.alloc_datum(Datum::Void));
                                }
                                let new_list_ref = player.alloc_datum(Datum::List(
                                    crate::director::lingo::datum::DatumType::List, items, false,
                                ));
                                let shader_name_owned = s3d_ref.name.clone();
                                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                        w3d.runtime_state.shader_texture_lists.insert(shader_name_owned, new_list_ref.clone());
                                    }
                                }
                                new_list_ref
                            };
                            let idx = (index as usize).saturating_sub(1);
                            // Pre-allocate all needed refs before mutating
                            let value_ref = player.alloc_datum(value);
                            let mut void_refs = Vec::new();
                            {
                                let list_datum = player.get_datum(&list_ref);
                                if let Datum::List(_, items, _) = list_datum {
                                    let needed = if idx >= items.len() { idx - items.len() + 1 } else { 0 };
                                    for _ in 0..needed {
                                        void_refs.push(player.alloc_datum(Datum::Void));
                                    }
                                }
                            }
                            let list = player.get_datum_mut(&list_ref).to_list_mut();
                            if let Ok((_, list_vec, _)) = list {
                                list_vec.extend(void_refs);
                                if idx < list_vec.len() {
                                    list_vec[idx] = value_ref;
                                }
                            }
                        }
                    } else if args.len() == 2 {
                        // setAt(index, value) — accept silently
                    }
                    Ok(player.alloc_datum(Datum::Void))
                }
                "getPropRef" | "getProp" => {
                    // model.shaderList[I] → getPropRef(#shaderList, I)
                    // args[0] = property name (symbol/string), args[1] = index
                    if args.len() >= 2 {
                        let prop_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        let index = player.get_datum(&args[1]).int_value()?;
                        let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };

                        // Direct indexed access: compute the N-th item name from the scene
                        // without allocating an intermediate list (avoids datum aliasing)
                        let scene = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                                .ok_or_else(|| ScriptError::new("3D member not found".to_string()))?;
                            let w3d = member.member_type.as_shockwave3d()
                                .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
                            match w3d.parsed_scene.clone() {
                                Some(s) => s,
                                None => return Ok(player.alloc_datum(Datum::Void)),
                            }
                        };

                        // For collection props (shaderList, textureList, etc), directly get the Nth item
                        let idx = (index as usize).saturating_sub(1); // 1-based to 0-based
                        let collection_result = match prop_name.as_str() {
                            "shaderList" => {
                                // Get per-mesh shaders from model resource bindings
                                use crate::director::chunks::w3d::types::W3dNodeType;
                                let mut shader_names: Vec<String> = Vec::new();

                                let node = scene.nodes.iter().find(|n| n.name == s3d_ref.name);
                                if let Some(n) = node {
                                    let resource = if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name };
                                    if let Some(res) = scene.model_resources.get(resource) {
                                        // For each mesh index, find the best shader from bindings
                                        // Prefer bindings with textures (non-default shaders)
                                        let mesh_count = res.shader_bindings.iter()
                                            .map(|b| b.mesh_bindings.len())
                                            .max()
                                            .unwrap_or(1);
                                        for mesh_idx in 0..mesh_count {
                                            let mut best_name = String::new();
                                            let mut default_name = String::new();
                                            for binding in &res.shader_bindings {
                                                if mesh_idx < binding.mesh_bindings.len() {
                                                    let name = &binding.mesh_bindings[mesh_idx];
                                                    if !name.is_empty() && scene.shaders.iter().any(|s| s.name == *name) {
                                                        let is_default = binding.name == "default" || name == "DefaultShader";
                                                        if is_default {
                                                            if default_name.is_empty() { default_name = name.clone(); }
                                                        } else {
                                                            best_name = name.clone();
                                                        }
                                                    }
                                                }
                                            }
                                            if best_name.is_empty() { best_name = default_name; }
                                            if best_name.is_empty() && !n.shader_name.is_empty() {
                                                best_name = n.shader_name.clone();
                                            }
                                            shader_names.push(best_name);
                                        }
                                    }
                                    // If no resource bindings, use node's shader_name
                                    if shader_names.is_empty() && !n.shader_name.is_empty() {
                                        shader_names.push(n.shader_name.clone());
                                    }
                                }
                                if idx < shader_names.len() {
                                    Some(player.alloc_datum(Datum::Shockwave3dObjectRef(
                                        crate::director::lingo::datum::Shockwave3dObjectRef {
                                            cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                            object_type: "shader".to_string(),
                                            name: shader_names[idx].clone(),
                                        }
                                    )))
                                } else {
                                    Some(player.alloc_datum(Datum::Void))
                                }
                            }
                            "textureList" => {
                                // Read from persistent textureList, auto-extend if needed
                                let list_ref = {
                                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                    member.and_then(|m| m.member_type.as_shockwave3d())
                                        .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(&s3d_ref.name))
                                        .cloned()
                                };
                                if let Some(list_ref) = list_ref {
                                    // Auto-extend the list if needed
                                    let needs_extend = {
                                        let list_datum = player.get_datum(&list_ref);
                                        if let Datum::List(_, items, _) = list_datum {
                                            idx >= items.len()
                                        } else { false }
                                    };
                                    if needs_extend {
                                        let mut void_refs = Vec::new();
                                        let current_len = {
                                            let d = player.get_datum(&list_ref);
                                            if let Datum::List(_, items, _) = d { items.len() } else { 0 }
                                        };
                                        for _ in current_len..=idx {
                                            void_refs.push(player.alloc_datum(Datum::Void));
                                        }
                                        if let Ok((_, list_vec, _)) = player.get_datum_mut(&list_ref).to_list_mut() {
                                            list_vec.extend(void_refs);
                                        }
                                    }
                                    let list_datum = player.get_datum(&list_ref).clone();
                                    if let Datum::List(_, items, _) = list_datum {
                                        if idx < items.len() {
                                            Some(items[idx].clone())
                                        } else {
                                            Some(player.alloc_datum(Datum::Void))
                                        }
                                    } else {
                                        Some(player.alloc_datum(Datum::Void))
                                    }
                                } else {
                                    // Lazily create the persistent textureList from scene data
                                    let shader = scene.shaders.iter().find(|s| s.name == s3d_ref.name);
                                    let mut items = VecDeque::new();
                                    if let Some(s) = shader {
                                        for layer in &s.texture_layers {
                                            if !layer.name.is_empty() {
                                                items.push_back(player.alloc_datum(Datum::String(layer.name.clone())));
                                            } else {
                                                items.push_back(player.alloc_datum(Datum::Void));
                                            }
                                        }
                                    }
                                    while items.len() < 8 {
                                        items.push_back(player.alloc_datum(Datum::Void));
                                    }
                                    let new_list_ref = player.alloc_datum(Datum::List(
                                        crate::director::lingo::datum::DatumType::List, items, false,
                                    ));
                                    let shader_name_owned = s3d_ref.name.clone();
                                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                            w3d.runtime_state.shader_texture_lists.insert(shader_name_owned, new_list_ref.clone());
                                        }
                                    }
                                    // Return item at requested index
                                    let list_datum = player.get_datum(&new_list_ref).clone();
                                    if let Datum::List(_, items, _) = list_datum {
                                        if idx < items.len() {
                                            Some(items[idx].clone())
                                        } else {
                                            Some(player.alloc_datum(Datum::Void))
                                        }
                                    } else {
                                        Some(player.alloc_datum(Datum::Void))
                                    }
                                }
                            }
                            "textureTransformList" | "wrapTransformList" => {
                                // Return persistent Transform3D from the texture transform list
                                let list_ref = {
                                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                    member.and_then(|m| m.member_type.as_shockwave3d())
                                        .and_then(|w3d| w3d.runtime_state.shader_texture_transform_lists.get(&s3d_ref.name))
                                        .cloned()
                                };
                                if let Some(list_ref) = list_ref {
                                    // Auto-extend the list with identity transforms if needed
                                    let needs_extend = {
                                        let list_datum = player.get_datum(&list_ref);
                                        if let Datum::List(_, items, _) = list_datum {
                                            idx >= items.len()
                                        } else { false }
                                    };
                                    if needs_extend {
                                        let mut new_refs = Vec::new();
                                        let current_len = {
                                            let d = player.get_datum(&list_ref);
                                            if let Datum::List(_, items, _) = d { items.len() } else { 0 }
                                        };
                                        for _ in current_len..=idx {
                                            new_refs.push(player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX)));
                                        }
                                        if let Ok((_, list_vec, _)) = player.get_datum_mut(&list_ref).to_list_mut() {
                                            list_vec.extend(new_refs);
                                        }
                                    }
                                    let list_datum = player.get_datum(&list_ref).clone();
                                    if let Datum::List(_, items, _) = list_datum {
                                        if idx < items.len() {
                                            Some(items[idx].clone())
                                        } else {
                                            Some(player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX)))
                                        }
                                    } else {
                                        Some(player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX)))
                                    }
                                } else {
                                    // Create persistent list and store it
                                    let transform_ref = player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX));
                                    let mut items = VecDeque::new();
                                    for _ in 0..idx {
                                        items.push_back(player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX)));
                                    }
                                    items.push_back(transform_ref.clone());
                                    let list_ref = player.alloc_datum(Datum::List(
                                        crate::director::lingo::datum::DatumType::List, items, false,
                                    ));
                                    let shader_name = s3d_ref.name.clone();
                                    if let Some(member) = player.movie.cast_manager.find_member_by_ref_mut(&member_ref) {
                                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                            w3d.runtime_state.shader_texture_transform_lists.insert(shader_name, list_ref);
                                        }
                                    }
                                    Some(transform_ref)
                                }
                            }
                            "blendFunctionList" => {
                                // Return the blend function at index from texture layers
                                let shader = scene.shaders.iter().find(|s| s.name == s3d_ref.name);
                                if let Some(s) = shader {
                                    if let Some(layer) = s.texture_layers.get(idx) {
                                        let sym = match layer.blend_func {
                                            1 => "add",
                                            2 => "replace",
                                            3 => "blend",
                                            _ => "multiply",
                                        };
                                        Some(player.alloc_datum(Datum::Symbol(sym.to_string())))
                                    } else {
                                        Some(player.alloc_datum(Datum::Symbol("multiply".to_string())))
                                    }
                                } else {
                                    Some(player.alloc_datum(Datum::Symbol("multiply".to_string())))
                                }
                            }
                            "textureModeList" | "textureRepeatList" => {
                                Some(player.alloc_datum(Datum::Void))
                            }
                            "blendSourceList" => {
                                // Return blend source for texture layer at index (default #constant)
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                let val = member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.parsed_scene.as_ref())
                                    .and_then(|scene| scene.shaders.iter().find(|s| s.name == s3d_ref.name))
                                    .and_then(|shader| shader.texture_layers.get(idx))
                                    .map(|layer| {
                                        if layer.blend_src == 1 { "alpha" } else { "constant" }
                                    })
                                    .unwrap_or("constant");
                                Some(player.alloc_datum(Datum::Symbol(val.to_string())))
                            }
                            "blendConstantList" => {
                                // Return blend constant for texture layer at index (default 50.0)
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                let val = member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.parsed_scene.as_ref())
                                    .and_then(|scene| scene.shaders.iter().find(|s| s.name == s3d_ref.name))
                                    .and_then(|shader| shader.texture_layers.get(idx))
                                    .map(|layer| layer.blend_const as f64 * 100.0)
                                    .unwrap_or(50.0);
                                Some(player.alloc_datum(Datum::Float(val)))
                            }
                            // bonesPlayer.bone[n] — return bone ref
                            "bone" if s3d_ref.object_type == "bonesPlayer" => {
                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                Some(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: member_ref.cast_lib,
                                    cast_member: member_ref.cast_member,
                                    object_type: "bone".to_string(),
                                    name: format!("{}:{}", s3d_ref.name, idx), // modelName:boneIndex(0-based)
                                })))
                            }
                            // meshDeform.mesh[n] — return meshDeformMesh ref directly
                            "mesh" if s3d_ref.object_type == "meshDeform" => {
                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                // s3d_ref.name is the model name
                                Some(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: member_ref.cast_lib,
                                    cast_member: member_ref.cast_member,
                                    object_type: "meshDeformMesh".to_string(),
                                    name: format!("{}:{}", s3d_ref.name, idx),
                                })))
                            }
                            // modelResource.face[n] — return item from persistent face list
                            "face" if s3d_ref.object_type == "modelResource" => {
                                let face_key = format!("face:{}", s3d_ref.name);
                                let list_ref = {
                                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                    member.and_then(|m| m.member_type.as_shockwave3d())
                                        .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(&face_key))
                                        .cloned()
                                };
                                if let Some(list_ref) = list_ref {
                                    let list_datum = player.get_datum(&list_ref).clone();
                                    if let Datum::List(_, items, _) = list_datum {
                                        if idx < items.len() {
                                            Some(items[idx].clone())
                                        } else {
                                            Some(player.alloc_datum(Datum::Void))
                                        }
                                    } else {
                                        Some(player.alloc_datum(Datum::Void))
                                    }
                                } else {
                                    // Force creation of persistent list by calling get_model_resource_prop
                                    // then retry
                                    let _ = Self::get_model_resource_prop(player, &scene, &s3d_ref.name, "face", &member_ref);
                                    let list_ref = {
                                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                        member.and_then(|m| m.member_type.as_shockwave3d())
                                            .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(&face_key))
                                            .cloned()
                                    };
                                    if let Some(list_ref) = list_ref {
                                        let list_datum = player.get_datum(&list_ref).clone();
                                        if let Datum::List(_, items, _) = list_datum {
                                            if idx < items.len() {
                                                Some(items[idx].clone())
                                            } else {
                                                Some(player.alloc_datum(Datum::Void))
                                            }
                                        } else {
                                            Some(player.alloc_datum(Datum::Void))
                                        }
                                    } else {
                                        Some(player.alloc_datum(Datum::Void))
                                    }
                                }
                            }
                            // meshDeformMesh.textureLayer[n] — return the PropList item from persistent list
                            "textureLayer" if s3d_ref.object_type == "meshDeformMesh" => {
                                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                                let model_name = parts.get(0).unwrap_or(&"").to_string();
                                let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                                // Get the persistent textureLayer list DatumRef
                                let list_ref = {
                                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                    member.and_then(|m| m.member_type.as_shockwave3d())
                                        .and_then(|w3d| w3d.runtime_state.mesh_deform.get(&model_name))
                                        .and_then(|md| md.meshes.get(mesh_idx))
                                        .and_then(|mesh| mesh.texture_layer_datum_ref.clone())
                                };
                                if let Some(list_ref) = list_ref {
                                    let list_datum = player.get_datum(&list_ref).clone();
                                    if let Datum::List(_, items, _) = list_datum {
                                        if idx < items.len() {
                                            Some(items[idx].clone())
                                        } else {
                                            Some(player.alloc_datum(Datum::Void))
                                        }
                                    } else {
                                        Some(player.alloc_datum(Datum::Void))
                                    }
                                } else {
                                    Some(player.alloc_datum(Datum::Void))
                                }
                            }
                            // meshDeformMesh.vertexList[j] — return the j-th vertex vector
                            "vertexList" if s3d_ref.object_type == "meshDeformMesh" => {
                                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                                let model_name = parts.get(0).unwrap_or(&"").to_string();
                                let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                                let node = scene.nodes.iter().find(|n| n.name == *model_name);
                                let model_res = node.map(|n| n.model_resource_name.as_str()).unwrap_or("");
                                let res = node.map(|n| n.resource_name.as_str()).unwrap_or("");
                                let keys: Vec<&str> = [model_res, res].iter()
                                    .filter(|k| !k.is_empty() && **k != ".")
                                    .copied().collect();

                                for key in &keys {
                                    if let Some(meshes) = scene.clod_meshes.get(*key) {
                                        if let Some(mesh) = meshes.get(mesh_idx) {
                                            if idx < mesh.positions.len() {
                                                let pos = &mesh.positions[idx];
                                                return Ok(player.alloc_datum(Datum::Vector([pos[0] as f64, pos[1] as f64, pos[2] as f64])));
                                            }
                                        }
                                    }
                                }
                                // Fallback to raw_meshes with both keys
                                for key in &keys {
                                    for raw in &scene.raw_meshes {
                                        if raw.name == *key && raw.chain_index as usize == mesh_idx {
                                            if idx < raw.positions.len() {
                                                let pos = &raw.positions[idx];
                                                return Ok(player.alloc_datum(Datum::Vector([pos[0] as f64, pos[1] as f64, pos[2] as f64])));
                                            }
                                            break;
                                        }
                                    }
                                }
                                Some(player.alloc_datum(Datum::Void))
                            }
                            // camera.overlay[n] / camera.backdrop[n] — indexed overlay access
                            "overlay" | "backdrop" if s3d_ref.object_type == "camera" => {
                                let is_overlay = prop_name == "overlay";
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                let count = member.and_then(|m| m.member_type.as_shockwave3d())
                                    .map(|w3d| {
                                        let map = if is_overlay { &w3d.runtime_state.camera_overlays } else { &w3d.runtime_state.camera_backdrops };
                                        map.get(&s3d_ref.name).map(|v| v.len()).unwrap_or(0)
                                    })
                                    .unwrap_or(0);
                                if idx < count {
                                    Some(player.alloc_datum(Datum::Shockwave3dObjectRef(
                                        crate::director::lingo::datum::Shockwave3dObjectRef {
                                            cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member,
                                            object_type: prop_name.to_string(),
                                            name: format!("{}:{}", s3d_ref.name, idx),
                                        }
                                    )))
                                } else {
                                    Some(player.alloc_datum(Datum::Void))
                                }
                            }
                            _ => None, // Not a known collection, fall through to general get_prop_inner
                        };

                        if let Some(result) = collection_result {
                            Ok(result)
                        } else {
                            // General case: get the property and index into the result
                            let prop_result = Self::get_prop_inner(player, &s3d_ref, &member_ref, &scene, &prop_name)?;
                            let prop_datum = player.get_datum(&prop_result).clone();
                            match prop_datum {
                                Datum::List(_, items, _) => {
                                    if idx < items.len() {
                                        Ok(items[idx].clone())
                                    } else {
                                        Ok(player.alloc_datum(Datum::Void))
                                    }
                                }
                                _ => Ok(prop_result)
                            }
                        }
                    } else if args.len() == 1 {
                        // getProp(#propName) — just get the property
                        let prop_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                        let scene = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                                .ok_or_else(|| ScriptError::new("3D member not found".to_string()))?;
                            let w3d = member.member_type.as_shockwave3d()
                                .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
                            match w3d.parsed_scene.clone() {
                                Some(s) => s,
                                None => return Ok(player.alloc_datum(Datum::Void)),
                            }
                        };
                        Self::get_prop_inner(player, &s3d_ref, &member_ref, &scene, &prop_name)
                    } else {
                        Ok(player.alloc_datum(Datum::Void))
                    }
                }
                "getAt" => {
                    // getAt on a 3D object
                    if !args.is_empty() {
                        let arg = player.get_datum(&args[0]).clone();
                        match arg {
                            // String/symbol arg: treat as safe property access
                            // (only return simple values, not allocated lists)
                            Datum::String(ref s) | Datum::Symbol(ref s) => {
                                let prop = s.clone();
                                Self::get_prop(datum, &prop)
                            }
                            _ => Ok(player.alloc_datum(Datum::Void)),
                        }
                    } else {
                        Ok(player.alloc_datum(Datum::Void))
                    }
                }
                "count" => {
                    // count(#propName) on a 3D object — compute count directly without
                    // allocating intermediate lists (avoids datum slot recycling)
                    if !args.is_empty() {
                        let prop_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                        let scene = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                                .ok_or_else(|| ScriptError::new("3D member not found".to_string()))?;
                            let w3d = member.member_type.as_shockwave3d()
                                .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
                            match w3d.parsed_scene.clone() {
                                Some(s) => s,
                                None => return Ok(player.alloc_datum(Datum::Int(0))),
                            }
                        };
                        // Direct count computation for known collection properties
                        let count = match prop_name.as_str() {
                            "shaderList" => {
                                // Count = number of meshes in the model resource
                                let node = scene.nodes.iter().find(|n| n.name == s3d_ref.name);
                                let resource_name = node.map(|n| {
                                    if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name }
                                }).unwrap_or(&s3d_ref.name);
                                scene.model_resources.get(resource_name.as_str())
                                    .map(|res| res.shader_bindings.iter()
                                        .map(|b| b.mesh_bindings.len())
                                        .max()
                                        .unwrap_or(0))
                                    .unwrap_or(if node.map(|n| !n.shader_name.is_empty()).unwrap_or(false) { 1 } else { 0 })
                            }
                            "textureList" | "textureModeList" | "textureRepeatList"
                            | "blendFunctionList" | "blendSourceList" | "blendConstantList"
                            | "textureTransformList" | "wrapTransformList" => {
                                let shader = scene.shaders.iter().find(|s| s.name == s3d_ref.name);
                                shader.map(|s| s.texture_layers.len().max(1)).unwrap_or(1)
                            }
                            "mesh" => {
                                // meshDeform.mesh.count
                                let node = scene.nodes.iter().find(|n| n.name == s3d_ref.name);
                                let resource_name = node.map(|n| {
                                    if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name }
                                });
                                resource_name
                                    .and_then(|rn| scene.model_resources.get(rn.as_str()))
                                    .map(|res| res.mesh_infos.len())
                                    .unwrap_or(1)
                            }
                            // Scene-level collections
                            "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).count(),
                            "shader" => scene.shaders.len(),
                            "texture" => scene.texture_images.len(),
                            "light" => scene.lights.len(),
                            "camera" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::View).count(),
                            "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).count(),
                            "motion" => scene.motions.len(),
                            "modelResource" => scene.model_resources.len(),
                            "playList" => 0, // bonesPlayer.playList.count
                            "overlay" | "backdrop" => {
                                // camera.overlay.count / camera.backdrop.count
                                let is_overlay = prop_name == "overlay";
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                member.and_then(|m| m.member_type.as_shockwave3d())
                                    .map(|w3d| {
                                        let map = if is_overlay { &w3d.runtime_state.camera_overlays } else { &w3d.runtime_state.camera_backdrops };
                                        map.get(&s3d_ref.name).map(|v| v.len()).unwrap_or(0)
                                    })
                                    .unwrap_or(0)
                            }
                            "vertexList" => {
                                // meshDeformMesh.vertexList.count — get vertex count from mesh data
                                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                                let mdl_name = parts.get(0).unwrap_or(&"").to_string();
                                let m_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                                let node = scene.nodes.iter().find(|n| n.name == *mdl_name);
                                let model_res = node.map(|n| n.model_resource_name.as_str()).unwrap_or("");
                                let res = node.map(|n| n.resource_name.as_str()).unwrap_or("");
                                let keys: Vec<&str> = [model_res, res].iter()
                                    .filter(|k| !k.is_empty() && **k != ".")
                                    .copied().collect();

                                let mut count = 0usize;
                                for key in &keys {
                                    if let Some(meshes) = scene.clod_meshes.get(*key) {
                                        if let Some(mesh) = meshes.get(m_idx) {
                                            count = mesh.positions.len();
                                        }
                                        break;
                                    }
                                }
                                if count == 0 {
                                    for key in &keys {
                                        for raw in &scene.raw_meshes {
                                            if raw.name == *key && raw.chain_index as usize == m_idx {
                                                count = raw.positions.len();
                                                break;
                                            }
                                        }
                                        if count > 0 { break; }
                                    }
                                }
                                // Also try mesh_infos num_vertices as fallback
                                if count == 0 {
                                    for key in &keys {
                                        if let Some(res_info) = scene.model_resources.get(*key) {
                                            if let Some(info) = res_info.mesh_infos.get(m_idx) {
                                                count = info.num_vertices as usize;
                                            }
                                            break;
                                        }
                                    }
                                }
                                count
                            }
                            _ => 0,
                        };
                        Ok(player.alloc_datum(Datum::Int(count as i32)))
                    } else {
                        Ok(player.alloc_datum(Datum::Int(0)))
                    }
                }
                "worldSpaceToSpriteSpace" => {
                    // Project a world-space vector to 2D sprite-space point
                    if args.is_empty() {
                        return Ok(player.alloc_datum(Datum::Void));
                    }
                    let world_pos = player.get_datum(&args[0]).to_vector()?;
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    let scene = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("3D member not found".to_string()))?;
                        let w3d = member.member_type.as_shockwave3d()
                            .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
                        match w3d.parsed_scene.clone() {
                            Some(s) => s,
                            None => return Ok(player.alloc_datum(Datum::Void)),
                        }
                    };
                    // Get camera transform
                    let cam_transform = get_node_transform(player, &member_ref, &s3d_ref.name);
                    let cam_pos = [cam_transform[12], cam_transform[13], cam_transform[14]];
                    let view_matrix = invert_transform_f32(&cam_transform);
                    // Get viewport size from sprite (default 320x240)
                    let vw = player.movie.rect.width() as f32;
                    let vh = player.movie.rect.height() as f32;
                    let fov = scene.nodes.iter()
                        .find(|n| n.name == s3d_ref.name)
                        .map(|n| n.fov)
                        .unwrap_or(30.0);
                    let aspect = vw / vh;
                    let proj = build_perspective_f32(fov, aspect, 1.0, 10000.0);
                    // Transform world pos to clip space
                    let wp = [world_pos[0] as f32, world_pos[1] as f32, world_pos[2] as f32, 1.0];
                    let vp = mat4_mul_vec4(&view_matrix, &wp);
                    let cp = mat4_mul_vec4(&proj, &vp);
                    if cp[3].abs() < 1e-6 {
                        return Ok(player.alloc_datum(Datum::Void)); // behind camera
                    }
                    let ndc_x = cp[0] / cp[3];
                    let ndc_y = cp[1] / cp[3];
                    // NDC to sprite space: x: [-1,1] -> [0, vw], y: [1,-1] -> [0, vh]
                    let sx = ((ndc_x + 1.0) * 0.5 * vw) as i32;
                    let sy = ((1.0 - ndc_y) * 0.5 * vh) as i32;
                    let px = player.alloc_datum(Datum::Int(sx));
                    let py = player.alloc_datum(Datum::Int(sy));
                    Ok(player.alloc_datum(Datum::Point([px, py])))
                }
                "spriteSpaceToWorldSpace" => {
                    // Unproject a 2D sprite-space point to world-space position on projection plane
                    if args.is_empty() {
                        return Ok(player.alloc_datum(Datum::Void));
                    }
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    let scene = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("3D member not found".to_string()))?;
                        let w3d = member.member_type.as_shockwave3d()
                            .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
                        match w3d.parsed_scene.clone() {
                            Some(s) => s,
                            None => return Ok(player.alloc_datum(Datum::Void)),
                        }
                    };
                    let (sx, sy) = {
                        let arg = player.get_datum(&args[0]).clone();
                        match arg {
                            Datum::Point(refs) => {
                                let x = player.get_datum(&refs[0]).int_value().unwrap_or(0) as f32;
                                let y = player.get_datum(&refs[1]).int_value().unwrap_or(0) as f32;
                                (x, y)
                            }
                            _ => (0.0, 0.0),
                        }
                    };
                    let cam_transform = get_node_transform(player, &member_ref, &s3d_ref.name);
                    let vw = player.movie.rect.width() as f32;
                    let vh = player.movie.rect.height() as f32;
                    let fov = scene.nodes.iter()
                        .find(|n| n.name == s3d_ref.name)
                        .map(|n| n.fov)
                        .unwrap_or(30.0);
                    // Distance at which 1 pixel = 1 world unit
                    let half_h = (fov.to_radians() * 0.5).tan();
                    let proj_dist = (vh * 0.5) / half_h;
                    // Convert sprite coords to camera-local coords
                    let cx = sx - vw * 0.5;
                    let cy = -(sy - vh * 0.5); // flip Y
                    // Camera axes from transform
                    let right = [cam_transform[0], cam_transform[1], cam_transform[2]];
                    let up = [cam_transform[4], cam_transform[5], cam_transform[6]];
                    let fwd = [cam_transform[8], cam_transform[9], cam_transform[10]];
                    let pos = [cam_transform[12], cam_transform[13], cam_transform[14]];
                    let wx = pos[0] + right[0] * cx + up[0] * cy + fwd[0] * proj_dist;
                    let wy = pos[1] + right[1] * cx + up[1] * cy + fwd[1] * proj_dist;
                    let wz = pos[2] + right[2] * cx + up[2] * cy + fwd[2] * proj_dist;
                    Ok(player.alloc_datum(Datum::Vector([wx as f64, wy as f64, wz as f64])))
                }
                _ => {
                    // Treat as property get
                    Self::get_prop(datum, handler_name)
                }
            }
        })
    }

    // ─── Model property getters ───

    fn get_model_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        model_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let node = scene.nodes.iter()
            .find(|n| n.node_type == W3dNodeType::Model && n.name == model_name);

        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(model_name.to_string()))),
            "visible" | "visibility" => Ok(player.alloc_datum(Datum::Int(1))),
            "transform" => {
                Ok(get_persistent_node_transform(player, member_ref, model_name))
            }
            "worldPosition" => {
                let m = get_node_transform(player, member_ref, model_name);
                Ok(player.alloc_datum(Datum::Vector([
                    m[12] as f64, m[13] as f64, m[14] as f64,
                ])))
            }
            "resource" => {
                if let Some(n) = node {
                    let res_name = if !n.model_resource_name.is_empty() {
                        n.model_resource_name.clone()
                    } else {
                        n.resource_name.clone()
                    };
                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib,
                        cast_member: member_ref.cast_member,
                        object_type: "modelResource".to_string(),
                        name: res_name,
                    })))
                } else {
                    Ok(player.alloc_datum(Datum::Void))
                }
            }
            "sds" => {
                // Subdivision Surface modifier — return SDS object ref
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "sds".to_string(),
                    name: model_name.to_string(),
                })))
            }
            "parent" => {
                if let Some(n) = node {
                    Ok(player.alloc_datum(Datum::String(n.parent_name.clone())))
                } else {
                    Ok(player.alloc_datum(Datum::Void))
                }
            }
            "child" | "child.count" | "childCount" => {
                // Count children of this model
                let count = scene.nodes.iter()
                    .filter(|n| n.parent_name == model_name)
                    .count();
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            "shader" => {
                // Return shader as a Shockwave3dObjectRef
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                let mut shader_name = String::new();
                // 1) Model index → shader index
                let model_index = scene.nodes.iter()
                    .filter(|n| n.node_type == W3dNodeType::Model)
                    .position(|n| n.name == model_name);
                if let Some(mi) = model_index {
                    if mi < scene.shaders.len() {
                        shader_name = scene.shaders[mi].name.clone();
                    }
                }
                // 2) Fallback: node's shader_name
                if shader_name.is_empty() {
                    if let Some(n) = node {
                        if !n.shader_name.is_empty() {
                            shader_name = n.shader_name.clone();
                        }
                    }
                }
                if shader_name.is_empty() {
                    shader_name = format!("{}_shader", model_name);
                }
                // Find cast_lib/cast_member from parent context
                // We don't have it here directly, use 0 as placeholder
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                    object_type: "shader".to_string(),
                    name: shader_name,
                })))
            }
            "shaderList" => {
                // Return a list of shader refs from model resource's shader bindings
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                let mut items = VecDeque::new();

                // Find model resource name
                let resource_name = if let Some(n) = node {
                    if !n.model_resource_name.is_empty() {
                        n.model_resource_name.clone()
                    } else {
                        n.resource_name.clone()
                    }
                } else {
                    String::new()
                };

                // Collect unique shader names from all shader bindings' mesh_bindings
                if let Some(res_info) = scene.model_resources.get(&resource_name) {
                    // Count meshes from any binding's mesh_bindings
                    let mesh_count = res_info.shader_bindings.iter()
                        .map(|b| b.mesh_bindings.len())
                        .max()
                        .unwrap_or(0);

                    for mesh_idx in 0..mesh_count {
                        // For each mesh, find the best shader
                        // Iterate bindings in reverse: named bindings override the "default" binding
                        let mut best_name = String::new();
                        let mut default_name = String::new();
                        for binding in &res_info.shader_bindings {
                            if mesh_idx < binding.mesh_bindings.len() && !binding.mesh_bindings[mesh_idx].is_empty() {
                                let name = &binding.mesh_bindings[mesh_idx];
                                let is_default = binding.name == "default" || name == "DefaultShader";
                                if is_default {
                                    if default_name.is_empty() {
                                        default_name = name.clone();
                                    }
                                } else {
                                    best_name = name.clone();
                                }
                            }
                        }
                        // Use named shader, fall back to default
                        if best_name.is_empty() {
                            best_name = default_name;
                        }
                        if best_name.is_empty() {
                            // Fallback: use first binding's name
                            if let Some(b) = res_info.shader_bindings.first() {
                                best_name = b.name.clone();
                            }
                        }
                        if !best_name.is_empty() {
                            items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                object_type: "shader".to_string(),
                                name: best_name,
                            })));
                        }
                    }
                }

                // Fallback if no resource info found
                if items.is_empty() {
                    if let Some(n) = node {
                        if !n.shader_name.is_empty() {
                            items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                object_type: "shader".to_string(),
                                name: n.shader_name.clone(),
                            })));
                        }
                    }
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            }
            "bonesPlayer" => {
                // Return a reference to self - bonesPlayer properties are accessed on the same object
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                // Return a bonesPlayer reference that shares the model name
                // The cast_lib/cast_member will be filled from the parent s3d_ref context
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "bonesPlayer".to_string(),
                    name: model_name.to_string(),
                })))
            }
            "playing" => {
                let playing = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| if w3d.runtime_state.animation_playing { 1 } else { 0 })
                    .unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(playing)))
            }
            "currentTime" => {
                let time = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.runtime_state.animation_time)
                    .unwrap_or(0.0);
                // Director returns currentTime in milliseconds
                Ok(player.alloc_datum(Datum::Int((time * 1000.0) as i32)))
            }
            "playRate" => {
                let rate = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.runtime_state.play_rate)
                    .unwrap_or(1.0);
                Ok(player.alloc_datum(Datum::Float(rate as f64)))
            }
            "rootLock" => {
                let locked = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| if w3d.runtime_state.root_lock { 1 } else { 0 })
                    .unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(locked)))
            }
            "boundingSphere" => {
                // Return [vector(0,0,0), 100.0] as placeholder
                let center = player.alloc_datum(Datum::Vector([0.0, 0.0, 0.0]));
                let radius = player.alloc_datum(Datum::Float(100.0));
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::from(vec![center, radius]),
                    false,
                )))
            }
            "debug" => Ok(player.alloc_datum(Datum::Int(0))),
            "meshDeform" => {
                // Return a meshDeform ref pointing to this model
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "meshDeform".to_string(),
                    name: model_name.to_string(),
                })))
            }
            "modifiers" | "modifier" => {
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::new(),
                    false,
                )))
            }
            "playList" => {
                // bonesPlayer.playList — list of queued motions
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::new(),
                    false,
                )))
            }
            // playing/currentTime/playRate handled above in the first match arm
            _ => {
                web_sys::console::log_1(&format!("[W3D] model(\"{}\").{} (stub)", model_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    // ─── Shader property getters ───

    fn get_shader_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        shader_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let shader = scene.shaders.iter().find(|s| s.name == shader_name);
        let material = shader.and_then(|s| {
            scene.materials.iter().find(|m| m.name == s.material_name)
        });

        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(shader_name.to_string()))),
            "diffuse" => {
                if let Some(mat) = material {
                    Ok(player.alloc_datum(color_to_datum(mat.diffuse)))
                } else {
                    Ok(player.alloc_datum(color_to_datum([0.8, 0.8, 0.8, 1.0])))
                }
            }
            "ambient" => {
                if let Some(mat) = material {
                    Ok(player.alloc_datum(color_to_datum(mat.ambient)))
                } else {
                    Ok(player.alloc_datum(color_to_datum([0.0, 0.0, 0.0, 1.0])))
                }
            }
            "specular" => {
                if let Some(mat) = material {
                    Ok(player.alloc_datum(color_to_datum(mat.specular)))
                } else {
                    Ok(player.alloc_datum(color_to_datum([0.0, 0.0, 0.0, 1.0])))
                }
            }
            "emissive" => {
                if let Some(mat) = material {
                    Ok(player.alloc_datum(color_to_datum(mat.emissive)))
                } else {
                    Ok(player.alloc_datum(color_to_datum([0.0, 0.0, 0.0, 1.0])))
                }
            }
            "shininess" => {
                let v = material.map(|m| m.shininess).unwrap_or(0.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            }
            "blend" => {
                let v = material.map(|m| m.opacity * 100.0).unwrap_or(100.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            }
            "transparent" => {
                let v = material.map(|m| m.opacity < 1.0).unwrap_or(false);
                Ok(player.alloc_datum(Datum::Int(if v { 1 } else { 0 })))
            }
            "renderStyle" => Ok(player.alloc_datum(Datum::Symbol("fill".to_string()))),
            "flat" => Ok(player.alloc_datum(Datum::Int(0))),
            "textureRepeat" => Ok(player.alloc_datum(Datum::Int(1))),
            "useDiffuseWithTexture" => Ok(player.alloc_datum(Datum::Int(1))),
            "texture" => {
                // Return first texture as a Shockwave3dObjectRef
                if let Some(s) = shader {
                    if let Some(layer) = s.texture_layers.first() {
                        if !layer.name.is_empty() {
                            use crate::director::lingo::datum::Shockwave3dObjectRef;
                            return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                cast_lib: member_ref.cast_lib,
                                cast_member: member_ref.cast_member,
                                object_type: "texture".to_string(),
                                name: layer.name.clone(),
                            })));
                        }
                    }
                }
                Ok(player.alloc_datum(Datum::Void))
            }
            "textureList" => {
                // Return a persistent textureList so assignments like
                // shaderList[m].textureList[n] = tex persist
                let existing_ref = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(shader_name))
                        .cloned()
                };
                if let Some(list_ref) = existing_ref {
                    Ok(list_ref)
                } else {
                    // Create with 8 slots (Director's max texture layers)
                    // Fill from scene data, pad with VOID
                    let mut items = VecDeque::new();
                    if let Some(s) = shader {
                        for layer in &s.texture_layers {
                            if !layer.name.is_empty() {
                                items.push_back(player.alloc_datum(Datum::String(layer.name.clone())));
                            } else {
                                items.push_back(player.alloc_datum(Datum::Void));
                            }
                        }
                    }
                    // Pad to 8 entries
                    while items.len() < 8 {
                        items.push_back(player.alloc_datum(Datum::Void));
                    }
                    let list_ref = player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List, items, false,
                    ));
                    // Store persistently
                    let shader_name_owned = shader_name.to_string();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.shader_texture_lists.insert(shader_name_owned, list_ref.clone());
                        }
                    }
                    Ok(list_ref)
                }
            }
            "textureModeList" => {
                let mut items = VecDeque::new();
                if let Some(s) = shader {
                    for layer in &s.texture_layers {
                        let mode = match layer.tex_mode {
                            0 => "none",
                            4 => "reflection",
                            5 => "wrapPlanar",
                            6 => "specular",
                            _ => "none",
                        };
                        items.push_back(player.alloc_datum(Datum::Symbol(mode.to_string())));
                    }
                }
                if items.is_empty() {
                    items.push_back(player.alloc_datum(Datum::Symbol("none".to_string())));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            }
            "blendFunctionList" => {
                let mut items = VecDeque::new();
                if let Some(s) = shader {
                    for layer in &s.texture_layers {
                        let sym = match layer.blend_func {
                            1 => "add",
                            2 => "replace",
                            3 => "blend",
                            _ => "multiply",
                        };
                        items.push_back(player.alloc_datum(Datum::Symbol(sym.to_string())));
                    }
                }
                if items.is_empty() {
                    items.push_back(player.alloc_datum(Datum::Symbol("multiply".to_string())));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            }
            "textureRepeatList" => {
                let item = player.alloc_datum(Datum::Int(1));
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, VecDeque::from(vec![item]), false,
                )))
            }
            "textureTransformList" | "wrapTransformList" => {
                Self::get_or_create_texture_transform_list(player, scene, shader_name, &member_ref)
            }
            "textureTransform" | "wrapTransform" => {
                // Shorthand for textureTransformList[1]
                let list_ref = Self::get_or_create_texture_transform_list(player, scene, shader_name, &member_ref)?;
                let list_datum = player.get_datum(&list_ref).clone();
                if let Datum::List(_, items, _) = list_datum {
                    if !items.is_empty() {
                        Ok(items[0].clone())
                    } else {
                        Ok(player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX)))
                    }
                } else {
                    Ok(player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX)))
                }
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] shader(\"{}\").{} (stub)", shader_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    /// Get or create the persistent textureTransformList for a shader.
    fn get_or_create_texture_transform_list(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        shader_name: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let existing_ref = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref);
            member.and_then(|m| m.member_type.as_shockwave3d())
                .and_then(|w3d| w3d.runtime_state.shader_texture_transform_lists.get(shader_name))
                .cloned()
        };
        if let Some(list_ref) = existing_ref {
            Ok(list_ref)
        } else {
            let layer_count = scene.shaders.iter()
                .find(|s| s.name == shader_name)
                .map(|s| s.texture_layers.len().max(1))
                .unwrap_or(1);
            let mut items = VecDeque::new();
            for _ in 0..layer_count {
                items.push_back(player.alloc_datum(Datum::Transform3d(IDENTITY_MATRIX)));
            }
            let list_ref = player.alloc_datum(Datum::List(
                crate::director::lingo::datum::DatumType::List, items, false,
            ));
            let shader_key = shader_name.to_string();
            if let Some(member) = player.movie.cast_manager.find_member_by_ref_mut(member_ref) {
                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                    w3d.runtime_state.shader_texture_transform_lists.insert(shader_key, list_ref.clone());
                }
            }
            Ok(list_ref)
        }
    }

    // ─── Camera/View property getters ───

    fn get_camera_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        camera_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let node = scene.nodes.iter()
            .find(|n| n.node_type == W3dNodeType::View && n.name == camera_name);

        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(camera_name.to_string()))),
            "transform" => {
                Ok(get_persistent_node_transform(player, member_ref, camera_name))
            }
            "fieldOfView" | "projectionAngle" => {
                let fov = node.map(|n| n.fov).unwrap_or(30.0);
                Ok(player.alloc_datum(Datum::Float(fov as f64)))
            }
            "nearClipPlane" | "hither" => {
                let v = node.map(|n| n.near_plane).unwrap_or(1.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            }
            "farClipPlane" | "yon" => {
                let v = node.map(|n| n.far_plane).unwrap_or(10000.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            }
            "worldPosition" => {
                if let Some(n) = node {
                    Ok(player.alloc_datum(Datum::Vector([
                        n.transform[12] as f64,
                        n.transform[13] as f64,
                        n.transform[14] as f64,
                    ])))
                } else {
                    Ok(player.alloc_datum(Datum::Vector([0.0, 0.0, 500.0])))
                }
            }
            "projectionType" => Ok(player.alloc_datum(Datum::Symbol("perspective".to_string()))),
            "visible" => Ok(player.alloc_datum(Datum::Int(1))),
            "rect" => {
                // Camera viewport rect (normalized: 0,0,1,1 = full viewport)
                let l = player.alloc_datum(Datum::Float(0.0));
                let t = player.alloc_datum(Datum::Float(0.0));
                let r = player.alloc_datum(Datum::Float(1.0));
                let b = player.alloc_datum(Datum::Float(1.0));
                Ok(player.alloc_datum(Datum::Rect([l, t, r, b])))
            }
            "fog.enabled" => Ok(player.alloc_datum(Datum::Int(0))),
            "fog.near" => Ok(player.alloc_datum(Datum::Float(1.0))),
            "fog.far" => Ok(player.alloc_datum(Datum::Float(1000.0))),
            "fog.color" => {
                Ok(player.alloc_datum(color_to_datum([0.5, 0.5, 0.5, 1.0])))
            }
            "overlay" | "backdrop" => {
                // Return overlay/backdrop list — each item is an overlay object ref
                let is_overlay = prop == "overlay";
                let count = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .map(|w3d| {
                            let map = if is_overlay { &w3d.runtime_state.camera_overlays } else { &w3d.runtime_state.camera_backdrops };
                            map.get(camera_name).map(|v| v.len()).unwrap_or(0)
                        })
                        .unwrap_or(0)
                };
                let mut items = VecDeque::new();
                for i in 0..count {
                    items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(
                        crate::director::lingo::datum::Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib,
                            cast_member: member_ref.cast_member,
                            object_type: prop.to_string(), // "overlay" or "backdrop"
                            name: format!("{}:{}", camera_name, i),
                        }
                    )));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            }
            "rootNode" => {
                let root = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.camera_root_nodes.get(camera_name))
                        .cloned()
                };
                if let Some(root_name) = root {
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(
                        crate::director::lingo::datum::Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                            object_type: "group".to_string(), name: root_name,
                        }
                    )))
                } else {
                    Ok(player.alloc_datum(Datum::Void))
                }
            }
            "colorBuffer" | "colorBuffer.clearAtRender" => {
                // Return a camera-specific colorBuffer ref for .clearAtRender property
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(
                    crate::director::lingo::datum::Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                        object_type: "colorBuffer".to_string(),
                        name: camera_name.to_string(),
                    }
                )))
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] camera(\"{}\").{} (stub)", camera_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    // ─── Light property getters ───

    fn get_light_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        light_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let light = scene.lights.iter().find(|l| l.name == light_name);

        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(light_name.to_string()))),
            "type" => {
                let sym = match light.map(|l| &l.light_type) {
                    Some(W3dLightType::Ambient) => "ambient",
                    Some(W3dLightType::Directional) => "directional",
                    Some(W3dLightType::Point) => "point",
                    Some(W3dLightType::Spot) => "spot",
                    None => "directional",
                };
                Ok(player.alloc_datum(Datum::Symbol(sym.to_string())))
            }
            "color" => {
                if let Some(l) = light {
                    Ok(player.alloc_datum(color_to_datum([l.color[0], l.color[1], l.color[2], 1.0])))
                } else {
                    Ok(player.alloc_datum(color_to_datum([1.0, 1.0, 1.0, 1.0])))
                }
            }
            "visible" => {
                let v = light.map(|l| l.enabled).unwrap_or(true);
                Ok(player.alloc_datum(Datum::Int(if v { 1 } else { 0 })))
            }
            "spotAngle" => {
                let v = light.map(|l| l.spot_angle).unwrap_or(90.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            }
            "transform" => {
                Ok(get_persistent_node_transform(player, member_ref, light_name))
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] light(\"{}\").{} (stub)", light_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    // ─── Generic node property getters ───

    fn get_node_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        node_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(node_name.to_string()))),
            "parent" => {
                let parent = scene.nodes.iter().find(|n| n.name == node_name)
                    .map(|n| n.parent_name.clone())
                    .unwrap_or_default();
                Ok(player.alloc_datum(Datum::String(parent)))
            }
            "transform" => {
                Ok(get_persistent_node_transform(player, member_ref, node_name))
            }
            "worldPosition" => {
                let m = get_node_transform(player, member_ref, node_name);
                Ok(player.alloc_datum(Datum::Vector([
                    m[12] as f64, m[13] as f64, m[14] as f64,
                ])))
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] group(\"{}\").{} (stub)", node_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    // ─── ModelResource property getters ───

    fn get_model_resource_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        resource_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let res = scene.model_resources.get(resource_name);

        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(resource_name.to_string()))),
            "type" => Ok(player.alloc_datum(Datum::Symbol("fromFile".to_string()))),
            "face.count" | "faceCount" => {
                let count: u32 = res.map(|r| r.mesh_infos.iter().map(|m| m.num_faces).sum()).unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            "face" => {
                // Return a persistent face list using shader_texture_lists with "face:" prefix
                let face_key = format!("face:{}", resource_name);
                let existing_ref = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(&face_key))
                        .cloned()
                };
                if let Some(list_ref) = existing_ref {
                    Ok(list_ref)
                } else {
                    let count: u32 = res.map(|r| r.mesh_infos.iter().map(|m| m.num_faces).sum()).unwrap_or(0);
                    let mut items = VecDeque::new();
                    for _ in 0..count {
                        let sk = player.alloc_datum(Datum::Symbol("shader".to_string()));
                        let sv = player.alloc_datum(Datum::Void);
                        let vk = player.alloc_datum(Datum::Symbol("vertices".to_string()));
                        let vv = player.alloc_datum(Datum::List(crate::director::lingo::datum::DatumType::List, VecDeque::new(), false));
                        let tk = player.alloc_datum(Datum::Symbol("textureCoordinates".to_string()));
                        let tv = player.alloc_datum(Datum::List(crate::director::lingo::datum::DatumType::List, VecDeque::new(), false));
                        let ck = player.alloc_datum(Datum::Symbol("colors".to_string()));
                        let cv = player.alloc_datum(Datum::List(crate::director::lingo::datum::DatumType::List, VecDeque::new(), false));
                        let nk = player.alloc_datum(Datum::Symbol("normals".to_string()));
                        let nv = player.alloc_datum(Datum::List(crate::director::lingo::datum::DatumType::List, VecDeque::new(), false));
                        items.push_back(player.alloc_datum(Datum::PropList(VecDeque::from(vec![(sk, sv), (vk, vv), (tk, tv), (ck, cv), (nk, nv)]), false)));
                    }
                    let list_ref = player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List, items, false,
                    ));
                    let face_key_owned = face_key.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.shader_texture_lists.insert(face_key_owned, list_ref.clone());
                        }
                    }
                    Ok(list_ref)
                }
            }
            "lod" => {
                // Return a PropList with lod properties (auto, level, bias)
                let auto_key = player.alloc_datum(Datum::Symbol("auto".to_string()));
                let auto_val = player.alloc_datum(Datum::Int(1));
                let level_key = player.alloc_datum(Datum::Symbol("level".to_string()));
                let level_val = player.alloc_datum(Datum::Int(100));
                let bias_key = player.alloc_datum(Datum::Symbol("bias".to_string()));
                let bias_val = player.alloc_datum(Datum::Float(100.0));
                Ok(player.alloc_datum(Datum::PropList(VecDeque::from(vec![
                    (auto_key, auto_val), (level_key, level_val), (bias_key, bias_val),
                ]), false)))
            }
            "sds" => {
                // Subdivision Surface modifier — return object with depth/tension properties
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "sds".to_string(),
                    name: resource_name.to_string(),
                })))
            }
            "emitter" => {
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "emitter".to_string(),
                    name: resource_name.to_string(),
                })))
            }
            // Particle system resource properties — range objects with #start and #end
            "colorRange" => {
                let sk = player.alloc_datum(Datum::Symbol("start".to_string()));
                let sv = player.alloc_datum(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(255, 255, 255)));
                let ek = player.alloc_datum(Datum::Symbol("end".to_string()));
                let ev = player.alloc_datum(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(255, 255, 255)));
                Ok(player.alloc_datum(Datum::PropList(VecDeque::from(vec![(sk, sv), (ek, ev)]), false)))
            }
            "sizeRange" => {
                let sk = player.alloc_datum(Datum::Symbol("start".to_string()));
                let sv = player.alloc_datum(Datum::Float(1.0));
                let ek = player.alloc_datum(Datum::Symbol("end".to_string()));
                let ev = player.alloc_datum(Datum::Float(1.0));
                Ok(player.alloc_datum(Datum::PropList(VecDeque::from(vec![(sk, sv), (ek, ev)]), false)))
            }
            "blendRange" => {
                let sk = player.alloc_datum(Datum::Symbol("start".to_string()));
                let sv = player.alloc_datum(Datum::Int(100));
                let ek = player.alloc_datum(Datum::Symbol("end".to_string()));
                let ev = player.alloc_datum(Datum::Int(100));
                Ok(player.alloc_datum(Datum::PropList(VecDeque::from(vec![(sk, sv), (ek, ev)]), false)))
            }
            "lifetime" => Ok(player.alloc_datum(Datum::Int(1000))),
            "gravity" => Ok(player.alloc_datum(Datum::Vector([0.0, -9.8, 0.0]))),
            "wind" => Ok(player.alloc_datum(Datum::Vector([0.0, 0.0, 0.0]))),
            "drag" => Ok(player.alloc_datum(Datum::Float(0.0))),
            // Accept common resource properties silently
            "width" | "length" | "lengthVertices" | "widthVertices"
            | "height" | "numVertices" | "numFaces" => {
                Ok(player.alloc_datum(Datum::Int(0)))
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] modelResource(\"{}\").{} (stub)", resource_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    // ─── MeshDeform property getters ───

    fn get_mesh_deform_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        model_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        // Find the model's resource to get mesh info
        let node = scene.nodes.iter().find(|n| n.name == model_name);
        let resource_name = node.map(|n| {
            if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name }
        });
        let mesh_count = resource_name
            .and_then(|rn| scene.model_resources.get(rn.as_str()))
            .map(|res| res.mesh_infos.len())
            .unwrap_or(1);

        let face_count: u32 = resource_name
            .and_then(|rn| scene.model_resources.get(rn.as_str()))
            .map(|res| res.mesh_infos.iter().map(|m| m.num_faces).sum())
            .unwrap_or(0);

        match prop {
            "mesh" | "mesh.count" | "meshCount" => {
                if prop.contains("count") || prop.contains("Count") {
                    return Ok(player.alloc_datum(Datum::Int(mesh_count as i32)));
                }
                // Return a list of meshDeformMesh refs that route to persistent state
                let mut items = VecDeque::new();
                for i in 0..mesh_count {
                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                    items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib,
                        cast_member: member_ref.cast_member,
                        object_type: "meshDeformMesh".to_string(),
                        name: format!("{}:{}", model_name, i),
                    })));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            }
            "face" | "face.count" | "faceCount" => {
                Ok(player.alloc_datum(Datum::Int(face_count as i32)))
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] meshDeform(\"{}\").{} (stub)", model_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    // ─── Motion property getters ───

    fn get_motion_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        motion_name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let motion = scene.motions.iter().find(|m| m.name == motion_name);

        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(motion_name.to_string()))),
            "duration" => {
                let dur = motion.map(|m| m.duration()).unwrap_or(0.0);
                Ok(player.alloc_datum(Datum::Float((dur * 1000.0) as f64))) // ms
            }
            "type" => Ok(player.alloc_datum(Datum::Symbol("bones".to_string()))),
            _ => {
                web_sys::console::log_1(&format!("[W3D] motion(\"{}\").{} (stub)", motion_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }

    // ─── Texture property getters ───

    fn get_texture_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        texture_name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        match prop {
            "name" => Ok(player.alloc_datum(Datum::String(texture_name.to_string()))),
            "type" => Ok(player.alloc_datum(Datum::Symbol("fromFile".to_string()))),
            "renderFormat" => Ok(player.alloc_datum(Datum::Symbol("rgba8880".to_string()))),
            "quality" => Ok(player.alloc_datum(Datum::Symbol("default".to_string()))),
            "width" | "height" => {
                // Look up actual texture dimensions from scene data
                let dim = get_texture_dimensions(scene, texture_name);
                let val = if prop == "width" { dim.0 } else { dim.1 };
                Ok(player.alloc_datum(Datum::Int(val as i32)))
            }
            "nearFiltering" => Ok(player.alloc_datum(Datum::Int(1))),
            _ => {
                web_sys::console::log_1(&format!("[W3D] texture(\"{}\").{} (stub)", texture_name, prop).into());
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }
}

/// Convert an RGBA color array to a Datum::ColorRef
fn color_to_datum(c: [f32; 4]) -> Datum {
    use crate::player::sprite::ColorRef;
    Datum::ColorRef(ColorRef::Rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    ))
}

// ─── Transform mutation helpers ───

fn read_xyz_args(player: &crate::player::DirPlayer, args: &[crate::player::DatumRef]) -> (f32, f32, f32) {
    if args.len() >= 3 {
        let x = player.get_datum(&args[0]).float_value().unwrap_or(0.0) as f32;
        let y = player.get_datum(&args[1]).float_value().unwrap_or(0.0) as f32;
        let z = player.get_datum(&args[2]).float_value().unwrap_or(0.0) as f32;
        (x, y, z)
    } else if !args.is_empty() {
        if let Datum::Vector(v) = player.get_datum(&args[0]) {
            (v[0] as f32, v[1] as f32, v[2] as f32)
        } else {
            (0.0, 0.0, 0.0)
        }
    } else {
        (0.0, 0.0, 0.0)
    }
}

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

fn get_node_transform(
    player: &crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f32; 16] {
    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d() {
            // Check runtime override first
            if let Some(m) = w3d.runtime_state.node_transforms.get(node_name) {
                return *m;
            }
            // Fall back to parsed scene
            if let Some(scene) = &w3d.parsed_scene {
                if let Some(node) = scene.nodes.iter().find(|n| n.name == node_name) {
                    return node.transform;
                }
            }
        }
    }
    IDENTITY
}

fn get_or_init_node_transform(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f32; 16] {
    // Check persistent datum first (may have been modified in-place via .position = etc.)
    let from_persistent = {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        member.and_then(|m| m.member_type.as_shockwave3d())
            .and_then(|w3d| w3d.runtime_state.node_transform_datums.get(node_name))
            .cloned()
    };
    if let Some(datum_ref) = from_persistent {
        if let Datum::Transform3d(m64) = player.get_datum(&datum_ref) {
            let m32: [f32; 16] = m64.map(|v| v as f32);
            // Ensure node_transforms is up to date
            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                    w3d.runtime_state.node_transforms.insert(node_name.to_string(), m32);
                }
            }
            return m32;
        }
    }

    // Fall back to node_transforms or scene
    let current = get_node_transform(player, member_ref, node_name);

    // Ensure it's in the runtime overrides
    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
            w3d.runtime_state.node_transforms.entry(node_name.to_string()).or_insert(current);
        }
    }
    current
}

fn set_node_transform(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    m: [f32; 16],
) {
    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
            w3d.runtime_state.node_transforms.insert(node_name.to_string(), m);
            // Mark this node as explicitly modified — overrides motion animation
            w3d.runtime_state.node_transforms_dirty.insert(node_name.to_string());
            // Also update persistent datum if it exists
            if let Some(datum_ref) = w3d.runtime_state.node_transform_datums.get(node_name) {
                let m64: [f64; 16] = m.map(|v| v as f64);
                let datum_ref = datum_ref.clone();
                *player.get_datum_mut(&datum_ref) = Datum::Transform3d(m64);
                return; // member_ref borrow released above via clone
            }
        }
    }
}

/// Get or create a persistent Transform3d DatumRef for a node.
/// Returns the same DatumRef on subsequent calls so that in-place mutations persist.
fn get_persistent_node_transform(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> DatumRef {
    // Check if persistent datum already exists
    let existing = {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        member.and_then(|m| m.member_type.as_shockwave3d())
            .and_then(|w3d| w3d.runtime_state.node_transform_datums.get(node_name))
            .cloned()
    };
    if let Some(datum_ref) = existing {
        return datum_ref;
    }

    // Create new persistent datum from current transform
    let m = get_node_transform(player, member_ref, node_name);
    let m64: [f64; 16] = m.map(|v| v as f64);
    let datum_ref = player.alloc_datum(Datum::Transform3d(m64));

    // Store in runtime state
    let node_key = node_name.to_string();
    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
            w3d.runtime_state.node_transform_datums.insert(node_key, datum_ref.clone());
        }
    }
    datum_ref
}

/// Sync all persistent transform datums back to node_transforms for the renderer.
/// Call this before each render frame.
pub fn sync_persistent_transforms(player: &mut crate::player::DirPlayer) {
    // Collect all (cast_lib, cast_member, node_name, datum_ref) tuples
    let mut entries: Vec<(i32, u32, String, DatumRef)> = Vec::new();
    for cast in &player.movie.cast_manager.casts {
        for (member_num, member) in &cast.members {
            if let Some(w3d) = member.member_type.as_shockwave3d() {
                for (node_name, datum_ref) in &w3d.runtime_state.node_transform_datums {
                    entries.push((cast.number as i32, *member_num, node_name.clone(), datum_ref.clone()));
                }
            }
        }
    }

    for (cast_lib, cast_member, node_name, datum_ref) in entries {
        if let Datum::Transform3d(m64) = player.get_datum(&datum_ref) {
            let m32: [f32; 16] = m64.map(|v| v as f32);
            let member_ref = CastMemberRef { cast_lib, cast_member: cast_member as i32 };
            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                    w3d.runtime_state.node_transforms.insert(node_name, m32);
                }
            }
        }
    }
}

/// Sync persistent shader textureList DatumRefs back to shader.texture_layers in the scene.
/// This ensures the renderer sees textures assigned via Lingo (shader.textureList[n] = tex).
pub fn sync_shader_texture_lists(player: &mut crate::player::DirPlayer) {
    // Collect (cast_lib, member_num, shader_name, list_ref) tuples
    let mut entries: Vec<(i32, u32, String, DatumRef)> = Vec::new();
    for cast in &player.movie.cast_manager.casts {
        for (member_num, member) in &cast.members {
            if let Some(w3d) = member.member_type.as_shockwave3d() {
                for (shader_name, list_ref) in &w3d.runtime_state.shader_texture_lists {
                    entries.push((cast.number as i32, *member_num, shader_name.clone(), list_ref.clone()));
                }
            }
        }
    }

    for (cast_lib, cast_member, shader_name, list_ref) in entries {
        // Read texture names from the persistent list
        let tex_names: Vec<String> = if let Datum::List(_, items, _) = player.get_datum(&list_ref) {
            items.iter().map(|item_ref| {
                match player.get_datum(item_ref) {
                    Datum::Shockwave3dObjectRef(r) if r.object_type == "texture" => r.name.clone(),
                    Datum::String(s) => s.clone(),
                    _ => String::new(),
                }
            }).collect()
        } else {
            continue;
        };

        // Update shader.texture_layers in the parsed scene
        let member_ref = CastMemberRef { cast_lib, cast_member: cast_member as i32 };
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                if let Some(scene) = w3d.scene_mut() {
                    if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == shader_name) {
                        // Extend texture_layers if needed
                        use crate::director::chunks::w3d::types::W3dTextureLayer;
                        while shader.texture_layers.len() < tex_names.len() {
                            shader.texture_layers.push(W3dTextureLayer::default());
                        }
                        for (i, name) in tex_names.iter().enumerate() {
                            if !name.is_empty() {
                                shader.texture_layers[i].name = name.clone();
                            }
                        }
                    }
                }
            }
        }
    }
}

fn apply_translation(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    dx: f32, dy: f32, dz: f32,
) {
    let mut m = get_or_init_node_transform(player, member_ref, node_name);
    m[12] += dx;
    m[13] += dy;
    m[14] += dz;
    set_node_transform(player, member_ref, node_name, m);
}

fn apply_rotation(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    rx_deg: f32, ry_deg: f32, rz_deg: f32,
) {
    let m = get_or_init_node_transform(player, member_ref, node_name);
    let rot = euler_to_matrix_f32(rx_deg, ry_deg, rz_deg);
    // Apply rotation in world axes but keep the node positioned in place.
    // Post-multiplying by `rot` rotates around the node's existing local basis,
    // which inherits bind/rest orientation and can turn a yaw into an apparent roll.
    let mut result = mat4_mul_f32(&rot, &m);
    result[12] = m[12];
    result[13] = m[13];
    result[14] = m[14];
    set_node_transform(player, member_ref, node_name, result);
}

fn apply_scale(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    sx: f32, sy: f32, sz: f32,
) {
    let mut m = get_or_init_node_transform(player, member_ref, node_name);
    // Scale the rotation columns
    for i in 0..3 { m[i] *= sx; }
    for i in 4..7 { m[i] *= sy; }
    for i in 8..11 { m[i] *= sz; }
    set_node_transform(player, member_ref, node_name, m);
}

fn apply_point_at(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    tx: f32, ty: f32, tz: f32,
    up_x: f32, up_y: f32, up_z: f32,
) {
    let m = get_or_init_node_transform(player, member_ref, node_name);
    let pos = [m[12], m[13], m[14]];

    // Forward = toward target
    let mut fwd = [tx - pos[0], ty - pos[1], tz - pos[2]];
    let len = (fwd[0]*fwd[0] + fwd[1]*fwd[1] + fwd[2]*fwd[2]).sqrt();
    if len > 1e-6 {
        fwd[0] /= len; fwd[1] /= len; fwd[2] /= len;
    } else {
        return;
    }

    // Z axis = -forward (away from target, Director/standard look-at convention)
    let neg_fwd = [-fwd[0], -fwd[1], -fwd[2]];

    // Use provided up hint; fall back to world X if forward is parallel to up
    let mut up = [up_x, up_y, up_z];
    let dot_up_fwd = up[0]*fwd[0] + up[1]*fwd[1] + up[2]*fwd[2];
    if dot_up_fwd.abs() > 0.999 {
        up = [1.0, 0.0, 0.0];
    }

    // right = cross(up, negFwd)
    let mut right = [
        up[1]*neg_fwd[2] - up[2]*neg_fwd[1],
        up[2]*neg_fwd[0] - up[0]*neg_fwd[2],
        up[0]*neg_fwd[1] - up[1]*neg_fwd[0],
    ];
    let rlen = (right[0]*right[0] + right[1]*right[1] + right[2]*right[2]).sqrt();
    if rlen > 1e-6 {
        right[0] /= rlen; right[1] /= rlen; right[2] /= rlen;
    }

    // Recomputed up = cross(negFwd, right)
    let up2 = [
        neg_fwd[1]*right[2] - neg_fwd[2]*right[1],
        neg_fwd[2]*right[0] - neg_fwd[0]*right[2],
        neg_fwd[0]*right[1] - neg_fwd[1]*right[0],
    ];

    // Column-major: X=right, Y=up, Z=-forward (away from target), W=position
    let result = [
        right[0],   right[1],   right[2],   0.0,
        up2[0],     up2[1],     up2[2],     0.0,
        neg_fwd[0], neg_fwd[1], neg_fwd[2], 0.0,
        pos[0],     pos[1],     pos[2],     1.0,
    ];
    set_node_transform(player, member_ref, node_name, result);
}

/// Column-major 4x4 matrix multiply: C = A * B
fn mat4_mul_f32(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] =
                a[0 * 4 + row] * b[col * 4 + 0] +
                a[1 * 4 + row] * b[col * 4 + 1] +
                a[2 * 4 + row] * b[col * 4 + 2] +
                a[3 * 4 + row] * b[col * 4 + 3];
        }
    }
    r
}

/// Euler angles (degrees) to column-major rotation matrix
fn euler_to_matrix_f32(rx_deg: f32, ry_deg: f32, rz_deg: f32) -> [f32; 16] {
    let rx = rx_deg.to_radians();
    let ry = (-ry_deg).to_radians(); // Director uses left-handed Y rotation
    let rz = rz_deg.to_radians();
    let (sx, cx) = (rx.sin(), rx.cos());
    let (sy, cy) = (ry.sin(), ry.cos());
    let (sz, cz) = (rz.sin(), rz.cos());

    // Column-major layout: each group of 4 is a column
    [
        cy*cz,              sx*sy*cz - cx*sz,   cx*sy*cz + sx*sz,  0.0,  // col 0
        cy*sz,              sx*sy*sz + cx*cz,   cx*sy*sz - sx*cz,  0.0,  // col 1
        -sy,                sx*cy,              cx*cy,             0.0,  // col 2
        0.0,                0.0,                0.0,               1.0,  // col 3
    ]
}

/// Invert a column-major affine transform
fn invert_transform_f32(m: &[f32; 16]) -> [f32; 16] {
    // Column-major: R[row][col] = m[col*4 + row]
    let (tx, ty, tz) = (m[12], m[13], m[14]);
    // -R^T * t
    let itx = -(m[0]*tx + m[1]*ty + m[2]*tz);
    let ity = -(m[4]*tx + m[5]*ty + m[6]*tz);
    let itz = -(m[8]*tx + m[9]*ty + m[10]*tz);
    [
        m[0], m[4], m[8],  0.0,  // R^T col 0
        m[1], m[5], m[9],  0.0,  // R^T col 1
        m[2], m[6], m[10], 0.0,  // R^T col 2
        itx,  ity,  itz,   1.0,
    ]
}

fn mat4_mul_vec4(m: &[f32; 16], v: &[f32; 4]) -> [f32; 4] {
    [
        m[0]*v[0] + m[4]*v[1] + m[8]*v[2]  + m[12]*v[3],
        m[1]*v[0] + m[5]*v[1] + m[9]*v[2]  + m[13]*v[3],
        m[2]*v[0] + m[6]*v[1] + m[10]*v[2] + m[14]*v[3],
        m[3]*v[0] + m[7]*v[1] + m[11]*v[2] + m[15]*v[3],
    ]
}

/// Get texture dimensions from scene data. Returns (width, height).
fn get_texture_dimensions(scene: &W3dScene, texture_name: &str) -> (u32, u32) {
    // Try exact name, then lowercase
    let data = scene.texture_images.get(texture_name)
        .or_else(|| scene.texture_images.get(&texture_name.to_lowercase()));
    if let Some(data) = data {
        if data.len() < 4 { return (256, 256); }
        // Check for JPEG
        if data[0] == 0xFF && data[1] == 0xD8 {
            if let Ok(img) = image::load_from_memory(data) {
                return (img.width(), img.height());
            }
        }
        // Check for PNG
        if data[0] == 0x89 && data[1] == 0x50 {
            if let Ok(img) = image::load_from_memory(data) {
                return (img.width(), img.height());
            }
        }
        // Raw RGBA: width(u32 LE) + height(u32 LE) + pixels
        if data.len() >= 8 {
            let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            if w > 0 && w <= 4096 && h > 0 && h <= 4096 {
                return (w, h);
            }
        }
    }
    (256, 256) // fallback
}

fn build_perspective_f32(fov_deg: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1.0 / (fov_deg.to_radians() * 0.5).tan();
    let nf = 1.0 / (near - far);
    [
        f / aspect, 0.0, 0.0,               0.0,
        0.0,        f,   0.0,               0.0,
        0.0,        0.0, (far + near) * nf, -1.0,
        0.0,        0.0, 2.0 * far * near * nf, 0.0,
    ]
}
