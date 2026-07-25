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
    console_warn,
};

const IDENTITY_MATRIX: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

const W3D_OBJECT_LOG: bool = false;

fn log(msg: &str) {
    if W3D_OBJECT_LOG {
        debug!("[W3D-OBJECT] {}", msg);
    }
}

pub struct Shockwave3dObjectDatumHandlers {}

impl Shockwave3dObjectDatumHandlers {
    pub fn get_prop(obj_ref: &DatumRef, prop_name: &str) -> Result<DatumRef, ScriptError> {
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
        match_ci!(s3d_ref.object_type.as_str(), {
            "model" | "bonesPlayer" | "keyframePlayer" => Self::get_model_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "shader" => Self::get_shader_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "texture" => Self::get_texture_prop(player, scene, &s3d_ref.name, prop_name),
            "camera" => Self::get_camera_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "fog" => {
                // s3d_ref.name is the owning camera name; fog state is per-W3D-member.
                let rs = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| (
                        w3d.runtime_state.fog_enabled,
                        w3d.runtime_state.fog_near,
                        w3d.runtime_state.fog_far,
                        w3d.runtime_state.fog_color,
                        w3d.runtime_state.fog_mode,
                    ))
                    .unwrap_or((false, 1.0, 1000.0, (0.5, 0.5, 0.5), 0));
                match_ci!(prop_name, {
                    "enabled" => Ok(player.alloc_datum(Datum::Int(if rs.0 { 1 } else { 0 }))),
                    "near" => Ok(player.alloc_datum(Datum::Float(rs.1 as f64))),
                    "far" => Ok(player.alloc_datum(Datum::Float(rs.2 as f64))),
                    "color" => Ok(player.alloc_datum(color_to_datum([rs.3.0, rs.3.1, rs.3.2, 1.0]))),
                    "decayMode" => {
                        let sym = match rs.4 { 1 => "exponential", 2 => "exponential2", _ => "linear" };
                        Ok(player.alloc_datum(Datum::Symbol(sym.to_string())))
                    },
                    _ => Ok(player.alloc_datum(Datum::Void)),
                })
            },
            "light" => Self::get_light_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "group" => Self::get_node_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "modelResource" => Self::get_model_resource_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "motion" => Self::get_motion_prop(player, scene, &s3d_ref.name, prop_name),
            "colorBuffer" => {
                // colorBuffer.clearAtRender property
                let cam_name = s3d_ref.name.clone();
                match_ci!(prop_name, {
                    "clearAtRender" => {
                        let val = {
                            let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                            member.and_then(|m| m.member_type.as_shockwave3d())
                                .and_then(|w3d| w3d.runtime_state.camera_clear_at_render.get(&cam_name.to_ascii_lowercase()))
                                .copied()
                                .unwrap_or(true)
                        };
                        Ok(player.alloc_datum(Datum::Int(if val { 1 } else { 0 })))
                    },
                    _ => Ok(player.alloc_datum(Datum::Void)),
                })
            },
            "meshDeform" => Self::get_mesh_deform_prop(player, scene, &s3d_ref.name, prop_name, member_ref),
            "collision" => {
                // Native #collision modifier object — s3d_ref.name is the model name.
                let cm = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .and_then(|w3d| w3d.runtime_state.collision_modifiers.get(&s3d_ref.name)
                        .or_else(|| w3d.runtime_state.collision_modifiers.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&s3d_ref.name)).map(|(_, v)| v)))
                    .cloned()
                    .unwrap_or_default();
                match_ci!(prop_name, {
                    "enabled" => Ok(player.alloc_datum(Datum::Int(if cm.enabled { 1 } else { 0 }))),
                    "resolve" => Ok(player.alloc_datum(Datum::Int(if cm.resolve { 1 } else { 0 }))),
                    "immovable" => Ok(player.alloc_datum(Datum::Int(if cm.immovable { 1 } else { 0 }))),
                    "mode" => Ok(player.alloc_datum(Datum::Symbol(cm.mode.clone()))),
                    _ => Ok(player.alloc_datum(Datum::Void)),
                })
            },
            "overlay" | "backdrop" => {
                // overlay/backdrop object: name format "cameraName:index".
                // camera_overlays/camera_backdrops are keyed by lowercased camera
                // name (see addOverlay), so the lookup must be case-insensitive.
                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                let cam_name = parts.get(0).unwrap_or(&"").to_ascii_lowercase();
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
                match_ci!(prop_name, {
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
                    },
                    "loc" => {
                        let ov = overlay.unwrap_or_default();
                        // Preserve int-ness when each component is a whole
                        // number. Director Lingo distinguishes `int / int =
                        // int` (truncating) from float division — script flow
                        // like `point(P1.locH / 32, P1.locV / 32) * 32` to
                        // snap to a 32-pixel grid relies on integer division
                        // when locs are integers. Returning floats here makes
                        // every subsequent arithmetic chain float and the
                        // snap turns into pixel-precise tracking.
                        let flag_h = if ov.loc[0].fract() == 0.0 { 0 } else { 1 };
                        let flag_v = if ov.loc[1].fract() == 0.0 { 0 } else { 1 };
                        let flags: u8 = (flag_h as u8) | ((flag_v as u8) << 1);
                        Ok(player.alloc_datum(Datum::Point([ov.loc[0], ov.loc[1]], flags)))
                    },
                    "blend" => Ok(player.alloc_datum(Datum::Float(overlay.map(|o| o.blend).unwrap_or(100.0)))),
                    "scale" => Ok(player.alloc_datum(Datum::Float(overlay.map(|o| o.scale).unwrap_or(1.0)))),
                    "rotation" => Ok(player.alloc_datum(Datum::Float(overlay.map(|o| o.rotation).unwrap_or(0.0)))),
                    "regPoint" => {
                        let ov = overlay.unwrap_or_default();
                        // Same int-preservation logic as `loc` above.
                        let flag_h = if ov.reg_point[0].fract() == 0.0 { 0 } else { 1 };
                        let flag_v = if ov.reg_point[1].fract() == 0.0 { 0 } else { 1 };
                        let flags: u8 = (flag_h as u8) | ((flag_v as u8) << 1);
                        Ok(player.alloc_datum(Datum::Point([ov.reg_point[0], ov.reg_point[1]], flags)))
                    },
                    _ => Ok(player.alloc_datum(Datum::Void)),
                })
            },
            "emitter" => {
                // Get or create persistent emitter state
                let emitter = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.emitters.get(&s3d_ref.name))
                        .cloned()
                        .unwrap_or_default()
                };
                match_ci!(prop_name, {
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
                        log(&format!("[W3D] emitter(\"{}\").{} (stub)", s3d_ref.name, prop_name));
                        Ok(player.alloc_datum(Datum::Void))
                    },
                })
            },
            // #particle range objects — read .start / .end back from the particle state.
            // Scripts gate flow on these (e.g. `if resource.blendRange.start > 0`), so the
            // getter must return what was set, not a placeholder.
            "colorRange" | "sizeRange" | "blendRange" => {
                let ps = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.particles.get(&s3d_ref.name))
                        .cloned()
                };
                let is_start = prop_name.eq_ignore_ascii_case("start");
                match_ci!(s3d_ref.object_type.as_str(), {
                    "colorRange" => {
                        let c = ps.as_ref().map(|p| if is_start { p.color_start } else { p.color_end })
                            .unwrap_or([1.0, 1.0, 1.0]);
                        let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
                        Ok(player.alloc_datum(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(
                            to_u8(c[0]), to_u8(c[1]), to_u8(c[2])))))
                    },
                    "sizeRange" => {
                        let v = ps.as_ref().map(|p| if is_start { p.size_start } else { p.size_end }).unwrap_or(0.0);
                        Ok(player.alloc_datum(Datum::Float(v as f64)))
                    },
                    "blendRange" => {
                        // Stored as the raw IFX alpha (0..1, default 0.1); report as set.
                        let v = ps.as_ref().map(|p| if is_start { p.blend_start } else { p.blend_end }).unwrap_or(0.1);
                        Ok(player.alloc_datum(Datum::Float(v as f64)))
                    },
                    _ => Ok(player.alloc_datum(Datum::Void)),
                })
            },
            "sds" => {
                // Subdivision Surface modifier properties
                let sds = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.sds_state.get(&s3d_ref.name))
                        .cloned()
                        .unwrap_or_default()
                };
                match_ci!(prop_name, {
                    "depth" => Ok(player.alloc_datum(Datum::Int(sds.depth))),
                    "tension" => Ok(player.alloc_datum(Datum::Int(sds.tension as i32))),
                    "error" => Ok(player.alloc_datum(Datum::Int(sds.error as i32))),
                    "enabled" => Ok(player.alloc_datum(Datum::Int(if sds.enabled { 1 } else { 0 }))),
                    _ => {
                        log(&format!("[W3D] sds(\"{}\").{} not implemented", s3d_ref.name, prop_name));
                        Ok(player.alloc_datum(Datum::Void))
                    },
                })
            },
            "lod" => {
                let lod = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.lod_state.get(&s3d_ref.name))
                        .cloned()
                        .unwrap_or_default()
                };
                match_ci!(prop_name, {
                    "level" => Ok(player.alloc_datum(Datum::Int(lod.level))),
                    "auto" => Ok(player.alloc_datum(Datum::Int(if lod.auto_mode { 1 } else { 0 }))),
                    "bias" => Ok(player.alloc_datum(Datum::Float(lod.bias as f64))),
                    _ => Ok(player.alloc_datum(Datum::Void)),
                })
            },
            "bone" => {
                // name format is "modelName:boneIndex"
                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                let model_name = parts.get(0).unwrap_or(&"");
                let bone_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                match_ci!(prop_name, {
                    "transform" => {
                        // Bone-LOCAL transform — relative to the bone's parent
                        // bone in the skeleton hierarchy. (Director also has
                        // `bone.transform` = local; only `worldTransform`
                        // accumulates.)
                        let bone_matrix = player.movie.cast_manager.find_member_by_ref(member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| {
                                let scene = w3d.parsed_scene.as_ref()?;
                                let skeleton = find_skeleton_for_model(scene, model_name)?;
                                if bone_idx >= skeleton.bones.len() { return None; }
                                let bp = w3d.runtime_state.bones_player(model_name)
                                    .filter(|b| b.current_motion.is_some());
                                let motion = bp.and_then(|bp| bp.current_motion.as_deref())
                                    .or_else(|| w3d.runtime_state.current_motion.as_deref())
                                    .and_then(|name| scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(name)));
                                let t = match bp {
                                    Some(bp) => compute_motion_t_bp(motion, bp),
                                    None => compute_motion_t(motion, &w3d.runtime_state),
                                };
                                let matrices = crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, motion, t);
                                let bone_m = matrices.get(bone_idx).copied()?;
                                // Relativize by the idle-pose root to MATCH the renderer's
                                // skin (scene3d setup_skinning), so a weapon attached via
                                // bone[].worldTransform lines up with the relativized body.
                                // Only biped actors have an idle-rest motion; others unchanged.
                                let idle = scene.motions.iter()
                                    .find(|m| m.name.to_ascii_lowercase().contains("idle_rest"))
                                    .or_else(|| scene.motions.iter().find(|m| m.name.to_ascii_lowercase().contains("idle")))
                                    .map(|im| crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, Some(im), 0.0));
                                match idle {
                                    Some(im) if !im.is_empty() => Some(mat4_mul_f32(&invert_transform_f32(&im[0]), &bone_m)),
                                    _ => Some(bone_m),
                                }
                            });
                        if let Some(m) = bone_matrix {
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
                    },
                    "worldTransform" => {
                        // World-space bone transform — accumulates the owning
                        // model's own world transform AND the bone's hierarchy
                        // matrix. ClubMarian's BehaviorScript 3 uses this to
                        // pin the head model on top of the body:
                        //   `player.Head.transform = body.bonesPlayer.bone[6].worldTransform`
                        // The body has `transform.position = vector(0, 85, 0)`
                        // and bone[6] (the head bone) sits well above the
                        // body's local origin. Without applying the body's
                        // model transform, the bone position came back at
                        // ~Y=79 (just bone-local) instead of Y≈168 (body
                        // offset + bone), and the head rendered between the
                        // legs.
                        let bone_matrix = player.movie.cast_manager.find_member_by_ref(member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| {
                                let scene = w3d.parsed_scene.as_ref()?;
                                let skeleton = find_skeleton_for_model(scene, model_name)?;
                                if bone_idx >= skeleton.bones.len() { return None; }
                                let bp = w3d.runtime_state.bones_player(model_name)
                                    .filter(|b| b.current_motion.is_some());
                                let motion = bp.and_then(|bp| bp.current_motion.as_deref())
                                    .or_else(|| w3d.runtime_state.current_motion.as_deref())
                                    .and_then(|name| scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(name)));
                                let t = match bp {
                                    Some(bp) => compute_motion_t_bp(motion, bp),
                                    None => compute_motion_t(motion, &w3d.runtime_state),
                                };
                                let matrices = crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, motion, t);
                                let bone_m = matrices.get(bone_idx).copied()?;
                                // Relativize by the idle-pose root to MATCH the renderer's
                                // skin (scene3d setup_skinning), so a weapon attached via
                                // bone[].worldTransform lines up with the relativized body.
                                // Only biped actors have an idle-rest motion; others unchanged.
                                let idle = scene.motions.iter()
                                    .find(|m| m.name.to_ascii_lowercase().contains("idle_rest"))
                                    .or_else(|| scene.motions.iter().find(|m| m.name.to_ascii_lowercase().contains("idle")))
                                    .map(|im| crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, Some(im), 0.0));
                                match idle {
                                    Some(im) if !im.is_empty() => Some(mat4_mul_f32(&invert_transform_f32(&im[0]), &bone_m)),
                                    _ => Some(bone_m),
                                }
                            });
                        if let Some(bone_m) = bone_matrix {
                            let model_world = get_node_transform(player, member_ref, model_name);
                            let combined = mat4_mul_f32(&model_world, &bone_m);
                            let m64: [f64; 16] = combined.map(|v| v as f64);
                            Ok(player.alloc_datum(Datum::Transform3d(m64)))
                        } else {
                            Ok(get_persistent_node_transform(player, member_ref, model_name))
                        }
                    },
                    "name" => {
                        // Return actual bone name from skeleton
                        let name = player.movie.cast_manager.find_member_by_ref(member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.parsed_scene.as_ref())
                            .and_then(|s| find_skeleton_for_model(s, model_name))
                            .and_then(|skel| skel.bones.get(bone_idx))
                            .map(|b| b.name.clone())
                            .unwrap_or_else(|| format!("bone_{}", bone_idx));
                        Ok(player.alloc_datum(Datum::String(name)))
                    },
                    _ => {
                        log(&format!("[W3D] bone[{}].{} (stub)", bone_idx, prop_name));
                        Ok(player.alloc_datum(Datum::Void))
                    },
                })
            },
            "meshDeformMesh" => {
                // mesh[m].textureLayer — return persistent list from runtime state
                // name format is "modelName:meshIndex"
                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                let model_name = parts.get(0).unwrap_or(&"").to_string();
                let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                match_ci!(prop_name, {
                    "textureLayer" => {
                        // Get or create a persistent texture layer list DatumRef in runtime state.
                        // The list contains meshDeformTexLayer refs so that indexed access like
                        // textureLayer[N].textureCoordinateList = data goes through our set_prop handler.
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
                            log(&format!(
                                "[W3D-TEXLAYER] persistent ref NOT found for model='{}' mesh_idx={} member=({},{})",
                                model_name, mesh_idx, member_ref_owned.cast_lib, member_ref_owned.cast_member
                            ));
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
                    },
                    "face" => {
                        // Return a LIST of faces, each a 3-element list of 1-based vertex
                        // indices [v1,v2,v3] (Director's meshDeform face[] convention). This
                        // makes BOTH `face.count` (list length) and `face[j]` (the j-th triple)
                        // work, matching Director's message-window output [1,2,3],[4,5,6],…
                        let node = scene.nodes.iter().find(|n| n.name == *model_name);
                        let model_res = node.map(|n| n.model_resource_name.as_str()).unwrap_or("");
                        let res = node.map(|n| n.resource_name.as_str()).unwrap_or("");
                        let keys: Vec<&str> = [model_res, res].iter()
                            .filter(|k| !k.is_empty() && **k != ".")
                            .copied().collect();
                        let mut faces: Vec<[u32; 3]> = Vec::new();
                        for key in &keys {
                            if let Some(meshes) = scene.clod_meshes.get(*key) {
                                if let Some(mesh) = meshes.get(mesh_idx) {
                                    faces = mesh.faces.clone();
                                }
                            }
                            if !faces.is_empty() { break; }
                        }
                        if faces.is_empty() {
                            for key in &keys {
                                for raw in &scene.raw_meshes {
                                    if raw.name == *key && raw.chain_index as usize == mesh_idx {
                                        faces = raw.faces.clone();
                                        break;
                                    }
                                }
                                if !faces.is_empty() { break; }
                            }
                        }
                        let mut items = VecDeque::new();
                        for f in &faces {
                            let tri = VecDeque::from(vec![
                                player.alloc_datum(Datum::Int(f[0] as i32 + 1)),
                                player.alloc_datum(Datum::Int(f[1] as i32 + 1)),
                                player.alloc_datum(Datum::Int(f[2] as i32 + 1)),
                            ]);
                            items.push_back(player.alloc_datum(Datum::List(
                                crate::director::lingo::datum::DatumType::List, tri, false)));
                        }
                        Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, items, false)))
                    },
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
                    },
                    "normalList" => {
                        // Full list of per-vertex normal vectors (parallels vertexList).
                        let mut items = VecDeque::new();
                        let node = scene.nodes.iter().find(|n| n.name == *model_name);
                        let model_res_name = node.map(|n| n.model_resource_name.as_str()).unwrap_or("");
                        let res_name = node.map(|n| n.resource_name.as_str()).unwrap_or("");
                        let keys_to_try: Vec<&str> = [model_res_name, res_name].iter()
                            .filter(|k| !k.is_empty() && **k != ".")
                            .copied().collect();
                        for key in &keys_to_try {
                            if let Some(meshes) = scene.clod_meshes.get(*key) {
                                if let Some(mesh) = meshes.get(mesh_idx) {
                                    for n in &mesh.normals {
                                        items.push_back(player.alloc_datum(Datum::Vector([n[0] as f64, n[1] as f64, n[2] as f64])));
                                    }
                                }
                                if !items.is_empty() { break; }
                            }
                        }
                        if items.is_empty() {
                            for key in &keys_to_try {
                                for raw in &scene.raw_meshes {
                                    if raw.name == *key && raw.chain_index as usize == mesh_idx {
                                        for n in &raw.normals {
                                            items.push_back(player.alloc_datum(Datum::Vector([n[0] as f64, n[1] as f64, n[2] as f64])));
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
                    },
                    // `mesh[m].face.count` is resolved as a single compound property
                    // "face.count" (not `.face` then `.count`), so handle it here by
                    // reading the actual triangle count of this mesh group from the
                    // built geometry. (`.vertexList.count` / `.normalList.count` are
                    // handled by Lingo's list `.count` since those return real lists.)
                    "face.count" | "facecount" | "faceCount" => {
                        let node = scene.nodes.iter().find(|n| n.name == *model_name);
                        let model_res_name = node.map(|n| n.model_resource_name.as_str()).unwrap_or("");
                        let res_name = node.map(|n| n.resource_name.as_str()).unwrap_or("");
                        let keys_to_try: Vec<&str> = [model_res_name, res_name].iter()
                            .filter(|k| !k.is_empty() && **k != ".")
                            .copied().collect();
                        let count = keys_to_try.iter()
                            .find_map(|k| scene.clod_meshes.get(*k).and_then(|ms| ms.get(mesh_idx)).map(|m| m.faces.len()))
                            .or_else(|| keys_to_try.iter().find_map(|k| scene.raw_meshes.iter()
                                .find(|raw| raw.name == **k && raw.chain_index as usize == mesh_idx)
                                .map(|raw| raw.faces.len())))
                            .unwrap_or(0);
                        Ok(player.alloc_datum(Datum::Int(count as i32)))
                    },
                    _ => {
                        Ok(player.alloc_datum(Datum::Void))
                    },
                })
            },
            "meshDeformTexLayer" => {
                // textureLayer[n].textureCoordinateList — get from runtime state
                let parts: Vec<&str> = s3d_ref.name.splitn(3, ':').collect();
                let model_name = parts.get(0).unwrap_or(&"");
                let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let layer_idx: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                match_ci!(prop_name, {
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
                    },
                    _ => Ok(player.alloc_datum(Datum::Void)),
                })
            },
            _ => Err(ScriptError::new(format!("Unknown 3D object type '{}'", s3d_ref.object_type))),
        })
    }

    pub fn set_prop(obj_ref: &DatumRef, prop_name: &str, value: &Datum) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            let s3d_ref = match player.get_datum(obj_ref) {
                Datum::Shockwave3dObjectRef(r) => {
                    if r.object_type == "meshDeformTexLayer" {
                        log(&format!(
                            "[W3D-TEXLAYER-SET] meshDeformTexLayer.{} name=\"{}\"",
                            prop_name, r.name
                        ));
                    }
                    r.clone()
                },
                _ => return Err(ScriptError::new("Expected Shockwave3dObjectRef".to_string())),
            };

            let member_ref = crate::player::cast_lib::CastMemberRef {
                cast_lib: s3d_ref.cast_lib,
                cast_member: s3d_ref.cast_member,
            };

            // `bonesPlayer.bone[i].transform = t` — store a manual per-bone LOCAL
            // override (ref name is "modelName:boneIndex"). Director 11.5: the bone
            // is no longer driven by the current motion; updateBoneRotation-style
            // scripts re-set it every frame for procedural animation (the SweeTarts
            // snake's S-wiggle). The skeleton build substitutes it, resolving the
            // bone's rest length for the (typically zero) translation.
            if s3d_ref.object_type == "bone" && prop_name.eq_ignore_ascii_case("transform") {
                if let Datum::Transform3d(m) = value {
                    let mut mat = [0.0f32; 16];
                    for i in 0..16 { mat[i] = m[i] as f32; }
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bone_transform_overrides
                                .insert(s3d_ref.name.to_ascii_lowercase(), mat);
                        }
                    }
                }
                return Ok(());
            }

            // Setters on a #collision modifier object ref:
            // `model.collision.enabled = 1`, `.resolve`, `.immovable`, `.mode`.
            // Handled before the generic match_ci! (it can't guard prop names by
            // object_type, and these names overlap with other object types).
            if s3d_ref.object_type == "collision" {
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                        // Resolve the (case-insensitive) modifier entry, creating
                        // it if a property was set without addModifier first.
                        let key = w3d.runtime_state.collision_modifiers.keys()
                            .find(|k| k.eq_ignore_ascii_case(&s3d_ref.name))
                            .cloned()
                            .unwrap_or_else(|| s3d_ref.name.clone());
                        let cm = w3d.runtime_state.collision_modifiers.entry(key).or_default();
                        match prop_name.to_ascii_lowercase().as_str() {
                            "enabled" => cm.enabled = !matches!(value, Datum::Int(0)),
                            "resolve" => cm.resolve = !matches!(value, Datum::Int(0)),
                            "immovable" => cm.immovable = !matches!(value, Datum::Int(0)),
                            "mode" => cm.mode = match value {
                                Datum::Symbol(s) | Datum::String(s) => s.clone(),
                                _ => cm.mode.clone(),
                            },
                            _ => {}
                        }
                    }
                }
                return Ok(());
            }

            // Setters on a fog object ref: `cameraFog.near = X`, etc.
            // Handled before the generic match_ci! since the macro doesn't
            // support `if` guards for distinguishing prop names by object_type.
            if s3d_ref.object_type == "fog" {
                let lower = prop_name.to_ascii_lowercase();
                if matches!(lower.as_str(), "near" | "far" | "enabled" | "color" | "decaymode") {
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            match lower.as_str() {
                                "near" => {
                                    w3d.runtime_state.fog_near = match value {
                                        Datum::Float(f) => *f as f32,
                                        Datum::Int(i) => *i as f32,
                                        _ => 1.0,
                                    };
                                }
                                "far" => {
                                    w3d.runtime_state.fog_far = match value {
                                        Datum::Float(f) => *f as f32,
                                        Datum::Int(i) => *i as f32,
                                        _ => 1000.0,
                                    };
                                }
                                "enabled" => {
                                    w3d.runtime_state.fog_enabled = match value {
                                        Datum::Int(v) => *v != 0,
                                        _ => false,
                                    };
                                }
                                "color" => {
                                    w3d.runtime_state.fog_color = match value {
                                        Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)) => {
                                            (*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0)
                                        }
                                        _ => (0.5, 0.5, 0.5),
                                    };
                                }
                                "decaymode" => {
                                    w3d.runtime_state.fog_mode = match value {
                                        Datum::Symbol(s) => match s.to_ascii_lowercase().as_str() {
                                            "exponential" => 1,
                                            "exponential2" => 2,
                                            _ => 0,
                                        },
                                        _ => 0,
                                    };
                                }
                                _ => {}
                            }
                        }
                    }
                    return Ok(());
                }
            }

            // ── #particle model-resource property sets ──
            // colorRange/sizeRange/blendRange are range objects (.start/.end); lifeTime,
            // texture, gravity, wind, drag are direct resource props. All persist into the
            // ParticleSystemState (created lazily; gravity defaults to vector(0,0,0) per the
            // Director dictionary, NOT the struct's fire-style default). Handled before the
            // generic match so the named `texture` (shader) arm doesn't intercept it.
            {
                let ot = s3d_ref.object_type.as_str();
                let is_range = ot.eq_ignore_ascii_case("colorRange")
                    || ot.eq_ignore_ascii_case("sizeRange")
                    || ot.eq_ignore_ascii_case("blendRange");
                let is_res_prop = ot.eq_ignore_ascii_case("modelResource") && (
                    prop_name.eq_ignore_ascii_case("lifetime")
                    || prop_name.eq_ignore_ascii_case("texture")
                    || prop_name.eq_ignore_ascii_case("gravity")
                    || prop_name.eq_ignore_ascii_case("wind")
                    || prop_name.eq_ignore_ascii_case("drag"));
                if is_range || is_res_prop {
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let ps = w3d.runtime_state.particles
                                .entry(s3d_ref.name.clone())
                                .or_insert_with(|| {
                                    let mut p = crate::player::cast_member::ParticleSystemState::default();
                                    p.gravity = [0.0, 0.0, 0.0]; // #particle default gravity
                                    p
                                });
                            if is_range {
                                let is_start = prop_name.eq_ignore_ascii_case("start");
                                match_ci!(ot, {
                                    "colorRange" => {
                                        if let Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)) = value {
                                            let c = [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0];
                                            if is_start { ps.color_start = c; } else { ps.color_end = c; }
                                        }
                                    },
                                    "sizeRange" => {
                                        let v = value.float_value().unwrap_or(0.0) as f32;
                                        if is_start { ps.size_start = v; } else { ps.size_end = v; }
                                    },
                                    "blendRange" => {
                                        // IFX particle blend is the per-particle ALPHA (0..1, default
                                        // 0.1) — NOT a 0..100 percentage. Store the raw value; values
                                        // >1 (the faucet's 2-3 / 6-7 at full flow) clamp to fully
                                        // opaque at render time. See CIFXShaderParticle (vertex RGBA).
                                        let v = value.float_value().unwrap_or(0.1) as f32;
                                        if is_start { ps.blend_start = v; } else { ps.blend_end = v; }
                                    },
                                    _ => {}
                                });
                            } else {
                                match_ci!(prop_name, {
                                    // lifetime is in milliseconds (default 10000); store seconds.
                                    "lifetime" => ps.lifetime = (value.float_value().unwrap_or(10000.0) as f32 / 1000.0).max(0.001),
                                    "texture" => {
                                        let tn = match value {
                                            Datum::Shockwave3dObjectRef(r) if r.object_type == "texture" => r.name.clone(),
                                            Datum::String(s) => s.clone(),
                                            _ => String::new(),
                                        };
                                        ps.texture_name = tn.to_lowercase();
                                    },
                                    "gravity" => if let Datum::Vector(v) = value { ps.gravity = [v[0] as f32, v[1] as f32, v[2] as f32]; },
                                    "wind" => if let Datum::Vector(v) = value { ps.wind = [v[0] as f32, v[1] as f32, v[2] as f32]; },
                                    // drag is 0 (none) .. 100 (full); store 0..1.
                                    "drag" => ps.drag = (value.float_value().unwrap_or(0.0) as f32 / 100.0).clamp(0.0, 1.0),
                                    _ => {}
                                });
                            }
                        }
                    }
                    return Ok(());
                }
            }

            match_ci!(prop_name, {
                "transform" => {
                    if let Datum::Transform3d(m) = value {
                        let m32: [f32; 16] = m.map(|v| v as f32);
                        if s3d_ref.name.eq_ignore_ascii_case("defaultview") {
                            log(&format!(
                                "[W3D] setting defaultview.transform directly! pos=({:.1},{:.1},{:.1}) obj_type={}",
                                m32[12], m32[13], m32[14], s3d_ref.object_type
                            ));
                        }
                        set_node_transform(player, &member_ref, &s3d_ref.name, m32);
                    }
                    Ok(())
                },
                "visible" => {
                    // visible = TRUE/FALSE — show/hide only, does NOT change face culling mode.
                    // The culling mode is controlled separately by the "visibility" property.
                    let show = match value {
                        Datum::Int(v) => *v != 0,
                        _ => true,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if show {
                                // Remove hide override — render with default culling
                                w3d.runtime_state.node_visibility.remove(&s3d_ref.name);
                            } else {
                                w3d.runtime_state.node_visibility.insert(s3d_ref.name.clone(), 0); // #none
                            }
                        }
                    }
                    Ok(())
                },
                "visibility" => {
                    // #front=1, #back=2, #both=3, #none=0
                    let mode: u8 = match value {
                        Datum::Symbol(s) => match_ci!(s.as_str(), {
                            "front" => 1u8,
                            "back" => 2,
                            "both" => 3,
                            "none" => 0,
                            _ => 3,
                        }),
                        _ => 3,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.node_visibility.insert(s3d_ref.name.clone(), mode);
                        }
                    }
                    Ok(())
                },
                "pointAtOrientation" | "pointatorientation" => {
                    // pointAtOrientation = [vector(front), vector(up)]
                    // Defines which local axes map to "toward target" and "up" for pointAt()
                    if let Datum::List(_, items, _) = value {
                        if items.len() >= 2 {
                            let front = match player.get_datum(&items[0]) {
                                Datum::Vector(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
                                _ => [0.0, 0.0, 1.0],
                            };
                            let up = match player.get_datum(&items[1]) {
                                Datum::Vector(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
                                _ => [0.0, 1.0, 0.0],
                            };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.point_at_orientations.insert(
                                        s3d_ref.name.clone(), (front, up)
                                    );
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "shader" => {
                    // Director scripts assign `model.shader = sp.shader("Name")`
                    // — `value` is a Shockwave3dObjectRef of object_type "shader",
                    // not a String. Reading it via string_value() returns empty
                    // and we'd silently store an unnamed shader → renderer falls
                    // back to DefaultShader/DefaultTexture. Pull the name out
                    // of the ref directly; only fall back to string_value()
                    // for movies that pass the name as a literal string.
                    let shader_name = match value {
                        Datum::Shockwave3dObjectRef(r) if r.object_type == "shader" => r.name.clone(),
                        Datum::String(s) => s.clone(),
                        Datum::Symbol(s) => s.clone(),
                        _ => value.string_value().unwrap_or_default(),
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let shader_map = w3d.runtime_state.node_shaders
                                .entry(s3d_ref.name.clone())
                                .or_insert_with(std::collections::HashMap::new);
                            shader_map.insert(0, shader_name);
                        }
                    }
                    Ok(())
                },
                "shaderList" => {
                    // Whole-list assignment: `model.shaderList = singleShader` (or
                    // `[shaderA, shaderB, ...]`). Distinct from `.shader =` (which
                    // only sets the first shader) and the indexed `shaderList[i] =`
                    // (setProp). A single shader applies to EVERY mesh via the
                    // index-0 fallback in node_shader_override; a list maps
                    // positionally. frog01 uses this form everywhere — e.g.
                    // `model("ft").shaderList = shader("clearS")` (makes the frog's
                    // collision box invisible) and `textModel.shaderList =
                    // shader("redS")` (colors the title text); without this arm the
                    // assignment was silently dropped, so those models rendered with
                    // the default opaque checker texture.
                    let shader_names: Vec<(usize, String)> = match value {
                        Datum::List(_, items, _) => items.iter().enumerate().filter_map(|(i, item)| {
                            match player.get_datum(item) {
                                Datum::Shockwave3dObjectRef(r) if r.object_type == "shader" => Some((i, r.name.clone())),
                                Datum::String(s) => Some((i, s.clone())),
                                _ => None,
                            }
                        }).collect(),
                        Datum::Shockwave3dObjectRef(r) if r.object_type == "shader" => vec![(0, r.name.clone())],
                        Datum::String(s) => vec![(0, s.clone())],
                        Datum::Symbol(s) => vec![(0, s.clone())],
                        _ => vec![],
                    };
                    if !shader_names.is_empty() {
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                let map = w3d.runtime_state.node_shaders
                                    .entry(s3d_ref.name.clone())
                                    .or_insert_with(std::collections::HashMap::new);
                                map.clear();
                                for (i, name) in shader_names {
                                    map.insert(i, name);
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "worldPosition" => {
                    if let Datum::Vector(v) = value {
                        // Guard against NaN - skip update if any component is NaN
                        if v[0].is_finite() && v[1].is_finite() && v[2].is_finite() {
                            let mut m = get_or_init_node_transform(player, &member_ref, &s3d_ref.name);
                            m[12] = v[0] as f32;
                            m[13] = v[1] as f32;
                            m[14] = v[2] as f32;
                            set_node_transform(player, &member_ref, &s3d_ref.name, m);
                        }
                    }
                    Ok(())
                },
                "playRate" => {
                    let rate = match value {
                        Datum::Float(f) => *f as f32,
                        Datum::Int(i) => *i as f32,
                        _ => 1.0,
                    };
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bones_player_mut(&model_name).play_rate = rate;
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(())
                },
                "blendTime" => {
                    let ms = match value {
                        Datum::Float(f) => *f as f32,
                        Datum::Int(i) => *i as f32,
                        _ => 0.0,
                    };
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bones_player_mut(&model_name).animation_blend_time = ms;
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(())
                },
                "rootLock" => {
                    let locked = match value { Datum::Int(i) => *i != 0, _ => false };
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bones_player_mut(&model_name).root_lock = locked;
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(())
                },
                "currentTime" => {
                    let time = match value {
                        Datum::Int(i) => *i as f32 / 1000.0,
                        Datum::Float(f) => *f as f32 / 1000.0,
                        _ => 0.0,
                    };
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bones_player_mut(&model_name).animation_time = time;
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(())
                },
                "currentLoopState" => {
                    let looping = match value { Datum::Int(i) => *i != 0, _ => false };
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bones_player_mut(&model_name).animation_loop = looping;
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(())
                },
                "autoBlend" | "blendFactor" | "positionReset" | "rotationReset" | "lockTranslation" => {
                    // Accept but don't apply yet
                    Ok(())
                },
                // Camera properties
                "fieldOfView" | "projectionAngle" => {
                    let fov = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 30.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    node.fov = fov;
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "hither" | "nearClipPlane" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 1.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    node.near_plane = v;
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "yon" | "farClipPlane" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 10000.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    node.far_plane = v;
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "resource" => {
                    // model.resource = modelResource — link model node to a model resource
                    let res_name = match value {
                        Datum::Shockwave3dObjectRef(r) if r.object_type == "modelResource" => r.name.clone(),
                        Datum::String(s) => s.clone(),
                        _ => String::new(),
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    node.model_resource_name = res_name;
                                }
                            }
                        }
                    }
                    Ok(())
                },
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
                            // Track detached nodes for renderer filtering (case-insensitive)
                            if is_detach {
                                w3d.runtime_state.detached_nodes.insert(s3d_ref.name.clone());
                            } else {
                                // Remove by case-insensitive match
                                w3d.runtime_state.detached_nodes.retain(|n| !n.eq_ignore_ascii_case(&s3d_ref.name));
                            }
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    node.parent_name = parent_name;
                                }
                            }
                        }
                    }
                    Ok(())
                },
                // Light properties
                "color" => {
                    if s3d_ref.object_type == "light" {
                        let (r, g, b) = match value {
                            Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)) => (*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0),
                            _ => (1.0, 1.0, 1.0),
                        };
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    if let Some(light) = scene.lights.iter_mut().find(|l| l.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                        light.color = [r, g, b];
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "specular" => {
                    // For lights: specular = 1/0 (enable/disable specular contribution)
                    // For shaders: handled elsewhere
                    Ok(())
                },
                "spotAngle" => {
                    if s3d_ref.object_type == "light" {
                        let angle = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 30.0 };
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    if let Some(light) = scene.lights.iter_mut().find(|l| l.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                        light.spot_angle = angle;
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                },
                // Camera fog properties (flat name form: camera.fog.near = X via Lingo
                // bytecode that flattens the chain). The two-step form
                // `camera.fog → fog ref; fog.near = X` lands in the "near"/"far"/...
                // arms further down with object_type == "fog".
                "fog.enabled" => {
                    let enabled = match value { Datum::Int(v) => *v != 0, _ => false };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.fog_enabled = enabled;
                        }
                    }
                    Ok(())
                },
                "fog.near" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 1.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.fog_near = v;
                        }
                    }
                    Ok(())
                },
                "fog.far" => {
                    let v = match value { Datum::Float(f) => *f as f32, Datum::Int(i) => *i as f32, _ => 1000.0 };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.fog_far = v;
                        }
                    }
                    Ok(())
                },
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
                            if let Datum::List(_, items, _) = player.get_datum_mut(&list_ref) {
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
                },
                "diffuse" | "ambient" | "emissive" | "specular" => {
                  if s3d_ref.object_type != "shader" { return Ok(()); }
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
                                match_ci!(prop_name, {
                                    "diffuse" => mat.diffuse = color,
                                    "ambient" => mat.ambient = color,
                                    "emissive" => {
                                        mat.emissive = color;
                                        if s3d_ref.name.contains("overlay") {
                                            static EM_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                                            if EM_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                                                log(&format!(
                                                    "[EMISSIVE] shader='{}' mat='{}' emissive=({:.2},{:.2},{:.2})",
                                                    s3d_ref.name, mat.name, color[0], color[1], color[2]
                                                ));
                                            }
                                        }
                                    },
                                    "specular" => mat.specular = color,
                                    _ => {},
                                })
                            }
                        }
                    }
                    Ok(())
                },
                "blend" => {
                    // For overlay / backdrop refs, write through to the
                    // CameraOverlay.blend field. Without this, the script
                    // pattern `View.camera.overlay[N].blend = 0` (used to
                    // hide the emoticon panel and similar UI overlays) was
                    // silently swallowed by the original
                    // `if s3d_ref.object_type != "shader" { return Ok(()); }`
                    // guard, so overlays could never be hidden after they
                    // were shown.
                    if s3d_ref.object_type == "overlay" || s3d_ref.object_type == "backdrop" {
                        let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                        let cam_name = parts.get(0).unwrap_or(&"").to_ascii_lowercase();
                        let ov_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let new_blend = value.to_float().unwrap_or(100.0);
                        let is_overlay = s3d_ref.object_type == "overlay";
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                let map = if is_overlay {
                                    &mut w3d.runtime_state.camera_overlays
                                } else {
                                    &mut w3d.runtime_state.camera_backdrops
                                };
                                if let Some(list) = map.get_mut(&cam_name) {
                                    if let Some(ov) = list.get_mut(ov_idx) {
                                        ov.blend = new_blend;
                                    }
                                }
                            }
                        }
                        return Ok(());
                    }
                    if s3d_ref.object_type != "shader" { return Ok(()); }
                    // blend = 0-100 → opacity 0.0-1.0
                    let blend_val = value.to_float().unwrap_or(100.0) as f32;
                    if s3d_ref.name == "DefaultShader" || blend_val < 99.0 {
                        log(&format!(
                            "[W3D-BLEND] shader=\"{}\" blend={:.1} → opacity={:.3}",
                            s3d_ref.name, blend_val, blend_val / 100.0
                        ));
                    }
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
                },
                "transparent" => {
                    // transparent = 1 → the shader alpha-BLENDS (soft), Director's
                    // default. Track the shader so a model wearing it is drawn in the
                    // transparent pass even at blend=100, rather than the hard alpha-test
                    // cutout pass. transparent = 0 → opaque (texture alpha ignored).
                    if s3d_ref.object_type == "shader" {
                        let on = match &value {
                            Datum::Int(v) => *v != 0,
                            Datum::Float(v) => *v != 0.0,
                            _ => true,
                        };
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if on {
                                    w3d.runtime_state.transparent_shaders.insert(s3d_ref.name.clone());
                                } else {
                                    w3d.runtime_state.transparent_shaders.remove(&s3d_ref.name);
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "shininess" => {
                    // Director shader.shininess is 0..100; IFX stores it on the material
                    // as a 0..1 value (GL_SHININESS = that * 128). Route it onto the
                    // shader's material so the renderer's specular exponent picks it up.
                    if s3d_ref.object_type == "shader" {
                        let s01 = (value.to_float().unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    // Get-or-create the shader's material (same pattern as
                                    // the blend setter): runtime newShader()s have no material
                                    // until a property is set, so create one and link it.
                                    let mat_info = {
                                        let shader = scene.shaders.iter_mut()
                                            .find(|s| s.name.eq_ignore_ascii_case(&s3d_ref.name));
                                        shader.map(|shader| {
                                            if shader.material_name.is_empty() {
                                                shader.material_name = format!("{}_mat", s3d_ref.name);
                                            }
                                            shader.material_name.clone()
                                        })
                                    };
                                    if let Some(mat_name) = mat_info {
                                        if let Some(mat) = scene.materials.iter_mut().find(|m| m.name.eq_ignore_ascii_case(&mat_name)) {
                                            mat.shininess = s01;
                                        } else {
                                            use crate::director::chunks::w3d::types::W3dMaterial;
                                            scene.materials.push(W3dMaterial { name: mat_name, shininess: s01, ..Default::default() });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "flat" | "renderStyle" => {
                    // Accept these shader properties silently
                    Ok(())
                },
                "useDiffuseWithTexture" | "usediffusewithtexture" => {
                    let val = match value {
                        Datum::Int(v) => *v != 0,
                        Datum::Float(v) => *v != 0.0,
                        _ => true,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                    shader.use_diffuse_with_texture = val;
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "reflectionMap" | "reflectionmap" => {
                    // 3D shader helper property (Director 11.5 Scripting Dictionary,
                    // "reflectionMap"): sets the texture used for reflections on the
                    // model surface. It is applied to the THIRD texture layer and is
                    // exactly equivalent to:
                    //   shader.textureList[3]       = tex
                    //   shader.textureModeList[3]   = #reflection
                    //   shader.blendFunctionList[3] = #blend
                    //   shader.blendSourceList[3]   = #constant
                    //   shader.blendConstantList[3] = 50.0
                    // (encodings mirror the blend*List setters in this file:
                    //  tex_mode 4=#reflection, blend_func 3=#blend, blend_src 1=#constant.)
                    if s3d_ref.object_type != "shader" { return Ok(()); }
                    let tex_name = match value {
                        Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                        Datum::String(s) => s.clone(),
                        Datum::Void => String::new(),
                        _ => String::new(),
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                    // The reflection texture lives on layer index 2 (Lingo
                                    // textureList[3]); grow the list so it exists.
                                    while shader.texture_layers.len() <= 2 {
                                        shader.texture_layers.push(
                                            crate::director::chunks::w3d::types::W3dTextureLayer::default(),
                                        );
                                    }
                                    let layer = &mut shader.texture_layers[2];
                                    layer.name = tex_name;
                                    layer.tex_mode = 4;     // #reflection
                                    layer.blend_func = 3;   // #blend
                                    layer.blend_src = 1;    // #constant (IFX BlendSource: 0=alpha,1=constant)
                                    // Default constant 50% per the dictionary; a later
                                    // blendConstantList[3] assignment overrides it.
                                    if layer.blend_const <= 0.0 {
                                        layer.blend_const = 0.5;
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                },
                "rootNode" => {
                  if s3d_ref.object_type != "camera" { return Ok(()); }
                    let cam_key = s3d_ref.name.to_ascii_lowercase();
                    let root_name = match value {
                        Datum::Shockwave3dObjectRef(r) => Some(r.name.clone()),
                        Datum::Void => None,
                        _ => None,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(name) = root_name {
                                w3d.runtime_state.camera_root_nodes.insert(cam_key, name);
                            } else {
                                w3d.runtime_state.camera_root_nodes.remove(&cam_key);
                            }
                        }
                    }
                    Ok(())
                },
                "clearAtRender" => {
                  if s3d_ref.object_type != "colorBuffer" { return Ok(()); }
                    let cam_key = s3d_ref.name.to_ascii_lowercase();
                    let val = match value {
                        Datum::Int(v) => *v != 0,
                        _ => true,
                    };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.camera_clear_at_render.insert(cam_key, val);
                        }
                    }
                    Ok(())
                },
                _ => {
                  if s3d_ref.object_type == "overlay" || s3d_ref.object_type == "backdrop" {
                    // Set overlay/backdrop properties: source, loc, blend, scale, regPoint, rotation.
                    // Lookup must be case-insensitive — camera_overlays is keyed by
                    // lowercased camera name (see addOverlay).
                    let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                    let cam_name = parts.get(0).unwrap_or(&"").to_ascii_lowercase();
                    let ov_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let is_overlay = s3d_ref.object_type == "overlay";

                    // Pre-extract values that need player borrows (for Point datums)
                    let (loc_vals, reg_vals) = match prop_name {
                        "loc" => {
                            if let Datum::Point(p, _f) = value {
                                (Some([p[0], p[1]]), None)
                            } else { (None, None) }
                        }
                        "regPoint" => {
                            if let Datum::Point(p, _f) = value {
                                (None, Some([p[0], p[1]]))
                            } else { (None, None) }
                        }
                        _ => (None, None),
                    };

                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let map = if is_overlay { &mut w3d.runtime_state.camera_overlays } else { &mut w3d.runtime_state.camera_backdrops };
                            if let Some(list) = map.get_mut(&cam_name) {
                                if let Some(ov) = list.get_mut(ov_idx) {
                                    match_ci!(prop_name, {
                                        "source" => {
                                            ov.source_texture = match value {
                                                Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                                                Datum::String(s) => s.clone(),
                                                _ => String::new(),
                                            };
                                            ov.source_texture_lower = ov.source_texture.to_lowercase();
                                        },
                                        "loc" => { if let Some(v) = loc_vals { ov.loc = v; } },
                                        "blend" => ov.blend = value.to_float().unwrap_or(100.0),
                                        "scale" => ov.scale = value.to_float().unwrap_or(1.0),
                                        "rotation" => ov.rotation = value.to_float().unwrap_or(0.0),
                                        "regPoint" => { if let Some(v) = reg_vals { ov.reg_point = v; } },
                                        _ => {},
                                    })
                                }
                            }
                        }
                    }
                    Ok(())
                  } else if s3d_ref.object_type == "lod" {
                    // LOD modifier set properties
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let lod = w3d.runtime_state.lod_state
                                .entry(s3d_ref.name.clone())
                                .or_insert_with(crate::player::cast_member::LodState::default);
                            match_ci!(prop_name, {
                                "level" => lod.level = value.int_value().unwrap_or(100),
                                "auto" => lod.auto_mode = value.int_value().unwrap_or(1) != 0,
                                "bias" => lod.bias = value.to_float().unwrap_or(100.0) as f32,
                                _ => {},
                            });
                            // Re-decode CLOD meshes ONLY when an explicit `level` is set.
                            // `bias`/`auto` configure the auto-LOD system (distance-based) and must
                            // NOT re-tessellate the mesh on assignment — Director's `lod.bias` "has
                            // no effect when auto is FALSE" and never swaps geometry by itself.
                            // Re-decoding on a `bias` set replaced the good mesh with a re-decoded
                            // one whose texcoords were lost, so a skinned, script-textured model
                            // (the LEGO minifig) went untextured/yellow one frame after
                            // `lod.bias = 15`.
                            let lod_level = lod.level;
                            let node_name = s3d_ref.name.clone();
                            if prop_name.eq_ignore_ascii_case("level") {
                              if let Some(scene) = w3d.scene_mut() {
                                // Find the resource name for this model node
                                let resource_key = scene.nodes.iter()
                                    .find(|n| n.name == node_name)
                                    .map(|n| {
                                        if !n.model_resource_name.is_empty() {
                                            n.model_resource_name.clone()
                                        } else {
                                            n.resource_name.clone()
                                        }
                                    });
                                if let Some(ref key) = resource_key {
                                    if let Some(decoder) = scene.clod_decoders.get(key) {
                                        let lod_f = (lod_level as f32) / 100.0;
                                        let meshes = decoder.get_decoded_meshes_at_lod(lod_f);
                                        log(&format!(
                                            "[W3D-LOD] model=\"{}\" resource=\"{}\" level={} lod_f={:.2} meshes={}",
                                            node_name, key, lod_level, lod_f, meshes.len()
                                        ));
                                        scene.clod_meshes.insert(key.clone(), meshes);
                                        scene.mesh_content_version += 1;
                                    }
                                }
                              }
                            }
                        }
                    }
                    Ok(())
                  } else if s3d_ref.object_type == "sds" {
                    // Subdivision Surface modifier set properties
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let sds = w3d.runtime_state.sds_state
                                .entry(s3d_ref.name.clone())
                                .or_insert_with(crate::player::cast_member::SdsState::default);
                            match_ci!(prop_name, {
                                "depth" => sds.depth = value.int_value().unwrap_or(1) as i32,
                                "tension" => sds.tension = value.to_float().unwrap_or(0.0) as f32,
                                "error" => sds.error = value.to_float().unwrap_or(0.0) as f32,
                                "enabled" => sds.enabled = value.int_value().unwrap_or(1) != 0,
                                _ => {},
                            })
                        }
                    }
                    Ok(())
                  } else if s3d_ref.object_type == "texture" && prop_name.eq_ignore_ascii_case("member") {
                    // texture("name").member = castMember — re-snapshot the
                    // texture's RGBA from the source member's current image.
                    // newTexture(#fromCastMember) only copies once at creation;
                    // scripts that build a UI image into a member then assign
                    // `texture.member = member` rely on this re-bind to push
                    // the updated pixels to the texture. Without it, overlays
                    // (Avatar Options, ToolTips, etc.) keep showing whatever
                    // stale content was captured at newTexture time — usually
                    // a blank image.
                    let source_member_ref = match value {
                        Datum::CastMember(r) => Some(r.clone()),
                        _ => None,
                    };
                    if let Some(src_ref) = source_member_ref {
                        let rgba_data = {
                            let src_member = player.movie.cast_manager.find_member_by_ref(&src_ref);
                            src_member.and_then(|m| {
                                match &m.member_type {
                                    crate::player::cast_member::CastMemberType::Bitmap(bmp_member) => {
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
                                        Some((w, h, rgba))
                                    }
                                    _ => None,
                                }
                            })
                        };
                        if let Some((w, h, rgba)) = rgba_data {
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
                  } else if s3d_ref.object_type == "texture" && prop_name.eq_ignore_ascii_case("image") {
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
                  } else if s3d_ref.object_type == "modelResource" {
                    // modelResource property set: vertexList, textureCoordinateList, colorList, normalList
                    use crate::player::cast_member::MeshBuildData;
                    match_ci!(prop_name, {
                        // Text3D geometry params on an extrude3d resource (textres.tunnelDepth
                        // = 5, .bevelType = #round, ...): update state + re-extrude. A no-op
                        // for non-text resources, so it never errors.
                        "tunnelDepth" | "tunneldepth" | "bevelDepth" | "beveldepth"
                        | "bevelType" | "beveltype" | "smoothness" => {
                            crate::player::handlers::datum_handlers::cast_member::shockwave3d::Shockwave3dMemberHandlers::set_extruded_text_param(
                                player, &member_ref, &s3d_ref.name, prop_name, value,
                            );
                        },
                        "vertexList" => {
                            let verts: Vec<[f32; 3]> = if let Datum::List(_, items, _) = value {
                                items.iter().map(|r| {
                                    match player.get_datum(r) {
                                        Datum::Vector(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
                                        _ => [0.0, 0.0, 0.0],
                                    }
                                }).collect()
                            } else { vec![] };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.mesh_build_data
                                        .entry(s3d_ref.name.clone())
                                        .or_insert_with(MeshBuildData::default)
                                        .vertex_list = verts;
                                }
                            }
                        },
                        "textureCoordinateList" => {
                            let coords: Vec<[f32; 2]> = if let Datum::List(_, items, _) = value {
                                items.iter().map(|r| {
                                    match player.get_datum(r) {
                                        Datum::List(_, uv, _) if uv.len() >= 2 => {
                                            let u = player.get_datum(&uv[0]).to_float().unwrap_or(0.0) as f32;
                                            let v_val = player.get_datum(&uv[1]).to_float().unwrap_or(0.0) as f32;
                                            [u, v_val]
                                        }
                                        _ => [0.0, 0.0],
                                    }
                                }).collect()
                            } else { vec![] };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.mesh_build_data
                                        .entry(s3d_ref.name.clone())
                                        .or_insert_with(MeshBuildData::default)
                                        .texture_coordinate_list = coords;
                                }
                            }
                        },
                        "colorList" => {
                            let colors: Vec<(u8, u8, u8)> = if let Datum::List(_, items, _) = value {
                                items.iter().map(|r| {
                                    match player.get_datum(r) {
                                        Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)) => (*r, *g, *b),
                                        _ => (255, 255, 255),
                                    }
                                }).collect()
                            } else { vec![] };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.mesh_build_data
                                        .entry(s3d_ref.name.clone())
                                        .or_insert_with(MeshBuildData::default)
                                        .color_list = colors;
                                }
                            }
                        },
                        "normalList" => {
                            let normals: Vec<[f32; 3]> = if let Datum::List(_, items, _) = value {
                                items.iter().map(|r| {
                                    match player.get_datum(r) {
                                        Datum::Vector(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
                                        _ => [0.0, 1.0, 0.0],
                                    }
                                }).collect()
                            } else { vec![] };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.mesh_build_data
                                        .entry(s3d_ref.name.clone())
                                        .or_insert_with(MeshBuildData::default)
                                        .normal_list = normals;
                                }
                            }
                        },
                        "width" | "length" | "height" | "radius"
                        | "topRadius" | "bottomRadius" | "resolution"
                        | "startAngle" | "endAngle" | "topCap" | "bottomCap" => {
                            use crate::director::chunks::w3d::types::ClodDecodedMesh;
                            let val = value.to_float().unwrap_or(1.0) as f32;
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(res) = scene.model_resources.get_mut(&s3d_ref.name) {
                                            // The outer match_ci! matched case-insensitively; normalise
                                            // for the inner exact match.
                                            match prop_name.to_ascii_lowercase().as_str() {
                                                "width" => res.primitive_width = val,
                                                "length" => res.primitive_length = val,
                                                "height" => res.primitive_height = val,
                                                // #cylinder taper: bottomRadius -> radius, topRadius -> top.
                                                // A bare `radius` (e.g. sphere) sets both so a cylinder
                                                // stays uniform unless top/bottom are set explicitly.
                                                "radius" => { res.primitive_radius = val; res.primitive_top_radius = val; },
                                                "topradius" => res.primitive_top_radius = val,
                                                "bottomradius" => res.primitive_radius = val,
                                                "resolution" => res.primitive_resolution = val.round().max(3.0) as u32,
                                                "startangle" => res.primitive_start_angle = val,
                                                "endangle" => res.primitive_end_angle = val,
                                                "topcap" => res.primitive_top_cap = val != 0.0,
                                                "bottomcap" => res.primitive_bottom_cap = val != 0.0,
                                                _ => {}
                                            }
                                        }
                                        // Regenerate the mesh from the primitive dimensions.
                                        // Box: scale unit cube by (width, length, height).
                                        // Sphere: scale unit sphere by radius.
                                        if let Some(res) = scene.model_resources.get(&s3d_ref.name) {
                                            let ptype = res.primitive_type.as_deref().unwrap_or("");
                                            let meshes = match ptype {
                                                "box" => {
                                                    // Director convention: width=X, height=Y, length=Z
                                                    let hx = res.primitive_width / 2.0;  // X half
                                                    let hy = res.primitive_height / 2.0; // Y half
                                                    let hz = res.primitive_length / 2.0; // Z half
                                                    let p = vec![
                                                        [-hx,-hy,hz],[hx,-hy,hz],[hx,hy,hz],[-hx,hy,hz],
                                                        [hx,-hy,-hz],[-hx,-hy,-hz],[-hx,hy,-hz],[hx,hy,-hz],
                                                        [-hx,hy,hz],[hx,hy,hz],[hx,hy,-hz],[-hx,hy,-hz],
                                                        [-hx,-hy,-hz],[hx,-hy,-hz],[hx,-hy,hz],[-hx,-hy,hz],
                                                        [hx,-hy,hz],[hx,-hy,-hz],[hx,hy,-hz],[hx,hy,hz],
                                                        [-hx,-hy,-hz],[-hx,-hy,hz],[-hx,hy,hz],[-hx,hy,-hz],
                                                    ];
                                                    let n = vec![
                                                        [0.0,0.0,1.0],[0.0,0.0,1.0],[0.0,0.0,1.0],[0.0,0.0,1.0],
                                                        [0.0,0.0,-1.0],[0.0,0.0,-1.0],[0.0,0.0,-1.0],[0.0,0.0,-1.0],
                                                        [0.0,1.0,0.0],[0.0,1.0,0.0],[0.0,1.0,0.0],[0.0,1.0,0.0],
                                                        [0.0,-1.0,0.0],[0.0,-1.0,0.0],[0.0,-1.0,0.0],[0.0,-1.0,0.0],
                                                        [1.0,0.0,0.0],[1.0,0.0,0.0],[1.0,0.0,0.0],[1.0,0.0,0.0],
                                                        [-1.0,0.0,0.0],[-1.0,0.0,0.0],[-1.0,0.0,0.0],[-1.0,0.0,0.0],
                                                    ];
                                                    let uv_face = vec![[0.0,1.0_f32],[1.0,1.0],[1.0,0.0],[0.0,0.0]];
                                                    let mut uv = Vec::with_capacity(24);
                                                    for _ in 0..6 { uv.extend_from_slice(&uv_face); }
                                                    let f = vec![
                                                        [0u32,1,2],[0,2,3],[4,5,6],[4,6,7],[8,9,10],[8,10,11],
                                                        [12,13,14],[12,14,15],[16,17,18],[16,18,19],[20,21,22],[20,22,23],
                                                    ];
                                                    vec![ClodDecodedMesh {
                                                        name: s3d_ref.name.clone(),
                                                        positions: p, normals: n,
                                                        tex_coords: vec![uv], faces: f,
                                                        diffuse_colors: vec![], specular_colors: vec![],
                                                        bone_indices: vec![], bone_weights: vec![],
                                                    }]
                                                },
                                                "sphere" => {
                                                    // Exact match to Director's #sphere, verified by dumping pacModel
                                                    // via the meshDeform modifier:
                                                    //   phi (polar, 0=+Y pole .. PI=-Y pole) over `stacks` steps,
                                                    //   L = startAngle + sweep*(j/slices) swept around the Y axis
                                                    //   from +Z toward +X, over `slices` steps.
                                                    //   x = r·sin(phi)·sin(L),  y = r·cos(phi),  z = r·sin(phi)·cos(L)
                                                    //   u = j/slices  (fraction along the sweep)
                                                    //   v = 1 - phi/PI  (1 at the +Y pole, 0 at -Y)
                                                    // startAngle 25 / endAngle 335 leaves the 50° Pacman mouth open;
                                                    // startAngle 180 gives the ghost-dome hemisphere.
                                                    use std::f32::consts::PI;
                                                    let r = res.primitive_radius;
                                                    let start = res.primitive_start_angle.to_radians();
                                                    let mut sweep = (res.primitive_end_angle - res.primitive_start_angle).to_radians();
                                                    if sweep <= 0.0 { sweep = 2.0 * PI; }
                                                    // Director's #sphere tessellation vs `resolution`, dumped from real
                                                    // Director via meshDeform across res 2..50 (verts=(stacks+1)(slices+1)):
                                                    //   slices = resolution + 3,  stacks = round((2·res + 5) / 3).
                                                    // When resolution is unset (0) keep the legacy default density
                                                    // (stacks=20, slices=28 → 609 verts) that PacMan3D's spheres rely on.
                                                    // Partial sweeps (Pacman mouth / ghost dome) scale slices by the swept
                                                    // fraction so angular density stays constant.
                                                    let (stacks, full_slices) = if res.primitive_resolution >= 1 {
                                                        let rn = res.primitive_resolution;
                                                        ((((2 * rn + 5) as f32) / 3.0).round() as u32, rn + 3)
                                                    } else {
                                                        (20u32, 28u32)
                                                    };
                                                    let slices = ((sweep / (2.0 * PI)) * full_slices as f32).round().max(3.0) as u32;
                                                    let stride = slices + 1;
                                                    let mut pos = Vec::new();
                                                    let mut nrm = Vec::new();
                                                    let mut uvs = Vec::new();
                                                    let mut faces = Vec::new();
                                                    for i in 0..=stacks {
                                                        let phi = PI * i as f32 / stacks as f32; // 0 = +Y pole .. PI = -Y pole
                                                        let (sp, cp) = (phi.sin(), phi.cos());
                                                        for j in 0..=slices {
                                                            let l = start + sweep * (j as f32 / slices as f32);
                                                            let (sl, cl) = (l.sin(), l.cos());
                                                            // Verified against BOTH the Pacman and ghost-dome
                                                            // meshDeform dumps: x = -sin(theta), z = cos(theta).
                                                            // (For a sphere whose sweep is centred on theta=0, like
                                                            // Pacman's mouth or a full sphere, +sin and -sin are
                                                            // identical; the ghost dome's startAngle 180 hemisphere
                                                            // exposes the real sign — it must sit on +X so Rz(90)
                                                            // lifts it into a head.)
                                                            let nx = -sp * sl; let ny = cp; let nz = sp * cl;
                                                            pos.push([r * nx, r * ny, r * nz]);
                                                            nrm.push([nx, ny, nz]);
                                                            // Director-dump texcoords: u = 1 - j/slices, v = 1 - i/stacks.
                                                            // The shared 3D vertex shader applies the CLOD remap
                                                            // (u+0.5, 0.5-v) to the main texcoord, so emit PRE-CENTERED
                                                            // here; after the remap they reconstruct the dumped (u, v).
                                                            let u = 1.0 - j as f32 / slices as f32;
                                                            let v = 1.0 - i as f32 / stacks as f32;
                                                            uvs.push([u - 0.5, 0.5 - v]);
                                                        }
                                                    }
                                                    for i in 0..stacks {
                                                        for j in 0..slices {
                                                            let a = i * stride + j;
                                                            let b = a + stride;
                                                            // With x=-sin·sin, z=cos·sin the outward winding is
                                                            // [a, a+1, b]. Partial sweeps simply omit the gap faces,
                                                            // leaving the mouth/section open.
                                                            faces.push([a, a + 1, b]);
                                                            faces.push([a + 1, b + 1, b]);
                                                        }
                                                    }
                                                    vec![ClodDecodedMesh {
                                                        name: s3d_ref.name.clone(),
                                                        positions: pos, normals: nrm,
                                                        tex_coords: vec![uvs], faces,
                                                        diffuse_colors: vec![], specular_colors: vec![],
                                                        bone_indices: vec![], bone_weights: vec![],
                                                    }]
                                                },
                                                "cylinder" => {
                                                    // Director #cylinder: axis along Y, centered. The radial segment
                                                    // count comes from `resolution` (default 20; Pacman pipes use 6
                                                    // for hexagonal tubes). topRadius/bottomRadius give the taper
                                                    // (topRadius 0 => cone). topCap/bottomCap default TRUE (real-
                                                    // world cans/logs have solid ends; the ghost body sets both 0).
                                                    //
                                                    // startAngle/endAngle sweep the side wall around the Y axis
                                                    // (Director 11.5 dict, startAngle/endAngle: "The surface of a
                                                    // cylinder is generated by sweeping a 2D line around the Y axis
                                                    // from startAngle to endAngle ... To draw a section of a cylinder,
                                                    // set endAngle to a value less than 360"). A section is left OPEN
                                                    // along the angular cut — only the caps become half-discs; there
                                                    // is no flat closing face (same as the sphere-section example).
                                                    // frog01's floating logs are `logRes` cylinders with
                                                    // startAngle=180 (a 180° half-pipe) rotated vector(dir,0,90): the
                                                    // +X wall rolls to +Y (rounded side up) and the open −X side to
                                                    // −Y (down into the water), so it reads as a solid floating log.
                                                    //
                                                    // Angular convention MUST match the meshDeform-verified #sphere:
                                                    // x = -sin(L), z = cos(L) (L=startAngle ⇒ +Z). Using cos/sin
                                                    // instead is harmless for a full circle but places a partial arc
                                                    // 90° off, which after the log's roll would gape sideways.
                                                    use std::f32::consts::PI;
                                                    let bottom_r = res.primitive_radius;
                                                    let top_r = res.primitive_top_radius;
                                                    let hy = res.primitive_height / 2.0;
                                                    let start = res.primitive_start_angle.to_radians();
                                                    let mut sweep = (res.primitive_end_angle - res.primitive_start_angle).to_radians();
                                                    if sweep <= 0.0 { sweep = 2.0 * PI; }
                                                    // Keep the facet density of the default full cylinder: scale the
                                                    // radial segment count by the swept fraction (half sweep → half
                                                    // the segments) so a 180° log stays smooth without over-tessellating.
                                                    let base = if res.primitive_resolution >= 3 { res.primitive_resolution } else { 20 };
                                                    // Director does NOT thin out a section's facets. meshDeform of
                                                    // frog01's 180° log (resolution=20 default) shows ~22 radial
                                                    // segments (≈8°/facet) — it keeps the full facet COUNT for the
                                                    // swept arc rather than scaling it by the arc fraction. dirplayer
                                                    // was doing `sweep/2π * base` (180° log → only 10 facets), so the
                                                    // logs rendered visibly polygonal / "smaller" with flat-panel
                                                    // edges. Use the full `base` count for the arc. A full circle
                                                    // (sweep=2π) is unchanged (still `base`), so Coke-can / Pacman-pipe
                                                    // (resolution 6 → hexagon) cylinders keep their exact facet counts.
                                                    let segs = base.max(2);
                                                    let stride = segs + 1;
                                                    // A SECTIONED cylinder (sweep < 360°, e.g. frog01's 180° half-pipe
                                                    // logs) is an open trough — you can see its INSIDE. dirplayer
                                                    // back-face-culls, so looking into the open end showed THROUGH the
                                                    // single-sided wall to the water (the "gap in the caps"). Director
                                                    // renders the trough two-sided (wood inside). Emit reversed faces
                                                    // for the wall + caps when sectioned so the interior is solid. A
                                                    // full cylinder (Coke can / Pacman pipe) stays single-sided.
                                                    // A sectioned cylinder (open trough) is two-sided, AND a
                                                    // resource authored #back/#both (skybox cylinder viewed from
                                                    // inside) must render its inward surface — otherwise the full
                                                    // cylinder is single-sided outward and the inside is culled
                                                    // to black. SweeTarts' skybox is newModelResource(#cylinder,#back).
                                                    let facing = res.primitive_facing.as_str();
                                                    let two_sided = sweep < (2.0 * PI - 1e-3)
                                                        || facing == "back" || facing == "both";
                                                    // Director's #cylinder is THREE mesh groups: side wall, top cap,
                                                    // bottom cap — so shaderList[1]/[2]/[3] can each differ (the Coke
                                                    // can is the cokeT label on the side and silver canTop on the
                                                    // caps). Emit each as a separate same-named mesh; the renderer
                                                    // binds a per-mesh-index shader from node_shaders (set by Lingo
                                                    // shaderList[i]=ref). Cap groups are gated by the cap flags (the
                                                    // ghost body sets both to 0 → side only).
                                                    let mut out: Vec<ClodDecodedMesh> = Vec::new();
                                                    // group 0 → shaderList[1]: side wall
                                                    {
                                                        let mut pos = Vec::new();
                                                        let mut nrm = Vec::new();
                                                        let mut uvs = Vec::new();
                                                        let mut faces = Vec::new();
                                                        for ring in 0..=1u32 {
                                                            let (y, r) = if ring == 0 { (hy, top_r) } else { (-hy, bottom_r) };
                                                            for i in 0..=segs {
                                                                let u = i as f32 / segs as f32;
                                                                let l = start + sweep * u;
                                                                let (sl, cl) = (l.sin(), l.cos());
                                                                pos.push([-sl * r, y, cl * r]);
                                                                nrm.push([-sl, 0.0, cl]);
                                                                // pre-centre to cancel the shader CLOD remap; final UV
                                                                // is (u, ring): v=0 at the top ring, v=1 at the bottom
                                                                // edge (where GTEX bakes its black sawtooth).
                                                                uvs.push([u - 0.5, 0.5 - ring as f32]);
                                                            }
                                                        }
                                                        // segs quads connect slice i→i+1; a full sweep's last vertex
                                                        // coincides with the first so it closes, a partial sweep stops
                                                        // at endAngle leaving the cut open.
                                                        for i in 0..segs {
                                                            let a = i;
                                                            let b = a + stride;
                                                            faces.push([a, a + 1, b]);
                                                            faces.push([a + 1, b + 1, b]);
                                                            if two_sided {
                                                                // reversed winding → inner trough surface
                                                                faces.push([a, b, a + 1]);
                                                                faces.push([a + 1, b, b + 1]);
                                                            }
                                                        }
                                                        out.push(ClodDecodedMesh {
                                                            name: s3d_ref.name.clone(), positions: pos, normals: nrm,
                                                            tex_coords: vec![uvs], faces,
                                                            diffuse_colors: vec![], specular_colors: vec![],
                                                            bone_indices: vec![], bone_weights: vec![],
                                                        });
                                                    }
                                                    // group 1 → shaderList[2]: top cap (+Y), disc UV pre-centred.
                                                    // SEAL with a FULL disc (0..2π), not just the swept arc: a half-disc
                                                    // cap leaves the trough side of the END open, so a sideways log
                                                    // (axis along X) showed the open trough at its ends ("caps open").
                                                    // A full cylinder's cap is already a full circle, so it's unchanged.
                                                    // Use 2× segments when sectioned so the +arc half stays as smooth as
                                                    // the wall.
                                                    if res.primitive_top_cap {
                                                        let cap_segs = if two_sided { segs * 2 } else { segs };
                                                        let mut pos: Vec<[f32; 3]> = vec![[0.0, hy, 0.0]];
                                                        let mut nrm: Vec<[f32; 3]> = vec![[0.0, 1.0, 0.0]];
                                                        let mut uvs: Vec<[f32; 2]> = vec![[0.0, 0.0]];
                                                        let mut faces = Vec::new();
                                                        for i in 0..=cap_segs {
                                                            let l = 2.0 * PI * (i as f32 / cap_segs as f32);
                                                            let (sl, cl) = (l.sin(), l.cos());
                                                            pos.push([-sl * top_r, hy, cl * top_r]);
                                                            nrm.push([0.0, 1.0, 0.0]);
                                                            uvs.push([-sl * 0.5, cl * 0.5]);
                                                        }
                                                        // +Y outward winding for x=-sin,z=cos is [center, i+1, i].
                                                        for i in 0..cap_segs {
                                                            faces.push([0u32, 1 + i + 1, 1 + i]);
                                                            if two_sided { faces.push([0u32, 1 + i, 1 + i + 1]); }
                                                        }
                                                        out.push(ClodDecodedMesh {
                                                            name: s3d_ref.name.clone(), positions: pos, normals: nrm,
                                                            tex_coords: vec![uvs], faces,
                                                            diffuse_colors: vec![], specular_colors: vec![],
                                                            bone_indices: vec![], bone_weights: vec![],
                                                        });
                                                    }
                                                    // group 2 → shaderList[3]: bottom cap (-Y) — full disc seal (see top cap)
                                                    if res.primitive_bottom_cap {
                                                        let cap_segs = if two_sided { segs * 2 } else { segs };
                                                        let mut pos: Vec<[f32; 3]> = vec![[0.0, -hy, 0.0]];
                                                        let mut nrm: Vec<[f32; 3]> = vec![[0.0, -1.0, 0.0]];
                                                        let mut uvs: Vec<[f32; 2]> = vec![[0.0, 0.0]];
                                                        let mut faces = Vec::new();
                                                        for i in 0..=cap_segs {
                                                            let l = 2.0 * PI * (i as f32 / cap_segs as f32);
                                                            let (sl, cl) = (l.sin(), l.cos());
                                                            pos.push([-sl * bottom_r, -hy, cl * bottom_r]);
                                                            nrm.push([0.0, -1.0, 0.0]);
                                                            uvs.push([-sl * 0.5, cl * 0.5]);
                                                        }
                                                        for i in 0..cap_segs {
                                                            faces.push([0u32, 1 + i, 1 + i + 1]);
                                                            if two_sided { faces.push([0u32, 1 + i + 1, 1 + i]); }
                                                        }
                                                        out.push(ClodDecodedMesh {
                                                            name: s3d_ref.name.clone(), positions: pos, normals: nrm,
                                                            tex_coords: vec![uvs], faces,
                                                            diffuse_colors: vec![], specular_colors: vec![],
                                                            bone_indices: vec![], bone_weights: vec![],
                                                        });
                                                    }
                                                    out
                                                },
                                                "plane" => {
                                                    // Director #plane lies in the XY plane: width=X, length=Y,
                                                    // centred, normal +Z (the default-facing plane is two-sided:
                                                    // front mesh +Z, back mesh -Z, so shaderList[1]/[2] address the
                                                    // two faces — the water sets shaderList[2]). The dimension
                                                    // setter had no plane case, so width/length never rebuilt the
                                                    // mesh and every large plane (road 42×18, water, banks, sides,
                                                    // front/back) stayed at the 1×1 default — rendering tiny.
                                                    let hw = res.primitive_width / 2.0;   // X half (width)
                                                    let hl = res.primitive_length / 2.0;  // Y half (length)
                                                    // UVs are PRE-CENTERED to [-0.5, 0.5]. The shared 3D vertex
                                                    // shader applies the CLOD remap `base_uv = (u+0.5, 0.5-v)` to
                                                    // every non-overlay mesh, so a plain [0,1] plane gets shifted
                                                    // half a texture in both axes (the road's pavement bands landed
                                                    // in the middle: "2 lanes / 2 sidewalks / 2 lanes" instead of
                                                    // "sidewalk / 2 lanes / 2 lanes / sidewalk"). Storing
                                                    // (u-0.5, 0.5-v) makes the shader reconstruct the intended [0,1]
                                                    // — same convention the #sphere/#cylinder primitives use.
                                                    //
                                                    // MESH ORDER MUST MATCH DIRECTOR: a meshDeform dump of a real
                                                    // Director #plane shows mesh[1].normal = (0,0,-1) and
                                                    // mesh[2].normal = (0,0,+1). So shaderList[1] is the -Z face and
                                                    // shaderList[2] the +Z face. frog01's walls set one face to
                                                    // clearS (transparent) and the other to a texture; with the order
                                                    // reversed, every clearS/texture landed on the wrong face (the
                                                    // game-over `back` wall showed its opaque side toward the camera
                                                    // and filled the view; `front`'s banner was on the culled side).
                                                    // Emit -Z FIRST (shaderList[1]) then +Z (shaderList[2]).
                                                    vec![
                                                        ClodDecodedMesh {
                                                            name: s3d_ref.name.clone(),
                                                            positions: vec![[-hw,-hl,0.0],[hw,-hl,0.0],[hw,hl,0.0],[-hw,hl,0.0]],
                                                            normals: vec![[0.0,0.0,-1.0]; 4],
                                                            tex_coords: vec![vec![[0.5,-0.5],[-0.5,-0.5],[-0.5,0.5],[0.5,0.5]]],
                                                            faces: vec![[0,2,1],[0,3,2]],
                                                            diffuse_colors: vec![], specular_colors: vec![],
                                                            bone_indices: vec![], bone_weights: vec![],
                                                        },
                                                        ClodDecodedMesh {
                                                            name: s3d_ref.name.clone(),
                                                            positions: vec![[-hw,-hl,0.0],[hw,-hl,0.0],[hw,hl,0.0],[-hw,hl,0.0]],
                                                            normals: vec![[0.0,0.0,1.0]; 4],
                                                            tex_coords: vec![vec![[-0.5,-0.5],[0.5,-0.5],[0.5,0.5],[-0.5,0.5]]],
                                                            faces: vec![[0,1,2],[0,2,3]],
                                                            diffuse_colors: vec![], specular_colors: vec![],
                                                            bone_indices: vec![], bone_weights: vec![],
                                                        },
                                                    ]
                                                },
                                                _ => vec![],
                                            };
                                            if !meshes.is_empty() {
                                                scene.clod_meshes.insert(s3d_ref.name.clone(), meshes);
                                                scene.mesh_content_version += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        _ => {},
                    });
                    Ok(())
                  } else {
                    // Handle meshDeformTexLayer.textureCoordinateList = data
                    if s3d_ref.object_type == "emitter" {
                        use crate::player::cast_member::EmitterState;
                        // emitter.region is assigned as a LIST of positions (Director's API,
                        // e.g. the car demos' `emitter.region = [exhaust.worldPosition]` to make
                        // the smoke follow the tailpipe); a bare vector is also accepted. Resolve
                        // the first position here (needs immutable player access) BEFORE taking the
                        // mutable member borrow. Without this the list silently failed to match
                        // `Datum::Vector`, region stayed (0,0,0), and the white exhaust particles
                        // piled up at the origin — right at the camera — whiting out the scene.
                        let region_override: Option<[f64; 3]> = if prop_name.eq_ignore_ascii_case("region") {
                            match value {
                                Datum::Vector(v) => Some(*v),
                                Datum::List(_, items, _) => items.front().and_then(|r| match player.get_datum(r) {
                                    Datum::Vector(v) => Some(*v),
                                    _ => None,
                                }),
                                _ => None,
                            }
                        } else { None };
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                let em = w3d.runtime_state.emitters
                                    .entry(s3d_ref.name.clone())
                                    .or_insert_with(EmitterState::default);
                                match_ci!(prop_name, {
                                    "loop" => em.is_loop = value.int_value().unwrap_or(1) != 0,
                                    "mode" => em.mode = value.symbol_value().unwrap_or_else(|_| value.string_value().unwrap_or_default()),
                                    "numParticles" => em.num_particles = value.int_value().unwrap_or(100),
                                    "direction" => if let Datum::Vector(v) = value { em.direction = *v; },
                                    "region" => if let Some(rv) = region_override { em.region = rv; em.has_region = true; },
                                    "distribution" => em.distribution = value.symbol_value().unwrap_or_else(|_| value.string_value().unwrap_or_default()),
                                    "angle" => em.angle = value.float_value().unwrap_or(30.0),
                                    "minSpeed" => em.min_speed = value.float_value().unwrap_or(1.0),
                                    "maxSpeed" => em.max_speed = value.float_value().unwrap_or(1.0),
                                    "pathStrength" => em.path_strength = value.float_value().unwrap_or(0.0),
                                    _ => {},
                                })
                            }
                        }
                        return Ok(());
                    }
                    if s3d_ref.object_type == "meshDeformTexLayer" && prop_name == "textureCoordinateList" {
                        let parts: Vec<&str> = s3d_ref.name.splitn(3, ':').collect();
                        let model_name = parts.get(0).unwrap_or(&"").to_string();
                        let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let _layer_idx: usize = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

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

                        let uv_count = coords.len();

                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                // Find the model resource name from the model node
                                let resource_name = w3d.parsed_scene.as_ref().and_then(|scene| {
                                    scene.nodes.iter().find(|n| n.name == model_name).map(|n| {
                                        if !n.model_resource_name.is_empty() {
                                            n.model_resource_name.clone()
                                        } else {
                                            n.resource_name.clone()
                                        }
                                    })
                                });

                                // Try resource name first, then model name, then suffix match
                                let clod_key = w3d.parsed_scene.as_ref().and_then(|scene| {
                                    // 1. Try resource name from node
                                    if let Some(ref rn) = resource_name {
                                        if scene.clod_meshes.contains_key(rn) {
                                            return Some(rn.clone());
                                        }
                                    }
                                    // 2. Try exact model name
                                    if scene.clod_meshes.contains_key(&model_name) {
                                        return Some(model_name.clone());
                                    }
                                    // 3. Try suffix match on model name
                                    let suffix = format!("_{}", model_name);
                                    if let Some(k) = scene.clod_meshes.keys().find(|k| k.ends_with(&suffix)) {
                                        return Some(k.clone());
                                    }
                                    // 4. Try case-insensitive match
                                    let lower = model_name.to_lowercase();
                                    scene.clod_meshes.keys()
                                        .find(|k| k.to_lowercase() == lower)
                                        .cloned()
                                });

                                let found = clod_key.is_some();
                                // Write UVs into the scene's CLOD mesh tex_coords.
                                // Layer 0 → tex_coords[0] (primary UV), layers 1+ → tex_coords[1] (secondary UV).
                                // Only write non-empty data to tex_coords[1] to avoid clearing shadow UVs
                                // when the base layer (layer 0) is cleared.
                                let tc_idx = if _layer_idx == 0 { 0 } else { 1 };
                                if let Some(ref key) = clod_key {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(clod_meshes) = scene.clod_meshes.get_mut(key) {
                                            if let Some(mesh) = clod_meshes.get_mut(mesh_idx) {
                                                // Only write to tex_coords[1] if we have actual data
                                                // (avoid overwriting shadow UVs with empty layer 0 clear)
                                                if tc_idx == 0 && coords.is_empty() {
                                                    // Layer 0 clear — skip, don't overwrite primary UVs
                                                } else {
                                                    while mesh.tex_coords.len() <= tc_idx {
                                                        mesh.tex_coords.push(Vec::new());
                                                    }
                                                    if mesh.tex_coords[tc_idx] != coords {
                                                        mesh.tex_coords[tc_idx] = coords;
                                                        scene.mesh_content_version =
                                                            scene.mesh_content_version.wrapping_add(1);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                log(&format!(
                                    "[W3D-UV2-SET] model=\"{}\" resource={:?} mesh_idx={} layer_idx={} uv_count={} clod_key={:?} found={}",
                                    model_name, resource_name, mesh_idx, _layer_idx, uv_count, clod_key, found
                                ));
                            }
                        }
                        return Ok(());
                    }
                    // meshDeformMesh.vertexList / .normalList = list — deform the
                    // mesh (Director: "get or set the list of vertices/normals used
                    // by the specified mesh"). Write the new vectors into the model's
                    // CLOD mesh and bump the content version so the renderer
                    // re-uploads. Splat's pip tower hides eaten dots and the text
                    // models deform their glyphs this way.
                    if s3d_ref.object_type == "meshDeformMesh"
                        && (prop_name.eq_ignore_ascii_case("vertexList")
                            || prop_name.eq_ignore_ascii_case("normalList"))
                    {
                        let is_normals = prop_name.eq_ignore_ascii_case("normalList");
                        let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                        let model_name = parts.get(0).unwrap_or(&"").to_string();
                        let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                        let verts: Vec<[f32; 3]> = if let Datum::List(_, items, _) = value {
                            items.iter().map(|item_ref| match player.get_datum(item_ref) {
                                Datum::Vector(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
                                _ => [0.0, 0.0, 0.0],
                            }).collect()
                        } else { vec![] };
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                let key = w3d.parsed_scene.as_ref().and_then(|scene| {
                                    let rn = scene.nodes.iter()
                                        .find(|n| n.name.eq_ignore_ascii_case(&model_name))
                                        .map(|n| if !n.model_resource_name.is_empty() {
                                            n.model_resource_name.clone()
                                        } else { n.resource_name.clone() });
                                    rn.filter(|k| scene.clod_meshes.contains_key(k))
                                        .or_else(|| if scene.clod_meshes.contains_key(&model_name) {
                                            Some(model_name.clone())
                                        } else { None })
                                });
                                if let (Some(key), false) = (key, verts.is_empty()) {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(mesh) = scene.clod_meshes.get_mut(&key)
                                            .and_then(|meshes| meshes.get_mut(mesh_idx))
                                        {
                                            if is_normals { mesh.normals = verts; } else { mesh.positions = verts; }
                                            scene.mesh_content_version = scene.mesh_content_version.wrapping_add(1);
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(());
                    }
                    // Log unhandled set_prop for meshDeform types
                    if s3d_ref.object_type.contains("meshDeform") || s3d_ref.object_type.contains("MeshDeform") {
                        console_warn!(
                            "[W3D-SETPROP] unhandled: type=\"{}\" name=\"{}\" prop=\"{}\"",
                            s3d_ref.object_type, s3d_ref.name, prop_name
                        );
                    }
                    Ok(())
                  }
                }
            })
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
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

            match_ci!(handler_name, {
                // ─── Node transform methods ───
                "translate" => {
                    let (dx, dy, dz) = read_xyz_args(player, args);
                    let world = args_relative_to_world(player, args);
                    apply_translation(player, &member_ref, &s3d_ref.name, dx, dy, dz, world);
                    Ok(player.alloc_datum(Datum::Void))
                },
                "rotate" => {
                    let (rx, ry, rz) = read_xyz_args(player, args);
                    let world = args_relative_to_world(player, args);
                    apply_rotation(player, &member_ref, &s3d_ref.name, rx, ry, rz, world);
                    Ok(player.alloc_datum(Datum::Void))
                },
                "scale" => {
                    // Director's model.scale() takes EITHER a single uniform factor
                    // (`scale(9)` → 9,9,9), a vector, or three components. read_xyz_args
                    // only handles the vector / 3-arg forms (a lone scalar fell through
                    // to 0,0,0, collapsing the model — SweeTarts candies/snake vanished).
                    let (sx, sy, sz) = if args.len() == 1 {
                        match player.get_datum(&args[0]) {
                            Datum::Vector(v) => (v[0] as f32, v[1] as f32, v[2] as f32),
                            other => {
                                let f = other.float_value().unwrap_or(1.0) as f32;
                                (f, f, f)
                            }
                        }
                    } else {
                        read_xyz_args(player, args)
                    };
                    apply_scale(player, &member_ref, &s3d_ref.name, sx, sy, sz);
                    Ok(player.alloc_datum(Datum::Void))
                },
                "pointAt" => {
                    if !args.is_empty() {
                        // Director's pointAt accepts EITHER a vector OR a node
                        // (model/group/light/camera) — in the node case it aims at that
                        // node's worldPosition. frog01's death camera uses
                        // `camera.pointAt(s.model("frog"))` (a model) while normal play uses
                        // `camera.pointAt(frog.worldPosition)` (a vector); handling only the
                        // vector form left the death camera at the right spot but never
                        // rotated to face the frog ("camera moves, view wrong"). Also fixes
                        // `light("spot2").pointAt(s.model("frog"))`, same handler.
                        let target_opt: Option<[f32; 3]> = match player.get_datum(&args[0]) {
                            Datum::Vector(target) => {
                                Some([target[0] as f32, target[1] as f32, target[2] as f32])
                            }
                            Datum::Shockwave3dObjectRef(r) => {
                                let name = r.name.clone();
                                let wp = get_world_position(player, &member_ref, &name);
                                Some([wp[0] as f32, wp[1] as f32, wp[2] as f32])
                            }
                            _ => None,
                        };
                        if let Some(target) = target_opt {
                            let (ux, uy, uz) = if args.len() > 1 {
                                if let Datum::Vector(up) = player.get_datum(&args[1]) {
                                    (up[0] as f32, up[1] as f32, up[2] as f32)
                                } else { (0.0f32, 1.0, 0.0) }
                            } else { (0.0f32, 1.0, 0.0) };
                            apply_point_at(player, &member_ref, &s3d_ref.name,
                                target[0], target[1], target[2],
                                ux, uy, uz);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "getWorldTransform" => {
                    // Return world-relative transform (accumulated through parent chain)
                    // Uses case-insensitive lookups throughout (Director is case-insensitive)
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                    let world_t = if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                if let Some(node) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    // Get local transform (runtime override or static)
                                    let local = get_node_transform(player, &member_ref, &node.name);
                                    // Walk parent chain
                                    let mut result = local;
                                    let mut current_parent = node.parent_name.clone();
                                    let mut depth = 0u32;
                                    for _ in 0..20 {
                                        if current_parent.is_empty() || current_parent.eq_ignore_ascii_case("World") { break; }
                                        if let Some(pn) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&current_parent)) {
                                            let pt = get_node_transform(player, &member_ref, &pn.name);
                                            result = mat4_mul_f32(&pt, &result);
                                            current_parent = pn.parent_name.clone();
                                            depth += 1;
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
                },
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

                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some((motion_name, is_loop, start_time_ms, end_time_ms, scale, offset_ms)) = play_args {
                                {
                                    let bp = w3d.runtime_state.bones_player_mut(&model_name);
                                    // Save interrupted motion into front of queue so it resumes later
                                    if let Some(ref cur) = bp.current_motion {
                                        if bp.animation_playing {
                                            let interrupted = crate::player::cast_member::QueuedMotion {
                                                name: cur.clone(),
                                                looped: bp.animation_loop,
                                                start_time: bp.animation_start_time,
                                                end_time: bp.animation_end_time,
                                                scale: bp.animation_scale,
                                                offset: bp.animation_time, // resume from current position
                                            };
                                            bp.motion_queue.insert(0, interrupted);
                                        }
                                    }
                                    // Set up crossfade blending using stored blendTime
                                    let blend_time = bp.animation_blend_time;
                                    if blend_time > 0.0 && bp.current_motion.is_some() {
                                        bp.previous_motion = bp.current_motion.clone();
                                        bp.blend_duration = blend_time / 1000.0;
                                        bp.blend_elapsed = 0.0;
                                        bp.blend_weight = 0.0;
                                    } else {
                                        bp.previous_motion = None;
                                        bp.blend_weight = 1.0;
                                    }

                                    bp.current_motion = Some(motion_name);
                                    bp.animation_playing = true;
                                    bp.animation_loop = is_loop;
                                    bp.animation_start_time = start_time_ms as f32 / 1000.0;
                                    bp.animation_end_time = end_time_ms as f32 / 1000.0;
                                    bp.animation_scale = scale as f32;
                                    bp.motion_ended = false;

                                    // Determine initial animation time from offset
                                    if offset_ms >= 0.0 {
                                        bp.animation_time = offset_ms as f32 / 1000.0;
                                    }
                                    // else: #synchronized — keep current relative position
                                }
                                w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                            } else {
                                // No args: Director's play() resumes a paused motion.
                                // If nothing is current but a motion is queued (the
                                // ClubMarian / Coke Studios pattern `queue(name); play()`),
                                // PROMOTE queue[0] to the current motion. dirplayer models
                                // the playList as `[current_motion] ++ motion_queue`, so we
                                // POP queue[0] when promoting (otherwise the playList getter,
                                // which prepends current_motion, would show it twice). The
                                // script's `if playList.count < 1` gate still sees count 1
                                // (the now-current motion), so it does not re-queue.
                                {
                                    let bp = w3d.runtime_state.bones_player_mut(&model_name);
                                    if bp.current_motion.is_some() {
                                        bp.animation_playing = true;
                                    } else if !bp.motion_queue.is_empty() {
                                        let q = bp.motion_queue.remove(0);
                                        bp.current_motion = Some(q.name);
                                        bp.animation_playing = true;
                                        bp.animation_loop = q.looped;
                                        bp.animation_start_time = q.start_time;
                                        bp.animation_end_time = q.end_time;
                                        bp.animation_scale = q.scale;
                                        bp.animation_time = if q.offset >= 0.0 { q.offset } else { q.start_time };
                                        bp.motion_ended = false;
                                        bp.previous_motion = None;
                                        bp.blend_weight = 1.0;
                                    }
                                }
                                w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "queue" => {
                    // queue(name,...) — add the motion to the END of the playList.
                    if !args.is_empty() {
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
                        let queued = crate::player::cast_member::QueuedMotion {
                            name: motion_name,
                            looped: is_loop,
                            start_time: start_time_ms as f32 / 1000.0,
                            end_time: end_time_ms as f32 / 1000.0,
                            scale: scale as f32,
                            offset: offset_ms as f32 / 1000.0,
                        };
                        let model_name = s3d_ref.name.clone();
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                w3d.runtime_state.bones_player_mut(&model_name).motion_queue.push(queued);
                                w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "playNext" => {
                    // playNext() — Director: interrupt and REMOVE the currently playing
                    // motion (playList[1]) and begin the next one (playList[2]). dirplayer's
                    // playList is `[current_motion] ++ motion_queue`, so drop current_motion
                    // and promote motion_queue[0]. (Was previously a no-op for the no-arg
                    // form, so scripts that do `queue(x); playNext()` — e.g. Rasterwerks
                    // C_BonesControl — never drained the queue; it accumulated until
                    // TrimPlayList()/removeLast() deleted the pile.)
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            {
                            let rs = w3d.runtime_state.bones_player_mut(&model_name);
                            // Blend out of the interrupted motion if autoBlend/blendTime set.
                            if rs.current_motion.is_some() && rs.animation_blend_time > 0.0 {
                                rs.previous_motion = rs.current_motion.clone();
                                rs.blend_duration = rs.animation_blend_time / 1000.0;
                                rs.blend_elapsed = 0.0;
                                rs.blend_weight = 0.0;
                            } else {
                                rs.previous_motion = None;
                                rs.blend_weight = 1.0;
                            }
                            if !rs.motion_queue.is_empty() {
                                let q = rs.motion_queue.remove(0);
                                rs.current_motion = Some(q.name);
                                rs.animation_loop = q.looped;
                                rs.animation_start_time = q.start_time;
                                rs.animation_end_time = q.end_time;
                                rs.animation_scale = q.scale;
                                rs.animation_time = if q.offset >= 0.0 { q.offset } else { q.start_time };
                                rs.animation_playing = true;
                                rs.motion_ended = false;
                            } else {
                                // Nothing left to play.
                                rs.current_motion = None;
                                rs.animation_playing = false;
                            }
                            }
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "removeLast" => {
                    // removeLast() — remove the LAST entry of the playList. The playList is
                    // `[current_motion] ++ motion_queue`, so drop the queue's tail; if the
                    // queue is empty the last (and only) entry is the current motion itself.
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            {
                                let bp = w3d.runtime_state.bones_player_mut(&model_name);
                                if bp.motion_queue.pop().is_none() {
                                    bp.current_motion = None;
                                    bp.animation_playing = false;
                                }
                            }
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "pause" => {
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bones_player_mut(&model_name).animation_playing = false;
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "resume" => {
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.bones_player_mut(&model_name).animation_playing = true;
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "stop" => {
                    let model_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            {
                                let bp = w3d.runtime_state.bones_player_mut(&model_name);
                                bp.animation_playing = false;
                                bp.animation_time = 0.0;
                                bp.current_motion = None;
                            }
                            w3d.runtime_state.sync_legacy_from_bones_player(&model_name);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                // ─── Scene management ───
                "clone" | "cloneDeep" => {
                    // Director 11.5 Scripting Dictionary (clone): "creates a copy of the
                    // model, group, light, or camera AND ALL OF ITS CHILDREN. The clone
                    // shares the parent ... and is assigned the same shaderList as the
                    // original." The previous implementation copied only the single node,
                    // so `clone.child[n]` was VOID and the cloned subtree lost its shaders
                    // (frog01 crashed on `s.model(cn).child[1].shaderList[5] = ...`).
                    // Recursively clone the whole subtree here.
                    let clone_name = if !args.is_empty() {
                        player.get_datum(&args[0]).string_value().unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    let source_name = s3d_ref.name.clone();
                    let source_lower = source_name.to_ascii_lowercase();

                    // 1. Snapshot the source node + its descendant subtree (in scene.nodes
                    //    order, so the clone's child[n] indexing matches the source's — the
                    //    child accessor filters scene.nodes in iteration order). Also grab
                    //    every existing node name so auto-generated clone names stay unique.
                    let (root_snapshot, descendants, mut used_names) = {
                        let scene_opt = player.movie.cast_manager.find_member_by_ref(&member_ref)
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.parsed_scene.as_ref());
                        if let Some(scene) = scene_opt {
                            let root = scene.nodes.iter()
                                .find(|n| n.name.eq_ignore_ascii_case(&source_name)).cloned();
                            // Parent-chain closure: a node is in the subtree if some
                            // ancestor is the source. (Parents may appear after their
                            // children in scene.nodes order, so iterate to a fixpoint.)
                            let pairs: Vec<(String, String)> = scene.nodes.iter()
                                .map(|n| (n.name.to_ascii_lowercase(), n.parent_name.to_ascii_lowercase()))
                                .collect();
                            let mut in_set: std::collections::HashSet<String> = std::collections::HashSet::new();
                            in_set.insert(source_lower.clone());
                            let mut changed = true;
                            while changed {
                                changed = false;
                                for (nm, pn) in &pairs {
                                    if !in_set.contains(nm) && in_set.contains(pn) {
                                        in_set.insert(nm.clone());
                                        changed = true;
                                    }
                                }
                            }
                            let descendants: Vec<crate::director::chunks::w3d::types::W3dNode> = scene.nodes.iter()
                                .filter(|n| {
                                    let nl = n.name.to_ascii_lowercase();
                                    nl != source_lower && in_set.contains(&nl)
                                })
                                .cloned()
                                .collect();
                            let used: std::collections::HashSet<String> = pairs.into_iter().map(|(nm, _)| nm).collect();
                            (root, descendants, used)
                        } else {
                            (None, Vec::new(), std::collections::HashSet::new())
                        }
                    };

                    // The clone root takes the explicit name. Director keeps an anonymous
                    // "" clone in the scene (just uncounted), so synthesize a stable key
                    // for it rather than leaving it nameless.
                    let effective_root_name = if clone_name.is_empty() {
                        format!("{}_clone", source_name)
                    } else {
                        clone_name.clone()
                    };

                    // Director rejects a clone whose name already names a node:
                    // `frog.clone("evil")` a second time errors "Object with duplicate
                    // name already exists" rather than appending a phantom subtree.
                    // `used_names` was seeded from every existing node, so a hit means
                    // the target name is taken. (In normal play this never fires —
                    // beginSprite calls resetWorld first, which clears the prior clone.)
                    if used_names.contains(&effective_root_name.to_ascii_lowercase()) {
                        return Err(ScriptError::new(
                            "Object with duplicate name already exists".to_string(),
                        ));
                    }

                    if let Some(root_node) = root_snapshot {
                        // 2. Pass 1 — assign every clone a name. Auto-named children use
                        //    Director's "<original>-copy<N>" convention (frog01 relies on
                        //    it, e.g. s.model("axe-copy2")); N is a per-clone counter that
                        //    skips any name already taken so repeated clones of the same
                        //    subtree (16 cars × {acar, wheel1..4}) never collide — runtime
                        //    state here is keyed by node name, so collisions would corrupt
                        //    sibling clones.
                        let mut name_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                        name_map.insert(source_lower.clone(), effective_root_name.clone());
                        used_names.insert(effective_root_name.to_ascii_lowercase());
                        // Director assigns "-copyN" by walking the source descendants in
                        // creation (scene.nodes) order and, for each not-yet-named node,
                        // naming it AND THEN its parent (when the parent is also in the
                        // cloned subtree and still unnamed). Verified against real
                        // Director's `frog.clone("evil")`: frog01 creates the bones
                        // connector-first (body, ll, rl, lc, …) with their joints last
                        // (…, lhip, lk, la, rhip, rk, ra, axe), so the (node, parent)
                        // pairing yields body=copy1, axe=copy2, ll=copy3, lhip=copy4,
                        // rl=copy5, rhip=copy6, … — exactly the names the walk-cycle
                        // hard-codes (s.model("lhip-copy4").rotate(…)). Plain creation-
                        // order numbering put the joints last (lhip-copy15) so the lookups
                        // returned VOID. The loop skips names already taken so repeated
                        // clones of the same subtree (16 cars × {acar,wheel1..4}) never
                        // collide; those address children by index, not by copyN name.
                        fn name_one(
                            orig: &crate::director::chunks::w3d::types::W3dNode,
                            name_map: &mut std::collections::HashMap<String, String>,
                            used_names: &mut std::collections::HashSet<String>,
                            counter: &mut usize,
                        ) {
                            let cand = loop {
                                let c = format!("{}-copy{}", orig.name, *counter);
                                *counter += 1;
                                if used_names.insert(c.to_ascii_lowercase()) { break c; }
                            };
                            name_map.insert(orig.name.to_ascii_lowercase(), cand);
                        }
                        let mut counter = 1usize;
                        for d in &descendants {
                            if !name_map.contains_key(&d.name.to_ascii_lowercase()) {
                                name_one(d, &mut name_map, &mut used_names, &mut counter);
                            }
                            let pl = d.parent_name.to_ascii_lowercase();
                            if pl != source_lower && !name_map.contains_key(&pl) {
                                if let Some(parent) = descendants.iter()
                                    .find(|n| n.name.eq_ignore_ascii_case(&pl)) {
                                    name_one(parent, &mut name_map, &mut used_names, &mut counter);
                                }
                            }
                        }

                        // 3. Pass 2 — build the cloned nodes with re-parented names and
                        //    collect the per-node runtime state to copy (live transform +
                        //    shader overrides + visibility). Read under an immutable borrow.
                        type ClonedNode = (crate::director::chunks::w3d::types::W3dNode, [f32; 16], Option<std::collections::HashMap<usize, String>>, Option<u8>);
                        let mut planned: Vec<ClonedNode> = Vec::with_capacity(descendants.len() + 1);
                        // (orig_node, new_name, new_parent): root keeps the source's parent
                        // ("clone shares the parent"); descendants map their parent through
                        // the complete name_map built in pass 1.
                        let mut work: Vec<(&crate::director::chunks::w3d::types::W3dNode, String, String)> =
                            Vec::with_capacity(descendants.len() + 1);
                        work.push((&root_node, effective_root_name.clone(), root_node.parent_name.clone()));
                        for d in &descendants {
                            let new_name = name_map.get(&d.name.to_ascii_lowercase()).cloned().unwrap();
                            let new_parent = name_map.get(&d.parent_name.to_ascii_lowercase()).cloned()
                                .unwrap_or_else(|| d.parent_name.clone());
                            work.push((d, new_name, new_parent));
                        }
                        for (orig, new_name, new_parent) in &work {
                            let transform = get_node_transform_live(player, &member_ref, &orig.name);
                            let (shaders, visibility) = {
                                let w3d = player.movie.cast_manager.find_member_by_ref(&member_ref)
                                    .and_then(|m| m.member_type.as_shockwave3d());
                                let shaders = w3d.and_then(|w| {
                                    w.runtime_state.node_shaders.get(&orig.name)
                                        .or_else(|| w.runtime_state.node_shaders.iter()
                                            .find(|(k, _)| k.eq_ignore_ascii_case(&orig.name)).map(|(_, v)| v))
                                        .cloned()
                                });
                                let visibility = w3d.and_then(|w| {
                                    w.runtime_state.node_visibility.get(&orig.name)
                                        .or_else(|| w.runtime_state.node_visibility.iter()
                                            .find(|(k, _)| k.eq_ignore_ascii_case(&orig.name)).map(|(_, v)| v))
                                        .copied()
                                });
                                (shaders, visibility)
                            };
                            let mut node = (*orig).clone();
                            node.name = new_name.clone();
                            node.parent_name = new_parent.clone();
                            node.transform = transform;
                            planned.push((node, transform, shaders, visibility));
                        }

                        // 4. Commit — push cloned nodes and their runtime state.
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                for (node, transform, shaders, visibility) in &planned {
                                    w3d.runtime_state.node_transforms.insert(node.name.clone(), *transform);
                                    if let Some(sh) = shaders {
                                        w3d.runtime_state.node_shaders.insert(node.name.clone(), sh.clone());
                                    }
                                    if let Some(v) = visibility {
                                        w3d.runtime_state.node_visibility.insert(node.name.clone(), *v);
                                    }
                                }
                                if let Some(scene) = w3d.scene_mut() {
                                    for (node, _, _, _) in planned {
                                        scene.nodes.push(node);
                                    }
                                }
                            }
                        }
                    } else {
                        // Source not in the scene graph — keep the returned ref valid by
                        // creating a bare model node (mirrors the old fallback).
                        use crate::director::chunks::w3d::types::*;
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    scene.nodes.push(W3dNode {
                                        name: effective_root_name.clone(), node_type: W3dNodeType::Model,
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
                        name: effective_root_name,
                    })))
                },
                "addChild" => {
                    // addChild(childNodeRef {, #preserveWorld | #preserveParent})
                    // Sets child's parent to this node. Default is #preserveParent.
                    if !args.is_empty() {
                        let child_name = match player.get_datum(&args[0]) {
                            Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                            Datum::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        // 2nd arg selects transform handling. Director's DEFAULT (no symbol)
                        // is #preserveWorld: the child keeps its WORLD transform and its
                        // parent-relative (local) transform is recomputed
                        // (local = inverse(parentWorld) * childWorld). Only #preserveParent
                        // keeps the child's existing LOCAL transform (the child JUMPS in world
                        // space). Dict line 18344: "#preserveParent ... the parent-relative
                        // transform of the child remains unchanged"; "#preserveWorld ... the
                        // world transform of the child remains unchanged. Its parent-relative
                        // transform is recalculated." Used by the Pacman maze (pipes parented
                        // to pipePink1) and ghosts (top/eyes parented to the body), AND frog01:
                        // it sets each wake's transform in WORLD space then `log.addChild(wake)`
                        // with NO symbol — real Director rebases it under the log (local ≈
                        // (0.1,0,0)). Defaulting to #preserveParent kept the world transform AS
                        // the local, double-transforming the wake off-screen edge-on.
                        let preserve_world = match args.get(1).map(|a| player.get_datum(a)) {
                            Some(Datum::Symbol(s)) => !s.eq_ignore_ascii_case("preserveParent"),
                            Some(Datum::String(s)) => !s.eq_ignore_ascii_case("preserveParent"),
                            _ => true, // Director default = #preserveWorld
                        };
                        if !child_name.is_empty() {
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            if preserve_world {
                                // Compute both world transforms under the OLD hierarchy, then
                                // rebase the child's local transform onto the new parent.
                                let child_world = node_world_transform(player, &member_ref, &child_name);
                                let parent_world = node_world_transform(player, &member_ref, &s3d_ref.name);
                                let new_local = mat4_mul_f32(&invert_transform_f32(&parent_world), &child_world);
                                set_node_transform(player, &member_ref, &child_name, new_local);
                            }
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.detached_nodes.remove(&child_name);
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == child_name) {
                                            node.parent_name = s3d_ref.name.clone();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "removeChild" => {
                    // removeChild(childNodeRef) — detach child from this node
                    if !args.is_empty() {
                        let child_name = match player.get_datum(&args[0]) {
                            Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                            Datum::String(s) => s.clone(),
                            _ => String::new(),
                        };
                        if !child_name.is_empty() {
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(node) = scene.nodes.iter_mut().find(|n| n.name == child_name && n.parent_name == s3d_ref.name) {
                                            node.parent_name = "World".to_string();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "addToWorld" => {
                    // Set model's parent to World and remove from detached nodes
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.detached_nodes.remove(&s3d_ref.name);
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    if node.parent_name.is_empty() {
                                        node.parent_name = "World".to_string();
                                    }
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "removeFromWorld" => {
                    // Detach model from world
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.detached_nodes.insert(s3d_ref.name.clone());
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(node) = scene.nodes.iter_mut().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name)) {
                                    node.parent_name = String::new();
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "isInWorld" => {
                    // Check if node is in the world (not detached)
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                    let in_world = if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if w3d.runtime_state.detached_nodes.contains(&s3d_ref.name) {
                                false
                            } else if let Some(ref scene) = w3d.parsed_scene {
                                scene.nodes.iter().any(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name) && !n.parent_name.is_empty())
                            } else { false }
                        } else { false }
                    } else { false };
                    Ok(player.alloc_datum(Datum::Int(if in_world { 1 } else { 0 })))
                },
                "addModifier" => {
                    // Initialize meshDeform state when #meshDeform modifier is added
                    if !args.is_empty() {
                        let mod_name = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        log(&format!(
                            "[W3D-ADDMOD] model=\"{}\" modifier=\"{}\" member=({},{})",
                            s3d_ref.name, mod_name, s3d_ref.cast_lib, s3d_ref.cast_member
                        ));
                        if mod_name == "lod" {
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.lod_state.entry(s3d_ref.name.clone())
                                        .or_insert_with(crate::player::cast_member::LodState::default);
                                }
                            }
                        } else if mod_name == "collision" {
                            // Native #collision modifier: register state so
                            // `model.collision` resolves to a collision object and
                            // events::tick_w3d_collisions includes this model.
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    w3d.runtime_state.collision_modifiers
                                        .entry(s3d_ref.name.clone())
                                        .or_default();
                                }
                            }
                        } else if mod_name == "meshDeform" {
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let (mesh_count, node_found, res_found) = {
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.parsed_scene.as_ref())
                                    .map(|scene| {
                                        let node = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name));
                                        let nf = node.is_some();
                                        let res_name = node.map(|n| if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name });
                                        let rf = res_name.and_then(|rn| scene.model_resources.get(rn.as_str())).is_some();
                                        let mc = res_name.and_then(|rn| scene.model_resources.get(rn.as_str()))
                                            .map(|res| res.mesh_infos.len())
                                            .unwrap_or(1);
                                        (mc, nf, rf)
                                    })
                                    .unwrap_or((1, false, false))
                            };
                            log(&format!(
                                "[W3D-MESHDEFORM] model=\"{}\" mesh_count={} node_found={} res_found={} member=({},{})",
                                s3d_ref.name, mesh_count, node_found, res_found, s3d_ref.cast_lib, s3d_ref.cast_member
                            ));
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
                },
                "removeModifier" => Ok(player.alloc_datum(Datum::Void)),
                "registerScript" | "registerForEvent" => Ok(player.alloc_datum(Datum::Void)),
                "setCollisionCallback" => {
                    // model.collision.setCollisionCallback(#handler, scriptInstance):
                    // register the handler that events::tick_w3d_collisions fires
                    // when this model is involved in a collision.
                    if s3d_ref.object_type == "collision" && args.len() >= 2 {
                        let handler = player.get_datum(&args[0]).string_value()
                            .unwrap_or_default().trim_start_matches('#').to_string();
                        let target_datum = player.get_datum(&args[1]).clone();
                        let instance = match &target_datum {
                            Datum::ScriptInstanceRef(r) => Some(r.clone()),
                            _ => None,
                        };
                        let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                let key = w3d.runtime_state.collision_modifiers.keys()
                                    .find(|k| k.eq_ignore_ascii_case(&s3d_ref.name))
                                    .cloned()
                                    .unwrap_or_else(|| s3d_ref.name.clone());
                                let cm = w3d.runtime_state.collision_modifiers.entry(key).or_default();
                                cm.callback_handler = Some(handler);
                                cm.callback_instance = instance;
                                cm.callback_target = Some(target_datum);
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "isInWorld" => Ok(player.alloc_datum(Datum::Int(1))),
                // ─── Camera methods ───
                "modelUnderLoc" => {
                    if !args.is_empty() {
                        // Get screen point from argument
                        let (sx, sy) = match player.get_datum(&args[0]) {
                            Datum::Point(vals, _flags) => {
                                (vals[0] as f32, vals[1] as f32)
                            }
                            _ => (0.0, 0.0),
                        };
                        debug!(
                            "[modelUnderLoc] point=({:.0},{:.0})", sx, sy
                        );

                        // Get scene for ray casting
                        let scene = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                                .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                            let w3d = member.member_type.as_shockwave3d()
                                .ok_or_else(|| ScriptError::new("Not 3D".to_string()))?;
                            w3d.parsed_scene.clone()
                        };

                        // Also get runtime state for camera/model transforms
                        let runtime_state = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                                .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                            let w3d = member.member_type.as_shockwave3d()
                                .ok_or_else(|| ScriptError::new("Not 3D".to_string()))?;
                            w3d.runtime_state.clone()
                        };

                        if let Some(scene) = scene {
                            use crate::director::chunks::w3d::raycast;
                            use crate::player::score::get_concrete_sprite_rect;

                            let view_node = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View);
                            let fov_deg = view_node.map(|n| n.fov).unwrap_or(30.0);
                            // Use runtime camera transform (set by Lingo) if available
                            let cam_name = view_node.map(|n| n.name.as_str()).unwrap_or("DefaultView");
                            let cam_transform = runtime_state.node_transforms
                                .get(cam_name)
                                .or_else(|| runtime_state.node_transforms.iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case(cam_name)).map(|(_, v)| v))
                                .copied()
                                .unwrap_or_else(|| view_node.map(|n| n.transform).unwrap_or([
                                    1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,500.0,1.0,
                                ]));

                            // Find the sprite that holds this 3D member for viewport dimensions.
                            // Coordinates are sprite-relative (not stage-relative).
                            let sprite_rect = player.movie.score.channels.iter()
                                .find(|ch| ch.sprite.member.as_ref() == Some(&member_ref))
                                .map(|ch| get_concrete_sprite_rect(player, &ch.sprite));
                            let (width, height) = if let Some(r) = sprite_rect {
                                (r.width() as f32, r.height() as f32)
                            } else {
                                let w = player.movie.rect.width() as f32;
                                let h = player.movie.rect.height() as f32;
                                (if w > 0.0 { w } else { 320.0 }, if h > 0.0 { h } else { 240.0 })
                            };
                            // IFX uses the member's original (default_rect) dimensions for distToProj
                            let (orig_w, orig_h) = get_member_default_rect_size(player, &member_ref);

                            let ray = raycast::screen_to_ray_shockwave(sx, sy, width, height, orig_w, orig_h, fov_deg, &cam_transform);
                            // First try ray-sphere test against each model's
                            // bounding sphere. This is more robust than mesh-triangle
                            // intersection for clicking, especially when there's a
                            // small projection mismatch between renderer and raycast.
                            let mut best_sphere_hit: Option<(f32, String)> = None;
                            for node in scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model) {
                                // Only sphere-test models with sphere primitive type
                                let res_key = if !node.model_resource_name.is_empty() {
                                    &node.model_resource_name
                                } else { &node.resource_name };
                                let is_sphere = scene.model_resources.get(res_key.as_str())
                                    .and_then(|r| r.primitive_type.as_deref())
                                    .map_or(false, |t| t == "sphere");
                                if !is_sphere { continue; }
                                let pos = runtime_state.node_transforms.get(&node.name)
                                    .map(|t| [t[12], t[13], t[14]])
                                    .unwrap_or([node.transform[12], node.transform[13], node.transform[14]]);
                                let res_name = if !node.model_resource_name.is_empty() {
                                    &node.model_resource_name
                                } else { &node.resource_name };
                                // Compute bounding radius from mesh half-extents
                                let radius = scene.model_resources.get(res_name.as_str())
                                    .and_then(|r| {
                                        let he = [r.primitive_width, r.primitive_height, r.primitive_length, r.primitive_radius];
                                        let max_he = he.iter().cloned().fold(0.0f32, f32::max);
                                        if max_he > 0.01 { Some(max_he) } else { None }
                                    })
                                    .or_else(|| {
                                        scene.clod_meshes.get(res_name.as_str()).map(|meshes| {
                                            let mut max_r = 0.0f32;
                                            for mesh in meshes {
                                                for p in &mesh.positions {
                                                    let r = (p[0]*p[0] + p[1]*p[1] + p[2]*p[2]).sqrt();
                                                    if r > max_r { max_r = r; }
                                                }
                                            }
                                            max_r
                                        })
                                    })
                                    .unwrap_or(5.0);
                                // Ray-sphere intersection: |O + tD - C|² = r²
                                // Use 3× radius for picking to compensate for the small
                                // projection mismatch between renderer and raycast.
                                let pick_radius = radius * 3.0;
                                let oc = [ray.origin[0]-pos[0], ray.origin[1]-pos[1], ray.origin[2]-pos[2]];
                                let a = ray.direction[0]*ray.direction[0] + ray.direction[1]*ray.direction[1] + ray.direction[2]*ray.direction[2];
                                let b = 2.0 * (oc[0]*ray.direction[0] + oc[1]*ray.direction[1] + oc[2]*ray.direction[2]);
                                let c = oc[0]*oc[0] + oc[1]*oc[1] + oc[2]*oc[2] - pick_radius*pick_radius;
                                let disc = b*b - 4.0*a*c;
                                if disc >= 0.0 {
                                    let t = (-b - disc.sqrt()) / (2.0 * a);
                                    if t > 0.0 {
                                        if best_sphere_hit.as_ref().map_or(true, |(bt, _)| t < *bt) {
                                            best_sphere_hit = Some((t, node.name.clone()));
                                        }
                                    }
                                }
                            }
                            if let Some((_, ref name)) = best_sphere_hit {
                                debug!(
                                    "[modelUnderLoc] SPHERE HIT '{}'", name
                                );
                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: s3d_ref.cast_lib,
                                    cast_member: s3d_ref.cast_member,
                                    object_type: "model".to_string(),
                                    name: name.clone(),
                                })));
                            }

                            // Fall back to mesh-triangle intersection
                            if let Some(hit) = raycast::raycast_scene_multi(
                                &ray, &scene, 100000.0, 1,
                                Some(&runtime_state.node_transforms), None, None,
                            ).into_iter().next() {
                                debug!(
                                    "[modelUnderLoc] MESH HIT '{}'", hit.model_name
                                );
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
                },
                "modelsUnderLoc" => {
                    // modelsUnderLoc(point {, maxModels, #simple|#detailed})
                    if !args.is_empty() {
                        let (sx, sy) = match player.get_datum(&args[0]) {
                            Datum::Point(vals, _flags) => {
                                (vals[0] as f32, vals[1] as f32)
                            }
                            _ => (0.0, 0.0),
                        };
                        // modelsUnderLoc(point {, maxModels|options {, options}}). The
                        // optional trailing args come in either order/forms: an integer is
                        // the max model count; a symbol (#simple/#detailed) is the level of
                        // detail. The unicraft galaxy calls `modelsUnderLoc(loc, #simple)` —
                        // parsing that symbol as maxModels yielded 0, silently returning no
                        // hits (nothing was ever selectable).
                        let mut max_models = 100usize;
                        let mut detailed = false;
                        for a in args.iter().skip(1) {
                            match player.get_datum(a) {
                                Datum::Symbol(s) => { if s.eq_ignore_ascii_case("detailed") { detailed = true; } }
                                Datum::Int(n) => { if *n > 0 { max_models = *n as usize; } }
                                d => { if let Ok(n) = d.int_value() { if n > 0 { max_models = n as usize; } } }
                            }
                        }

                        let (scene, node_transforms, excluded) = {
                            let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                            if let Some(m) = member {
                                if let Some(w3d) = m.member_type.as_shockwave3d() {
                                    let s = w3d.parsed_scene.clone();
                                    let nt = w3d.runtime_state.node_transforms.clone();
                                    let ex = w3d.runtime_state.detached_nodes.clone();
                                    (s, nt, ex)
                                } else { (None, std::collections::HashMap::new(), std::collections::HashSet::new()) }
                            } else { (None, std::collections::HashMap::new(), std::collections::HashSet::new()) }
                        };

                        if let Some(scene) = scene {
                            use crate::director::chunks::w3d::raycast;
                            use crate::player::score::get_concrete_sprite_rect;

                            let view_node = scene.nodes.iter()
                                .find(|n| n.node_type == W3dNodeType::View && n.name.eq_ignore_ascii_case(&s3d_ref.name))
                                .or_else(|| scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View));
                            let fov_deg = view_node.map(|n| n.fov).unwrap_or(30.0);

                            // Find the sprite that holds this 3D member for viewport dimensions.
                            // Coordinates are sprite-relative (not stage-relative).
                            let sprite_rect = player.movie.score.channels.iter()
                                .find(|ch| ch.sprite.member.as_ref() == Some(&member_ref))
                                .map(|ch| get_concrete_sprite_rect(player, &ch.sprite));
                            let (width, height) = if let Some(r) = sprite_rect {
                                (r.width() as f32, r.height() as f32)
                            } else {
                                let w = player.movie.rect.width() as f32;
                                let h = player.movie.rect.height() as f32;
                                (if w > 0.0 { w } else { 320.0 }, if h > 0.0 { h } else { 240.0 })
                            };

                            // Read camera transform from persistent datum (which Lingo keeps
                            // up to date) rather than node_transforms which may have a stale
                            // initial value under a different case key.
                            let cam_name = view_node.map(|n| n.name.as_str()).unwrap_or(&s3d_ref.name);
                            let cam_world = get_node_transform(player, &member_ref, cam_name);
                            // IFX uses the member's original (default_rect) dimensions for distToProj
                            let (orig_w, orig_h) = get_member_default_rect_size(player, &member_ref);

                            let ray = raycast::screen_to_ray_shockwave(sx, sy, width, height, orig_w, orig_h, fov_deg, &cam_world);
                            // Director's modelsUnderLoc has no pick-distance limit — it hits any
                            // model along the ray. A 100000 cap missed large-coordinate scenes
                            // (unicraft's galaxy: camera ~140000 units from the planets → hits=0,
                            // so no hover/select). Use an effectively-unbounded range.
                            let mut hits = raycast::raycast_scene_multi(
                                &ray, &scene, 1.0e9, max_models,
                                Some(&node_transforms), Some(&excluded), None,
                            );

                            // #sphere (and other) primitives are generated at RUNTIME in the
                            // renderer, so their geometry is not in parsed_scene and the
                            // mesh-based raycast above can't see them. Add an analytic ray-sphere
                            // test for sphere-primitive models (mirrors the singular modelUnderLoc
                            // path) — without it the galaxy's planets were never pickable.
                            for node in scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model) {
                                if excluded.contains(&node.name) { continue; }
                                if hits.iter().any(|h| h.model_name == node.name) { continue; }
                                let res_key = if !node.model_resource_name.is_empty() {
                                    &node.model_resource_name
                                } else { &node.resource_name };
                                let is_sphere = scene.model_resources.get(res_key.as_str())
                                    .and_then(|r| r.primitive_type.as_deref())
                                    .map_or(false, |t| t == "sphere");
                                if !is_sphere { continue; }
                                let pos = node_transforms.get(&node.name)
                                    .map(|t| [t[12], t[13], t[14]])
                                    .unwrap_or([node.transform[12], node.transform[13], node.transform[14]]);
                                let radius = scene.model_resources.get(res_key.as_str())
                                    .map(|r| {
                                        let he = [r.primitive_width, r.primitive_height, r.primitive_length, r.primitive_radius];
                                        he.iter().cloned().fold(0.0f32, f32::max)
                                    })
                                    .filter(|r| *r > 0.01)
                                    .unwrap_or(5.0);
                                // 3× radius picking tolerance (matches modelUnderLoc) to absorb
                                // the small projection mismatch between renderer and raycast.
                                let pick_radius = radius * 3.0;
                                let oc = [ray.origin[0]-pos[0], ray.origin[1]-pos[1], ray.origin[2]-pos[2]];
                                let a = ray.direction[0]*ray.direction[0] + ray.direction[1]*ray.direction[1] + ray.direction[2]*ray.direction[2];
                                let b = 2.0 * (oc[0]*ray.direction[0] + oc[1]*ray.direction[1] + oc[2]*ray.direction[2]);
                                let c = oc[0]*oc[0] + oc[1]*oc[1] + oc[2]*oc[2] - pick_radius*pick_radius;
                                let disc = b*b - 4.0*a*c;
                                if disc >= 0.0 {
                                    let t = (-b - disc.sqrt()) / (2.0 * a);
                                    if t > 0.0 {
                                        let p = [ray.origin[0]+t*ray.direction[0], ray.origin[1]+t*ray.direction[1], ray.origin[2]+t*ray.direction[2]];
                                        let nrm = [p[0]-pos[0], p[1]-pos[1], p[2]-pos[2]];
                                        let nl = (nrm[0]*nrm[0]+nrm[1]*nrm[1]+nrm[2]*nrm[2]).sqrt().max(1e-6);
                                        hits.push(raycast::RayHit {
                                            model_name: node.name.clone(),
                                            distance: t,
                                            position: p,
                                            normal: [nrm[0]/nl, nrm[1]/nl, nrm[2]/nl],
                                            face_index: 0, mesh_id: 0,
                                            vertices: [[0.0f32; 3]; 3], uv_coord: [0.0, 0.0],
                                        });
                                    }
                                }
                            }
                            // Nearest-first, capped to the requested model count.
                            hits.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
                            hits.truncate(max_models);

                            if !hits.is_empty() {
                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                let mut items = VecDeque::new();
                                for hit in &hits {
                                    if detailed {
                                        // #detailed: return proplist with #model, #distance, #isectPosition, #isectNormal, #meshID, #faceID, #vertices, #uvCoord
                                        let mk = player.alloc_datum(Datum::Symbol("model".to_string()));
                                        let mv = player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                            cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member,
                                            object_type: "model".to_string(), name: hit.model_name.clone(),
                                        }));
                                        let dk = player.alloc_datum(Datum::Symbol("distance".to_string()));
                                        let dv = player.alloc_datum(Datum::Float(hit.distance as f64));
                                        let pk = player.alloc_datum(Datum::Symbol("isectPosition".to_string()));
                                        let pv = player.alloc_datum(Datum::Vector([
                                            hit.position[0] as f64, hit.position[1] as f64, hit.position[2] as f64,
                                        ]));
                                        let nk = player.alloc_datum(Datum::Symbol("isectNormal".to_string()));
                                        let nv = player.alloc_datum(Datum::Vector([
                                            hit.normal[0] as f64, hit.normal[1] as f64, hit.normal[2] as f64,
                                        ]));
                                        let midk = player.alloc_datum(Datum::Symbol("meshID".to_string()));
                                        let midv = player.alloc_datum(Datum::Int(hit.mesh_id as i32));
                                        let fidk = player.alloc_datum(Datum::Symbol("faceID".to_string()));
                                        let fidv = player.alloc_datum(Datum::Int(hit.face_index as i32 + 1)); // 1-based
                                        let vk = player.alloc_datum(Datum::Symbol("vertices".to_string()));
                                        let mut vert_items = VecDeque::new();
                                        for vtx in &hit.vertices {
                                            vert_items.push_back(player.alloc_datum(Datum::Vector([
                                                vtx[0] as f64, vtx[1] as f64, vtx[2] as f64,
                                            ])));
                                        }
                                        let vv = player.alloc_datum(Datum::List(
                                            crate::director::lingo::datum::DatumType::List, vert_items, false,
                                        ));
                                        let uk = player.alloc_datum(Datum::Symbol("uvCoord".to_string()));
                                        let u_ref = player.alloc_datum(Datum::Symbol("u".to_string()));
                                        let u_val = player.alloc_datum(Datum::Float(hit.uv_coord[0] as f64));
                                        let v_ref = player.alloc_datum(Datum::Symbol("v".to_string()));
                                        let v_val = player.alloc_datum(Datum::Float(hit.uv_coord[1] as f64));
                                        let uv = player.alloc_datum(Datum::PropList(
                                            VecDeque::from(vec![(u_ref, u_val), (v_ref, v_val)]), false,
                                        ));
                                        let props = VecDeque::from(vec![
                                            (mk, mv), (dk, dv), (pk, pv), (nk, nv),
                                            (midk, midv), (fidk, fidv), (vk, vv), (uk, uv),
                                        ]);
                                        items.push_back(player.alloc_datum(Datum::PropList(props, false)));
                                    } else {
                                        // #simple: just return model refs
                                        items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                            cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member,
                                            object_type: "model".to_string(), name: hit.model_name.clone(),
                                        })));
                                    }
                                }
                                return Ok(player.alloc_datum(Datum::List(
                                    crate::director::lingo::datum::DatumType::List, items, false,
                                )));
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List, VecDeque::new(), false,
                    )))
                },
                "screenToWorld" => {
                    if !args.is_empty() {
                        let (sx, sy) = match player.get_datum(&args[0]) {
                            Datum::Point(vals, _flags) => {
                                (vals[0] as f32, vals[1] as f32)
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
                },
                "worldToScreen" => {
                    // Project 3D world point to 2D screen coords via view-projection matrix
                    let world_pt = if !args.is_empty() {
                        match player.get_datum(&args[0]) {
                            Datum::Vector(v) => [v[0] as f32, v[1] as f32, v[2] as f32],
                            _ => [0.0, 0.0, 0.0],
                        }
                    } else { [0.0, 0.0, 0.0] };

                    // Get viewport size: use movie rect (actual rendered viewport)
                    let vw = player.movie.rect.width().max(1) as f32;
                    let vh = player.movie.rect.height().max(1) as f32;

                    let (sx, sy) = if let Some(member) = player.movie.cast_manager.find_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d() {
                            if let Some(ref _scene) = w3d.parsed_scene {
                                let cam_t = w3d.runtime_state.node_transforms.get("DefaultView")
                                    .or_else(|| w3d.runtime_state.node_transforms.get("defaultview"));
                                if let Some(cam_t) = cam_t {
                                    // Transform to camera space
                                    let vx = world_pt[0] - cam_t[12];
                                    let vy = world_pt[1] - cam_t[13];
                                    let vz = world_pt[2] - cam_t[14];
                                    // Perspective divide
                                    let depth = (cam_t[8]*vx + cam_t[9]*vy + cam_t[10]*vz).abs().max(0.01);
                                    let ndx = (cam_t[0]*vx + cam_t[1]*vy + cam_t[2]*vz) / depth;
                                    let ndy = (cam_t[4]*vx + cam_t[5]*vy + cam_t[6]*vz) / depth;
                                    // NDC to screen: center of viewport + NDC offset
                                    (vw * 0.5 + ndx * vw * 0.5, vh * 0.5 - ndy * vh * 0.5)
                                } else {
                                    (0.0, 0.0)
                                }
                            } else { (0.0, 0.0) }
                        } else { (0.0, 0.0) }
                    } else { (0.0, 0.0) };

                    Ok(player.alloc_datum(Datum::Point([sx as f64, sy as f64], 0b11)))
                },
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
                                log(&format!(
                                    "[W3D] camera(\"{}\").renderDirect(\"{}\") — render target set",
                                    cam_name, target_tex_name
                                ));
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "addOverlay" | "addBackdrop" | "insertOverlay" | "insertBackdrop" => {
                    // addOverlay(texture, point, rotation)
                    // insertOverlay(index, texture, point, rotation) — the 1-based
                    // index shifts texture/loc/rotation one slot right and the new
                    // overlay is INSERTED at that index instead of appended.
                    // PacMan3D2's intro builds its 5 ffTitle title images this way.
                    let is_overlay = handler_name.contains("Overlay");
                    let is_insert = handler_name.starts_with("insert");
                    let arg_off = if is_insert { 1 } else { 0 };
                    let insert_index = if is_insert && !args.is_empty() {
                        player.get_datum(&args[0]).int_value().unwrap_or(1).max(1) as usize
                    } else { 0 };
                    let tex_name = if args.len() > arg_off {
                        match player.get_datum(&args[arg_off]) {
                            Datum::Shockwave3dObjectRef(r) if r.object_type == "texture" => r.name.clone(),
                            Datum::String(s) => s.clone(),
                            _ => String::new(),
                        }
                    } else { String::new() };
                    let loc = if args.len() > arg_off + 1 {
                        match player.get_datum(&args[arg_off + 1]) {
                            Datum::Point(vals, _flags) => {
                                [vals[0], vals[1]]
                            }
                            _ => [0.0, 0.0],
                        }
                    } else { [0.0, 0.0] };
                    let rotation = if args.len() > arg_off + 2 {
                        player.get_datum(&args[arg_off + 2]).to_float().unwrap_or(0.0)
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
                            let cam_key = camera_name.to_ascii_lowercase();
                            let list = if is_overlay {
                                w3d.runtime_state.camera_overlays.entry(cam_key).or_insert_with(Vec::new)
                            } else {
                                w3d.runtime_state.camera_backdrops.entry(cam_key).or_insert_with(Vec::new)
                            };
                            if is_insert {
                                // 1-based index → 0-based, clamped to the list end.
                                let idx = insert_index.saturating_sub(1).min(list.len());
                                list.insert(idx, overlay);
                            } else {
                                list.push(overlay);
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "removeOverlay" | "removeBackdrop" => {
                    // removeOverlay(index) — 1-based
                    let is_overlay = handler_name == "removeOverlay";
                    let index = if !args.is_empty() {
                        player.get_datum(&args[0]).int_value().unwrap_or(1) as usize
                    } else { 1 };
                    let cam_key = s3d_ref.name.to_ascii_lowercase();
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };

                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            let list = if is_overlay {
                                w3d.runtime_state.camera_overlays.get_mut(&cam_key)
                            } else {
                                w3d.runtime_state.camera_backdrops.get_mut(&cam_key)
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
                },
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

                        // meshDeformMesh.vertexList[idx] / .normalList[idx] = vector
                        // — deform a single vertex/normal, persisting into the CLOD
                        // mesh (the transient-list update below never reaches the
                        // geometry). Splat's pip tower hides eaten dots by setting
                        // their verts to vector(0,1000,0).
                        if s3d_ref.object_type == "meshDeformMesh"
                            && (prop.eq_ignore_ascii_case("vertexList") || prop.eq_ignore_ascii_case("normalList"))
                        {
                            let is_normals = prop.eq_ignore_ascii_case("normalList");
                            let v = match &value_datum {
                                Datum::Vector(vec) => [vec[0] as f32, vec[1] as f32, vec[2] as f32],
                                _ => [0.0, 0.0, 0.0],
                            };
                            let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                            let model_name = parts.get(0).unwrap_or(&"").to_string();
                            let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                            let vidx = (index as usize).saturating_sub(1);
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    let key = w3d.parsed_scene.as_ref().and_then(|scene| {
                                        let rn = scene.nodes.iter()
                                            .find(|n| n.name.eq_ignore_ascii_case(&model_name))
                                            .map(|n| if !n.model_resource_name.is_empty() {
                                                n.model_resource_name.clone()
                                            } else { n.resource_name.clone() });
                                        rn.filter(|k| scene.clod_meshes.contains_key(k))
                                            .or_else(|| if scene.clod_meshes.contains_key(&model_name) {
                                                Some(model_name.clone())
                                            } else { None })
                                    });
                                    if let Some(key) = key {
                                        if let Some(scene) = w3d.scene_mut() {
                                            if let Some(mesh) = scene.clod_meshes.get_mut(&key)
                                                .and_then(|m| m.get_mut(mesh_idx))
                                            {
                                                let target = if is_normals { &mut mesh.normals } else { &mut mesh.positions };
                                                if vidx < target.len() {
                                                    target[vidx] = v;
                                                    scene.mesh_content_version = scene.mesh_content_version.wrapping_add(1);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            return Ok(player.alloc_datum(Datum::Void));
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
                                if let Datum::List(_, items, _) = player.get_datum_mut(&list_ref) {
                                    items.extend(new_items);
                                }
                            } else {
                                if let Datum::List(_, items, _) = player.get_datum_mut(&list_ref) {
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
                },
                "getaProp" => {
                    if !args.is_empty() {
                        let prop = player.get_datum(&args[0]).string_value().unwrap_or_default();
                        Self::get_prop(datum, &prop)
                    } else {
                        Ok(player.alloc_datum(Datum::Void))
                    }
                },
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
                            // IFX BlendFunction bytes: 0=replace(SELECT_ARG0), 1=add,
                            // 2=multiply(MODULATE), 3=blend(INTERPOLATE). See IFXShaderLitTexture.h.
                            let blend_val = match &value {
                                Datum::Symbol(s) => match s.as_str() {
                                    "replace" => 0u8,
                                    "add" => 1,
                                    "blend" => 3,
                                    _ => 2, // multiply
                                },
                                _ => 2,
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
                            // IFX BlendSource bytes: 0=alpha, 1=constant. See IFXShaderLitTexture.h.
                            let src_val = match &value {
                                Datum::Symbol(s) if s == "alpha" => 0u8,
                                _ => 1, // constant
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
                        } else if prop_name == "textureRepeatList" {
                            // Set texture repeat mode for a texture layer (0=clamp, 1=repeat)
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let repeat_val = match &value {
                                Datum::Int(i) => *i as u8,
                                Datum::Float(f) => *f as u8,
                                _ => 1,
                            };
                            let idx = (index as usize).saturating_sub(1);
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                            while shader.texture_layers.len() <= idx {
                                                shader.texture_layers.push(crate::director::chunks::w3d::types::W3dTextureLayer::default());
                                            }
                                            shader.texture_layers[idx].repeat_s = repeat_val;
                                            shader.texture_layers[idx].repeat_t = repeat_val;
                                        }
                                    }
                                }
                            }
                        } else if prop_name == "textureModeList" {
                            // Symbol → tex_mode int. Mirrors the getter at the bottom of this file.
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            let mode_val: u8 = match &value {
                                Datum::Symbol(s) => match s.to_ascii_lowercase().as_str() {
                                    "none" => 0,
                                    "reflection" => 4,
                                    "wrapplanar" => 5,
                                    "specular" => 6,
                                    _ => 0,
                                },
                                Datum::Int(i) => *i as u8,
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
                                            shader.texture_layers[idx].tex_mode = mode_val;
                                        }
                                    }
                                }
                            }
                        } else if prop_name == "textureList" {
                            // Update the persistent textureList at the given index
                            let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };
                            // Extract the texture name from the value for scene data sync
                            let tex_name = match &value {
                                Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                                Datum::String(s) => s.clone(),
                                Datum::Void => String::new(),
                                _ => String::new(),
                            };
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
                                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                                items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                                    cast_lib: s3d_ref.cast_lib,
                                                    cast_member: s3d_ref.cast_member,
                                                    object_type: "texture".to_string(),
                                                    name: layer.name.clone(),
                                                })));
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
                            // Sync texture name to scene data so the GPU renderer sees it
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == s3d_ref.name) {
                                            while shader.texture_layers.len() <= idx {
                                                shader.texture_layers.push(crate::director::chunks::w3d::types::W3dTextureLayer::default());
                                            }
                                            shader.texture_layers[idx].name = tex_name;
                                        }
                                    }
                                }
                            }
                        }
                    } else if args.len() == 2 {
                        // setAt(index, value) — accept silently
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                // node.child(whichChildNodeName) / node.child(index) —
                // Director 11.5 Scripting Dictionary, "child (3D)": returns the
                // child node with that name, or at that 1-based index, in the
                // parent node's child list. Without this arm the call fell
                // through to `get_prop(datum, "child")`, which ignores the
                // argument and hands back the whole child LIST — age_of_speed's
                // `lAccRef = lModelRef.child(lWingName)` then did
                // `lAccRef.shaderList[i] = …` against a list ("No handler
                // setProp for list datum"). Undocumented lookups return VOID,
                // matching the movie's `if lAccRef <> VOID` guard.
                "child" => {
                    use crate::director::chunks::w3d::types::W3dNodeType;
                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                    // Bare `node.child` (no argument) is the child LIST property.
                    if args.len() != 1 {
                        return Self::get_prop(datum, "child");
                    }
                    let scene = match player.movie.cast_manager.find_member_by_ref(&member_ref)
                        .and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.parsed_scene.clone())
                    {
                        Some(s) => s,
                        None => return Ok(player.alloc_datum(Datum::Void)),
                    };
                    let children: Vec<_> = scene.nodes.iter()
                        .filter(|n| n.parent_name.eq_ignore_ascii_case(&s3d_ref.name))
                        .collect();
                    // Node names are matched case-insensitively (as elsewhere in
                    // this module); an Int argument selects by 1-based index.
                    let found = match player.get_datum(&args[0]) {
                        Datum::Int(i) => {
                            let idx = *i - 1;
                            if idx < 0 { None } else { children.get(idx as usize).copied() }
                        }
                        other => {
                            let name = other.string_value().unwrap_or_default();
                            children.iter().copied()
                                .find(|n| n.name.eq_ignore_ascii_case(&name))
                        }
                    };
                    let child = match found {
                        Some(c) => c,
                        None => return Ok(player.alloc_datum(Datum::Void)),
                    };
                    let obj_type = match child.node_type {
                        W3dNodeType::View => "camera",
                        W3dNodeType::Light => "light",
                        W3dNodeType::Group => "group",
                        _ => "model",
                    };
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: s3d_ref.cast_lib,
                        cast_member: s3d_ref.cast_member,
                        object_type: obj_type.to_string(),
                        name: child.name.clone(),
                    })))
                },
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
                                // Check node_shaders first for per-mesh overrides
                                // (set by Lingo: model.shaderList[i] = clonedShader)
                                {
                                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                    if let Some(override_name) = member
                                        .and_then(|m| m.member_type.as_shockwave3d())
                                        .and_then(|w3d| w3d.runtime_state.node_shaders.get(&s3d_ref.name))
                                        .and_then(|map| map.get(&idx))
                                    {
                                        let result = player.alloc_datum(Datum::Shockwave3dObjectRef(
                                            crate::director::lingo::datum::Shockwave3dObjectRef {
                                                cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                                object_type: "shader".to_string(),
                                                name: override_name.clone(),
                                            }
                                        ));
                                        return Ok(result);
                                    }
                                }
                                // Fall through to model resource bindings
                                use crate::director::chunks::w3d::types::W3dNodeType;
                                let mut shader_names: Vec<String> = Vec::new();

                                let node = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name));
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
                                // Apply node_shaders overrides (from Lingo shaderList[i] = clone)
                                {
                                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                    if let Some(overrides) = member
                                        .and_then(|m| m.member_type.as_shockwave3d())
                                        .and_then(|w3d| w3d.runtime_state.node_shaders.get(&s3d_ref.name))
                                    {
                                        for (mesh_idx, shader_name) in overrides {
                                            if *mesh_idx < shader_names.len() {
                                                shader_names[*mesh_idx] = shader_name.clone();
                                            }
                                        }
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
                                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                                items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                                    cast_lib: s3d_ref.cast_lib,
                                                    cast_member: s3d_ref.cast_member,
                                                    object_type: "texture".to_string(),
                                                    name: layer.name.clone(),
                                                })));
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
                                            0 => "replace",
                                            1 => "add",
                                            3 => "blend",
                                            _ => "multiply", // 2=MODULATE, and MODULATE2X/4X → closest
                                        };
                                        Some(player.alloc_datum(Datum::Symbol(sym.to_string())))
                                    } else {
                                        Some(player.alloc_datum(Datum::Symbol("multiply".to_string())))
                                    }
                                } else {
                                    Some(player.alloc_datum(Datum::Symbol("multiply".to_string())))
                                }
                            }
                            "textureModeList" => {
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                let mode = member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.parsed_scene.as_ref())
                                    .and_then(|scene| scene.shaders.iter().find(|s| s.name == s3d_ref.name))
                                    .and_then(|shader| shader.texture_layers.get(idx))
                                    .map(|layer| match layer.tex_mode {
                                        0 => "none",
                                        4 => "reflection",
                                        5 => "wrapPlanar",
                                        6 => "specular",
                                        _ => "none",
                                    })
                                    .unwrap_or("none");
                                Some(player.alloc_datum(Datum::Symbol(mode.to_string())))
                            }
                            "textureRepeatList" => {
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                let val = member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.parsed_scene.as_ref())
                                    .and_then(|scene| scene.shaders.iter().find(|s| s.name == s3d_ref.name))
                                    .and_then(|shader| shader.texture_layers.get(idx))
                                    .map(|layer| layer.repeat_s as i32)
                                    .unwrap_or(1);
                                Some(player.alloc_datum(Datum::Int(val)))
                            }
                            "blendSourceList" => {
                                // Return blend source for texture layer at index (default #constant)
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                let val = member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.parsed_scene.as_ref())
                                    .and_then(|scene| scene.shaders.iter().find(|s| s.name == s3d_ref.name))
                                    .and_then(|shader| shader.texture_layers.get(idx))
                                    .map(|layer| {
                                        if layer.blend_src == 0 { "alpha" } else { "constant" }
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
                            // meshDeformMesh.textureLayer[n] — return a meshDeformTexLayer ref
                            "textureLayer" if s3d_ref.object_type == "meshDeformMesh" => {
                                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                                let model_name = parts.get(0).unwrap_or(&"").to_string();
                                let mesh_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                Some(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: member_ref.cast_lib,
                                    cast_member: member_ref.cast_member,
                                    object_type: "meshDeformTexLayer".to_string(),
                                    name: format!("{}:{}:{}", model_name, mesh_idx, idx),
                                })))
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
                            // meshDeformMesh.face[j] — return the j-th face's 1-based vertex
                            // indices [v1,v2,v3] (Director's meshDeform face[] convention; the
                            // Director message-window shows e.g. [1,2,3],[4,5,6],…). Lets us diff
                            // dirplayer's decoded triangulation against Director's.
                            "face" if s3d_ref.object_type == "meshDeformMesh" => {
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
                                            if idx < mesh.faces.len() {
                                                let f = mesh.faces[idx];
                                                let items = VecDeque::from(vec![
                                                    player.alloc_datum(Datum::Int(f[0] as i32 + 1)),
                                                    player.alloc_datum(Datum::Int(f[1] as i32 + 1)),
                                                    player.alloc_datum(Datum::Int(f[2] as i32 + 1)),
                                                ]);
                                                return Ok(player.alloc_datum(Datum::List(
                                                    crate::director::lingo::datum::DatumType::List, items, false)));
                                            }
                                        }
                                    }
                                }
                                for key in &keys {
                                    for raw in &scene.raw_meshes {
                                        if raw.name == *key && raw.chain_index as usize == mesh_idx {
                                            if idx < raw.faces.len() {
                                                let f = raw.faces[idx];
                                                let items = VecDeque::from(vec![
                                                    player.alloc_datum(Datum::Int(f[0] as i32 + 1)),
                                                    player.alloc_datum(Datum::Int(f[1] as i32 + 1)),
                                                    player.alloc_datum(Datum::Int(f[2] as i32 + 1)),
                                                ]);
                                                return Ok(player.alloc_datum(Datum::List(
                                                    crate::director::lingo::datum::DatumType::List, items, false)));
                                            }
                                            break;
                                        }
                                    }
                                }

                                Some(player.alloc_datum(Datum::Void))
                            }
                            // camera.overlay[n] / camera.backdrop[n] — indexed overlay access.
                            // camera_overlays is keyed by lowercased camera name (see
                            // addOverlay), so the lookup must lowercase too — otherwise a
                            // mixed-case camera (e.g. "GameCamera") returns Void and the
                            // subsequent `.blend = 0` silently no-ops, leaving every
                            // overlay rendered at default blend=100.
                            "overlay" | "backdrop" if s3d_ref.object_type == "camera" => {
                                let is_overlay = prop_name == "overlay";
                                let cam_key = s3d_ref.name.to_ascii_lowercase();
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                let count = member.and_then(|m| m.member_type.as_shockwave3d())
                                    .map(|w3d| {
                                        let map = if is_overlay { &w3d.runtime_state.camera_overlays } else { &w3d.runtime_state.camera_backdrops };
                                        map.get(&cam_key).map(|v| v.len()).unwrap_or(0)
                                    })
                                    .unwrap_or(0);
                                if idx < count {
                                    Some(player.alloc_datum(Datum::Shockwave3dObjectRef(
                                        crate::director::lingo::datum::Shockwave3dObjectRef {
                                            cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member,
                                            object_type: prop_name.to_string(),
                                            name: format!("{}:{}", cam_key, idx),
                                        }
                                    )))
                                } else {
                                    Some(player.alloc_datum(Datum::Void))
                                }
                            }
                            // node.child[n] — return the n-th child node as a ref
                            "child" => {
                                let children: Vec<&crate::director::chunks::w3d::types::W3dNode> = scene.nodes.iter()
                                    .filter(|n| n.parent_name.eq_ignore_ascii_case(&s3d_ref.name))
                                    .collect();
                                if idx < children.len() {
                                    let child = &children[idx];
                                    let obj_type = match child.node_type {
                                        crate::director::chunks::w3d::types::W3dNodeType::View => "camera",
                                        crate::director::chunks::w3d::types::W3dNodeType::Light => "light",
                                        crate::director::chunks::w3d::types::W3dNodeType::Group => "group",
                                        _ => "model",
                                    };
                                    Some(player.alloc_datum(Datum::Shockwave3dObjectRef(
                                        crate::director::lingo::datum::Shockwave3dObjectRef {
                                            cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member,
                                            object_type: obj_type.to_string(),
                                            name: child.name.clone(),
                                        }
                                    )))
                                } else {
                                    // Director quirk: out-of-range child[N] acts as IDENTITY in a
                                    // chain — it returns the node itself, so e.g. (verified in
                                    // real Director) evil.child[2].child[1].child[1] resolves to the
                                    // body even though evil has only one child (child[2] collapses,
                                    // continuing from evil). frog01's evil-frog black/red shader
                                    // setup addresses evil.child[2] (evil has 1 child) and relies on
                                    // this; without it the chain dies to VOID and no shaders apply.
                                    Some(player.alloc_datum(Datum::Shockwave3dObjectRef(s3d_ref.clone())))
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
                },
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
                },
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
                                let node = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name));
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
                                let node = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name));
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
                            "bone" => {
                                // bonesPlayer.bone.count / resource.bone.count — the
                                // owning model's (or resource's) skeleton bone count.
                                find_skeleton_for_model(&scene, &s3d_ref.name)
                                    .map(|s| s.bones.len())
                                    .unwrap_or(0)
                            }
                            "playList" => {
                                // Director's playList includes the currently playing motion
                                // as entry [1], then the queued motions.
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                member.and_then(|m| m.member_type.as_shockwave3d())
                                    .map(|w3d| match w3d.runtime_state.bones_player(&s3d_ref.name).filter(|b| b.current_motion.is_some()) {
                                        Some(bp) => (if bp.current_motion.is_some() { 1 } else { 0 }) + bp.motion_queue.len(),
                                        None => (if w3d.runtime_state.current_motion.is_some() { 1 } else { 0 })
                                            + w3d.runtime_state.motion_queue.len(),
                                    })
                                    .unwrap_or(0)
                            }
                            "overlay" | "backdrop" => {
                                // camera.overlay.count / camera.backdrop.count.
                                // camera_overlays is keyed by the LOWERCASED camera name
                                // (see addOverlay), so this lookup must lowercase too. Without
                                // it, a mixed-case camera name (`w.camera[1]` → "DefaultView")
                                // returned count 0, so scripts capturing `pN = cam.overlay.count`
                                // got 0 and then wrote `overlay[0]` — hitting the wrong overlay
                                // (the unicraft galaxy blanked its "Choose Planet" title this way).
                                let is_overlay = prop_name == "overlay";
                                let cam_key = s3d_ref.name.to_ascii_lowercase();
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                member.and_then(|m| m.member_type.as_shockwave3d())
                                    .map(|w3d| {
                                        let map = if is_overlay { &w3d.runtime_state.camera_overlays } else { &w3d.runtime_state.camera_backdrops };
                                        map.get(&cam_key).map(|v| v.len()).unwrap_or(0)
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
                            "face" => {
                                // meshDeformMesh.face.count — triangle count of this mesh
                                // group. Compiled as the `count(obj, #face)` builtin (objcall),
                                // NOT a property getter, so it must be resolved here. Mirrors
                                // the vertexList case but reads `faces`. Without this,
                                // `mesh[m].face.count` returned 0 (newMesh-from-primitive code
                                // like Splat's pip tower then built an empty mesh → crash).
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
                                            count = mesh.faces.len();
                                        }
                                        break;
                                    }
                                }
                                if count == 0 {
                                    for key in &keys {
                                        for raw in &scene.raw_meshes {
                                            if raw.name == *key && raw.chain_index as usize == m_idx {
                                                count = raw.faces.len();
                                                break;
                                            }
                                        }
                                        if count > 0 { break; }
                                    }
                                }
                                if count == 0 {
                                    for key in &keys {
                                        if let Some(res_info) = scene.model_resources.get(*key) {
                                            if let Some(info) = res_info.mesh_infos.get(m_idx) {
                                                count = info.num_faces as usize;
                                            }
                                            break;
                                        }
                                    }
                                }
                                count
                            }
                            "child" => {
                                scene.nodes.iter().filter(|n| n.parent_name.eq_ignore_ascii_case(&s3d_ref.name)).count()
                            }
                            "textureLayer" => {
                                // meshDeformMesh.count(#textureLayer) — read from persistent list
                                let parts: Vec<&str> = s3d_ref.name.splitn(2, ':').collect();
                                let mdl_name = parts.get(0).unwrap_or(&"").to_string();
                                let m_idx: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                                let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                                member.and_then(|m| m.member_type.as_shockwave3d())
                                    .and_then(|w3d| w3d.runtime_state.mesh_deform.get(&mdl_name))
                                    .and_then(|md| md.meshes.get(m_idx))
                                    .and_then(|mesh| mesh.texture_layer_datum_ref.as_ref())
                                    .map(|list_ref| {
                                        match player.get_datum(list_ref) {
                                            Datum::List(_, items, _) => items.len(),
                                            _ => 0,
                                        }
                                    })
                                    .unwrap_or(0)
                            }
                            _ => 0,
                        };
                        Ok(player.alloc_datum(Datum::Int(count as i32)))
                    } else {
                        Ok(player.alloc_datum(Datum::Int(0)))
                    }
                },
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
                        .find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name))
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
                    Ok(player.alloc_datum(Datum::Point([sx as f64, sy as f64], 0)))
                },
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
                        match player.get_datum(&args[0]) {
                            Datum::Point(vals, _flags) => {
                                (vals[0] as f32, vals[1] as f32)
                            }
                            _ => (0.0, 0.0),
                        }
                    };
                    let cam_transform = get_node_transform(player, &member_ref, &s3d_ref.name);
                    let vw = player.movie.rect.width() as f32;
                    let vh = player.movie.rect.height() as f32;
                    let fov = scene.nodes.iter()
                        .find(|n| n.name.eq_ignore_ascii_case(&s3d_ref.name))
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
                },
                // ─── Mesh build workflow: generateNormals() and build() ───
                "generateNormals" => {
                    // modelResource.generateNormals(#flat | #smooth)
                    let style = if !args.is_empty() {
                        match player.get_datum(&args[0]).string_value().unwrap_or_default().as_str() {
                            "smooth" => 1u8,
                            _ => 0u8, // #flat
                        }
                    } else { 0 };
                    let res_name = s3d_ref.name.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            use crate::player::cast_member::MeshBuildData;
                            w3d.runtime_state.mesh_build_data
                                .entry(res_name)
                                .or_insert_with(MeshBuildData::default)
                                .generate_normals_style = Some(style);
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                },
                "build" => {
                    // modelResource.build() — construct mesh geometry from face data
                    let res_name = s3d_ref.name.clone();
                    let member_ref = CastMemberRef { cast_lib: s3d_ref.cast_lib, cast_member: s3d_ref.cast_member };

                    // 1. Read face data from persistent face list
                    let face_key = format!("face:{}", res_name);
                    let face_list_ref = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                        member.and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.runtime_state.shader_texture_lists.get(&face_key))
                            .cloned()
                    };
                    let dbg_had_face_list = face_list_ref.is_some();

                    // 2. Read build data (vertexList, textureCoordinateList, etc.)
                    let build_data = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                        member.and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.runtime_state.mesh_build_data.get(&res_name))
                            .cloned()
                            .unwrap_or_default()
                    };

                    // 3. Extract face vertex/texcoord/color indices and shader assignments
                    struct FaceData {
                        vertex_indices: [u32; 3],       // 1-based → 0-based
                        texcoord_indices: [u32; 3],     // 1-based → 0-based
                        shader_name: String,
                    }
                    let mut faces: Vec<FaceData> = Vec::new();

                    if let Some(face_list_ref) = face_list_ref {
                        let face_list = player.get_datum(&face_list_ref).clone();
                        if let Datum::List(_, face_items, _) = face_list {
                            for face_ref in &face_items {
                                let face_datum = player.get_datum(face_ref).clone();
                                if let Datum::PropList(props, _) = face_datum {
                                    let mut verts = [0u32; 3];
                                    let mut tcs = [0u32; 3];
                                    let mut shader_name = String::new();

                                    for (k_ref, v_ref) in &props {
                                        let key = player.get_datum(k_ref).string_value().unwrap_or_default();
                                        // Director is case-insensitive and accepts the abbreviated
                                        // `texCoords` (what Splat's spike build uses) as well as the
                                        // full `textureCoordinates`. Match lowercase so the authored
                                        // per-face texcoord indices are actually read (otherwise every
                                        // face defaults to index 0 → all-identical UVs → the GPU
                                        // upload's all_same check regenerates positional UVs).
                                        match key.to_ascii_lowercase().as_str() {
                                            "shader" => {
                                                match player.get_datum(v_ref) {
                                                    Datum::Shockwave3dObjectRef(r) => shader_name = r.name.clone(),
                                                    _ => {}
                                                }
                                            }
                                            "vertices" => {
                                                if let Datum::List(_, items, _) = player.get_datum(v_ref) {
                                                    for (i, item) in items.iter().enumerate().take(3) {
                                                        let idx = player.get_datum(item).int_value().unwrap_or(1);
                                                        verts[i] = (idx.max(1) - 1) as u32; // 1-based → 0-based
                                                    }
                                                }
                                            }
                                            "texturecoordinates" | "texcoords" => {
                                                if let Datum::List(_, items, _) = player.get_datum(v_ref) {
                                                    // Only overwrite when this key actually carries data —
                                                    // the proplist holds BOTH an empty "textureCoordinates"
                                                    // (initial) and the authored "texCoords"; the empty one
                                                    // must not clobber the real indices.
                                                    if !items.is_empty() {
                                                        for (i, item) in items.iter().enumerate().take(3) {
                                                            let idx = player.get_datum(item).int_value().unwrap_or(1);
                                                            tcs[i] = (idx.max(1) - 1) as u32; // 1-based → 0-based
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    faces.push(FaceData { vertex_indices: verts, texcoord_indices: tcs, shader_name });
                                }
                            }
                        }
                    }

                    let _ = dbg_had_face_list;
                    if faces.is_empty() || build_data.vertex_list.is_empty() {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // 4. Group faces by shader
                    let mut shader_groups: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
                    for (i, f) in faces.iter().enumerate() {
                        shader_groups.entry(f.shader_name.clone()).or_default().push(i);
                    }

                    // 5. Build ClodDecodedMesh per shader group. Track the group's
                    // shader name in lock-step with `meshes` so mesh_infos /
                    // shaderList / meshDeform.mesh[i] all index the same group.
                    use crate::director::chunks::w3d::types::ClodDecodedMesh;
                    let mut meshes: Vec<ClodDecodedMesh> = Vec::new();
                    let mut group_names: Vec<String> = Vec::new();
                    let gen_normals = build_data.generate_normals_style;

                    for (shader_name, face_indices) in &shader_groups {
                        group_names.push(shader_name.clone());
                        // Collect unique vertex indices used by this group
                        let mut vert_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
                        let mut positions: Vec<[f32; 3]> = Vec::new();
                        let mut normals: Vec<[f32; 3]> = Vec::new();
                        let mut tex_coords: Vec<[f32; 2]> = Vec::new();
                        let mut mesh_faces: Vec<[u32; 3]> = Vec::new();

                        // For flat normals, we emit unique vertices per face
                        if gen_normals == Some(0) {
                            // Flat shading: each face gets 3 unique vertices with the face normal
                            for &fi in face_indices {
                                let f = &faces[fi];
                                let base = positions.len() as u32;
                                for k in 0..3usize {
                                    let vi = f.vertex_indices[k] as usize;
                                    let pos = build_data.vertex_list.get(vi).copied().unwrap_or([0.0; 3]);
                                    positions.push(pos);
                                    let ti = f.texcoord_indices[k] as usize;
                                    let tc = build_data.texture_coordinate_list.get(ti).copied().unwrap_or([0.0; 2]);
                                    tex_coords.push(tc);
                                }
                                // Compute face normal from cross product
                                let v0 = positions[(base) as usize];
                                let v1 = positions[(base + 1) as usize];
                                let v2 = positions[(base + 2) as usize];
                                let e1 = [v1[0]-v0[0], v1[1]-v0[1], v1[2]-v0[2]];
                                let e2 = [v2[0]-v0[0], v2[1]-v0[1], v2[2]-v0[2]];
                                let nx = e1[1]*e2[2] - e1[2]*e2[1];
                                let ny = e1[2]*e2[0] - e1[0]*e2[2];
                                let nz = e1[0]*e2[1] - e1[1]*e2[0];
                                let len = (nx*nx + ny*ny + nz*nz).sqrt().max(1e-10);
                                let n = [nx/len, ny/len, nz/len];
                                normals.push(n);
                                normals.push(n);
                                normals.push(n);
                                mesh_faces.push([base, base+1, base+2]);
                            }
                        } else {
                            // Smooth or no normals: share vertices
                            for &fi in face_indices {
                                let f = &faces[fi];
                                let mut tri = [0u32; 3];
                                for k in 0..3usize {
                                    let vi = f.vertex_indices[k];
                                    let new_idx = if let Some(&idx) = vert_map.get(&vi) {
                                        idx
                                    } else {
                                        let idx = positions.len() as u32;
                                        let pos = build_data.vertex_list.get(vi as usize).copied().unwrap_or([0.0; 3]);
                                        positions.push(pos);
                                        let ti = f.texcoord_indices[k] as usize;
                                        let tc = build_data.texture_coordinate_list.get(ti).copied().unwrap_or([0.0; 2]);
                                        tex_coords.push(tc);
                                        if gen_normals == Some(1) {
                                            normals.push([0.0; 3]); // Will accumulate
                                        }
                                        vert_map.insert(vi, idx);
                                        idx
                                    };
                                    tri[k] = new_idx;
                                }
                                // Accumulate face normals for smooth shading
                                if gen_normals == Some(1) {
                                    let v0 = positions[tri[0] as usize];
                                    let v1 = positions[tri[1] as usize];
                                    let v2 = positions[tri[2] as usize];
                                    let e1 = [v1[0]-v0[0], v1[1]-v0[1], v1[2]-v0[2]];
                                    let e2 = [v2[0]-v0[0], v2[1]-v0[1], v2[2]-v0[2]];
                                    let nx = e1[1]*e2[2] - e1[2]*e2[1];
                                    let ny = e1[2]*e2[0] - e1[0]*e2[2];
                                    let nz = e1[0]*e2[1] - e1[1]*e2[0];
                                    for &idx in &tri {
                                        let n = &mut normals[idx as usize];
                                        n[0] += nx; n[1] += ny; n[2] += nz;
                                    }
                                }
                                mesh_faces.push(tri);
                            }
                            // Normalize accumulated normals for smooth shading
                            if gen_normals == Some(1) {
                                for n in &mut normals {
                                    let len = (n[0]*n[0] + n[1]*n[1] + n[2]*n[2]).sqrt().max(1e-10);
                                    n[0] /= len; n[1] /= len; n[2] /= len;
                                }
                            }
                        }

                        // newMesh UVs are authored in Director's standard [0,1] space; the 3D
                        // vertex shader applies the CLOD remap (u+0.5, 0.5-v) which expects
                        // pre-centered [-0.5,0.5] UVs. Pre-center so the remap yields (u, 1-v):
                        // u-0.5 cancels the +0.5; the bare -0.5 leaves the shader's 0.5-v as a net
                        // V flip (Director image v=0=top → our GL texture v=0=bottom). For Splat's
                        // spikes this puts the texture's colored band (image bottom 35%) on the
                        // spike TIPS (texcoord v≈0.01 at the apex) and the black band on the
                        // base/core — Director's "black body, colored spike tips". Uniform
                        // textures (the maze) are unaffected by the flip.
                        for uv in tex_coords.iter_mut() { uv[0] -= 0.5; uv[1] -= 0.5; }
                        meshes.push(ClodDecodedMesh {
                            name: res_name.clone(),
                            positions,
                            normals,
                            tex_coords: vec![tex_coords],
                            faces: mesh_faces,
                            diffuse_colors: vec![],
                            specular_colors: vec![],
                            bone_indices: vec![],
                            bone_weights: vec![],
                        });
                    }

                    // 6. Store meshes in scene and update model_resources
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            if let Some(scene) = w3d.scene_mut() {
                                // Update model resource info. Director exposes ONE
                                // mesh per shader group (meshDeform.mesh.count ==
                                // groups, shaderList[i] ↔ mesh[i]), so rebuild
                                // mesh_infos with one entry per built mesh (was a
                                // single entry, which made meshDeform.mesh.count
                                // collapse to 1 and broke per-group deformation /
                                // shader mapping — e.g. Splat's pip tower).
                                if let Some(res_info) = scene.model_resources.get_mut(&res_name) {
                                    let template = res_info.mesh_infos.get(0).cloned().unwrap_or_default();
                                    res_info.mesh_infos = meshes.iter().map(|m| {
                                        let mut mi = template.clone();
                                        mi.num_faces = m.faces.len() as u32;
                                        mi.num_vertices = m.positions.len() as u32;
                                        mi
                                    }).collect();
                                    // shader bindings, aligned 1:1 with `meshes` via group_names.
                                    res_info.shader_bindings.clear();
                                    let shader_names: Vec<String> = shader_groups.keys().cloned().collect();
                                    let mesh_bindings: Vec<String> = shader_names.iter().cloned().collect();
                                    res_info.shader_bindings.push(crate::director::chunks::w3d::types::ModelShaderBinding {
                                        name: "default".to_string(),
                                        mesh_bindings,
                                    });
                                }
                                scene.clod_meshes.insert(res_name.clone(), meshes);
                                // Bump BOTH content versions: a `newMesh` +
                                // `build()` flow inserts into clod_meshes,
                                // and the renderer's `ensure_member_loaded`
                                // path checks `mesh_content_version` to
                                // decide whether to rebuild GPU mesh buffers
                                // (Mesh3dBuffers VBOs in `mesh_groups`).
                                // Without this bump the new vertices live
                                // in `scene.clod_meshes` but never reach the
                                // GPU — the model renders empty / placeholder.
                                scene.mesh_content_version += 1;
                                scene.texture_content_version += 1; // trigger GPU texture re-upload
                            }
                        }
                    }

                    log(&format!(
                        "[W3D] modelResource(\"{}\").build() — {} faces, {} vertices, {} shader groups",
                        res_name, faces.len(), build_data.vertex_list.len(), shader_groups.len()
                    ));
                    Ok(player.alloc_datum(Datum::Void))
                },
                _ => {
                    // Treat as property get
                    Self::get_prop(datum, handler_name)
                },
            })
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

        match_ci!(prop, {
            "name" => Ok(player.alloc_datum(Datum::String(model_name.to_string()))),
            "bone.count" | "boneCount" => {
                let count = find_skeleton_for_model(scene, model_name)
                    .map(|s| s.bones.len()).unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            },
            "bone" => {
                // bonesPlayer.bone — return a LIST of bone refs (one per skeleton bone),
                // so `bone.count` and `bone[i]` (and `bone[i].worldTransform`) work from
                // the console/Lingo. Lingo `bone[1]` → items[0] → the root bone, matching
                // both Director and the compiled indexed path. The skeleton is resolved by
                // the model's OWN resource (each cloned bot has its own skeleton), fixing
                // the previous `bonesPlayer.bone` = VOID / `bone.count` = 0 for clones.
                let count = find_skeleton_for_model(scene, model_name)
                    .map(|s| s.bones.len())
                    .unwrap_or(0);
                let mut items = VecDeque::new();
                for i in 0..count {
                    items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(
                        crate::director::lingo::datum::Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib,
                            cast_member: member_ref.cast_member,
                            object_type: "bone".to_string(),
                            name: format!("{}:{}", model_name, i),
                        },
                    )));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            },
            "collision" => {
                // model.collision — returns the #collision modifier object if one
                // was added via addModifier(#collision), else VOID (Director).
                let has = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.runtime_state.collision_modifiers.keys()
                        .any(|k| k.eq_ignore_ascii_case(model_name)))
                    .unwrap_or(false);
                if has {
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(crate::director::lingo::datum::Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib,
                        cast_member: member_ref.cast_member,
                        object_type: "collision".to_string(),
                        name: model_name.to_string(),
                    })))
                } else {
                    Ok(player.alloc_datum(Datum::Void))
                }
            },
            "visible" | "visibility" => Ok(player.alloc_datum(Datum::Int(1))),
            "pointAtOrientation" | "pointatorientation" => {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                let orientation = member.and_then(|m| m.member_type.as_shockwave3d())
                    .and_then(|w3d| w3d.runtime_state.point_at_orientations.get(model_name))
                    .copied();
                let (front, up) = orientation.unwrap_or(([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]));
                let v1 = player.alloc_datum(Datum::Vector([front[0] as f64, front[1] as f64, front[2] as f64]));
                let v2 = player.alloc_datum(Datum::Vector([up[0] as f64, up[1] as f64, up[2] as f64]));
                let items = std::collections::VecDeque::from(vec![v1, v2]);
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            },
            "transform" => {
                Ok(get_persistent_node_transform(player, member_ref, model_name))
            },
            "userData" => {
                // Director chapter 15 (`director_reference.md:80586`):
                // returns the userData property list of a model. Default is
                // an empty PropList `[:]`. The returned ref must be the same
                // across reads so `model.userData.setaProp(#k, v)` mutations
                // are visible on subsequent accesses (Director's userData is
                // a live reference, not a snapshot).
                //
                // Lazy allocation: first access creates an empty PropList
                // datum and stashes its DatumRef on the cast member's
                // runtime_state; subsequent reads return the same ref.
                Ok(get_or_create_node_user_data(player, member_ref, model_name))
            },
            "worldPosition" => {
                let wp = get_world_position(player, member_ref, model_name);
                Ok(player.alloc_datum(Datum::Vector(wp)))
            },
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
            },
            "sds" => {
                // Subdivision Surface modifier — return SDS object ref
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "sds".to_string(),
                    name: model_name.to_string(),
                })))
            },
            "lod" => {
                // LOD modifier — return LOD object ref
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "lod".to_string(),
                    name: model_name.to_string(),
                })))
            },
            "parent" => {
                if let Some(n) = node {
                    // A model removed from the world (removeFromWorld) keeps its
                    // scene node + parent_name (so addToWorld can re-link it), but
                    // Director reports `model.parent` as VOID while it's detached.
                    // Without this, `voidp(model.parent)` stays false and callers
                    // that gate on it loop forever — SweeTarts' collectobjects
                    // re-collects a removeFromWorld'd number candy every frame.
                    let detached = player.movie.cast_manager.find_member_by_ref(member_ref)
                        .and_then(|m| m.member_type.as_shockwave3d())
                        .map(|w3d| w3d.runtime_state.detached_nodes.iter()
                            .any(|d| d.eq_ignore_ascii_case(&n.name)))
                        .unwrap_or(false);
                    if detached {
                        Ok(player.alloc_datum(Datum::Void))
                    } else {
                        // Director returns the parent NODE object (model/camera/light/
                        // group), not its name — so chained access like
                        // `model.parent.getWorldTransform()` works. The world root
                        // ("World") is a group. Resolve the parent node's type.
                        use crate::director::lingo::datum::Shockwave3dObjectRef;
                        use crate::director::chunks::w3d::types::W3dNodeType;
                        let pname = n.parent_name.clone();
                        if pname.is_empty() {
                            Ok(player.alloc_datum(Datum::Void))
                        } else {
                            let obj_type = scene.nodes.iter()
                                .find(|pn| pn.name.eq_ignore_ascii_case(&pname))
                                .map(|pn| match pn.node_type {
                                    W3dNodeType::View => "camera",
                                    W3dNodeType::Light => "light",
                                    W3dNodeType::Group => "group",
                                    _ => "model",
                                })
                                .unwrap_or("group");
                            Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                object_type: obj_type.to_string(), name: pname,
                            })))
                        }
                    }
                } else {
                    Ok(player.alloc_datum(Datum::Void))
                }
            },
            "child.count" | "childCount" => {
                // Count children of this node
                let count = scene.nodes.iter()
                    .filter(|n| n.parent_name.eq_ignore_ascii_case(model_name))
                    .count();
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            },
            "child" => {
                // Return list of child node refs
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                let children: Vec<_> = scene.nodes.iter()
                    .filter(|n| n.parent_name.eq_ignore_ascii_case(model_name))
                    .collect();
                let mut items = VecDeque::new();
                for child in &children {
                    let obj_type = match child.node_type {
                        crate::director::chunks::w3d::types::W3dNodeType::View => "camera",
                        crate::director::chunks::w3d::types::W3dNodeType::Light => "light",
                        crate::director::chunks::w3d::types::W3dNodeType::Group => "group",
                        _ => "model",
                    };
                    items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                        object_type: obj_type.to_string(), name: child.name.clone(),
                    })));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            },
            "shader" => {
                // Return the model's first shader (equivalent to shaderList[1])
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                let mut shader_name = String::new();
                // 1) Check runtime shader override (from Lingo shaderList[1] = shaderRef)
                if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d() {
                        if let Some(overrides) = w3d.runtime_state.node_shaders.get(model_name) {
                            if let Some(name) = overrides.get(&0) {
                                shader_name = name.clone();
                            }
                        }
                    }
                }
                // 2) Check model resource shader bindings (first mesh's shader)
                if shader_name.is_empty() {
                    let resource = node.map(|n| {
                        if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name }
                    });
                    if let Some(rn) = resource {
                        if let Some(res) = scene.model_resources.get(rn.as_str()) {
                            for binding in &res.shader_bindings {
                                if !binding.mesh_bindings.is_empty() && !binding.mesh_bindings[0].is_empty() {
                                    // Prefer non-DefaultShader bindings
                                    let name = &binding.mesh_bindings[0];
                                    if !name.eq_ignore_ascii_case("DefaultShader") {
                                        shader_name = name.clone();
                                        break;
                                    } else if shader_name.is_empty() {
                                        shader_name = name.clone();
                                    }
                                }
                            }
                        }
                    }
                }
                // 3) Fallback: node's shader_name
                if shader_name.is_empty() {
                    if let Some(n) = node {
                        if !n.shader_name.is_empty() {
                            shader_name = n.shader_name.clone();
                        }
                    }
                }
                // 4) Last resort: model index → shader index (Director behavior for W3D-loaded scenes)
                if shader_name.is_empty() {
                    let model_index = scene.nodes.iter()
                        .filter(|n| n.node_type == W3dNodeType::Model)
                        .position(|n| n.name == model_name);
                    if let Some(mi) = model_index {
                        if mi < scene.shaders.len() {
                            shader_name = scene.shaders[mi].name.clone();
                        }
                    }
                }
                if shader_name.is_empty() {
                    shader_name = "DefaultShader".to_string();
                }
                // Find cast_lib/cast_member from parent context
                // We don't have it here directly, use 0 as placeholder
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                    object_type: "shader".to_string(),
                    name: shader_name,
                })))
            },
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
                        // Apply node_shaders override (from Lingo shaderList[i] = clone)
                        let member_check = player.movie.cast_manager.find_member_by_ref(member_ref);
                        if let Some(override_name) = member_check
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.runtime_state.node_shaders.get(model_name))
                            .and_then(|map| map.get(&mesh_idx))
                        {
                            best_name = override_name.clone();
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
            },
            "bonesPlayer" | "keyframePlayer" => {
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: prop.to_string(),
                    name: model_name.to_string(),
                })))
            },
            "playing" => {
                let playing = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| if w3d.runtime_state.bones_player(model_name).filter(|b| b.current_motion.is_some()).map(|bp| bp.animation_playing).unwrap_or(w3d.runtime_state.animation_playing) { 1 } else { 0 })
                    .unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(playing)))
            },
            "currentTime" => {
                let time = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.runtime_state.bones_player(model_name).filter(|b| b.current_motion.is_some()).map(|bp| bp.animation_time).unwrap_or(w3d.runtime_state.animation_time))
                    .unwrap_or(0.0);
                // Director returns currentTime in milliseconds
                Ok(player.alloc_datum(Datum::Int((time * 1000.0) as i32)))
            },
            "playRate" => {
                let rate = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.runtime_state.bones_player(model_name).filter(|b| b.current_motion.is_some()).map(|bp| bp.play_rate).unwrap_or(w3d.runtime_state.play_rate))
                    .unwrap_or(1.0);
                Ok(player.alloc_datum(Datum::Float(rate as f64)))
            },
            "rootLock" => {
                let locked = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| if w3d.runtime_state.bones_player(model_name).filter(|b| b.current_motion.is_some()).map(|bp| bp.root_lock).unwrap_or(w3d.runtime_state.root_lock) { 1 } else { 0 })
                    .unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(locked)))
            },
            "currentLoopState" => {
                let looping = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| if w3d.runtime_state.bones_player(model_name).filter(|b| b.current_motion.is_some()).map(|bp| bp.animation_loop).unwrap_or(w3d.runtime_state.animation_loop) { 1 } else { 0 })
                    .unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(looping)))
            },
            "autoBlend" => {
                Ok(player.alloc_datum(Datum::Int(1))) // default TRUE
            },
            "blendFactor" => {
                // blend_weight is 0.0-1.0, Director uses 0.0-100.0
                let factor = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.runtime_state.bones_player(model_name).filter(|b| b.current_motion.is_some()).map(|bp| bp.blend_weight).unwrap_or(w3d.runtime_state.blend_weight) * 100.0)
                    .unwrap_or(0.0);
                Ok(player.alloc_datum(Datum::Float(factor as f64)))
            },
            "positionReset" => {
                Ok(player.alloc_datum(Datum::Int(1))) // default TRUE
            },
            "rotationReset" => {
                Ok(player.alloc_datum(Datum::Symbol("all".to_string()))) // default #all
            },
            "lockTranslation" => {
                Ok(player.alloc_datum(Datum::Symbol("none".to_string()))) // default #none
            },
            "boundingSphere" => {
                // Director 11.5 Scripting Dictionary, `boundingSphere`: "describes
                // a sphere that contains the model, group, light, or camera AND ITS
                // CHILDREN", as [vector center, float radius] in world space.
                //
                // This used to return a hardcoded [vector(0,0,0), 100.0]. age_of_speed's
                // culling manager buckets every track token into a world grid with
                // `BoxOverlapsSphere(blockMax, blockMin, model.boundingSphere[2],
                // model.boundingSphere[1], 2)` — with every sphere pinned at the origin
                // and the track out around x = -68000, no block ever matched, so
                // `pTokenBlocks` stayed empty and `getToken` always returned "not found".
                let (center, radius) = model_bounding_sphere(player, scene, model_name, member_ref);
                let center = player.alloc_datum(Datum::Vector(center));
                let radius = player.alloc_datum(Datum::Float(radius));
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::from(vec![center, radius]),
                    false,
                )))
            },
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
            },
            "modifiers" | "modifier" => {
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::new(),
                    false,
                )))
            },
            "playList" => {
                // bonesPlayer.playList — list of motions as property lists. Director's
                // playList includes the CURRENTLY PLAYING motion as entry [1], then the
                // queued motions. dirplayer stores current_motion separately from
                // motion_queue, so prepend it here so playList[1] is the active motion
                // (the C_BonesControl `playList[1].name` / `.count` checks rely on this).
                let queue = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| {
                        let rs = &w3d.runtime_state;
                        // Prefer the per-model bonesPlayer state; fall back to legacy fields.
                        let (cur, loop_, start, end, scale, time, queue) = match rs.bones_player(model_name).filter(|b| b.current_motion.is_some()) {
                            Some(bp) => (bp.current_motion.clone(), bp.animation_loop, bp.animation_start_time,
                                bp.animation_end_time, bp.animation_scale, bp.animation_time, bp.motion_queue.clone()),
                            None => (rs.current_motion.clone(), rs.animation_loop, rs.animation_start_time,
                                rs.animation_end_time, rs.animation_scale, rs.animation_time, rs.motion_queue.clone()),
                        };
                        let mut list: Vec<crate::player::cast_member::QueuedMotion> = Vec::new();
                        if let Some(name) = cur {
                            list.push(crate::player::cast_member::QueuedMotion {
                                name,
                                looped: loop_,
                                start_time: start,
                                end_time: end,
                                scale,
                                offset: time,
                            });
                        }
                        list.extend(queue.into_iter());
                        list
                    })
                    .unwrap_or_default();
                let mut items: VecDeque<DatumRef> = VecDeque::new();
                for qm in &queue {
                    let mut pairs: VecDeque<(DatumRef, DatumRef)> = VecDeque::new();
                    let k = player.alloc_datum(Datum::Symbol("name".to_string()));
                    let v = player.alloc_datum(Datum::String(qm.name.clone()));
                    pairs.push_back((k, v));
                    let k = player.alloc_datum(Datum::Symbol("loop".to_string()));
                    let v = player.alloc_datum(Datum::Int(if qm.looped { 1 } else { 0 }));
                    pairs.push_back((k, v));
                    let k = player.alloc_datum(Datum::Symbol("startTime".to_string()));
                    let v = player.alloc_datum(Datum::Int((qm.start_time * 1000.0) as i32));
                    pairs.push_back((k, v));
                    let k = player.alloc_datum(Datum::Symbol("endTime".to_string()));
                    let v = player.alloc_datum(Datum::Int((qm.end_time * 1000.0) as i32));
                    pairs.push_back((k, v));
                    let k = player.alloc_datum(Datum::Symbol("scale".to_string()));
                    let v = player.alloc_datum(Datum::Float(qm.scale as f64));
                    pairs.push_back((k, v));
                    items.push_back(player.alloc_datum(Datum::PropList(pairs, false)));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    items,
                    false,
                )))
            },
            // playing/currentTime/playRate handled above in the first match arm
            _ => {
                log(&format!("[W3D] model(\"{}\").{} (stub)", model_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
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

        // Default material values matching Director's DefaultShader
        let default_ambient = [63.0 / 255.0, 63.0 / 255.0, 63.0 / 255.0, 1.0];
        let default_diffuse = [1.0_f32, 1.0, 1.0, 1.0];
        let default_specular = [1.0_f32, 1.0, 1.0, 1.0];
        let default_emissive = [0.0_f32, 0.0, 0.0, 1.0];

        match_ci!(prop, {
            "name" => Ok(player.alloc_datum(Datum::String(shader_name.to_string()))),
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("shader".to_string()))),
            "type" => Ok(player.alloc_datum(Datum::Symbol("standard".to_string()))),
            "diffuse" => {
                let c = material.map(|m| m.diffuse).unwrap_or(default_diffuse);
                Ok(player.alloc_datum(color_to_datum(c)))
            },
            "ambient" => {
                let c = material.map(|m| m.ambient).unwrap_or(default_ambient);
                Ok(player.alloc_datum(color_to_datum(c)))
            },
            "specular" => {
                let c = material.map(|m| m.specular).unwrap_or(default_specular);
                Ok(player.alloc_datum(color_to_datum(c)))
            },
            "emissive" => {
                let c = material.map(|m| m.emissive).unwrap_or(default_emissive);
                Ok(player.alloc_datum(color_to_datum(c)))
            },
            "shininess" => {
                // Stored 0..1 (IFX); Director exposes it as 0..100.
                let v = material.map(|m| m.shininess * 100.0).unwrap_or(0.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            },
            "blend" => {
                let v = material.map(|m| m.opacity * 100.0).unwrap_or(100.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            },
            "transparent" => {
                // Director default is 1 (transparency enabled)
                Ok(player.alloc_datum(Datum::Int(1)))
            },
            "renderStyle" => Ok(player.alloc_datum(Datum::Symbol("fill".to_string()))),
            "flat" => Ok(player.alloc_datum(Datum::Int(0))),
            "useDiffuseWithTexture" => {
                let val = shader.map(|s| s.use_diffuse_with_texture).unwrap_or(false);
                Ok(player.alloc_datum(Datum::Int(if val { 1 } else { 0 })))
            },
            "diffuseLightMap" | "glossMap" | "specularLightMap" => {
                Ok(player.alloc_datum(Datum::Void))
            },
            "blendConstant" => {
                // First texture layer's blend constant * 100
                let v = shader.and_then(|s| s.texture_layers.first())
                    .map(|l| l.blend_const * 100.0).unwrap_or(50.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            },
            "blendFunction" => {
                // First texture layer's blend function
                let v = shader.and_then(|s| s.texture_layers.first())
                    .map(|l| l.blend_func).unwrap_or(0);
                let sym = match v { 0 => "replace", 1 => "add", 3 => "blend", _ => "multiply" };
                Ok(player.alloc_datum(Datum::Symbol(sym.to_string())))
            },
            "blendSource" => {
                // First texture layer's blend source
                let v = shader.and_then(|s| s.texture_layers.first())
                    .map(|l| l.blend_src).unwrap_or(0);
                let sym = if v == 0 { "alpha" } else { "constant" };
                Ok(player.alloc_datum(Datum::Symbol(sym.to_string())))
            },
            "textureMode" => {
                let v = shader.and_then(|s| s.texture_layers.first())
                    .map(|l| l.tex_mode).unwrap_or(0);
                let sym = match v { 4 => "reflection", 5 => "wrapPlanar", 6 => "specular", _ => "none" };
                Ok(player.alloc_datum(Datum::Symbol(sym.to_string())))
            },
            "textureRepeat" => {
                let v = shader.and_then(|s| s.texture_layers.first())
                    .map(|l| l.repeat_s as i32).unwrap_or(1);
                Ok(player.alloc_datum(Datum::Int(v)))
            },
            "texture" => {
                // Return first texture as a Shockwave3dObjectRef
                // Falls back to "DefaultTexture" (Director always has one)
                let tex_name = shader.and_then(|s| s.texture_layers.first())
                    .map(|l| l.name.as_str())
                    .filter(|n| !n.is_empty())
                    .unwrap_or("DefaultTexture");
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "texture".to_string(),
                    name: tex_name.to_string(),
                })))
            },
            "reflectionMap" | "reflectionmap" => {
                // Getter for the reflection helper property: returns the texture on
                // the third texture layer, or VOID if none (Director 11.5 default).
                let tex_name = shader
                    .and_then(|s| s.texture_layers.get(2))
                    .map(|l| l.name.as_str())
                    .filter(|n| !n.is_empty());
                match tex_name {
                    Some(name) => {
                        use crate::director::lingo::datum::Shockwave3dObjectRef;
                        Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib,
                            cast_member: member_ref.cast_member,
                            object_type: "texture".to_string(),
                            name: name.to_string(),
                        })))
                    }
                    None => Ok(player.alloc_datum(Datum::Void)),
                }
            },
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
                    // Fill from scene data as texture object refs, pad with VOID
                    let mut items = VecDeque::new();
                    if let Some(s) = shader {
                        for layer in &s.texture_layers {
                            if !layer.name.is_empty() {
                                use crate::director::lingo::datum::Shockwave3dObjectRef;
                                items.push_back(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: member_ref.cast_lib,
                                    cast_member: member_ref.cast_member,
                                    object_type: "texture".to_string(),
                                    name: layer.name.clone(),
                                })));
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
            },
            "textureModeList" => {
                // Persist the list so `shader.textureModeList[i] = #wrapPlanar`
                // (which compiles to a generic Datum::List mutation) is visible
                // to sync_shader_texture_lists at render time.
                let existing_ref = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.shader_texture_mode_lists.get(shader_name))
                        .cloned()
                };
                if let Some(list_ref) = existing_ref {
                    return Ok(list_ref);
                }
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
                while items.len() < 8 {
                    items.push_back(player.alloc_datum(Datum::Symbol("none".to_string())));
                }
                let list_ref = player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                ));
                let shader_name_owned = shader_name.to_string();
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                        w3d.runtime_state.shader_texture_mode_lists.insert(shader_name_owned, list_ref.clone());
                    }
                }
                Ok(list_ref)
            },
            "blendFunctionList" => {
                let mut items = VecDeque::new();
                if let Some(s) = shader {
                    for layer in &s.texture_layers {
                        let sym = match layer.blend_func {
                            0 => "replace",
                            1 => "add",
                            3 => "blend",
                            _ => "multiply", // 2=MODULATE, and MODULATE2X/4X → closest
                        };
                        items.push_back(player.alloc_datum(Datum::Symbol(sym.to_string())));
                    }
                }
                while items.len() < 8 {
                    items.push_back(player.alloc_datum(Datum::Symbol("multiply".to_string())));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            },
            "blendSourceList" => {
                let mut items = VecDeque::new();
                if let Some(s) = shader {
                    for layer in &s.texture_layers {
                        let sym = if layer.blend_src == 0 { "alpha" } else { "constant" };
                        items.push_back(player.alloc_datum(Datum::Symbol(sym.to_string())));
                    }
                }
                while items.len() < 8 {
                    items.push_back(player.alloc_datum(Datum::Symbol("constant".to_string())));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            },
            "blendConstantList" => {
                // Persist the list so `shader.blendConstantList[i] = N` (e.g. the 30%
                // override after reflectionMap) is visible to sync_shader_texture_lists
                // at render time. Without persistence the index-set mutated a throwaway
                // list and the layer kept the reflectionMap helper's 50% default.
                let existing_ref = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .and_then(|w3d| w3d.runtime_state.shader_blend_constant_lists.get(shader_name))
                        .cloned()
                };
                if let Some(list_ref) = existing_ref {
                    return Ok(list_ref);
                }
                let mut items = VecDeque::new();
                if let Some(s) = shader {
                    for layer in &s.texture_layers {
                        items.push_back(player.alloc_datum(Datum::Float((layer.blend_const as f64) * 100.0)));
                    }
                }
                while items.len() < 8 {
                    items.push_back(player.alloc_datum(Datum::Float(50.0)));
                }
                let list_ref = player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                ));
                let shader_name_owned = shader_name.to_string();
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                        w3d.runtime_state.shader_blend_constant_lists.insert(shader_name_owned, list_ref.clone());
                    }
                }
                Ok(list_ref)
            },
            "textureRepeatList" => {
                let mut items = VecDeque::new();
                if let Some(s) = shader {
                    for layer in &s.texture_layers {
                        items.push_back(player.alloc_datum(Datum::Int(layer.repeat_s as i32)));
                    }
                }
                while items.len() < 8 {
                    items.push_back(player.alloc_datum(Datum::Int(1)));
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            },
            "textureTransformList" | "wrapTransformList" => {
                Self::get_or_create_texture_transform_list(player, scene, shader_name, &member_ref)
            },
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
            },
            _ => {
                log(&format!("[W3D] shader(\"{}\").{} (stub)", shader_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
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
            .find(|n| n.node_type == W3dNodeType::View && n.name.eq_ignore_ascii_case(camera_name));

        match_ci!(prop, {
            "name" => Ok(player.alloc_datum(Datum::String(camera_name.to_string()))),
            "transform" => {
                // Use the actual W3D node name (e.g. "defaultview") not the sprite property name ("DefaultView")
                // so that node_transform_datums keys match what the renderer looks up via node.name
                let resolved_name = node.map(|n| n.name.as_str()).unwrap_or(camera_name);
                let result = get_persistent_node_transform(player, member_ref, resolved_name);
                let typ = player.get_datum(&result).type_enum();
                log(&format!("[W3D-CAM] camera('{}').transform → type={:?}", resolved_name, typ));
                Ok(result)
            },
            "fieldOfView" | "projectionAngle" => {
                let fov = node.map(|n| n.fov).unwrap_or(30.0);
                Ok(player.alloc_datum(Datum::Float(fov as f64)))
            },
            "nearClipPlane" | "hither" => {
                let v = node.map(|n| n.near_plane).unwrap_or(1.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            },
            "farClipPlane" | "yon" => {
                let v = node.map(|n| n.far_plane).unwrap_or(10000.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            },
            "worldPosition" => {
                // Walk parent chain like model.worldPosition / group.worldPosition.
                // Cameras can be parented (e.g. attached to a vehicle).
                let wp = get_world_position(player, member_ref, camera_name);
                Ok(player.alloc_datum(Datum::Vector(wp)))
            },
            "projection" => Ok(player.alloc_datum(Datum::Symbol("perspective".to_string()))),
            "visible" => Ok(player.alloc_datum(Datum::Int(1))),
            "rect" => {
                // Camera viewport rect in pixel coordinates.
                // Default = the member's defaultRect (full sprite area).
                let r = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.info.default_rect)
                    .unwrap_or((0, 0, 320, 240));
                Ok(player.alloc_datum(Datum::Rect([
                    r.0 as f64, r.1 as f64, r.2 as f64, r.3 as f64
                ], 0)))
            },
            "fog" => {
                // Director: camera.fog returns a Fog object with .near/.far/
                // .enabled/.color/.decayMode props. Model as a Shockwave3dObjectRef
                // with object_type="fog" and the camera name; the per-prop
                // get/set handlers route to the W3D member's runtime_state.fog_*.
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "fog".to_string(),
                    name: camera_name.to_string(),
                })))
            },
            "fog.enabled" => Ok(player.alloc_datum(Datum::Int(0))),
            "fog.near" => Ok(player.alloc_datum(Datum::Float(1.0))),
            "fog.far" => Ok(player.alloc_datum(Datum::Float(1000.0))),
            "fog.color" => {
                Ok(player.alloc_datum(color_to_datum([0.5, 0.5, 0.5, 1.0])))
            },
            "overlay" | "backdrop" => {
                // Return overlay/backdrop list — each item is an overlay object ref.
                // camera_overlays is keyed by lowercased camera name (see addOverlay).
                let is_overlay = prop == "overlay";
                let cam_key = camera_name.to_ascii_lowercase();
                let count = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .map(|w3d| {
                            let map = if is_overlay { &w3d.runtime_state.camera_overlays } else { &w3d.runtime_state.camera_backdrops };
                            map.get(&cam_key).map(|v| v.len()).unwrap_or(0)
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
            },
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
            },
            "colorBuffer" | "colorBuffer.clearAtRender" => {
                // Return a camera-specific colorBuffer ref for .clearAtRender property
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(
                    crate::director::lingo::datum::Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                        object_type: "colorBuffer".to_string(),
                        name: camera_name.to_string(),
                    }
                )))
            },
            _ => {
                log(&format!("[W3D] camera(\"{}\").{} (stub)", camera_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
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

        match_ci!(prop, {
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
            },
            "color" => {
                if let Some(l) = light {
                    Ok(player.alloc_datum(color_to_datum([l.color[0], l.color[1], l.color[2], 1.0])))
                } else {
                    Ok(player.alloc_datum(color_to_datum([1.0, 1.0, 1.0, 1.0])))
                }
            },
            "visible" => {
                let v = light.map(|l| l.enabled).unwrap_or(true);
                Ok(player.alloc_datum(Datum::Int(if v { 1 } else { 0 })))
            },
            "spotAngle" => {
                let v = light.map(|l| l.spot_angle).unwrap_or(90.0);
                Ok(player.alloc_datum(Datum::Float(v as f64)))
            },
            "transform" => {
                Ok(get_persistent_node_transform(player, member_ref, light_name))
            },
            _ => {
                log(&format!("[W3D] light(\"{}\").{} (stub)", light_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
    }

    // ─── Generic node property getters ───

    fn get_node_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        node_name: &str,
        prop: &str,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        match_ci!(prop, {
            "name" => Ok(player.alloc_datum(Datum::String(node_name.to_string()))),
            "parent" => {
                // VOID while detached (removeFromWorld) — see get_model_prop.
                let detached = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .map(|w3d| w3d.runtime_state.detached_nodes.iter()
                        .any(|d| d.eq_ignore_ascii_case(node_name)))
                    .unwrap_or(false);
                if detached {
                    Ok(player.alloc_datum(Datum::Void))
                } else {
                    // Return the parent NODE object (not its name) so chained access
                    // like `node.parent.getWorldTransform()` works. World root = group.
                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                    use crate::director::chunks::w3d::types::W3dNodeType;
                    let pname = scene.nodes.iter().find(|n| n.name == node_name)
                        .map(|n| n.parent_name.clone())
                        .unwrap_or_default();
                    if pname.is_empty() {
                        Ok(player.alloc_datum(Datum::Void))
                    } else {
                        let obj_type = scene.nodes.iter()
                            .find(|pn| pn.name.eq_ignore_ascii_case(&pname))
                            .map(|pn| match pn.node_type {
                                W3dNodeType::View => "camera",
                                W3dNodeType::Light => "light",
                                W3dNodeType::Group => "group",
                                _ => "model",
                            })
                            .unwrap_or("group");
                        Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                            object_type: obj_type.to_string(), name: pname,
                        })))
                    }
                }
            },
            "transform" => {
                Ok(get_persistent_node_transform(player, member_ref, node_name))
            },
            "worldPosition" => {
                let wp = get_world_position(player, member_ref, node_name);
                Ok(player.alloc_datum(Datum::Vector(wp)))
            },
            _ => {
                log(&format!("[W3D] group(\"{}\").{} (stub)", node_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
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

        match_ci!(prop, {
            "name" => Ok(player.alloc_datum(Datum::String(resource_name.to_string()))),
            "type" => {
                // Real primitive type (#plane/#box/#sphere/#cylinder); fall back to
                // #fromFile for loaded/mesh resources. Was hardcoded to "fromFile".
                let t = res.and_then(|r| r.primitive_type.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "fromFile".to_string());
                Ok(player.alloc_datum(Datum::Symbol(t)))
            },
            "topCap" => Ok(player.alloc_datum(Datum::Int(
                res.map(|r| if r.primitive_top_cap { 1 } else { 0 }).unwrap_or(0)))),
            "bottomCap" => Ok(player.alloc_datum(Datum::Int(
                res.map(|r| if r.primitive_bottom_cap { 1 } else { 0 }).unwrap_or(0)))),
            "topRadius" => Ok(player.alloc_datum(Datum::Float(
                res.map(|r| r.primitive_top_radius as f64).unwrap_or(0.0)))),
            "bottomRadius" | "radius" => Ok(player.alloc_datum(Datum::Float(
                res.map(|r| r.primitive_radius as f64).unwrap_or(0.0)))),
            "height" => Ok(player.alloc_datum(Datum::Float(
                res.map(|r| r.primitive_height as f64).unwrap_or(0.0)))),
            "width" => Ok(player.alloc_datum(Datum::Float(
                res.map(|r| r.primitive_width as f64).unwrap_or(0.0)))),
            "length" => Ok(player.alloc_datum(Datum::Float(
                res.map(|r| r.primitive_length as f64).unwrap_or(0.0)))),
            "startAngle" => Ok(player.alloc_datum(Datum::Float(
                res.map(|r| r.primitive_start_angle as f64).unwrap_or(0.0)))),
            "endAngle" => Ok(player.alloc_datum(Datum::Float(
                res.map(|r| r.primitive_end_angle as f64).unwrap_or(360.0)))),
            "resolution" => Ok(player.alloc_datum(Datum::Int(
                res.map(|r| r.primitive_resolution as i32).unwrap_or(0)))),
            "vertexList" => {
                // For meshes built via newMesh()+build(), the positions live
                // in scene.clod_meshes keyed by the resource name. Director
                // exposes this list as `modelResource(name).vertexList`.
                let mut items = VecDeque::new();
                if let Some(meshes) = scene.clod_meshes.get(resource_name) {
                    for mesh in meshes {
                        for pos in &mesh.positions {
                            items.push_back(player.alloc_datum(Datum::Vector(
                                [pos[0] as f64, pos[1] as f64, pos[2] as f64]
                            )));
                        }
                    }
                }
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List, items, false,
                )))
            },
            "face.count" | "faceCount" => {
                let count: u32 = res.map(|r| r.mesh_infos.iter().map(|m| m.num_faces).sum()).unwrap_or(0);
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            },
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
                    let item_count = items.len();
                    let list_ref = player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List, items, false,
                    ));
                    let face_key_owned = face_key.clone();
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.shader_texture_lists.insert(face_key_owned, list_ref.clone());
                        }
                    }
                    let _ = item_count;
                    Ok(list_ref)
                }
            },
            "lod" => {
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "lod".to_string(),
                    name: resource_name.to_string(),
                })))
            },
            "sds" => {
                // Subdivision Surface modifier — return object with depth/tension properties
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "sds".to_string(),
                    name: resource_name.to_string(),
                })))
            },
            "emitter" => {
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "emitter".to_string(),
                    name: resource_name.to_string(),
                })))
            },
            // Particle system resource properties — range objects with #start and #end
            // #particle range objects — return a ref so `resource.colorRange.start = X`
            // routes to set_prop (object_type colorRange/sizeRange/blendRange). The set
            // persists into the ParticleSystemState; a thrown-away PropList wouldn't.
            "colorRange" | "sizeRange" | "blendRange" => {
                use crate::director::lingo::datum::Shockwave3dObjectRef;
                Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: prop.to_string(), // "colorRange" | "sizeRange" | "blendRange"
                    name: resource_name.to_string(),
                })))
            },
            "lifetime" => Ok(player.alloc_datum(Datum::Int(1000))),
            "gravity" => Ok(player.alloc_datum(Datum::Vector([0.0, -9.8, 0.0]))),
            "wind" => Ok(player.alloc_datum(Datum::Vector([0.0, 0.0, 0.0]))),
            "drag" => Ok(player.alloc_datum(Datum::Float(0.0))),
            // Accept common resource properties silently
            "width" | "length" | "lengthVertices" | "widthVertices"
            | "height" | "numVertices" | "numFaces" => {
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            _ => {
                log(&format!("[W3D] modelResource(\"{}\").{} (stub)", resource_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
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

        match_ci!(prop, {
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
            },
            "face" | "face.count" | "faceCount" => {
                Ok(player.alloc_datum(Datum::Int(face_count as i32)))
            },
            _ => {
                log(&format!("[W3D] meshDeform(\"{}\").{} (stub)", model_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
    }

    // ─── Motion property getters ───

    fn get_motion_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        motion_name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let motion = scene.motions.iter().find(|m| m.name == motion_name);

        match_ci!(prop, {
            "name" => Ok(player.alloc_datum(Datum::String(motion_name.to_string()))),
            "duration" => {
                let dur = motion.map(|m| m.duration()).unwrap_or(0.0);
                Ok(player.alloc_datum(Datum::Float((dur * 1000.0) as f64))) // ms
            },
            "type" => Ok(player.alloc_datum(Datum::Symbol("bones".to_string()))),
            _ => {
                log(&format!("[W3D] motion(\"{}\").{} (stub)", motion_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
    }

    // ─── Texture property getters ───

    fn get_texture_prop(
        player: &mut crate::player::DirPlayer,
        scene: &W3dScene,
        texture_name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        match_ci!(prop, {
            "name" => Ok(player.alloc_datum(Datum::String(texture_name.to_string()))),
            "type" => Ok(player.alloc_datum(Datum::Symbol("fromFile".to_string()))),
            "renderFormat" => Ok(player.alloc_datum(Datum::Symbol("rgba8880".to_string()))),
            "quality" => Ok(player.alloc_datum(Datum::Symbol("default".to_string()))),
            "width" | "height" => {
                // Look up actual texture dimensions from scene data
                let dim = get_texture_dimensions(scene, texture_name);
                let val = if prop == "width" { dim.0 } else { dim.1 };
                Ok(player.alloc_datum(Datum::Int(val as i32)))
            },
            "nearFiltering" => Ok(player.alloc_datum(Datum::Int(1))),
            _ => {
                log(&format!("[W3D] texture(\"{}\").{} (stub)", texture_name, prop));
                Ok(player.alloc_datum(Datum::Void))
            },
        })
    }
}

/// Convert an RGBA color array to a Datum::ColorRef
/// Mirror of [`scene3d::setup_skinning_for_resource`]'s `t` computation.
/// Used by bone-transform getters so the matrix returned to Lingo matches
/// what the skinning shader is using for the same model.
fn compute_motion_t(
    motion: Option<&crate::director::chunks::w3d::types::W3dMotion>,
    rs: &crate::player::cast_member::Shockwave3dRuntimeState,
) -> f32 {
    let Some(motion) = motion else { return 0.0; };
    let duration = motion.duration();
    let end_time = rs.animation_end_time;
    let start_time = rs.animation_start_time;
    let eff_end = if end_time >= 0.0 { end_time.min(duration) } else { duration };
    let eff_start = start_time.min(eff_end);
    let range = eff_end - eff_start;
    if range <= 0.0 { return 0.0; }
    let time = rs.animation_time;
    if rs.animation_loop {
        eff_start + ((time - eff_start) % range + range) % range
    } else {
        time.clamp(eff_start, eff_end)
    }
}

/// Per-model variant of `compute_motion_t` reading a model's own
/// [`BonesPlayerState`] (each model animates independently).
fn compute_motion_t_bp(
    motion: Option<&crate::director::chunks::w3d::types::W3dMotion>,
    bp: &crate::player::cast_member::BonesPlayerState,
) -> f32 {
    let Some(motion) = motion else { return 0.0; };
    let duration = motion.duration();
    let end_time = bp.animation_end_time;
    let start_time = bp.animation_start_time;
    let eff_end = if end_time >= 0.0 { end_time.min(duration) } else { duration };
    let eff_start = start_time.min(eff_end);
    let range = eff_end - eff_start;
    if range <= 0.0 { return 0.0; }
    let time = bp.animation_time;
    if bp.animation_loop {
        eff_start + ((time - eff_start) % range + range) % range
    } else {
        time.clamp(eff_start, eff_end)
    }
}

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

/// Read the optional trailing `relativeTo` symbol of translate/rotate.
/// Per the Director spec a NODE reference defaults to `#self` (the node's own
/// local axes); `#world`/`#parent` apply the change in the parent/world frame.
/// Returns true only for an explicit `#world`/`#parent`.
fn args_relative_to_world(player: &crate::player::DirPlayer, args: &[crate::player::DatumRef]) -> bool {
    let sym_ref = if args.len() >= 4 {
        Some(&args[3]) // translate(x, y, z, relativeTo)
    } else if args.len() == 2 {
        Some(&args[1]) // translate(vector, relativeTo)
    } else {
        None
    };
    if let Some(r) = sym_ref {
        if let Datum::Symbol(s) = player.get_datum(r) {
            let s = s.to_ascii_lowercase();
            return s == "world" || s == "parent";
        }
    }
    false
}

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

/// Get the member's default_rect dimensions (original content size).
/// IFX uses these for distToProj and pixelAspect in the picking ray.
fn get_member_default_rect_size(
    player: &crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
) -> (f32, f32) {
    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d() {
            let r = &w3d.info.default_rect;
            let w = (r.2 - r.0) as f32;
            let h = (r.3 - r.1) as f32;
            if w > 0.0 && h > 0.0 {
                return (w, h);
            }
        }
    }
    (320.0, 240.0)
}

/// Resolve the skeleton that belongs to a model node, by its resource name — the same
/// match `setup_skinning_for_resource` uses for the renderer. Falls back to a skeleton
/// named after the model itself, then to the first skeleton (single-skeleton scenes).
///
/// Fixes `bone[]` / `bone.count` / `resource.bone.count` for CLONED skinned models:
/// Rasterwerks spawns many bots, each a clone with its OWN cloned skeleton, so the old
/// `scene.skeletons.first()` returned the wrong (or original) skeleton and bone access
/// failed (`resource.bone.count = 0`). `model_or_resource_name` may be a model NODE name
/// (bonesPlayer.bone…) or a resource name (resource.bone…); both resolve here.
fn find_skeleton_for_model<'a>(
    scene: &'a crate::director::chunks::w3d::types::W3dScene,
    model_or_resource_name: &str,
) -> Option<&'a crate::director::chunks::w3d::types::W3dSkeleton> {
    // Model node → its resource → skeleton named after that resource.
    if let Some(node) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(model_or_resource_name)) {
        let res = if !node.model_resource_name.is_empty() {
            node.model_resource_name.as_str()
        } else {
            node.resource_name.as_str()
        };
        if !res.is_empty() {
            if let Some(sk) = scene.skeletons.iter().find(|s| s.name.eq_ignore_ascii_case(res)) {
                return Some(sk);
            }
        }
    }
    // Direct name match (resource.bone… passes the resource name, which IS the skeleton name).
    if let Some(sk) = scene.skeletons.iter().find(|s| s.name.eq_ignore_ascii_case(model_or_resource_name)) {
        return Some(sk);
    }
    scene.skeletons.first()
}

fn get_node_transform(
    player: &crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f32; 16] {
    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d() {
            // Check runtime override first (exact match, then case-insensitive fallback)
            if let Some(m) = w3d.runtime_state.node_transforms.get(node_name) {
                return *m;
            }
            // Case-insensitive fallback for runtime transforms (Director is case-insensitive)
            for (key, val) in &w3d.runtime_state.node_transforms {
                if key.eq_ignore_ascii_case(node_name) {
                    return *val;
                }
            }
            // Fall back to parsed scene (case-insensitive)
            if let Some(scene) = &w3d.parsed_scene {
                if let Some(node) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(node_name)) {
                    return node.transform;
                }
            }
        }
    }
    IDENTITY
}

/// Like get_node_transform, but reads the LIVE persistent transform datum first.
/// model.transform.position/rotation mutate that datum immediately, while the
/// node_transforms cache is only flushed once per frame (sync_persistent_transforms).
/// Mid-script callers (e.g. addChild #preserveWorld, run right after the position is
/// set) must see the live value, not the stale cache.
/// World matrix of a node: its live local transform composed with its ancestors'.
///
/// Mirrors the walk in `get_world_position` (live transforms, case-insensitive
/// parent lookup, depth-capped against cycles) but keeps the full matrix.
fn node_world_matrix(
    player: &crate::player::DirPlayer,
    scene: &W3dScene,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f32; 16] {
    let mut result = get_node_transform_live(player, member_ref, node_name);
    let mut current_parent = scene
        .nodes
        .iter()
        .find(|n| n.name.eq_ignore_ascii_case(node_name))
        .map(|n| n.parent_name.clone())
        .unwrap_or_default();
    for _ in 0..20 {
        if current_parent.is_empty() || current_parent.eq_ignore_ascii_case("World") {
            break;
        }
        match scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&current_parent)) {
            Some(pn) => {
                let pt = get_node_transform_live(player, member_ref, &pn.name);
                result = mat4_mul_f32(&pt, &result);
                current_parent = pn.parent_name.clone();
            }
            None => break,
        }
    }
    result
}

/// `model.boundingSphere` — [center, radius] in world space, covering the node
/// and all its descendants (Director 11.5 Scripting Dictionary, `boundingSphere`).
///
/// Vertices come from the node's model resource (CLOD meshes, or a raw mesh of
/// the same name), transformed to world space by the node's world matrix. The
/// center is the midpoint of the world-space AABB and the radius the greatest
/// distance from it to any vertex — a sphere that provably contains every point,
/// which is what the property promises.
///
/// A node with no geometry of its own (a group, light, camera, or an empty
/// parent) still contributes its own origin, so a childless one yields
/// [its world position, 0.0] rather than collapsing to the scene origin.
fn model_bounding_sphere(
    player: &crate::player::DirPlayer,
    scene: &W3dScene,
    model_name: &str,
    member_ref: &crate::player::cast_lib::CastMemberRef,
) -> ([f64; 3], f64) {
    // The node plus every descendant (case-insensitive parent match, as elsewhere).
    let mut names: Vec<String> = vec![model_name.to_string()];
    let mut stack = vec![model_name.to_string()];
    while let Some(parent) = stack.pop() {
        for n in &scene.nodes {
            if n.parent_name.eq_ignore_ascii_case(&parent)
                && !names.iter().any(|e| e.eq_ignore_ascii_case(&n.name))
            {
                names.push(n.name.clone());
                stack.push(n.name.clone());
            }
        }
    }

    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    let mut points: Vec<[f64; 3]> = Vec::new();

    for name in &names {
        let world = node_world_matrix(player, scene, member_ref, name);
        let node = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(name));

        // Collect this node's local-space vertices.
        let mut local: Vec<[f32; 3]> = Vec::new();
        if let Some(n) = node {
            let key = if !n.model_resource_name.is_empty() {
                n.model_resource_name.clone()
            } else {
                n.resource_name.clone()
            };
            if let Some(meshes) = scene.clod_meshes.get(&key) {
                for mesh in meshes {
                    local.extend_from_slice(&mesh.positions);
                }
            }
            if local.is_empty() {
                if let Some(raw) = scene.raw_meshes.iter().find(|m| m.name.eq_ignore_ascii_case(&key)) {
                    local.extend_from_slice(&raw.positions);
                }
            }
        }
        // No geometry: contribute the node's own origin so groups/lights still
        // report a sensible centre.
        if local.is_empty() {
            local.push([0.0, 0.0, 0.0]);
        }

        for v in &local {
            let (x, y, z) = (v[0] as f64, v[1] as f64, v[2] as f64);
            let w = [
                world[0] as f64 * x + world[4] as f64 * y + world[8] as f64 * z + world[12] as f64,
                world[1] as f64 * x + world[5] as f64 * y + world[9] as f64 * z + world[13] as f64,
                world[2] as f64 * x + world[6] as f64 * y + world[10] as f64 * z + world[14] as f64,
            ];
            for i in 0..3 {
                if w[i] < min[i] { min[i] = w[i]; }
                if w[i] > max[i] { max[i] = w[i]; }
            }
            points.push(w);
        }
    }

    if points.is_empty() {
        return ([0.0, 0.0, 0.0], 0.0);
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let mut radius_sq = 0.0f64;
    for p in &points {
        let d = (p[0] - center[0]).powi(2)
            + (p[1] - center[1]).powi(2)
            + (p[2] - center[2]).powi(2);
        if d > radius_sq {
            radius_sq = d;
        }
    }
    (center, radius_sq.sqrt())
}

fn get_node_transform_live(
    player: &crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f32; 16] {
    let dr_opt = player.movie.cast_manager.find_member_by_ref(member_ref)
        .and_then(|m| m.member_type.as_shockwave3d())
        .and_then(|w3d| {
            w3d.runtime_state.node_transform_datums.get(node_name).cloned()
                .or_else(|| w3d.runtime_state.node_transform_datums.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(node_name))
                    .map(|(_, v)| v.clone()))
        });
    if let Some(dr) = dr_opt {
        if let Datum::Transform3d(m) = player.get_datum(&dr) {
            let mut out = [0.0f32; 16];
            for i in 0..16 { out[i] = m[i] as f32; }
            return out;
        }
    }
    get_node_transform(player, member_ref, node_name)
}

/// Get the accumulated WORLD position for a node by walking the parent chain.
/// This matches Director's `model.worldPosition` / `group.worldPosition` behavior.
fn get_world_position(
    player: &crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f64; 3] {
    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d() {
            if let Some(ref scene) = w3d.parsed_scene {
                if let Some(node) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(node_name)) {
                    // Use the LIVE transform (persistent datum), not the once-per-frame
                    // node_transforms cache: a script may read `model.worldPosition`
                    // immediately after `model.transform.position = v` (e.g. frog01 does
                    // `snakeBox.position = snakedown.worldPosition + offset` right after
                    // positioning snakedown). Reading the stale cache returns the model's
                    // load-time (clone source) transform, which then poisons the
                    // subsequent addChild #preserveWorld math. Mirrors node_world_transform.
                    let local = get_node_transform_live(player, member_ref, &node.name);
                    let mut result = local;
                    let mut current_parent = node.parent_name.clone();
                    for _ in 0..20 {
                        if current_parent.is_empty() || current_parent.eq_ignore_ascii_case("World") { break; }
                        if let Some(pn) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&current_parent)) {
                            let pt = get_node_transform_live(player, member_ref, &pn.name);
                            result = mat4_mul_f32(&pt, &result);
                            current_parent = pn.parent_name.clone();
                        } else { break; }
                    }
                    return [result[12] as f64, result[13] as f64, result[14] as f64];
                }
            }
        }
    }
    let m = get_node_transform(player, member_ref, node_name);
    [m[12] as f64, m[13] as f64, m[14] as f64]
}

/// Find the canonical key for a node name in the node_transforms HashMap.
/// Returns the existing key if a case-insensitive match exists, otherwise the input name.
fn canonical_node_key(
    player: &crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> String {
    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d() {
            // Check exact match first
            if w3d.runtime_state.node_transforms.contains_key(node_name) {
                return node_name.to_string();
            }
            // Case-insensitive fallback: use the existing key
            for key in w3d.runtime_state.node_transforms.keys() {
                if key.eq_ignore_ascii_case(node_name) {
                    return key.clone();
                }
            }
            // Also check persistent transform datums
            for key in w3d.runtime_state.node_transform_datums.keys() {
                if key.eq_ignore_ascii_case(node_name) {
                    return key.clone();
                }
            }
        }
    }
    node_name.to_string()
}

fn get_or_init_node_transform(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f32; 16] {
    let key = canonical_node_key(player, member_ref, node_name);
    let current = get_node_transform(player, member_ref, &key);

    // Ensure it's in the runtime overrides
    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
            w3d.runtime_state.node_transforms.entry(key).or_insert(current);
        }
    }
    current
}

pub fn set_node_transform(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    m: [f32; 16],
) {
    // Reject transforms containing NaN - they corrupt the scene
    if m.iter().any(|v| !v.is_finite()) {
        return;
    }

    // Normalize key to prevent duplicate entries with different case
    let key = canonical_node_key(player, member_ref, node_name);

    // Get the persistent datum ref (case-insensitive lookup)
    let persistent_ref = {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        member.and_then(|m| m.member_type.as_shockwave3d())
            .and_then(|w3d| {
                w3d.runtime_state.node_transform_datums.get(&key)
                    .or_else(|| {
                        w3d.runtime_state.node_transform_datums.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                            .map(|(_, v)| v)
                    })
            })
            .cloned()
    };

    // Update the persistent datum if it exists
    if let Some(datum_ref) = &persistent_ref {
        let m64: [f64; 16] = m.map(|v| v as f64);
        *player.get_datum_mut(datum_ref) = Datum::Transform3d(m64);
    }

    // Update node_transforms using canonical key
    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
            w3d.runtime_state.node_transforms.insert(key, m);
        }
    }
}

/// World-relative transform of a node, accumulated up its parent chain (same
/// convention as the getWorldTransform handler). Used by addChild #preserveWorld.
fn node_world_transform(
    player: &crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> [f32; 16] {
    let mut result = get_node_transform_live(player, member_ref, node_name);
    let mut current = node_name.to_string();
    for _ in 0..20 {
        let parent_name = {
            player.movie.cast_manager.find_member_by_ref(member_ref)
                .and_then(|m| m.member_type.as_shockwave3d())
                .and_then(|w| w.parsed_scene.as_ref())
                .and_then(|s| s.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&current)))
                .map(|n| n.parent_name.clone())
                .unwrap_or_default()
        };
        if parent_name.is_empty() || parent_name.eq_ignore_ascii_case("World") { break; }
        let pt = get_node_transform_live(player, member_ref, &parent_name);
        result = mat4_mul_f32(&pt, &result);
        current = parent_name;
    }
    result
}

/// Get or create a persistent Transform3d DatumRef for a node.
/// Returns the same DatumRef on subsequent calls so that in-place mutations persist.
fn get_persistent_node_transform(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> DatumRef {
    if node_name.contains("overlay") {
        static PNT_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if PNT_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
            log(&format!("[PNT] get_persistent_node_transform('{}')", node_name));
        }
    }
    let key = canonical_node_key(player, member_ref, node_name);

    // Check if persistent datum already exists (case-insensitive)
    let existing = {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        member.and_then(|m| m.member_type.as_shockwave3d())
            .and_then(|w3d| {
                w3d.runtime_state.node_transform_datums.get(&key)
                    .or_else(|| {
                        w3d.runtime_state.node_transform_datums.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                            .map(|(_, v)| v)
                    })
            })
            .cloned()
    };
    if let Some(datum_ref) = existing {
        return datum_ref;
    }

    // Create new persistent datum from current transform
    let m = get_node_transform(player, member_ref, &key);
    let m64: [f64; 16] = m.map(|v| v as f64);
    let datum_ref = player.alloc_datum(Datum::Transform3d(m64));

    // Store in runtime state using canonical key
    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
            w3d.runtime_state.node_transform_datums.insert(key, datum_ref.clone());
        }
    }
    datum_ref
}

/// Returns a `DatumRef` to the userData PropList for the named 3D node.
/// Lazy-allocates an empty PropList on first access and caches it on the
/// cast member's `runtime_state.user_data` so subsequent reads return the
/// same ref — required because Lingo scripts mutate userData in place via
/// `setaProp` / `addProp` / `deleteProp`.
fn get_or_create_node_user_data(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
) -> DatumRef {
    let key = canonical_node_key(player, member_ref, node_name);

    // Try to fetch cached ref first (case-insensitive lookup, mirroring
    // node_transform_datums above).
    let existing = {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        member.and_then(|m| m.member_type.as_shockwave3d())
            .and_then(|w3d| {
                w3d.runtime_state.user_data.get(&key)
                    .or_else(|| {
                        w3d.runtime_state.user_data.iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                            .map(|(_, v)| v)
                    })
            })
            .cloned()
    };
    if let Some(datum_ref) = existing {
        return datum_ref;
    }

    // First access — allocate empty PropList and stash on the runtime state.
    let datum_ref = player.alloc_datum(Datum::PropList(VecDeque::new(), false));
    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
            w3d.runtime_state.user_data.insert(key, datum_ref.clone());
        }
    }
    datum_ref
}

/// Sync all persistent transform datums back to node_transforms for the renderer.
/// Call this before each render frame.
pub fn sync_persistent_transforms(player: &mut crate::player::DirPlayer) {
    // Only sync Transform3d datums that were mutated in-place (dirty)
    let dirty_ids = super::transform3d::take_dirty_ids();
    if dirty_ids.is_empty() { return; }

    // Collect entries for dirty datums only
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
        let is_dirty = dirty_ids.contains(&datum_ref.unwrap());
        if !is_dirty { continue; } // Only sync dirty datums
        if let Datum::Transform3d(m64) = player.get_datum(&datum_ref) {
            let m32: [f32; 16] = m64.map(|v| v as f32);
            if m32.iter().any(|v| !v.is_finite()) { continue; }
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
/// Invert the 2×2 UV-linear (scale/rotation) part of a texture transform and
/// cast to f32. Director's `textureTransform.scale` is the texture's APPARENT
/// size (0.125 = the texture tiles 8×), the inverse of the coordinate scale the
/// shader multiplies onto the UVs. Translation (m12,m13) is preserved so a
/// scrolling `textureTransform.position` still works.
fn invert_tex_uv_scale(m: &[f64; 16]) -> [f32; 16] {
    let (a, b, c, d) = (m[0], m[1], m[4], m[5]); // u' = a*u + c*v, v' = b*u + d*v
    let det = a * d - c * b;
    let mut out = [0.0f32; 16];
    for i in 0..16 {
        out[i] = m[i] as f32;
    }
    if det.abs() >= 1e-8 {
        let inv = 1.0 / det;
        out[0] = (d * inv) as f32;
        out[1] = (-b * inv) as f32;
        out[4] = (-c * inv) as f32;
        out[5] = (a * inv) as f32;
    }
    out
}

pub fn sync_shader_texture_lists(player: &mut crate::player::DirPlayer) {
    // Collect (cast_lib, member_num, shader_name, list_ref) tuples
    let mut entries: Vec<(i32, u32, String, DatumRef)> = Vec::new();
    let mut mode_entries: Vec<(i32, u32, String, DatumRef)> = Vec::new();
    let mut blend_entries: Vec<(i32, u32, String, DatumRef)> = Vec::new();
    let mut transform_entries: Vec<(i32, u32, String, DatumRef)> = Vec::new();
    for cast in &player.movie.cast_manager.casts {
        for (member_num, member) in &cast.members {
            if let Some(w3d) = member.member_type.as_shockwave3d() {
                for (shader_name, list_ref) in &w3d.runtime_state.shader_texture_lists {
                    entries.push((cast.number as i32, *member_num, shader_name.clone(), list_ref.clone()));
                }
                for (shader_name, list_ref) in &w3d.runtime_state.shader_texture_mode_lists {
                    mode_entries.push((cast.number as i32, *member_num, shader_name.clone(), list_ref.clone()));
                }
                for (shader_name, list_ref) in &w3d.runtime_state.shader_blend_constant_lists {
                    blend_entries.push((cast.number as i32, *member_num, shader_name.clone(), list_ref.clone()));
                }
                for (shader_name, list_ref) in &w3d.runtime_state.shader_texture_transform_lists {
                    transform_entries.push((cast.number as i32, *member_num, shader_name.clone(), list_ref.clone()));
                }
            }
        }
    }

    // Sync blend constants (0..100) back to shader.texture_layers[].blend_const (0..1).
    for (cast_lib, cast_member, shader_name, list_ref) in blend_entries {
        let consts: Vec<f32> = if let Datum::List(_, items, _) = player.get_datum(&list_ref) {
            items.iter().map(|item_ref| {
                (player.get_datum(item_ref).to_float().unwrap_or(50.0) as f32 / 100.0).clamp(0.0, 1.0)
            }).collect()
        } else {
            continue;
        };
        let member_ref = CastMemberRef { cast_lib, cast_member: cast_member as i32 };
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                if let Some(scene) = w3d.scene_mut() {
                    if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == shader_name) {
                        use crate::director::chunks::w3d::types::W3dTextureLayer;
                        while shader.texture_layers.len() < consts.len() {
                            shader.texture_layers.push(W3dTextureLayer::default());
                        }
                        for (i, c) in consts.iter().enumerate() {
                            shader.texture_layers[i].blend_const = *c;
                        }
                    }
                }
            }
        }
    }

    // Sync texture modes (#wrapPlanar etc.) back to shader.texture_layers[].tex_mode.
    // Done before the texture-name sync so layer slots already exist when names land.
    for (cast_lib, cast_member, shader_name, list_ref) in mode_entries {
        let modes: Vec<u8> = if let Datum::List(_, items, _) = player.get_datum(&list_ref) {
            items.iter().map(|item_ref| {
                match player.get_datum(item_ref) {
                    Datum::Symbol(s) => match s.to_ascii_lowercase().as_str() {
                        "none" => 0u8,
                        "reflection" => 4,
                        "wrapplanar" => 5,
                        "specular" => 6,
                        _ => 0,
                    },
                    Datum::Int(i) => *i as u8,
                    _ => 0,
                }
            }).collect()
        } else {
            continue;
        };

        let member_ref = CastMemberRef { cast_lib, cast_member: cast_member as i32 };
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                if let Some(scene) = w3d.scene_mut() {
                    if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == shader_name) {
                        use crate::director::chunks::w3d::types::W3dTextureLayer;
                        while shader.texture_layers.len() < modes.len() {
                            shader.texture_layers.push(W3dTextureLayer::default());
                        }
                        for (i, mode) in modes.iter().enumerate() {
                            shader.texture_layers[i].tex_mode = *mode;
                        }
                    }
                }
            }
        }
    }

    // Sync textureTransformList → shader.texture_layers[].tex_transform. Runtime
    // shaders set shader.textureTransform.scale on a Transform3d datum; the
    // renderer only reads texture_layers[].tex_transform, so without this the
    // transform is dropped (SweeTarts' skybox cloud tile mapped once around the
    // cylinder → blurry; runtime edits had no visible effect). Scale is inverted
    // to a tile factor (see invert_tex_uv_scale).
    for (cast_lib, cast_member, shader_name, list_ref) in transform_entries {
        let mats: Vec<[f32; 16]> = if let Datum::List(_, items, _) = player.get_datum(&list_ref) {
            items.iter().map(|item_ref| match player.get_datum(item_ref) {
                Datum::Transform3d(m) => invert_tex_uv_scale(m),
                _ => [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
            }).collect()
        } else {
            continue;
        };
        let member_ref = CastMemberRef { cast_lib, cast_member: cast_member as i32 };
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                if let Some(scene) = w3d.scene_mut() {
                    if let Some(shader) = scene.shaders.iter_mut().find(|s| s.name == shader_name) {
                        use crate::director::chunks::w3d::types::W3dTextureLayer;
                        while shader.texture_layers.len() < mats.len() {
                            shader.texture_layers.push(W3dTextureLayer::default());
                        }
                        for (i, m) in mats.iter().enumerate() {
                            shader.texture_layers[i].tex_transform = *m;
                        }
                    }
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
                        let prev_len = shader.texture_layers.len();
                        while shader.texture_layers.len() < tex_names.len() {
                            shader.texture_layers.push(W3dTextureLayer::default());
                        }
                        // Only sync names — do NOT overwrite blend_func/blend_src
                        // which may have been set by Lingo blendFunctionList assignments.
                        // Empty names are also significant because textureList[n] = VOID
                        // clears inherited layers on cloned shaders.
                        for (i, name) in tex_names.iter().enumerate() {
                            shader.texture_layers[i].name = name.clone();
                        }
                        // Log when shadow/lightmap layers are synced
                        let non_empty: Vec<String> = tex_names.iter().filter(|n| !n.is_empty()).cloned().collect();
                        if non_empty.len() > 1 {
                            let blend_funcs: Vec<u8> = shader.texture_layers.iter().map(|l| l.blend_func).collect();
                            debug!(
                                "[W3D-SYNC] shader=\"{}\" layers={} (was {}) textures={:?} blend_funcs={:?}",
                                shader_name, shader.texture_layers.len(), prev_len, non_empty, blend_funcs
                            );
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
    world_relative: bool,
) {
    // Flush any pending persistent Transform3d mutations into node_transforms
    // first — otherwise a prior `transform.position = v` on the cached datum is
    // invisible to get_or_init_node_transform (which reads node_transforms and
    // the parsed scene), and our subsequent set_node_transform writes back the
    // stale position, silently dropping the Lingo write. Mirrors apply_point_at.
    sync_persistent_transforms(player);
    let mut m = get_or_init_node_transform(player, member_ref, node_name);
    if world_relative {
        // #world / #parent: increments are in the (parent-space) position frame.
        m[12] += dx;
        m[13] += dy;
        m[14] += dz;
    } else {
        // #self (the node-reference default): increments run along the node's own
        // local axes, i.e. the rotation columns of its transform. Pacman's death
        // slide relies on this — rz=90 puts local +X along world +Y, so
        // translate(0.5,0,0) lifts him up (worldPosition.y climbs to the respawn
        // threshold) instead of sliding sideways.
        //
        // The axes are the UNIT rotation directions, NOT scaled by the node's own
        // scale: Director 11.5 dict (translate/#self) moves "x, y, z units along the
        // [local] axes". A scaled node must still move the full distance — frog01's
        // snake is cloned at scale 0.11 and animated with translate(0,±5,0); using the
        // scaled columns moved it 0.55 instead of 5 and broke its worldPosition.y
        // thresholds, and the logs (scale = log length) drifted at the wrong speed.
        let sx = (m[0]*m[0] + m[1]*m[1] + m[2]*m[2]).sqrt();
        let sy = (m[4]*m[4] + m[5]*m[5] + m[6]*m[6]).sqrt();
        let sz = (m[8]*m[8] + m[9]*m[9] + m[10]*m[10]).sqrt();
        let sx = if sx > 1e-8 { sx } else { 1.0 };
        let sy = if sy > 1e-8 { sy } else { 1.0 };
        let sz = if sz > 1e-8 { sz } else { 1.0 };
        m[12] += (m[0]/sx) * dx + (m[4]/sy) * dy + (m[8]/sz) * dz;
        m[13] += (m[1]/sx) * dx + (m[5]/sy) * dy + (m[9]/sz) * dz;
        m[14] += (m[2]/sx) * dx + (m[6]/sy) * dy + (m[10]/sz) * dz;
    }
    set_node_transform(player, member_ref, node_name, m);
}

fn apply_rotation(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    rx_deg: f32, ry_deg: f32, rz_deg: f32,
    world_relative: bool,
) {
    // See apply_translation comment — same flush requirement.
    sync_persistent_transforms(player);
    let m = get_or_init_node_transform(player, member_ref, node_name);
    // Director uses left-handed coordinates where Y rotation is opposite to OpenGL's
    // right-handed convention, so negate Y.
    let rot = euler_to_matrix_f32(rx_deg, -ry_deg, rz_deg);
    let mut result = if world_relative {
        // #world / #parent: compose in the world/parent frame (rot · R).
        mat4_mul_f32(&rot, &m)
    } else {
        // #self (the node-reference default): rotate about the node's own axes.
        //
        // Director PRESERVES the per-axis scale when the basis is a clean rotation×scale
        // (orthogonal columns): it composes the delta into the ROTATION part and re-applies
        // the scale — M' = (R·R_delta)·S. Rasterwerks' Z-up base map un-rotates m_si_fi_7
        // with rotate(-origRot.x,0,0); its grille (clean basis, scale (1.196,1.0,1.196))
        // must keep that scale, but the naive M·R_delta (=R·S·R_delta) put it on the wrong
        // axes → (1.196,1.196,1.0), too shallow, so it sat in front of its pipe. This mirrors
        // the existing transform.rotation SETTER (transform3d.rs ~126: R(v)·diag(sx,sy,sz)).
        //
        // An already-SHEARED basis (non-orthogonal columns — frog01's skeletal limbs, whose
        // bones bake scale in a rotated frame) can't be cleanly decomposed; re-orthonormalizing
        // it every frame compounds error and visibly stretches the limbs. Those fall back to the
        // post-multiply M·R_delta, which preserves the shear. For a UNIFORM scale both paths give
        // s·(R·R_delta), so ordinary rotates (Pacman's death spin etc.) are unaffected.
        let c0 = [m[0], m[1], m[2]];
        let c1 = [m[4], m[5], m[6]];
        let c2 = [m[8], m[9], m[10]];
        let sx = (c0[0]*c0[0] + c0[1]*c0[1] + c0[2]*c0[2]).sqrt();
        let sy = (c1[0]*c1[0] + c1[1]*c1[1] + c1[2]*c1[2]).sqrt();
        let sz = (c2[0]*c2[0] + c2[1]*c2[1] + c2[2]*c2[2]).sqrt();
        let scale_prod = (sx*sy).max(sx*sz).max(sy*sz).max(1e-6);
        let max_dot = (c0[0]*c1[0] + c0[1]*c1[1] + c0[2]*c1[2]).abs()
            .max((c0[0]*c2[0] + c0[1]*c2[1] + c0[2]*c2[2]).abs())
            .max((c1[0]*c2[0] + c1[1]*c2[1] + c1[2]*c2[2]).abs());
        let orthogonal = (max_dot / scale_prod) < 1e-3;
        if orthogonal && sx > 1e-8 && sy > 1e-8 && sz > 1e-8 {
            // Clean basis: rebuild the rotation (R·R_delta) and re-apply the per-axis scale.
            let rotm = [
                c0[0]/sx, c0[1]/sx, c0[2]/sx, 0.0,
                c1[0]/sy, c1[1]/sy, c1[2]/sy, 0.0,
                c2[0]/sz, c2[1]/sz, c2[2]/sz, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ];
            let rn = mat4_mul_f32(&rotm, &rot);
            [
                rn[0]*sx, rn[1]*sx, rn[2]*sx, 0.0,
                rn[4]*sy, rn[5]*sy, rn[6]*sy, 0.0,
                rn[8]*sz, rn[9]*sz, rn[10]*sz, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ]
        } else {
            // Sheared (or degenerate) basis: preserve it with a plain post-multiply.
            mat4_mul_f32(&m, &rot)
        }
    };
    // #self rotates the node in place about its OWN position, so restore the
    // translation (the #self branch built a pure rotation×scale basis with a zero
    // translation column). #world / #parent rotate relative to the frame's ORIGIN,
    // which orbits a non-origin node's position around it — Director's `rotate`
    // (unlike `preRotate`) moves the position (Scripting Dictionary: a transform
    // with a positional offset rotated 180° lands "on the opposite side of the
    // orbit"). The unicraft galaxy menu orbits its camera this way
    // (`camera.rotate(vector(0,0,v), #world)` + `pointAt(origin)`); pinning the
    // translation left the camera static and the platter frozen. `rot · m` already
    // carries the orbited translation, so keep it for the world/parent frame.
    if !world_relative {
        result[12] = m[12];
        result[13] = m[13];
        result[14] = m[14];
    }
    set_node_transform(player, member_ref, node_name, result);
}

fn apply_scale(
    player: &mut crate::player::DirPlayer,
    member_ref: &crate::player::cast_lib::CastMemberRef,
    node_name: &str,
    sx: f32, sy: f32, sz: f32,
) {
    // See apply_translation comment — same flush requirement.
    sync_persistent_transforms(player);
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
    // Flush any dirty persistent Transform3d datums to node_transforms first.
    // This ensures that if the caller just set transform.position = v on a
    // persistent datum (e.g. camera.transform.position before pointAt), the
    // position is propagated before we compute the forward direction.
    sync_persistent_transforms(player);

    // Look up custom pointAtOrientation for this node (if explicitly set)
    let custom_orientation = {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        member.and_then(|m| m.member_type.as_shockwave3d())
            .and_then(|w3d| w3d.runtime_state.point_at_orientations.get(node_name))
            .copied()
    };

    // Ensure the node has a runtime transform entry (side effect of get_or_init).
    let _ = get_or_init_node_transform(player, member_ref, node_name);
    // Use WORLD position for direction computation (target is in world coordinates)
    let world_pos = get_world_position(player, member_ref, node_name);
    let pos_w = [world_pos[0] as f32, world_pos[1] as f32, world_pos[2] as f32];

    if !tx.is_finite() || !ty.is_finite() || !tz.is_finite() { return; }

    // Forward = toward target in world space
    let mut fwd = [tx - pos_w[0], ty - pos_w[1], tz - pos_w[2]];
    let len = (fwd[0]*fwd[0] + fwd[1]*fwd[1] + fwd[2]*fwd[2]).sqrt();
    if len > 1e-6 {
        fwd[0] /= len; fwd[1] /= len; fwd[2] /= len;
    } else {
        return;
    }

    // Up hint from argument; fall back to world X if forward is parallel.
    // Normalize the hint BEFORE the parallelism check — Director scripts
    // routinely pass non-unit vectors like `vector(0, 45, 0)` (the literal
    // 45 is just a magnitude; only the direction matters). Without
    // normalization, a hint magnitude of 45 makes the dot product against
    // a unit fwd reach ~45 even when the angle between them is small,
    // and the parallel-axis override fires for any camera with a slight
    // tilt — replacing world-Y up with world-X up and rolling the camera
    // 90° around its forward axis. Symptom: avatar appears upside-down
    // / sideways in MoveUICamera scenes.
    let mut up_hint = {
        let l = (up_x * up_x + up_y * up_y + up_z * up_z).sqrt();
        if l > 1e-6 { [up_x / l, up_y / l, up_z / l] } else { [0.0, 1.0, 0.0] }
    };
    let dot = up_hint[0]*fwd[0] + up_hint[1]*fwd[1] + up_hint[2]*fwd[2];
    if dot.abs() > 0.999 {
        up_hint = [1.0, 0.0, 0.0];
    }

    let cross = |a: [f32;3], b: [f32;3]| -> [f32;3] {
        [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]
    };
    let normalize = |v: [f32;3]| -> [f32;3] {
        let l = (v[0]*v[0]+v[1]*v[1]+v[2]*v[2]).sqrt();
        if l > 1e-6 { [v[0]/l, v[1]/l, v[2]/l] } else { v }
    };

    // pointAt orients the node in WORLD space, but set_node_transform stores the
    // node's LOCAL transform (re-accumulated up the parent chain by
    // getWorldTransform / worldPosition). Convert the world-space look-at into the
    // parent's frame so a node with a non-identity parent — e.g. an aim/muzzle
    // group parented to a view-rotated weapon model (Rasterwerks PlayerAimUtil) —
    // aims correctly. For a node at the world root this is a strict no-op.
    let parent_world = {
        let parent_name = player.movie.cast_manager.find_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_shockwave3d())
            .and_then(|w| w.parsed_scene.as_ref())
            .and_then(|s| s.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(node_name)))
            .map(|n| n.parent_name.clone())
            .unwrap_or_default();
        if parent_name.is_empty() || parent_name.eq_ignore_ascii_case("World") {
            IDENTITY
        } else {
            node_world_transform(player, member_ref, &parent_name)
        }
    };
    let inv_parent = invert_transform_f32(&parent_world);
    // world_mat carries the look-at rotation + the node's WORLD position; converting
    // by inverse(parent) yields the LOCAL transform (and restores the local position,
    // since inverse(parent)·pos_w == local_pos), so pointAt never moves the node.
    let to_local = |world_mat: [f32; 16]| -> [f32; 16] {
        if inv_parent.iter().all(|v| v.is_finite()) {
            mat4_mul_f32(&inv_parent, &world_mat)
        } else {
            world_mat
        }
    };

    if let Some((front_axis, up_axis)) = custom_orientation {
        // Custom pointAtOrientation: map the specified local axes to world directions.
        // front_axis defines which local axis points toward the target.
        // up_axis defines which local axis points up.
        let right_world = normalize(cross(up_hint, fwd));
        let up_world = normalize(cross(fwd, right_world));

        // Determine which column (0=X, 1=Y, 2=Z) each orientation axis represents
        let dominant = |v: [f32;3]| -> (usize, f32) {
            let ax = v[0].abs(); let ay = v[1].abs(); let az = v[2].abs();
            if ax >= ay && ax >= az { (0, v[0].signum()) }
            else if ay >= ax && ay >= az { (1, v[1].signum()) }
            else { (2, v[2].signum()) }
        };

        let (front_col, front_sign) = dominant(front_axis);
        let (up_col, up_sign) = dominant(up_axis);
        let right_col = 3 - front_col - up_col;
        // Determine right sign to maintain right-handed coordinate system.
        // The world basis has fwd × right_world = up_world (by construction).
        // The permutation (front_col, right_col, up_col) must be even for
        // a right-handed matrix (col0 × col1 = col2).
        // Even permutations of (0,1,2): (0,1,2), (1,2,0), (2,0,1).
        let is_even_perm = (front_col + 1) % 3 == right_col;
        let right_sign = if is_even_perm {
            front_sign * up_sign
        } else {
            -front_sign * up_sign
        };

        let mut world_mat = [0.0f32; 16];
        world_mat[front_col * 4 + 0] = fwd[0] * front_sign;
        world_mat[front_col * 4 + 1] = fwd[1] * front_sign;
        world_mat[front_col * 4 + 2] = fwd[2] * front_sign;
        world_mat[up_col * 4 + 0] = up_world[0] * up_sign;
        world_mat[up_col * 4 + 1] = up_world[1] * up_sign;
        world_mat[up_col * 4 + 2] = up_world[2] * up_sign;
        world_mat[right_col * 4 + 0] = right_world[0] * right_sign;
        world_mat[right_col * 4 + 1] = right_world[1] * right_sign;
        world_mat[right_col * 4 + 2] = right_world[2] * right_sign;
        world_mat[12] = pos_w[0]; world_mat[13] = pos_w[1]; world_mat[14] = pos_w[2]; world_mat[15] = 1.0;
        set_node_transform(player, member_ref, node_name, to_local(world_mat));
    } else {
        // Default orientation: -Z toward target, Y up (standard look-at convention).
        // This matches the working camera behavior where cameras look along -Z.
        let neg_fwd = [-fwd[0], -fwd[1], -fwd[2]];
        let right = normalize(cross(up_hint, neg_fwd));
        let up2 = normalize(cross(neg_fwd, right));

        let world_mat = [
            right[0],   right[1],   right[2],   0.0,
            up2[0],     up2[1],     up2[2],     0.0,
            neg_fwd[0], neg_fwd[1], neg_fwd[2], 0.0,
            pos_w[0],   pos_w[1],   pos_w[2],   1.0,
        ];
        set_node_transform(player, member_ref, node_name, to_local(world_mat));
    }
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

/// Euler angles (degrees) to column-major rotation matrix (IFX convention: R = Rx * Ry * Rz)
fn euler_to_matrix_f32(rx_deg: f32, ry_deg: f32, rz_deg: f32) -> [f32; 16] {
    let rx = rx_deg.to_radians();
    let ry = (-ry_deg).to_radians();
    let rz = rz_deg.to_radians();
    let (sx, cx) = (rx.sin(), rx.cos());
    let (sy, cy) = (ry.sin(), ry.cos());
    let (sz, cz) = (rz.sin(), rz.cos());

    // R = Rz * Ry * Rx, true column-major: m[col*4+row]
    [
        cy*cz,              cy*sz,              -sy,               0.0,  // col 0
        sx*sy*cz - cx*sz,   sx*sy*sz + cx*cz,   sx*cy,            0.0,  // col 1
        cx*sy*cz + sx*sz,   cx*sy*sz - sx*cz,   cx*cy,            0.0,  // col 2
        0.0,                0.0,                0.0,               1.0,  // col 3
    ]
}

/// Invert a column-major affine transform (handles rotation, SCALE and
/// translation). A previous version transposed the upper-3×3 (assuming a pure
/// rotation, Rᵀ = R⁻¹), which is wrong when the matrix carries scale: for a 3×3
/// of R·S the transpose is S·Rᵀ, not the true inverse S⁻¹·Rᵀ. That made
/// `addChild #preserveWorld` (`inverse(parentWorld) × childWorld`) multiply the
/// parent's scale in instead of dividing it out, so scale compounded down a deep
/// hierarchy and shrank every limb toward a point (the frog01 frog: `body` at the
/// frog's 0.08 scale, but `ll` collapsed to ~0.01). Use a full 3×3 inverse via
/// the adjugate; falls back to the rigid inverse for a (near-)singular matrix.
fn invert_transform_f32(m: &[f32; 16]) -> [f32; 16] {
    // Upper-3×3 A (column-major: A[row][col] = m[col*4 + row]).
    let (a, b, c) = (m[0], m[4], m[8]);   // row 0
    let (d, e, f) = (m[1], m[5], m[9]);   // row 1
    let (g, h, i) = (m[2], m[6], m[10]);  // row 2
    let (tx, ty, tz) = (m[12], m[13], m[14]);

    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        // Singular — fall back to the rigid (rotation-only) inverse.
        let itx = -(a * tx + d * ty + g * tz);
        let ity = -(b * tx + e * ty + h * tz);
        let itz = -(c * tx + f * ty + i * tz);
        return [
            a, b, c, 0.0,
            d, e, f, 0.0,
            g, h, i, 0.0,
            itx, ity, itz, 1.0,
        ];
    }
    let inv_det = 1.0 / det;
    // inverse(A)[row][col] = cofactor / det
    let r00 = (e * i - f * h) * inv_det;
    let r01 = (c * h - b * i) * inv_det;
    let r02 = (b * f - c * e) * inv_det;
    let r10 = (f * g - d * i) * inv_det;
    let r11 = (a * i - c * g) * inv_det;
    let r12 = (c * d - a * f) * inv_det;
    let r20 = (d * h - e * g) * inv_det;
    let r21 = (b * g - a * h) * inv_det;
    let r22 = (a * e - b * d) * inv_det;

    // Inverse translation = -inverse(A) · t.
    let itx = -(r00 * tx + r01 * ty + r02 * tz);
    let ity = -(r10 * tx + r11 * ty + r12 * tz);
    let itz = -(r20 * tx + r21 * ty + r22 * tz);

    // Repack column-major: out[col*4 + row] = inverse(A)[row][col].
    [
        r00, r10, r20, 0.0, // col 0
        r01, r11, r21, 0.0, // col 1
        r02, r12, r22, 0.0, // col 2
        itx, ity, itz, 1.0, // col 3
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
