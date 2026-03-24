use std::collections::VecDeque;

use log::debug;

use crate::{
    director::lingo::datum::Datum,
    player::{
        cast_lib::CastMemberRef,
        cast_member::CastMemberType,
        reserve_player_mut,
        DatumRef, DirPlayer, ScriptError,
    },
};

pub struct Shockwave3dMemberHandlers {}

impl Shockwave3dMemberHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        // Clone info and scene data upfront to avoid borrow conflicts with player.alloc_datum
        let (info, scene_data) = {
            let member = player
                .movie
                .cast_manager
                .find_member_by_ref(cast_member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
            let w3d = member.member_type.as_shockwave3d()
                .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
            (w3d.info.clone(), w3d.parsed_scene.clone())
        };

        use crate::director::chunks::w3d::types::W3dNodeType;

        match prop.as_str() {
            // ─── Member-level properties ───
            "directToStage" => Ok(Datum::Int(if info.direct_to_stage { 1 } else { 0 })),
            "preLoad" | "preload" => Ok(Datum::Int(if info.preload { 1 } else { 0 })),
            "duration" => Ok(Datum::Int(info.duration as i32)),

            "regPoint" => {
                let x = player.alloc_datum(Datum::Int(info.reg_point.0));
                let y = player.alloc_datum(Datum::Int(info.reg_point.1));
                Ok(Datum::Point([x, y]))
            }
            "rect" => {
                let r = info.default_rect;
                Ok(Datum::Rect([
                    player.alloc_datum(Datum::Int(r.0)),
                    player.alloc_datum(Datum::Int(r.1)),
                    player.alloc_datum(Datum::Int(r.2)),
                    player.alloc_datum(Datum::Int(r.3)),
                ]))
            }
            "width" => Ok(Datum::Int(info.default_rect.2 - info.default_rect.0)),
            "height" => Ok(Datum::Int(info.default_rect.3 - info.default_rect.1)),

            // ─── Scene collection properties ───
            // These return lists of Shockwave3dObjectRefs, supporting .count and [index]
            "model" | "modelCount" | "modelResource" | "modelResourceCount"
            | "shader" | "shaderCount" | "texture" | "textureCount"
            | "light" | "lightCount" | "camera" | "cameraCount"
            | "group" | "groupCount" | "motion" | "motionCount" => {
                use crate::director::lingo::datum::{Shockwave3dObjectRef, DatumType};
                let collection = prop.trim_end_matches("Count");
                let names: Vec<String> = if let Some(scene) = &scene_data {
                    match collection {
                        "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).map(|n| n.name.clone()).collect(),
                        "modelResource" => scene.model_resources.keys().cloned().collect(),
                        "shader" => scene.shaders.iter().map(|s| s.name.clone()).collect(),
                        "texture" => scene.texture_images.keys().cloned().collect(),
                        "light" => scene.lights.iter().map(|l| l.name.clone()).collect(),
                        "camera" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::View).map(|n| n.name.clone()).collect(),
                        "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).map(|n| n.name.clone()).collect(),
                        "motion" => scene.motions.iter().map(|m| m.name.clone()).collect(),
                        _ => vec![],
                    }
                } else {
                    vec![]
                };
                // If prop ends with "Count", return just the count
                if prop.ends_with("Count") {
                    return Ok(Datum::Int(names.len() as i32));
                }
                // Return a list of Shockwave3dObjectRefs
                let items: VecDeque<_> = names.iter().map(|name| {
                    player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: cast_member_ref.cast_lib,
                        cast_member: cast_member_ref.cast_member,
                        object_type: collection.to_string(),
                        name: name.clone(),
                    }))
                }).collect();
                Ok(Datum::List(DatumType::List, items, false))
            }

            // ─── State ───
            "state" => Ok(Datum::Int(4)), // 4 = loaded
            "percentStreamed" => Ok(Datum::Int(100)),
            "animationEnabled" => Ok(Datum::Int(if info.animation_enabled { 1 } else { 0 })),
            "loop" => Ok(Datum::Int(if info.loops { 1 } else { 0 })),

            // ─── Rendering ───
            "image" => {
                // member("3d").image returns the rendered 3D world as a bitmap.
                let w = (info.default_rect.2 - info.default_rect.0).max(1) as u32;
                let h = (info.default_rect.3 - info.default_rect.1).max(1) as u32;

                // Try cached frame first (from sprite rendering), then offscreen render
                let key = (cast_member_ref.cast_lib, cast_member_ref.cast_member);
                if let Some(&bitmap_ref) = player.w3d_frame_buffers.get(&key) {
                    return Ok(Datum::BitmapRef(bitmap_ref));
                }

                // No cached frame — render offscreen
                let runtime_state = {
                    let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                        .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                    let w3d = member.member_type.as_shockwave3d()
                        .ok_or_else(|| ScriptError::new("Not 3D".to_string()))?;
                    w3d.runtime_state.clone()
                };

                let rgba_data = render_3d_to_rgba(&scene_data, &runtime_state, w, h);

                let mut bitmap = crate::player::bitmap::bitmap::Bitmap::new(
                    w as u16, h as u16, 32, 32, 8,
                    crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
                        crate::player::bitmap::bitmap::get_system_default_palette()
                    ),
                );
                bitmap.data = rgba_data;
                bitmap.use_alpha = true;
                let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                Ok(Datum::BitmapRef(bitmap_ref))
            }
            "backgroundColor" => {
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(50, 50, 50)))
            }
            "ambientColor" => {
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(25, 25, 25)))
            }
            "renderer" | "rendererDeviceList" => Ok(Datum::Symbol("openGL".to_string())),
            "colorBufferDepth" => Ok(Datum::Int(32)),
            "depthBufferDepth" => Ok(Datum::Int(24)),
            "antiAliasingEnabled" => Ok(Datum::Int(0)),
            "streamSize" => Ok(Datum::Int(0)),

            _ => {
                web_sys::console::log_1(&format!("[W3D] Unknown Shockwave3D property: {}", prop).into());
                Err(ScriptError::new(format!(
                    "Cannot get Shockwave3D property '{}'", prop
                )))
            }
        }
    }

    pub fn set_prop(
        _player: &mut DirPlayer,
        _cast_member_ref: &CastMemberRef,
        prop: &String,
        _value: &Datum,
    ) -> Result<(), ScriptError> {
        match prop.as_str() {
            "directToStage" | "preLoad" | "preload" | "loop" | "animationEnabled" => {
                // Accept but don't apply yet
                Ok(())
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] Unknown Shockwave3D set property: {}", prop).into());
                Err(ScriptError::new(format!(
                    "Cannot set Shockwave3D property '{}'", prop
                )))
            }
        }
    }

    // ─── Call handlers for Shockwave3D member methods ───
    // (moved from cast_member_ref.rs to consolidate 3D code)
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "getPropRef" => {
                // member("x").model[1] → getPropRef(#model, 1)
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    let collection = player.get_datum(&args[0]).string_value()?;
                    let index = if args.len() > 1 {
                        player.get_datum(&args[1]).int_value()? as usize
                    } else {
                        1
                    };
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                let obj_name = Self::get_3d_object_name_by_index(scene, &collection, index)
                                    .unwrap_or_default();
                                if !obj_name.is_empty() {
                                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                                    return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                        cast_lib: cast_member_ref.cast_lib,
                                        cast_member: cast_member_ref.cast_member,
                                        object_type: collection,
                                        name: obj_name,
                                    })));
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            "count" => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    if args.is_empty() {
                        return Err(ScriptError::new("count requires 1 argument".to_string()));
                    }
                    let count_of = player.get_datum(&args[0]).string_value()?;
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                let count = Self::get_3d_collection_count(scene, &count_of);
                                return Ok(player.alloc_datum(Datum::Int(count)));
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Int(0)))
                })
            }
            // Shockwave 3D collection accessors & mutators
            "model" | "modelResource" | "shader" | "texture" | "light" | "camera" | "group" | "motion"
            | "resetWorld" | "revertToWorldDefaults"
            | "newTexture" | "newShader" | "newModel" | "newModelResource" | "newLight" | "newCamera" | "newGroup" | "newMotion" | "newMesh"
            | "deleteTexture" | "deleteShader" | "deleteModel" | "deleteModelResource" | "deleteLight" | "deleteCamera" | "deleteGroup" | "deleteMotion"
            | "cloneModelFromCastmember" | "cloneMotionFromCastmember" | "cloneDeep"
            | "loadFile" | "extrude3d" | "getPref" | "setPref"
            | "image" => {
                reserve_player_mut(|player| {
                    let member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    let cast_member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                        .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                    let w3d = cast_member.member_type.as_shockwave3d()
                        .ok_or_else(|| {
                            ScriptError::new(format!(
                                "Cannot call .{}() on non-Shockwave3D member (type: {:?})",
                                handler_name, cast_member.member_type.member_type_id()
                            ))
                        })?;

                    if handler_name == "resetWorld" || handler_name == "revertToWorldDefaults" {
                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            // Reset to initial state from 3DPR data (preserves camera, bg color)
                            w3d.runtime_state = crate::player::cast_member::Shockwave3dRuntimeState::from_info(&w3d.info, w3d.parsed_scene.as_deref());
                        }
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // cloneModelFromCastmember / cloneMotionFromCastmember / cloneDeep
                    if handler_name == "cloneModelFromCastmember" || handler_name == "cloneMotionFromCastmember" || handler_name == "cloneDeep" {
                        let obj_name = if !args.is_empty() {
                            player.get_datum(&args[0]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let source_model_name = if args.len() > 1 {
                            player.get_datum(&args[1]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let source_member_ref = if args.len() > 2 {
                            match player.get_datum(&args[2]) {
                                Datum::CastMember(r) => Some(r.clone()),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let obj_type = if handler_name == "cloneMotionFromCastmember" {
                            "motion"
                        } else {
                            "model"
                        };

                        // Look up source model's shader/transform/resource from source member's scene
                        let identity = [1.0f32,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
                        let (source_shader_name, source_transform, source_resource_name, source_model_resource_name, src_motion_tracks) = if let Some(ref src_ref) = source_member_ref {
                            let src_member = player.movie.cast_manager.find_member_by_ref(src_ref);
                            if let Some(sm) = src_member {
                                if let Some(sw3d) = sm.member_type.as_shockwave3d() {
                                    if let Some(ref scene) = sw3d.parsed_scene {
                                        let node = scene.nodes.iter().find(|n| n.name == source_model_name);
                                        let (sn, st, sr, smr) = if let Some(n) = node {
                                            (n.shader_name.clone(), n.transform, n.resource_name.clone(), n.model_resource_name.clone())
                                        } else {
                                            (String::new(), identity, String::new(), String::new())
                                        };
                                        let motion_tracks = scene.motions.iter()
                                            .max_by_key(|m| m.tracks.len())
                                            .map(|m| m.tracks.clone())
                                            .unwrap_or_default();
                                        (sn, st, sr, smr, motion_tracks)
                                    } else { (String::new(), identity, String::new(), String::new(), vec![]) }
                                } else { (String::new(), identity, String::new(), String::new(), vec![]) }
                            } else { (String::new(), identity, String::new(), String::new(), vec![]) }
                        } else {
                            (String::new(), identity, String::new(), String::new(), vec![])
                        };

                        // Track shader name remapping for -clone suffix creation
                        let mut shader_name_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

                        // Copy source shaders, model resources, meshes, and textures that don't exist in target scene
                        if let Some(ref src_ref) = source_member_ref {
                            let (src_shaders, src_model_resources, src_clod_meshes, src_raw_meshes, src_textures, src_lights, src_light_nodes, src_skeletons) = {
                                let src_member = player.movie.cast_manager.find_member_by_ref(src_ref);
                                let scene = src_member.and_then(|sm| sm.member_type.as_shockwave3d())
                                    .and_then(|sw3d| sw3d.parsed_scene.as_ref());
                                let shaders: Vec<_> = scene.map(|s| s.shaders.clone()).unwrap_or_default();
                                let resources: Vec<_> = scene.map(|s| s.model_resources.iter()
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let meshes: Vec<_> = scene.map(|s| s.clod_meshes.iter()
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let raw: Vec<_> = scene.map(|s| s.raw_meshes.clone()).unwrap_or_default();
                                let textures: Vec<_> = scene.map(|s| s.texture_images.iter()
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let lights: Vec<_> = scene.map(|s| s.lights.clone()).unwrap_or_default();
                                let light_nodes: Vec<_> = scene.map(|s| s.nodes.iter()
                                    .filter(|n| n.node_type == crate::director::chunks::w3d::types::W3dNodeType::Light)
                                    .cloned().collect()).unwrap_or_default();
                                let skeletons: Vec<_> = scene.map(|s| s.skeletons.clone()).unwrap_or_default();
                                (shaders, resources, meshes, raw, textures, lights, light_nodes, skeletons)
                            };

                            debug!(
                                "[W3D-CLONE] {}(\"{}\") src_model=\"{}\" src_member={:?}: \
                                 {} shaders, {} model_resources, {} clod_meshes(keys={:?}), {} raw_meshes(names={:?}), {} textures, \
                                 src_res=\"{}\", src_mres=\"{}\"",
                                handler_name, obj_name, source_model_name, source_member_ref,
                                src_shaders.len(), src_model_resources.len(),
                                src_clod_meshes.len(), src_clod_meshes.iter().map(|(k,_)| k.clone()).collect::<Vec<String>>(),
                                src_raw_meshes.len(), src_raw_meshes.iter().map(|m| m.name.clone()).collect::<Vec<String>>(),
                                src_textures.len(),
                                source_resource_name, source_model_resource_name,
                            );

                            // Namespace prefix to avoid name collisions
                            let ns = format!("{}_", obj_name);

                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        // Shaders: reuse existing by name (Director behavior).
                                        for shader in &src_shaders {
                                            if !scene.shaders.iter().any(|s| s.name == shader.name) {
                                                scene.shaders.push(shader.clone());
                                            }
                                        }
                                        // Model resources: namespace to prevent collisions
                                        for (res_name, res_info) in &src_model_resources {
                                            let new_name = format!("{}{}", ns, res_name);
                                            if !scene.model_resources.contains_key(&new_name) {
                                                let mut cloned_res = res_info.clone();
                                                for binding in &mut cloned_res.shader_bindings {
                                                    for mesh_shader in &mut binding.mesh_bindings {
                                                        if let Some(new_name) = shader_name_map.get(mesh_shader.as_str()) {
                                                            *mesh_shader = new_name.clone();
                                                        }
                                                    }
                                                }
                                                scene.model_resources.insert(new_name, cloned_res);
                                            }
                                        }
                                        // CLOD meshes: namespace to prevent collisions
                                        for (mesh_name, mesh_data) in &src_clod_meshes {
                                            let new_name = format!("{}{}", ns, mesh_name);
                                            if !scene.clod_meshes.contains_key(&new_name) {
                                                scene.clod_meshes.insert(new_name, mesh_data.clone());
                                            }
                                        }
                                        // Textures: keep original names
                                        for (tex_name, tex_data) in &src_textures {
                                            if !scene.texture_images.contains_key(tex_name) {
                                                scene.texture_images.insert(tex_name.clone(), tex_data.clone());
                                                scene.texture_content_version += 1;
                                            }
                                        }
                                        // Raw meshes: namespace to prevent collisions
                                        for raw_mesh in &src_raw_meshes {
                                            let new_name = format!("{}{}", ns, raw_mesh.name);
                                            if !scene.raw_meshes.iter().any(|m| m.name == new_name) {
                                                let mut cloned = raw_mesh.clone();
                                                cloned.name = new_name;
                                                scene.raw_meshes.push(cloned);
                                            }
                                        }
                                        // Copy lights from source scene
                                        for light in &src_lights {
                                            if !scene.lights.iter().any(|l| l.name == light.name) {
                                                scene.lights.push(light.clone());
                                            }
                                        }
                                        // Copy light nodes from source scene
                                        for node in &src_light_nodes {
                                            if !scene.nodes.iter().any(|n| n.name == node.name) {
                                                scene.nodes.push(node.clone());
                                            }
                                        }
                                        // Copy skeletons
                                        let skel_key = if !source_model_resource_name.is_empty() {
                                            format!("{}{}", ns, source_model_resource_name)
                                        } else if !source_resource_name.is_empty() {
                                            format!("{}{}", ns, source_resource_name)
                                        } else { String::new() };
                                        for skeleton in &src_skeletons {
                                            if !skel_key.is_empty() && !scene.skeletons.iter().any(|s| s.name == skel_key) {
                                                let mut cloned = skeleton.clone();
                                                cloned.name = skel_key.clone();
                                                scene.skeletons.push(cloned);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Add the cloned object to the target scene
                        let ns = format!("{}_", obj_name);
                        let mapped_resource = if !source_resource_name.is_empty() {
                            format!("{}{}", ns, source_resource_name)
                        } else { source_resource_name.clone() };
                        let mapped_model_resource = if !source_model_resource_name.is_empty() {
                            format!("{}{}", ns, source_model_resource_name)
                        } else { source_model_resource_name.clone() };

                        let effective_shader_name = shader_name_map.get(&source_shader_name)
                            .cloned()
                            .unwrap_or(source_shader_name);

                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    use crate::director::chunks::w3d::types::*;
                                    if obj_type == "model" {
                                        scene.nodes.push(W3dNode {
                                            name: obj_name.clone(), node_type: W3dNodeType::Model,
                                            parent_name: "World".to_string(),
                                            resource_name: mapped_resource,
                                            model_resource_name: mapped_model_resource,
                                            shader_name: effective_shader_name,
                                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                            screen_width: 640, screen_height: 480,
                                            transform: source_transform,
                                        });
                                    } else if obj_type == "motion" {
                                        scene.motions.push(W3dMotion {
                                            name: obj_name.clone(),
                                            tracks: src_motion_tracks.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        use crate::director::lingo::datum::Shockwave3dObjectRef;
                        return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib,
                            cast_member: member_ref.cast_member,
                            object_type: obj_type.to_string(),
                            name: obj_name,
                        })));
                    }

                    // newTexture/newShader/newModel/etc. — create and return a ref
                    if handler_name.starts_with("new") || handler_name.starts_with("delete") {
                        let obj_type = match handler_name.as_str() {
                            "newTexture" | "deleteTexture" => "texture",
                            "newShader" | "deleteShader" => "shader",
                            "newModel" | "deleteModel" => "model",
                            "newModelResource" | "deleteModelResource" | "newMesh" => "modelResource",
                            "newLight" | "deleteLight" => "light",
                            "newCamera" | "deleteCamera" => "camera",
                            "newGroup" | "deleteGroup" => "group",
                            "newMotion" | "deleteMotion" => "motion",
                            _ => "unknown",
                        };
                        let obj_name = if !args.is_empty() {
                            player.get_datum(&args[0]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };

                        if handler_name.starts_with("delete") {
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        match obj_type {
                                            "model" | "group" | "camera" | "light" => {
                                                scene.nodes.retain(|n| n.name != obj_name);
                                            }
                                            "shader" => {
                                                scene.shaders.retain(|s| s.name != obj_name);
                                            }
                                            "motion" => {
                                                scene.motions.retain(|m| m.name != obj_name);
                                            }
                                            "texture" => {
                                                scene.texture_images.remove(&obj_name);
                                                scene.texture_content_version += 1;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            return Ok(player.alloc_datum(Datum::Void));
                        }

                        // Pre-read args for newMesh before mutable borrow
                        let mesh_num_faces = if handler_name == "newMesh" && args.len() >= 2 {
                            player.get_datum(&args[1]).int_value().unwrap_or(0) as u32
                        } else { 0 };

                        // Add to parsed scene
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    use crate::director::chunks::w3d::types::*;
                                    let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
                                    match obj_type {
                                        "model" => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::Model,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        "group" => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::Group,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        "camera" => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::View,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,500.0,1.0],
                                            });
                                        }
                                        "light" => {
                                            scene.lights.push(W3dLight {
                                                name: obj_name.clone(),
                                                light_type: W3dLightType::Point,
                                                color: [1.0, 1.0, 1.0],
                                                attenuation: [1.0, 0.0, 0.0],
                                                spot_angle: 30.0,
                                                enabled: true,
                                            });
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::Light,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        "shader" => {
                                            scene.shaders.push(W3dShader {
                                                name: obj_name.clone(),
                                                ..Default::default()
                                            });
                                        }
                                        "modelResource" => {
                                            let num_faces = mesh_num_faces;
                                            let mut mesh_info = ClodMeshInfo::default();
                                            mesh_info.num_faces = num_faces;
                                            scene.model_resources.insert(obj_name.clone(), ModelResourceInfo {
                                                name: obj_name.clone(),
                                                mesh_infos: vec![mesh_info],
                                                max_resolution: 0,
                                                shading_count: 0,
                                                shader_bindings: vec![],
                                                pos_iq: 0.0, norm_iq: 0.0, normal_crease: 0.0,
                                                tc_iq: 0.0, diff_iq: 0.0, spec_iq: 0.0,
                                                has_distal_edge_merge: false,
                                                has_neighbor_mesh: false,
                                                uv_gen_mode: None,
                                                sync_table: None,
                                                distal_edge_merges: None,
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        // For newTexture(name, #fromImageObject/#fromCastMember, source)
                        if handler_name == "newTexture" && args.len() >= 3 {
                            let tex_type = player.get_datum(&args[1]).string_value().unwrap_or_default();
                            if tex_type == "fromCastMember" {
                                let source_member_ref = match player.get_datum(&args[2]) {
                                    Datum::CastMember(r) => Some(r.clone()),
                                    _ => None,
                                };
                                if let Some(src_ref) = source_member_ref {
                                    let rgba_data = {
                                        let src_member = player.movie.cast_manager.find_member_by_ref(&src_ref);
                                        src_member.and_then(|m| {
                                            match &m.member_type {
                                                CastMemberType::Bitmap(bmp_member) => {
                                                    let bmp = player.bitmap_manager.get_bitmap(bmp_member.image_ref)?;
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
                                                    web_sys::console::log_1(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): {}x{} from member {}:{} '{}'",
                                                        obj_name, w, h, src_ref.cast_lib, src_ref.cast_member, m.name
                                                    ).into());
                                                    Some((w, h, rgba))
                                                }
                                                _ => {
                                                    web_sys::console::log_1(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): member {}:{} '{}' is {} not Bitmap",
                                                        obj_name, src_ref.cast_lib, src_ref.cast_member,
                                                        m.name, m.member_type.type_string()
                                                    ).into());
                                                    None
                                                }
                                            }
                                        })
                                    };
                                    if let Some((w, h, rgba)) = rgba_data {
                                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref);
                                        if let Some(member) = member {
                                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                                if let Some(scene) = w3d.scene_mut() {
                                                    let mut tex_data = Vec::with_capacity(8 + rgba.len());
                                                    tex_data.extend_from_slice(&(w as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&(h as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&rgba);
                                                    scene.texture_images.insert(obj_name.clone(), tex_data);
                                                    scene.texture_content_version += 1;
                                                    web_sys::console::log_1(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): stored {}x{} RGBA",
                                                        obj_name, w, h
                                                    ).into());
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if tex_type == "fromImageObject" {
                                if let Ok(bitmap_ref) = player.get_datum(&args[2]).to_bitmap_ref() {
                                    let rgba_data = if let Some(bmp) = player.bitmap_manager.get_bitmap(*bitmap_ref) {
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

                                    if let Some((w, h, rgba)) = rgba_data {
                                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref);
                                        if let Some(member) = member {
                                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                                if let Some(scene) = w3d.scene_mut() {
                                                    let mut tex_data = Vec::with_capacity(8 + rgba.len());
                                                    tex_data.extend_from_slice(&(w as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&(h as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&rgba);
                                                    scene.texture_images.insert(obj_name.clone(), tex_data);
                                                    scene.texture_content_version += 1;
                                                    web_sys::console::log_1(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromImageObject): stored {}x{} RGBA",
                                                        obj_name, w, h
                                                    ).into());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        use crate::director::lingo::datum::Shockwave3dObjectRef;
                        return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib,
                            cast_member: member_ref.cast_member,
                            object_type: obj_type.to_string(),
                            name: obj_name,
                        })));
                    }

                    // image — return the rendered 3D world as a bitmap ref
                    if handler_name == "image" {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // loadFile, extrude3d, getPref, setPref
                    if handler_name == "loadFile" || handler_name == "extrude3d"
                        || handler_name == "getPref" || handler_name == "setPref" {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // If no parsed scene exists, create a minimal empty scene
                    if w3d.parsed_scene.is_none() {
                        use crate::director::chunks::w3d::types::*;
                        use std::collections::HashMap;
                        let mut empty_scene = W3dScene {
                            materials: Vec::new(), shaders: Vec::new(), nodes: Vec::new(),
                            lights: Vec::new(), texture_images: HashMap::new(), texture_infos: Vec::new(),
                            skeletons: Vec::new(), motions: Vec::new(), model_resources: HashMap::new(),
                            clod_meshes: HashMap::new(), raw_meshes: Vec::new(),
                            texture_content_version: 0,
                        };
                        empty_scene.nodes.push(W3dNode {
                            name: "World".to_string(),
                            node_type: W3dNodeType::Group,
                            parent_name: String::new(),
                            resource_name: String::new(),
                            model_resource_name: String::new(),
                            shader_name: String::new(),
                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                            screen_width: player.movie.rect.right as i32,
                            screen_height: player.movie.rect.bottom as i32,
                            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
                        });
                        empty_scene.nodes.push(W3dNode {
                            name: "DefaultView".to_string(),
                            node_type: W3dNodeType::View,
                            parent_name: "World".to_string(),
                            resource_name: String::new(),
                            model_resource_name: String::new(),
                            shader_name: String::new(),
                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                            screen_width: player.movie.rect.right as i32,
                            screen_height: player.movie.rect.bottom as i32,
                            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,500.0,1.0],
                        });
                        empty_scene.shaders.push(W3dShader {
                            name: "DefaultShader".to_string(),
                            ..Default::default()
                        });
                        let member_mut = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d_mut) = member_mut.member_type.as_shockwave3d_mut() {
                            w3d_mut.parsed_scene = Some(std::rc::Rc::new(empty_scene));
                        }
                    }
                    // Re-fetch after potential mutation
                    let cast_member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                        .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                    let w3d = cast_member.member_type.as_shockwave3d()
                        .ok_or_else(|| ScriptError::new("Not a 3D member".to_string()))?;
                    let scene = w3d.parsed_scene.as_ref().unwrap();

                    // Resolve name from argument (string or int index)
                    let obj_name = if args.is_empty() {
                        let count = Self::get_3d_collection_count(scene, handler_name);
                        return Ok(player.alloc_datum(Datum::Int(count)));
                    } else {
                        let arg = player.get_datum(&args[0]).clone();
                        match arg {
                            Datum::String(s) => s,
                            Datum::Int(idx) => {
                                Self::get_3d_object_name_by_index(scene, handler_name, idx as usize)
                                    .unwrap_or_default()
                            }
                            _ => arg.string_value().unwrap_or_default(),
                        }
                    };

                    if obj_name.is_empty() {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib,
                        cast_member: member_ref.cast_member,
                        object_type: handler_name.to_string(),
                        name: obj_name,
                    })))
                })
            }
            "modelsUnderRay" => {
                reserve_player_mut(|player| {
                    let member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    if args.len() < 2 {
                        return Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, VecDeque::new(), false,
                        )));
                    }
                    let origin = player.get_datum(&args[0]).to_vector()?;
                    let direction = player.get_datum(&args[1]).to_vector()?;
                    let max_models = if args.len() > 2 { player.get_datum(&args[2]).int_value().unwrap_or(100) } else { 100 };
                    let detailed = if args.len() > 3 {
                        player.get_datum(&args[3]).string_value().unwrap_or_default() == "detailed"
                    } else { false };

                    let scene = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                        member.and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.parsed_scene.clone())
                    };

                    // Get runtime node transforms and build exclusion set for invisible/detached models
                    let (node_transforms, excluded_nodes) = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                        if let Some(w3d) = member.and_then(|m| m.member_type.as_shockwave3d()) {
                            let transforms = w3d.runtime_state.node_transforms.clone();
                            let mut excluded = std::collections::HashSet::new();
                            for (name, &visible) in &w3d.runtime_state.node_visibility {
                                if !visible { excluded.insert(name.clone()); }
                            }
                            for name in &w3d.runtime_state.detached_nodes {
                                excluded.insert(name.clone());
                            }
                            if let Some(ref scene) = w3d.parsed_scene {
                                for node in &scene.nodes {
                                    if excluded.contains(&node.name) { continue; }
                                    let mut parent = &node.parent_name;
                                    for _ in 0..10 {
                                        if parent.is_empty() {
                                            excluded.insert(node.name.clone());
                                            break;
                                        }
                                        if *parent == "World" { break; }
                                        if w3d.runtime_state.detached_nodes.contains(parent.as_str()) {
                                            excluded.insert(node.name.clone());
                                            break;
                                        }
                                        if let Some(pn) = scene.nodes.iter().find(|n| n.name == *parent) {
                                            parent = &pn.parent_name;
                                        } else { break; }
                                    }
                                }
                            }
                            (Some(transforms), excluded)
                        } else {
                            (None, std::collections::HashSet::new())
                        }
                    };

                    let mut results = Vec::new();
                    if let Some(scene) = scene {
                        use crate::director::chunks::w3d::raycast::{Ray, raycast_scene_multi};
                        let ray = Ray {
                            origin: [origin[0] as f32, origin[1] as f32, origin[2] as f32],
                            direction: [direction[0] as f32, direction[1] as f32, direction[2] as f32],
                        };
                        let excluded_ref = if excluded_nodes.is_empty() { None } else { Some(&excluded_nodes) };
                        let hits = raycast_scene_multi(
                            &ray, &scene, 100000.0, max_models as usize,
                            node_transforms.as_ref(),
                            excluded_ref,
                        );
                        for hit in &hits {
                            if detailed {
                                let model_key = player.alloc_datum(Datum::Symbol("model".to_string()));
                                let model_val = player.alloc_datum(Datum::Shockwave3dObjectRef(
                                    crate::director::lingo::datum::Shockwave3dObjectRef {
                                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                        object_type: "model".to_string(),
                                        name: hit.model_name.clone(),
                                    }
                                ));
                                let dist_key = player.alloc_datum(Datum::Symbol("distance".to_string()));
                                let dist_val = player.alloc_datum(Datum::Float(hit.distance as f64));
                                let pos_key = player.alloc_datum(Datum::Symbol("isectPosition".to_string()));
                                let pos_val = player.alloc_datum(Datum::Vector([
                                    hit.position[0] as f64, hit.position[1] as f64, hit.position[2] as f64,
                                ]));
                                let norm_key = player.alloc_datum(Datum::Symbol("isectNormal".to_string()));
                                let norm_val = player.alloc_datum(Datum::Vector([
                                    hit.normal[0] as f64, hit.normal[1] as f64, hit.normal[2] as f64,
                                ]));
                                let mesh_key = player.alloc_datum(Datum::Symbol("meshID".to_string()));
                                let mesh_val = player.alloc_datum(Datum::Int(1));
                                let face_key = player.alloc_datum(Datum::Symbol("faceID".to_string()));
                                let face_val = player.alloc_datum(Datum::Int(hit.face_index as i32));

                                let hit_proplist = player.alloc_datum(Datum::PropList(VecDeque::from(vec![
                                    (model_key, model_val), (dist_key, dist_val),
                                    (pos_key, pos_val), (norm_key, norm_val),
                                    (mesh_key, mesh_val), (face_key, face_val),
                                ]), false));
                                results.push(hit_proplist);
                            } else {
                                results.push(player.alloc_datum(Datum::Shockwave3dObjectRef(
                                    crate::director::lingo::datum::Shockwave3dObjectRef {
                                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                        object_type: "model".to_string(),
                                        name: hit.model_name.clone(),
                                    }
                                )));
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List, VecDeque::from(results), false,
                    )))
                })
            }
            "modelsUnderLoc" | "modelUnderLoc" => {
                reserve_player_mut(|player| {
                    if handler_name == "modelUnderLoc" {
                        Ok(player.alloc_datum(Datum::Void))
                    } else {
                        Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, VecDeque::new(), false,
                        )))
                    }
                })
            }
            _ => Err(ScriptError::new(format!(
                "No Shockwave3D member handler for '{}'", handler_name
            ))),
        }
    }

    pub fn get_3d_collection_count(scene: &crate::director::chunks::w3d::types::W3dScene, collection: &str) -> i32 {
        use crate::director::chunks::w3d::types::W3dNodeType;
        match collection {
            "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).count() as i32,
            "modelResource" => scene.model_resources.len() as i32,
            "shader" => scene.shaders.len() as i32,
            "texture" => scene.texture_images.len() as i32,
            "light" => scene.lights.len() as i32,
            "camera" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::View).count() as i32,
            "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).count() as i32,
            "motion" => scene.motions.len() as i32,
            _ => 0,
        }
    }

    pub fn get_3d_object_name_by_index(scene: &crate::director::chunks::w3d::types::W3dScene, collection: &str, index: usize) -> Option<String> {
        use crate::director::chunks::w3d::types::W3dNodeType;
        if index == 0 { return None; }
        let idx = index - 1; // 1-based to 0-based
        match collection {
            "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).nth(idx).map(|n| n.name.clone()),
            "modelResource" => scene.model_resources.keys().nth(idx).cloned(),
            "shader" => scene.shaders.get(idx).map(|s| s.name.clone()),
            "texture" => scene.texture_images.keys().nth(idx).cloned(),
            "light" => scene.lights.get(idx).map(|l| l.name.clone()),
            "camera" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::View).nth(idx).map(|n| n.name.clone()),
            "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).nth(idx).map(|n| n.name.clone()),
            "motion" => scene.motions.get(idx).map(|m| m.name.clone()),
            _ => None,
        }
    }
}

/// Render a Shockwave3D scene to RGBA pixels using a temporary offscreen WebGL2 context.
fn render_3d_to_rgba(
    scene_data: &Option<std::rc::Rc<crate::director::chunks::w3d::types::W3dScene>>,
    runtime_state: &crate::player::cast_member::Shockwave3dRuntimeState,
    width: u32,
    height: u32,
) -> Vec<u8> {
    use wasm_bindgen::JsCast;
    use web_sys::WebGl2RenderingContext;

    let scene = match scene_data {
        Some(s) => s,
        None => return vec![128u8; (width * height * 4) as usize], // grey fallback
    };

    // Create offscreen canvas
    let document = match web_sys::window().and_then(|w| w.document()) {
        Some(d) => d,
        None => return vec![128u8; (width * height * 4) as usize],
    };
    let canvas = match document.create_element("canvas") {
        Ok(el) => el,
        Err(_) => return vec![128u8; (width * height * 4) as usize],
    };
    let canvas: web_sys::HtmlCanvasElement = match canvas.dyn_into() {
        Ok(c) => c,
        Err(_) => return vec![128u8; (width * height * 4) as usize],
    };
    canvas.set_width(width);
    canvas.set_height(height);

    let mut context_attrs = web_sys::WebGlContextAttributes::new();
    context_attrs.alpha(true);
    context_attrs.depth(true);
    context_attrs.preserve_drawing_buffer(true); // needed for readPixels

    let gl: WebGl2RenderingContext = match canvas.get_context_with_context_options("webgl2", &context_attrs) {
        Ok(Some(ctx)) => match ctx.dyn_into() {
            Ok(gl) => gl,
            Err(_) => return vec![128u8; (width * height * 4) as usize],
        },
        _ => return vec![128u8; (width * height * 4) as usize],
    };

    let context = match crate::rendering_gpu::webgl2::context::WebGL2Context::new(gl.clone()) {
        Ok(c) => c,
        Err(_) => return vec![128u8; (width * height * 4) as usize],
    };

    // Render directly to the default framebuffer (the offscreen canvas), not to FBO
    let mut renderer = crate::rendering_gpu::webgl2::scene3d::Scene3dRenderer::new();
    match renderer.render_to_default_framebuffer(&context, (0, 0), scene, width, height, Some(runtime_state)) {
        Ok(_) => {}
        Err(e) => {
            web_sys::console::log_1(&format!("[W3D] render_3d_to_rgba failed: {:?}", e).into());
            return vec![200u8; (width * height * 4) as usize];
        }
    }

    // Read pixels from the default framebuffer
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let _ = gl.read_pixels_with_opt_u8_array(
        0, 0, width as i32, height as i32,
        WebGl2RenderingContext::RGBA,
        WebGl2RenderingContext::UNSIGNED_BYTE,
        Some(&mut pixels),
    );

    // Return pixels directly (no flip needed — Director bitmaps are top-to-bottom
    // which matches WebGL's bottom-to-top readPixels when used as a texture source)
    pixels
}
