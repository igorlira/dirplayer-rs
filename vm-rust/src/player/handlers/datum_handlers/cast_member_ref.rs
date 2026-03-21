use std::collections::VecDeque;

use log::{warn, debug};

use crate::{
    director::{
        enums::{ScriptType, ShapeType},
        lingo::datum::{datum_bool, Datum, DatumType},
    },
    js_api::JsApi,
    player::{
        cast_lib::CastMemberRef,
        cast_member::{BitmapMember, CastMember, CastMemberType, CastMemberTypeId, TextMember},
        handlers::types::TypeUtils,
        reserve_player_mut, reserve_player_ref, DatumRef, DirPlayer, ScriptError,
        sprite::ColorRef,
    },
};

use super::cast_member::{
    bitmap::BitmapMemberHandlers, button::ButtonMemberHandlers, field::FieldMemberHandlers,
    film_loop::FilmLoopMemberHandlers, font::FontMemberHandlers, shockwave3d::Shockwave3dMemberHandlers,
    sound::SoundMemberHandlers, text::TextMemberHandlers, palette::PaletteMemberHandlers,
};

pub struct CastMemberRefHandlers {}

pub fn borrow_member_mut<T1, F1, T2, F2>(member_ref: &CastMemberRef, player_f: F2, f: F1) -> T1
where
    F1: FnOnce(&mut CastMember, T2) -> T1,
    F2: FnOnce(&mut DirPlayer) -> T2,
{
    reserve_player_mut(|player| {
        let arg = player_f(player);
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(&member_ref)
            .unwrap();
        f(member, arg)
    })
}

fn get_text_member_line_height(text_data: &TextMember) -> u16 {
    return text_data.font_size + 3; // TODO: Implement text line height
}

impl CastMemberRefHandlers {
    pub fn get_cast_slot_number(cast_lib: u32, cast_member: u32) -> u32 {
        (cast_lib << 16) | (cast_member & 0xFFFF)
    }

    pub fn member_ref_from_slot_number(slot_number: u32) -> CastMemberRef {
        CastMemberRef {
            cast_lib: (slot_number >> 16) as i32,
            cast_member: (slot_number & 0xFFFF) as i32,
        }
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "duplicate" => Self::duplicate(datum, args),
            "erase" => Self::erase(datum, args),
            "charPosToLoc" => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call charPosToLoc on non-cast-member".to_string(),
                            ))
                        }
                    };
                    let cast_member = player
                        .movie
                        .cast_manager
                        .find_member_by_ref(&cast_member_ref)
                        .ok_or_else(|| ScriptError::new("charPosToLoc: member not found".to_string()))?;
                    let char_pos = player.get_datum(&args[0]).int_value()? as u16;
                    let char_width: i32 = 7;

                    let (text, line_height) = if let Some(text_data) = cast_member.member_type.as_text() {
                        (text_data.text.clone(), get_text_member_line_height(&text_data) as i32)
                    } else if let Some(field_data) = cast_member.member_type.as_field() {
                        let lh = if field_data.fixed_line_space > 0 {
                            field_data.fixed_line_space as i32
                        } else {
                            field_data.font_size as i32 + 4
                        };
                        (field_data.text.clone(), lh)
                    } else {
                        return Err(ScriptError::new("charPosToLoc: member is not a text or field member".to_string()));
                    };

                    let (x, y) = if text.is_empty() || char_pos <= 0 {
                        (0, 0)
                    } else if char_pos > text.len() as u16 {
                        (char_width * text.len() as i32, line_height)
                    } else {
                        (char_width * (char_pos - 1) as i32, line_height)
                    };

                    let x_ref = player.alloc_datum(Datum::Int(x));
                    let y_ref = player.alloc_datum(Datum::Int(y));
                    Ok(player.alloc_datum(Datum::Point([x_ref, y_ref])))
                })
            }
            "getProp" => {
                let result_ref = reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call getProp on non-cast-member".to_string(),
                            ))
                        }
                    };
                    let prop = player.get_datum(&args[0]).string_value()?;
                    let result = Self::get_prop(player, &cast_member_ref, &prop)?;
                    Ok(player.alloc_datum(result))
                })?;
                if args.len() > 1 {
                    reserve_player_mut(|player| {
                        TypeUtils::get_sub_prop(&result_ref, &args[1], player)
                    })
                } else {
                    Ok(result_ref)
                }
            }
            "getPropRef" => {
                // For Shockwave3D members: member.model[1] → getPropRef(#model, 1)
                // Check if this is a 3D member first, otherwise fall through to member type handlers
                let is_3d = reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Ok(false),
                    };
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    Ok(member.map_or(false, |m| m.member_type.as_shockwave3d().is_some()))
                })?;
                if is_3d {
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
                } else {
                    // Non-3D members: fall through to member type handlers (text, field, etc.)
                    // If the handler isn't supported, return VOID gracefully
                    Self::call_member_type(datum, handler_name, args)
                        .or_else(|_| reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Void))))
                }
            }
            "count" => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call count on non-cast-member".to_string(),
                            ))
                        }
                    };
                    
                    if args.is_empty() {
                        return Err(ScriptError::new("count requires 1 argument".to_string()));
                    }
                    
                    let count_of = player.get_datum(&args[0]).string_value()?;

                    // For Shockwave3D members, count of 3D collections (model, texture, shader, etc.)
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                let count = Self::get_3d_collection_count(scene, &count_of);
                                if count >= 0 {
                                    return Ok(player.alloc_datum(Datum::Int(count)));
                                }
                            }
                        }
                    }

                    // Try to get the member's text
                    // First try "text" property, then fallback to "previewText" for Font members
                    let text = match Self::get_prop(player, &cast_member_ref, &"text".to_string()) {
                        Ok(datum) => datum.string_value()?,
                        Err(_) => {
                            // Try previewText for Font members
                            match Self::get_prop(player, &cast_member_ref, &"previewText".to_string()) {
                                Ok(datum) => datum.string_value()?,
                                Err(_) => {
                                    return Err(ScriptError::new(format!(
                                        "Member type does not support count operation"
                                    )));
                                }
                            }
                        }
                    };
                    
                    let delimiter = player.movie.item_delimiter;
                    let count = crate::player::handlers::datum_handlers::string_chunk::StringChunkUtils::resolve_chunk_count(
                        &text,
                        crate::director::lingo::datum::StringChunkType::try_from_str(&count_of)
                            .ok_or_else(|| ScriptError::new(format!("Invalid string chunk type: {}", count_of)))?,
                        delimiter,
                    )?;
                    Ok(player.alloc_datum(Datum::Int(count as i32)))
                })
            }
            // Shockwave 3D collection accessors: member("x").model("name"), member("x").shader(1), etc.
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
                            w3d.runtime_state = crate::player::cast_member::Shockwave3dRuntimeState::from_info(&w3d.info);
                        }
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // cloneModelFromCastmember / cloneMotionFromCastmember / cloneDeep
                    // Returns a model/motion ref in this 3D world
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
                        // Also pre-read motion tracks for cloneMotionFromCastmember (before mutable borrow)
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
                                        // For cloneMotionFromCastmember: source_model_name is the MODEL name
                                        // (not motion name). Merge tracks from ALL motions in the source
                                        // (IFX files may split bone tracks across multiple MOTION_BLOCKs).
                                        let motion_tracks: Vec<_> = scene.motions.iter()
                                            .flat_map(|m| m.tracks.iter().cloned())
                                            .collect();
                                        (sn, st, sr, smr, motion_tracks)
                                    } else { (String::new(), identity, String::new(), String::new(), vec![]) }
                                } else { (String::new(), identity, String::new(), String::new(), vec![]) }
                            } else { (String::new(), identity, String::new(), String::new(), vec![]) }
                        } else {
                            (String::new(), identity, String::new(), String::new(), vec![])
                        };

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

                            web_sys::console::log_1(&format!(
                                "[W3D-CLONE] {}(\"{}\") src_model=\"{}\" src_member={:?}: \
                                 {} shaders, {} model_resources, {} clod_meshes(keys={:?}), {} raw_meshes(names={:?}), {} textures, \
                                 src_res=\"{}\", src_mres=\"{}\"",
                                handler_name, obj_name, source_model_name, source_member_ref,
                                src_shaders.len(), src_model_resources.len(),
                                src_clod_meshes.len(), src_clod_meshes.iter().map(|(k,_)| k.clone()).collect::<Vec<String>>(),
                                src_raw_meshes.len(), src_raw_meshes.iter().map(|m| m.name.clone()).collect::<Vec<String>>(),
                                src_textures.len(),
                                source_resource_name, source_model_resource_name,
                            ).into());

                            // Namespace prefix to avoid name collisions between
                            // different source members that share resource names like "Group01"
                            let ns = format!("{}_", obj_name);

                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        // Shaders: keep original names (shared safely, game creates per-instance via CloneShader)
                                        for shader in &src_shaders {
                                            if !scene.shaders.iter().any(|s| s.name == shader.name) {
                                                scene.shaders.push(shader.clone());
                                            }
                                        }
                                        // Model resources: namespace to prevent collisions
                                        for (res_name, res_info) in &src_model_resources {
                                            let new_name = format!("{}{}", ns, res_name);
                                            if !scene.model_resources.contains_key(&new_name) {
                                                scene.model_resources.insert(new_name, res_info.clone());
                                            }
                                        }
                                        // CLOD meshes: namespace to prevent collisions
                                        for (mesh_name, mesh_data) in &src_clod_meshes {
                                            let new_name = format!("{}{}", ns, mesh_name);
                                            if !scene.clod_meshes.contains_key(&new_name) {
                                                scene.clod_meshes.insert(new_name, mesh_data.clone());
                                            }
                                        }
                                        // Textures: keep original names (typically unique per cast member)
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
                                        // Copy skeletons — use namespaced RESOURCE names as skeleton key
                                        // so the renderer's lookup by model_resource_name finds them.
                                        // The renderer does: skeletons.find(|s| s.name == resource_name)
                                        // where resource_name = mapped_model_resource or mapped_resource.
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
                                                break; // only need one skeleton per resource
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Add the cloned object to the target scene
                        // Map resource names with namespace prefix to avoid collisions
                        let ns = format!("{}_", obj_name);
                        let mapped_resource = if !source_resource_name.is_empty() {
                            format!("{}{}", ns, source_resource_name)
                        } else { source_resource_name.clone() };
                        let mapped_model_resource = if !source_model_resource_name.is_empty() {
                            format!("{}{}", ns, source_model_resource_name)
                        } else { source_model_resource_name.clone() };

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
                                            shader_name: source_shader_name,
                                            near_plane: 1.0, far_plane: 10000.0, fov: 45.0,
                                            screen_width: 640, screen_height: 480,
                                            transform: source_transform,
                                        });
                                    } else if obj_type == "motion" {
                                        // src_motion_tracks was pre-read before mutable borrow
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
                        // Get name from first arg
                        let obj_name = if !args.is_empty() {
                            player.get_datum(&args[0]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };

                        if handler_name.starts_with("delete") {
                            // Remove from parsed scene
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
                                                near_plane: 1.0, far_plane: 10000.0, fov: 45.0,
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
                                                near_plane: 1.0, far_plane: 10000.0, fov: 45.0,
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
                                                near_plane: 1.0, far_plane: 10000.0, fov: 45.0,
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
                                                spot_angle: 45.0,
                                                enabled: true,
                                            });
                                            // Also add as a node so it can be transformed
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::Light,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 45.0,
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

                        // For newTexture(name, #fromImageObject/#fromCastMember, source) — store bitmap data
                        // in the W3D scene's texture_images so the renderer can use it
                        if handler_name == "newTexture" && args.len() >= 3 {
                            let tex_type = player.get_datum(&args[1]).string_value().unwrap_or_default();
                            if tex_type == "fromCastMember" {
                                // newTexture("name", #fromCastMember, member(...))
                                // Get the bitmap from the cast member
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
                                // Get the bitmap data from arg[2]
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
                                                    // Store raw RGBA with width/height prefix
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
                        // Return void for now — would need to render to bitmap
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // loadFile, extrude3d, getPref, setPref
                    if handler_name == "loadFile" || handler_name == "extrude3d"
                        || handler_name == "getPref" || handler_name == "setPref" {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // If no parsed scene exists, create a minimal empty scene
                    // (e.g., after resetWorld or for empty 3D members used as runtime containers)
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
                            near_plane: 1.0, far_plane: 10000.0, fov: 45.0,
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
                            near_plane: 1.0, far_plane: 10000.0, fov: 45.0,
                            screen_width: player.movie.rect.right as i32,
                            screen_height: player.movie.rect.bottom as i32,
                            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,500.0,1.0],
                        });
                        // Add DefaultShader
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
                        // No args: return the count
                        let count = Self::get_3d_collection_count(scene, handler_name);
                        return Ok(player.alloc_datum(Datum::Int(count)));
                    } else {
                        let arg = player.get_datum(&args[0]).clone();
                        match arg {
                            Datum::String(s) => s,
                            Datum::Int(idx) => {
                                // 1-based index
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
                // modelsUnderRay(locationVector, directionVector, maxModels, levelOfDetail)
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

                    // Get runtime node transforms for world-space raycasting
                    let node_transforms = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                        member.and_then(|m| m.member_type.as_shockwave3d())
                            .map(|w3d| w3d.runtime_state.node_transforms.clone())
                    };

                    let mut results = Vec::new();
                    if let Some(scene) = scene {
                        use crate::director::chunks::w3d::raycast::{Ray, raycast_scene_multi};
                        let ray = Ray {
                            origin: [origin[0] as f32, origin[1] as f32, origin[2] as f32],
                            direction: [direction[0] as f32, direction[1] as f32, direction[2] as f32],
                        };
                        let hits = raycast_scene_multi(
                            &ray, &scene, 100000.0, max_models as usize,
                            node_transforms.as_ref(),
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
                // Camera method — modelsUnderLoc(point, maxModels, levelOfDetail)
                // When called on member directly, return empty list / VOID
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
            _ => Self::call_member_type(datum, handler_name, args),
        }
    }

    fn get_3d_collection_count(scene: &crate::director::chunks::w3d::types::W3dScene, collection: &str) -> i32 {
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

    fn get_3d_object_name_by_index(scene: &crate::director::chunks::w3d::types::W3dScene, collection: &str, index: usize) -> Option<String> {
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

    fn call_member_type(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot call_member_type on non-cast-member".to_string(),
                    ))
                }
            };
            let cast_member = match player
                .movie
                .cast_manager
                .find_member_by_ref(&member_ref)
            {
                Some(m) => m,
                None => {
                    // preload/unload on non-existent members are no-ops in Director
                    if handler_name == "preload" || handler_name == "unload" {
                        return Ok(DatumRef::Void);
                    }
                    return Err(ScriptError::new(format!(
                        "Cannot call {} on non-existent member ({}, {})",
                        handler_name, member_ref.cast_lib, member_ref.cast_member
                    )));
                }
            };
            // preload/unload/stop/play/pause/rewind are no-ops for all member types in a web player
            if matches!(handler_name.as_str(), "preload" | "unload" | "stop" | "play" | "pause" | "rewind") {
                return Ok(DatumRef::Void);
            }
            match &cast_member.member_type {
                CastMemberType::Field(_) => {
                    FieldMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::Text(_) => {
                    TextMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::Button(_) => {
                    ButtonMemberHandlers::call(player, datum, handler_name, args)
                }
                _ => Err(ScriptError::new(format!(
                    "No handler {handler_name} for member type"
                ))),
            }
        })
    }

    fn erase(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => return Err(ScriptError::new("Cannot erase non-cast-member".to_string())),
            };
            // Silently ignore invalid cast lib or non-existent members, matching Director behavior
            let _ = player
                .movie
                .cast_manager
                .remove_member_with_ref(&cast_member_ref);
            Ok(DatumRef::Void)
        })
    }

    fn duplicate(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot duplicate non-cast-member".to_string(),
                    ))
                }
            };
            let dest_slot_number = args.get(0).map(|x| player.get_datum(x).int_value());

            if dest_slot_number.is_none() {
                return Err(ScriptError::new(
                    "Cannot duplicate cast member without destination slot number".to_string(),
                ));
            }
            let dest_slot_number = dest_slot_number.unwrap()?;
            let dest_ref = Self::member_ref_from_slot_number(dest_slot_number as u32);

            let mut new_member = {
                let src_member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&cast_member_ref);
                if src_member.is_none() {
                    return Err(ScriptError::new(
                        "Cannot duplicate non-existent cast member reference".to_string(),
                    ));
                }
                src_member.unwrap().clone()
            };
            new_member.number = dest_ref.cast_member as u32;

            let dest_cast = player
                .movie
                .cast_manager
                .get_cast_mut(dest_ref.cast_lib as u32);
            dest_cast.insert_member(dest_ref.cast_member as u32, new_member);

            Ok(player.alloc_datum(Datum::Int(dest_slot_number)))
        })
    }

    fn get_invalid_member_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        match prop.as_str() {
            "name" => Ok(Datum::String("".to_string())),
            "number" => Ok(Datum::Int(-1)),
            "type" => Ok(Datum::String("empty".to_string())),
            "castLibNum" => Ok(Datum::Int(-1)),
            "memberNum" => Ok(Datum::Int(-1)),
            "text" | "comments" => Ok(Datum::String("".to_string())),
            "loaded" | "mediaReady" => Ok(Datum::Int(1)),
            "width" | "height" | "rect" | "duration" => Ok(Datum::Void),
            "image" => Ok(Datum::Void),
            "regPoint" => Ok(Datum::Point([
                player.alloc_datum(Datum::Int(0)),
                player.alloc_datum(Datum::Int(0)),
            ])),
            _ => Err(ScriptError::new(format!(
                "Cannot get prop {} of invalid cast member ({}, {})",
                prop, member_ref.cast_lib, member_ref.cast_member
            ))),
        }
    }

    fn get_member_type_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        member_type: &CastMemberTypeId,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        debug!("Getting prop '{}' for member type {:?}", prop, member_type);
        match &member_type {
            CastMemberTypeId::Bitmap => {
                BitmapMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Field => FieldMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Text => TextMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Button => ButtonMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::FilmLoop => {
                FilmLoopMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Sound => SoundMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Font => FontMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Palette => PaletteMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Shockwave3d => Shockwave3dMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Script => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                let script_data = match &cast_member.member_type {
                    CastMemberType::Script(s) => s,
                    _ => return Err(ScriptError::new("Cast member is not a script".to_string())),
                };
                match prop.as_str() {
                    "text" => Ok(Datum::String("".to_string())),
                    "script" => Ok(Datum::ScriptRef(cast_member_ref.clone())),
                    "scriptText" => Ok(Datum::String("".to_string())),
                    "scriptType" => {
                        let symbol = match script_data.script_type {
                            ScriptType::Movie => "movie",
                            ScriptType::Parent => "parent",
                            ScriptType::Score => "score",
                            ScriptType::Member => "member",
                            _ => "unknown",
                        };
                        Ok(Datum::Symbol(symbol.to_string()))
                    }
                    "ilk" => Ok(Datum::Symbol("script".to_string())),
                    _ => Err(ScriptError::new(format!("Script members don't support property {}", prop))),
                }
            }
            CastMemberTypeId::Shape => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

                if let CastMemberType::Shape(shape_member) = &cast_member.member_type {
                    let info = &shape_member.shape_info;
                    match prop.as_str() {
                        "rect" => {
                            let width = info.width() as i32;
                            let height = info.height() as i32;
                            Ok(Datum::Rect([
                                player.alloc_datum(Datum::Int(0)),
                                player.alloc_datum(Datum::Int(0)),
                                player.alloc_datum(Datum::Int(width)),
                                player.alloc_datum(Datum::Int(height)),
                            ]))
                        }
                        "width" => Ok(Datum::Int(info.width() as i32)),
                        "height" => Ok(Datum::Int(info.height() as i32)),
                        "shapeType" => {
                            let symbol = match info.shape_type {
                                ShapeType::Rect => "rect",
                                ShapeType::OvalRect => "roundRect",
                                ShapeType::Oval => "oval",
                                ShapeType::Line => "line",
                                ShapeType::Unknown => "rect",
                            };
                            Ok(Datum::Symbol(symbol.to_string()))
                        }
                        "filled" => Ok(datum_bool(info.fill_type != 0)),
                        "lineSize" => Ok(Datum::Int(info.line_thickness as i32)),
                        "pattern" => Ok(Datum::Int(info.pattern as i32)),
                        "foreColor" => Ok(Datum::Int(info.fore_color as i32)),
                        "backColor" => Ok(Datum::Int(info.back_color as i32)),
                        _ => Err(ScriptError::new(format!(
                            "Shape members don't support property {}", prop
                        ))),
                    }
                } else {
                    Err(ScriptError::new("Expected shape member".to_string()))
                }
            }
            CastMemberTypeId::VectorShape => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

                if let CastMemberType::VectorShape(vs) = &cast_member.member_type {
                    // Extract data we need before dropping the borrow on player
                    let result: Result<Datum, ScriptError> = match prop.as_str() {
                        "width" => Ok(Datum::Int(vs.width().ceil() as i32)),
                        "height" => Ok(Datum::Int(vs.height().ceil() as i32)),
                        "strokeColor" => {
                            let (r, g, b) = vs.stroke_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        "strokeWidth" => Ok(Datum::Float(vs.stroke_width as f64)),
                        "closed" => Ok(datum_bool(vs.closed)),
                        "fillMode" => {
                            let sym = match vs.fill_mode {
                                0 => "none",
                                1 => "solid",
                                2 => "gradient",
                                _ => "none",
                            };
                            Ok(Datum::Symbol(sym.to_string()))
                        }
                        "fillColor" => {
                            let (r, g, b) = vs.fill_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        "backgroundColor" => {
                            let (r, g, b) = vs.bg_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        "endColor" => {
                            let (r, g, b) = vs.end_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        _ => Err(ScriptError::new(format!(
                            "VectorShape members don't support property {}", prop
                        ))),
                    };
                    // Handle props that need alloc_datum separately (to avoid borrow conflict)
                    if prop == "image" {
                        let w = vs.width().ceil() as u16;
                        let h = vs.height().ceil() as u16;
                        let fill = vs.fill_color;
                        let fill_mode = vs.fill_mode;
                        drop(cast_member);
                        // Create a bitmap filled with the fill color
                        let mut bitmap = crate::player::bitmap::bitmap::Bitmap::new(
                            w.max(1), h.max(1), 32, 32, 0,
                            crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
                                crate::player::bitmap::bitmap::BuiltInPalette::GrayScale
                            ),
                        );
                        if fill_mode > 0 {
                            // Fill with fill_color
                            let palettes = player.movie.cast_manager.palettes();
                            bitmap.flood_fill((0, 0), fill, &palettes);
                        }
                        let bitmap_id = player.bitmap_manager.add_bitmap(bitmap);
                        return Ok(Datum::BitmapRef(bitmap_id));
                    } else if prop == "rect" {
                        let w = vs.width().ceil() as i32;
                        let h = vs.height().ceil() as i32;
                        drop(cast_member);
                        Ok(Datum::Rect([
                            player.alloc_datum(Datum::Int(0)),
                            player.alloc_datum(Datum::Int(0)),
                            player.alloc_datum(Datum::Int(w)),
                            player.alloc_datum(Datum::Int(h)),
                        ]))
                    } else if prop == "vertexList" {
                        let vert_data: Vec<(i32, i32, i32, i32, i32, i32)> = vs.vertices.iter()
                            .map(|v| (
                                v.x as i32, v.y as i32,
                                v.handle1_x as i32, v.handle1_y as i32,
                                v.handle2_x as i32, v.handle2_y as i32,
                            ))
                            .collect();
                        drop(cast_member);
                        let list: VecDeque<DatumRef> = vert_data.iter().map(|(vx, vy, h1x, h1y, h2x, h2y)| {
                            let vertex_key = player.alloc_datum(Datum::Symbol("vertex".to_string()));
                            let vx_ref = player.alloc_datum(Datum::Int(*vx));
                            let vy_ref = player.alloc_datum(Datum::Int(*vy));
                            let vertex_val = player.alloc_datum(Datum::Point([vx_ref, vy_ref]));

                            let h1_key = player.alloc_datum(Datum::Symbol("handle1".to_string()));
                            let h1x_ref = player.alloc_datum(Datum::Int(*h1x));
                            let h1y_ref = player.alloc_datum(Datum::Int(*h1y));
                            let h1_val = player.alloc_datum(Datum::Point([h1x_ref, h1y_ref]));

                            let h2_key = player.alloc_datum(Datum::Symbol("handle2".to_string()));
                            let h2x_ref = player.alloc_datum(Datum::Int(*h2x));
                            let h2y_ref = player.alloc_datum(Datum::Int(*h2y));
                            let h2_val = player.alloc_datum(Datum::Point([h2x_ref, h2y_ref]));

                            let prop_list = Datum::PropList(VecDeque::from(vec![
                                (vertex_key, vertex_val),
                                (h1_key, h1_val),
                                (h2_key, h2_val),
                            ]), false);
                            player.alloc_datum(prop_list)
                        }).collect::<VecDeque<_>>();
                        Ok(Datum::List(DatumType::List, list, false))
                    } else {
                        result
                    }
                } else {
                    Err(ScriptError::new("Expected vectorShape member".to_string()))
                }
            }
            CastMemberTypeId::Flash => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                if let CastMemberType::Flash(flash) = &cast_member.member_type {
                    let (l, t, r, b) = flash.flash_info.as_ref()
                        .map(|fi| fi.flash_rect)
                        .unwrap_or((0, 0, 0, 0));
                    match prop.as_str() {
                        "width" => Ok(Datum::Int((r - l) as i32)),
                        "height" => Ok(Datum::Int((b - t) as i32)),
                        "rect" => Ok(Datum::Rect([
                            player.alloc_datum(Datum::Int(l as i32)),
                            player.alloc_datum(Datum::Int(t as i32)),
                            player.alloc_datum(Datum::Int(r as i32)),
                            player.alloc_datum(Datum::Int(b as i32)),
                        ])),
                        "regPoint" => {
                            let rp = flash.reg_point;
                            Ok(Datum::Point([
                                player.alloc_datum(Datum::Int(rp.0 as i32)),
                                player.alloc_datum(Datum::Int(rp.1 as i32)),
                            ]))
                        }
                        _ => Ok(Datum::Void),
                    }
                } else {
                    Ok(Datum::Void)
                }
            }
            _ => {
                // SWA/streaming media properties — return sensible defaults
                match prop.as_str() {
                    "soundChannel" => Ok(Datum::Int(0)),
                    "preloadTime" => Ok(Datum::Int(0)),
                    "volume" => Ok(Datum::Int(0)),
                    "url" => Ok(Datum::String(String::new())),
                    "state" => Ok(Datum::Int(0)), // 0=stopped
                    "currentTime" => Ok(Datum::Int(0)),
                    "duration" => Ok(Datum::Int(0)),
                    "percentPlayed" => Ok(Datum::Int(0)),
                    "percentStreamed" => Ok(Datum::Int(0)),
                    "loop" => Ok(Datum::Int(0)),
                    "pausedAtStart" => Ok(Datum::Int(0)),
                    _ => Err(ScriptError::new(format!(
                        "Cannot get castMember prop {} for member of type {:?}",
                        prop, member_type
                    ))),
                }
            }
        }
    }

    fn set_member_type_prop(
        member_ref: &CastMemberRef,
        prop: &String,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let member_type = reserve_player_ref(|player| {
            let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref);
            match cast_member {
                Some(cast_member) => Ok(Some(cast_member.member_type.member_type_id())),
                None => {
                    // Silently ignore setting props on erased members
                    web_sys::console::warn_1(&format!(
                        "Ignoring set prop {} on erased member {} of castLib {}",
                        prop, member_ref.cast_member, member_ref.cast_lib
                    ).into());
                    Ok(None)
                }
            }
        })?;

        let member_type = match member_type {
            Some(t) => t,
            None => return Ok(()), // Member was erased, silently ignore
        };

        // Handle Script-specific props before the main match so unrecognized
        // props fall through to the wildcard arm (e.g. implicit bitmap conversion).
        if member_type == CastMemberTypeId::Script {
            match prop.as_str() {
                "scriptText" => return Ok(()), // No-op: no lingo compiler
                "scriptType" => {
                    let type_str = value.string_value()?;
                    let script_type = match type_str.to_lowercase().as_str() {
                        "movie" => ScriptType::Movie,
                        "parent" => ScriptType::Parent,
                        "score" => ScriptType::Score,
                        "member" => ScriptType::Member,
                        _ => return Err(ScriptError::new(format!("Unknown scriptType: {}", type_str))),
                    };
                    return borrow_member_mut(
                        member_ref,
                        |_| {},
                        |cast_member, _| {
                            if let CastMemberType::Script(ref mut s) = cast_member.member_type {
                                s.script_type = script_type;
                            }
                            Ok(())
                        },
                    );
                }
                _ => {} // Fall through to main match
            }
        }

        match member_type {
            CastMemberTypeId::Field => FieldMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Text => TextMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Button => ButtonMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Font => reserve_player_mut(|player| {
                FontMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            CastMemberTypeId::Bitmap => BitmapMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Palette => reserve_player_mut(|player| {
                PaletteMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            CastMemberTypeId::VectorShape => {
                borrow_member_mut(
                    member_ref,
                    |_| {},
                    |cast_member, _| {
                        if let CastMemberType::VectorShape(vs) = &mut cast_member.member_type {
                            match prop.as_str() {
                                "fillColor" => {
                                    let color = value.to_color_ref()?;
                                    if let ColorRef::Rgb(r, g, b) = color {
                                        vs.fill_color = (*r, *g, *b);
                                    }
                                    Ok(())
                                }
                                "endColor" => {
                                    let color = value.to_color_ref()?;
                                    if let ColorRef::Rgb(r, g, b) = color {
                                        vs.end_color = (*r, *g, *b);
                                    }
                                    Ok(())
                                }
                                "bgColor" => {
                                    let color = value.to_color_ref()?;
                                    if let ColorRef::Rgb(r, g, b) = color {
                                        vs.bg_color = (*r, *g, *b);
                                    }
                                    Ok(())
                                }
                                "strokeColor" => {
                                    let color = value.to_color_ref()?;
                                    if let ColorRef::Rgb(r, g, b) = color {
                                        vs.stroke_color = (*r, *g, *b);
                                    }
                                    Ok(())
                                }
                                "strokeWidth" => {
                                    vs.stroke_width = value.to_float()? as f32;
                                    Ok(())
                                }
                                _ => Err(ScriptError::new(format!(
                                    "Cannot set VectorShape prop {}", prop
                                ))),
                            }
                        } else {
                            Err(ScriptError::new("Expected VectorShape member".to_string()))
                        }
                    },
                )
            }
            CastMemberTypeId::Flash => {
                // Flash members accept various properties silently
                // (directToStage, quality, scaleMode, etc.)
                Ok(())
            }
            CastMemberTypeId::Shockwave3d => reserve_player_mut(|player| {
                Shockwave3dMemberHandlers::set_prop(player, member_ref, prop, &value)
            }),
            _ => {
                // SWA/streaming media properties — accept silently as no-ops
                if matches!(prop.as_str(), "soundChannel" | "preloadTime" | "volume" | "url"
                    | "state" | "currentTime" | "duration" | "percentPlayed"
                    | "percentStreamed" | "loop" | "pausedAtStart") {
                    return Ok(());
                }
                // Check if this is a bitmap-specific property being set on a non-bitmap
                if prop == "image"
                    || prop == "regPoint"
                    || prop == "paletteRef"
                    || prop == "palette"
                {
                    // Director allows setting bitmap properties on non-bitmap members
                    // by implicitly converting them to bitmap members
                    reserve_player_mut(|player| {
                        let cast_member = player
                            .movie
                            .cast_manager
                            .find_mut_member_by_ref(member_ref)
                            .unwrap();

                        // If not already a bitmap, convert it
                        if cast_member.member_type.as_bitmap().is_none() {
                            // Create a new empty/default bitmap member
                            let new_bitmap = BitmapMember::default();

                            // Replace the member type
                            cast_member.member_type = CastMemberType::Bitmap(new_bitmap);
                        }

                        Ok(())
                    })?;
                    // Now try setting the property again
                    BitmapMemberHandlers::set_prop(member_ref, prop, value)
                } else {
                    Err(ScriptError::new(format!(
                        "Cannot set castMember prop {} for member of type {:?}",
                        prop, member_type
                    )))
                }
            }
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
        if is_invalid {
            return Self::get_invalid_member_prop(player, cast_member_ref, prop);
        }
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref);
        let (name, comments, slot_number, member_type, color, bg_color, member_num) = match cast_member {
            Some(cast_member) => {
                let name = cast_member.name.to_owned();
                let comments = cast_member.comments.to_owned();
                let slot_number = Self::get_cast_slot_number(
                    cast_member_ref.cast_lib as u32,
                    cast_member_ref.cast_member as u32,
                ) as i32;
                let member_type = cast_member.member_type.member_type_id();
                let member_num = cast_member.number;
                let color = cast_member.color.to_owned();
                let bg_color = cast_member.bg_color.to_owned();
                (name, comments, slot_number, member_type, color, bg_color, member_num)
            }
            None => {
                warn!(
                    "Getting prop {} of non-existent castMember reference {}, {}",
                    prop, cast_member_ref.cast_lib, cast_member_ref.cast_member
                );
                return Self::get_invalid_member_prop(player, cast_member_ref, prop);
            }
        };

        match prop.as_str() {
            "name" => Ok(Datum::String(name)),
            "memberNum" => Ok(Datum::Int(member_num as i32)),
            "number" => {
                if player.movie.dir_version >= 600 {
                    Ok(Datum::Int(slot_number))
                } else {
                    Ok(Datum::Int(member_num as i32))
                }
            }
            "type" => Ok(Datum::Symbol(member_type.symbol_string()?.to_string())),
            "castLibNum" => Ok(Datum::Int(cast_member_ref.cast_lib as i32)),
            "color" => Ok(Datum::ColorRef(color)),
            "bgColor" => Ok(Datum::ColorRef(bg_color)),
            "loaded" => Ok(Datum::Int(1)),
            "mediaReady" => Ok(Datum::Int(1)),
            "comments" => Ok(Datum::String(comments)),
            // In Director, member.member returns the member reference itself
            "member" => Ok(Datum::CastMember(cast_member_ref.clone())),
            _ => Self::get_member_type_prop(player, cast_member_ref, &member_type, prop),
        }
    }

    pub fn set_prop(
        cast_member_ref: &CastMemberRef,
        prop: &String,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
        if is_invalid {
            eprintln!(
                "Warning: Setting prop {} of invalid castMember reference (member {} of castLib {}), ignoring",
                prop, cast_member_ref.cast_member, cast_member_ref.cast_lib
            );
            return Ok(());
        }
        let exists = reserve_player_ref(|player| {
            player
                .movie
                .cast_manager
                .find_member_by_ref(cast_member_ref)
                .is_some()
        });
        let result = if exists {
            match prop.as_str() {
                "name" => borrow_member_mut(
                    cast_member_ref,
                    |_player| value.string_value(),
                    |cast_member, value| {
                        cast_member.name = value?;
                        Ok(())
                    },
                ),
                "comments" => borrow_member_mut(
                    cast_member_ref,
                    |_player| value.string_value(),
                    |cast_member, value| {
                        cast_member.comments = value?;
                        Ok(())
                    },
                ),
                "color" => borrow_member_mut(
                    cast_member_ref,
                    |_| {},
                    |cast_member, _| {
                        cast_member.color = value.to_color_ref()?.to_owned();
                        Ok(())
                    },
                ),
                "bgColor" => borrow_member_mut(
                    cast_member_ref,
                    |_| {},
                    |cast_member, _| {
                        cast_member.bg_color = value.to_color_ref()?.to_owned();
                        Ok(())
                    },
                ),
                _ => Self::set_member_type_prop(cast_member_ref, prop, value),
            }
        } else {
            // Silently ignore setting props on non-existent members
            // This can happen when a script erases a member but still holds a reference
            // Director silently ignores this case
            web_sys::console::warn_1(&format!(
                "Ignoring set prop {} on erased member {} of castLib {}",
                prop, cast_member_ref.cast_member, cast_member_ref.cast_lib
            ).into());
            Ok(())
        };
        if result.is_ok() {
            JsApi::dispatch_cast_member_changed(cast_member_ref.to_owned());
        }
        result
    }
}
