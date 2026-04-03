use std::collections::VecDeque;

use log::warn;

use crate::{
    director::lingo::datum::{Datum, DatumType, HavokObjectRef},
    player::{
        cast_lib::CastMemberRef,
        cast_member::{
            CastMemberType, HavokAngularDashpot, HavokCollisionInterest, HavokLinearDashpot,
            HavokPhysicsMember, HavokRigidBody, HavokSpring, RapierWorld,
        },
        reserve_player_mut, DatumRef, ScriptError,
    },
};

pub struct HavokPhysicsMemberHandlers {}

impl HavokPhysicsMemberHandlers {
    pub fn get_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        // First, read whatever we need from the member with an immutable borrow
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let state = &havok.state;

        // For simple properties, return immediately (no borrow conflict)
        match prop {
            "initialized" => return Ok(Datum::Int(if state.initialized { 1 } else { 0 })),
            "tolerance" => return Ok(Datum::Float(state.tolerance)),
            "scale" => return Ok(Datum::Float(state.scale)),
            "gravity" => return Ok(Datum::Vector(state.gravity)),
            "simTime" | "simtime" => return Ok(Datum::Float(state.sim_time)),
            "timeStep" | "timestep" => return Ok(Datum::Float(state.time_step)),
            "subSteps" | "substeps" => return Ok(Datum::Int(state.sub_steps)),
            "collisionList" | "collisionlist" => return Ok(Datum::List(DatumType::List, VecDeque::new(), false)),
            _ => {} // fall through to list properties below
        }

        // For list properties, collect data first, then drop the borrow
        let names: Vec<String>;
        let list_type: &str;
        match prop {
            "rigidBody" | "rigidbody" => {
                names = state.rigid_bodies.iter().map(|rb| rb.name.clone()).collect();
                list_type = "rigidBody";
            }
            "spring" => {
                names = state.springs.iter().map(|s| s.name.clone()).collect();
                list_type = "spring";
            }
            "linearDashpot" | "lineardashpot" => {
                names = state.linear_dashpots.iter().map(|d| d.name.clone()).collect();
                list_type = "linearDashpot";
            }
            "angularDashpot" | "angulardashpot" => {
                names = state.angular_dashpots.iter().map(|d| d.name.clone()).collect();
                list_type = "angularDashpot";
            }
            "deactivationParameters" | "deactivationparameters" => {
                let params = state.deactivation_params;
                // Drop borrow by falling out of scope, then alloc
                let a = player.alloc_datum(Datum::Float(params[0]));
                let b = player.alloc_datum(Datum::Float(params[1]));
                return Ok(Datum::List(DatumType::List, VecDeque::from([a, b]), false));
            }
            "dragParameters" | "dragparameters" => {
                let params = state.drag_params;
                let a = player.alloc_datum(Datum::Float(params[0]));
                let b = player.alloc_datum(Datum::Float(params[1]));
                return Ok(Datum::List(DatumType::List, VecDeque::from([a, b]), false));
            }
            _ => return Err(ScriptError::new(format!(
                "Cannot get Havok member property: {}",
                prop
            ))),
        }

        // Now borrow is dropped, we can use player.alloc_datum
        let items: VecDeque<DatumRef> = names.iter().map(|name| {
            player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
                cast_lib: member_ref.cast_lib,
                cast_member: member_ref.cast_member,
                object_type: list_type.to_string(),
                name: name.clone(),
            }))
        }).collect();
        Ok(Datum::List(DatumType::List, items, false))
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            // For list properties, extract float values before borrowing the member mutably
            match prop {
                "deactivationParameters" | "deactivationparameters" => {
                    if let Datum::List(_, items, _) = &value {
                        if items.len() >= 2 {
                            let v0 = player.get_datum(&items[0]).to_float()?;
                            let v1 = player.get_datum(&items[1]).to_float()?;
                            let member = player
                                .movie
                                .cast_manager
                                .find_mut_member_by_ref(member_ref)
                                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                            let havok = match &mut member.member_type {
                                CastMemberType::HavokPhysics(h) => h,
                                _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                            };
                            havok.state.deactivation_params = [v0, v1];
                        }
                    }
                    return Ok(());
                }
                "dragParameters" | "dragparameters" => {
                    if let Datum::List(_, items, _) = &value {
                        if items.len() >= 2 {
                            let v0 = player.get_datum(&items[0]).to_float()?;
                            let v1 = player.get_datum(&items[1]).to_float()?;
                            let member = player
                                .movie
                                .cast_manager
                                .find_mut_member_by_ref(member_ref)
                                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                            let havok = match &mut member.member_type {
                                CastMemberType::HavokPhysics(h) => h,
                                _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                            };
                            havok.state.drag_params = [v0, v1];
                        }
                    }
                    return Ok(());
                }
                _ => {}
            }

            let member = player
                .movie
                .cast_manager
                .find_mut_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
            let havok = match &mut member.member_type {
                CastMemberType::HavokPhysics(h) => h,
                _ => return Err(ScriptError::new("Not a Havok member".to_string())),
            };
            let state = &mut havok.state;
            match prop {
                "gravity" => {
                    if let Datum::Vector(v) = &value {
                        state.gravity = *v;
                        // Sync gravity to rapier world
                        if let Some(ref mut rapier) = state.rapier {
                            rapier.gravity = rapier3d_f64::prelude::Vector::new(v[0], v[1], v[2]);
                        }
                    } else {
                        return Err(ScriptError::new("gravity must be a vector".to_string()));
                    }
                    Ok(())
                }
                "timeStep" | "timestep" => {
                    state.time_step = value.to_float()?;
                    Ok(())
                }
                "subSteps" | "substeps" => {
                    state.sub_steps = value.int_value()?;
                    Ok(())
                }
                _ => Err(ScriptError::new(format!(
                    "Cannot set Havok member property: {}",
                    prop
                ))),
            }
        })
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let member_ref = match player.get_datum(datum) {
                Datum::CastMember(r) => r.to_owned(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot call Havok handler on non-cast-member".to_string(),
                    ))
                }
            };

            match handler_name {
                "initialize" | "Initialize" => Self::initialize(player, &member_ref, args),
                "shutdown" | "shutDown" | "Shutdown" => Self::shutdown(player, &member_ref),
                "step" => Self::step(player, &member_ref, args),
                "reset" => Self::reset(player, &member_ref),
                "rigidBody" | "rigidbody" => Self::get_rigid_body(player, &member_ref, args),
                "spring" => Self::get_spring(player, &member_ref, args),
                "linearDashpot" | "lineardashpot" => {
                    Self::get_linear_dashpot(player, &member_ref, args)
                }
                "angularDashpot" | "angulardashpot" => {
                    Self::get_angular_dashpot(player, &member_ref, args)
                }
                "makeMovableRigidBody" | "makemovablerigidbody" => {
                    Self::make_movable_rigid_body(player, &member_ref, args)
                }
                "makeFixedRigidBody" | "makefixedrigidbody" => {
                    Self::make_fixed_rigid_body(player, &member_ref, args)
                }
                "makeSpring" | "makespring" => Self::make_spring(player, &member_ref, args),
                "makeLinearDashpot" | "makelineardashpot" => {
                    Self::make_linear_dashpot(player, &member_ref, args)
                }
                "makeAngularDashpot" | "makeangulardashpot" => {
                    Self::make_angular_dashpot(player, &member_ref, args)
                }
                "deleteRigidBody" | "deleterigidbody" => {
                    Self::delete_rigid_body(player, &member_ref, args)
                }
                "deleteSpring" | "deletespring" => {
                    Self::delete_spring(player, &member_ref, args)
                }
                "deleteLinearDashpot" | "deletelineardashpot" => {
                    Self::delete_linear_dashpot(player, &member_ref, args)
                }
                "deleteAngularDashpot" | "deleteangulardashpot" => {
                    Self::delete_angular_dashpot(player, &member_ref, args)
                }
                "registerInterest" | "registerinterest" => {
                    Self::register_interest(player, &member_ref, args)
                }
                "removeInterest" | "removeinterest" => {
                    Self::remove_interest(player, &member_ref, args)
                }
                "registerStepCallback" | "registerstepcallback" => {
                    Self::register_step_callback(player, &member_ref, args)
                }
                "removeStepCallback" | "removestepcallback" => {
                    Self::remove_step_callback(player, &member_ref, args)
                }
                "enableCollision" | "enablecollision" => {
                    Self::enable_collision(player, &member_ref, args)
                }
                "disableCollision" | "disablecollision" => {
                    Self::disable_collision(player, &member_ref, args)
                }
                "enableAllCollisions" | "enableallcollisions" => {
                    Self::enable_all_collisions(player, &member_ref, args)
                }
                "disableAllCollisions" | "disableallcollisions" => {
                    Self::disable_all_collisions(player, &member_ref, args)
                }
                "getProp" => {
                    let prop = player.get_datum(&args[0]).string_value()?;
                    let result = Self::get_prop(player, &member_ref, &prop)?;
                    Ok(player.alloc_datum(result))
                }
                "count" => {
                    // count(#rigidBody) etc.
                    let prop = player.get_datum(&args[0]).string_value()?;
                    let list_datum = Self::get_prop(player, &member_ref, &prop)?;
                    if let Datum::List(_, items, _) = &list_datum {
                        Ok(player.alloc_datum(Datum::Int(items.len() as i32)))
                    } else {
                        Ok(player.alloc_datum(Datum::Int(0)))
                    }
                }
                "getAt" | "getPropRef" => {
                    // member("havok").rigidBody[i] — getAt dispatches here
                    let prop = player.get_datum(&args[0]).string_value()?;
                    let list_datum = Self::get_prop(player, &member_ref, &prop)?;
                    if args.len() > 1 {
                        let index = player.get_datum(&args[1]).int_value()?;
                        if let Datum::List(_, items, _) = &list_datum {
                            let idx = (index as usize).saturating_sub(1);
                            if idx < items.len() {
                                Ok(items[idx].clone())
                            } else {
                                Ok(DatumRef::Void)
                            }
                        } else {
                            Ok(DatumRef::Void)
                        }
                    } else {
                        Ok(player.alloc_datum(list_datum))
                    }
                }
                _ => Err(ScriptError::new(format!(
                    "No handler {} for Havok member",
                    handler_name
                ))),
            }
        })
    }

    // --- Method implementations ---

    fn initialize(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // initialize(w3dMember [, tolerance, worldScale])
        let w3d_ref = match player.get_datum(&args[0]) {
            Datum::CastMember(r) => r.to_owned(),
            _ => {
                return Err(ScriptError::new(
                    "Havok initialize: first argument must be a W3D member".to_string(),
                ))
            }
        };

        let tolerance = if args.len() > 1 {
            player.get_datum(&args[1]).to_float()?
        } else {
            0.1
        };
        let scale = if args.len() > 2 {
            player.get_datum(&args[2]).to_float()?
        } else {
            0.0254
        };

        // Read existing rigid body names from the W3D scene models
        let model_names: Vec<String> = {
            let w3d_member = player
                .movie
                .cast_manager
                .find_member_by_ref(&w3d_ref);
            if let Some(m) = w3d_member {
                if let Some(w3d) = m.member_type.as_shockwave3d() {
                    if let Some(scene) = &w3d.parsed_scene {
                        scene.nodes.iter().map(|n| n.name.clone()).collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        };

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        havok.state.initialized = true;
        havok.state.w3d_cast_lib = w3d_ref.cast_lib;
        havok.state.w3d_cast_member = w3d_ref.cast_member;
        havok.state.tolerance = tolerance;
        havok.state.scale = scale;
        // Default gravity in scene units (inches with 0.0254 scale = 386.22 in/s^2)
        havok.state.gravity = [0.0, 0.0, -386.22];
        havok.state.sim_time = 0.0;

        // Create rapier3d physics world
        havok.state.rapier = Some(Box::new(RapierWorld::new(havok.state.gravity)));

        // Parse HKE collision geometry and create fixed rigid bodies
        let hke_mesh_count = if !havok.state.hke_data.is_empty() {
            let hke = super::hke_parser::parse_hke(&havok.state.hke_data);
            let mut loaded = 0usize;
            if let Some(ref mut rapier) = havok.state.rapier {
                use rapier3d_f64::prelude::*;
                // HKE vertices are in Havok world space (meters).
                // Convert to Director units by dividing by worldScale.
                let inv_scale = if scale.abs() > 1e-10 { 1.0 / scale } else { 1.0 };

                for mesh in &hke.meshes {
                    if mesh.vertices.is_empty() || mesh.triangles.is_empty() { continue; }

                    // Convert HKE vertices from Havok meters to Director units
                    let vertices: Vec<Vector> = mesh.vertices.iter()
                        .map(|v| Vector::new(
                            v[0] as f64 * inv_scale,
                            v[1] as f64 * inv_scale,
                            v[2] as f64 * inv_scale,
                        ))
                        .collect();
                    let indices: Vec<[u32; 3]> = mesh.triangles.clone();

                    // Vertices are already in world space - place body at origin
                    let rapier_rb = RigidBodyBuilder::fixed().build();
                    let handle = rapier.rigid_body_set.insert(rapier_rb);

                    match ColliderBuilder::trimesh(vertices, indices) {
                        Ok(builder) => {
                            let collider = builder
                                .restitution(0.3)
                                .friction(0.5)
                                .build();
                            let col_handle = rapier.collider_set.insert_with_parent(
                                collider, handle, &mut rapier.rigid_body_set,
                            );
                            rapier.body_handles.insert(
                                format!("hke_{}", mesh.name), handle,
                            );
                            rapier.collider_handles.insert(
                                format!("hke_{}", mesh.name), col_handle,
                            );
                            loaded += 1;
                        }
                        Err(e) => {
                            web_sys::console::warn_1(&format!(
                                "HKE trimesh failed for '{}': {:?}",
                                mesh.name, e
                            ).into());
                        }
                    }
                }
            }
            web_sys::console::log_1(&format!(
                "HKE: parsed {} collision meshes, loaded {} into rapier",
                hke.meshes.len(), loaded
            ).into());
            loaded
        } else {
            0
        };

        web_sys::console::log_1(
            &format!(
                "Havok initialized: tolerance={}, scale={}, w3d_models={}, hke_colliders={}",
                tolerance, scale, model_names.len(), hke_mesh_count
            )
            .into(),
        );

        Ok(DatumRef::Void)
    }

    fn shutdown(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        havok.state.initialized = false;
        havok.state.rigid_bodies.clear();
        havok.state.springs.clear();
        havok.state.linear_dashpots.clear();
        havok.state.angular_dashpots.clear();
        havok.state.collision_interests.clear();
        havok.state.step_callbacks.clear();
        havok.state.disabled_collision_pairs.clear();
        havok.state.sim_time = 0.0;
        havok.state.rapier = None;

        Ok(DatumRef::Void)
    }

    fn step(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let time_increment = if !args.is_empty() {
            player.get_datum(&args[0]).to_float()?
        } else {
            let member = player
                .movie
                .cast_manager
                .find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
            match &member.member_type {
                CastMemberType::HavokPhysics(h) => h.state.time_step,
                _ => 1.0 / 60.0,
            }
        };
        let num_sub_steps = if args.len() > 1 {
            player.get_datum(&args[1]).int_value()?
        } else {
            let member = player
                .movie
                .cast_manager
                .find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
            match &member.member_type {
                CastMemberType::HavokPhysics(h) => h.state.sub_steps,
                _ => 4,
            }
        };

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        havok.state.sim_time += time_increment;

        // Keyboard hack removed - using game's native wheel spring controls
        if false {
        }

        // Step Rapier3D physics (now with HKE collision geometry loaded)
        if let Some(ref mut rapier) = havok.state.rapier {
            let sub_dt = if num_sub_steps > 0 {
                time_increment / num_sub_steps as f64
            } else {
                time_increment
            };
            rapier.integration_parameters.dt = sub_dt;
            for _ in 0..num_sub_steps.max(1) {
                rapier.step();
            }
        }

        // Sync Rapier positions back to HavokRigidBody state
        if let Some(ref rapier) = havok.state.rapier {
            for rb in &mut havok.state.rigid_bodies {
                if rb.is_fixed { continue; }
                if let Some(handle) = rapier.body_handles.get(&rb.name) {
                    if let Some(body) = rapier.rigid_body_set.get(*handle) {
                        let pos = body.translation();
                        if pos.x.is_finite() && pos.y.is_finite() && pos.z.is_finite() {
                            rb.position = [pos.x, pos.y, pos.z];
                        }
                        let vel = body.linvel();
                        if vel.x.is_finite() && vel.y.is_finite() && vel.z.is_finite() {
                            rb.linear_velocity = [vel.x, vel.y, vel.z];
                        }
                        let avel = body.angvel();
                        if avel.x.is_finite() && avel.y.is_finite() && avel.z.is_finite() {
                            rb.angular_velocity = [avel.x, avel.y, avel.z];
                        }
                    }
                }
            }
        }

        // Collect sync data before dropping havok borrow
        let w3d_cast_lib = havok.state.w3d_cast_lib;
        let w3d_cast_member = havok.state.w3d_cast_member;
        let sync_data: Vec<(String, [f64; 3])> = havok.state.rigid_bodies.iter()
            .filter(|rb| !rb.is_fixed && rb.active)
            .map(|rb| (rb.name.clone(), rb.position))
            .collect();

        // Clear accumulated forces each step.
        for rb in &mut havok.state.rigid_bodies {
            rb.force = [0.0; 3];
            rb.torque = [0.0; 3];
        }

        // Drop havok borrow, then sync positions to W3D model transforms
        drop(member);
        let w3d_ref = CastMemberRef { cast_lib: w3d_cast_lib, cast_member: w3d_cast_member };
        if let Some(w3d_member) = player.movie.cast_manager.find_mut_member_by_ref(&w3d_ref) {
            if let Some(w3d) = w3d_member.member_type.as_shockwave3d_mut() {
                for (name, pos) in &sync_data {
                    if !pos[0].is_finite() || !pos[1].is_finite() || !pos[2].is_finite() { continue; }
                    // Use identity rotation + position for W3D sync
                    let t = [
                        1.0f32, 0.0, 0.0, 0.0,
                        0.0, 1.0, 0.0, 0.0,
                        0.0, 0.0, 1.0, 0.0,
                        pos[0] as f32, pos[1] as f32, pos[2] as f32, 1.0,
                    ];
                    w3d.runtime_state.node_transforms.insert(name.clone(), t);
                }
            }
        }

        Ok(DatumRef::Void)
    }

    fn reset(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok.state.sim_time = 0.0;
        // Reset rigid body states to initial positions would go here
        Ok(DatumRef::Void)
    }

    fn get_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = player.get_datum(&args[0]).string_value()?;
        // Verify the rigid body exists
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let _found = havok
            .state
            .rigid_bodies
            .iter()
            .any(|rb| rb.name.eq_ignore_ascii_case(&name));

        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "rigidBody".to_string(),
            name,
        })))
    }

    fn get_spring(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = player.get_datum(&args[0]).string_value()?;
        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "spring".to_string(),
            name,
        })))
    }

    fn get_linear_dashpot(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = player.get_datum(&args[0]).string_value()?;
        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "linearDashpot".to_string(),
            name,
        })))
    }

    fn get_angular_dashpot(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = player.get_datum(&args[0]).string_value()?;
        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "angularDashpot".to_string(),
            name,
        })))
    }

    fn make_movable_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let model_name = player.get_datum(&args[0]).string_value()?;
        let mass = player.get_datum(&args[1]).to_float()?;
        let is_convex = if args.len() > 2 {
            player.get_datum(&args[2]).int_value()? != 0
        } else {
            true
        };

        // Try to read the model's initial transform from the W3D scene
        let initial_position = {
            let member = player
                .movie
                .cast_manager
                .find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
            let havok = match &member.member_type {
                CastMemberType::HavokPhysics(h) => h,
                _ => return Err(ScriptError::new("Not a Havok member".to_string())),
            };
            let w3d_ref = CastMemberRef {
                cast_lib: havok.state.w3d_cast_lib,
                cast_member: havok.state.w3d_cast_member,
            };
            let w3d_member = player.movie.cast_manager.find_member_by_ref(&w3d_ref);
            if let Some(m) = w3d_member {
                if let Some(w3d) = m.member_type.as_shockwave3d() {
                    w3d.runtime_state.node_transforms
                        .get(&model_name)
                        .map(|t| [t[12] as f64, t[13] as f64, t[14] as f64])
                        .unwrap_or([0.0; 3])
                } else {
                    [0.0; 3]
                }
            } else {
                [0.0; 3]
            }
        };

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        let mut rb = HavokRigidBody::new_movable(&model_name, mass, is_convex);
        rb.position = initial_position;

        // Create rapier dynamic rigid body + collider
        if let Some(ref mut rapier) = havok.state.rapier {
            use rapier3d_f64::prelude::*;
            let rapier_rb = RigidBodyBuilder::dynamic()
                .translation(Vector::new(initial_position[0], initial_position[1], initial_position[2]))
                .build();
            let handle = rapier.rigid_body_set.insert(rapier_rb);

            // Use a box approximation for dynamic bodies (can be refined later with actual mesh geometry)
            let collider = ColliderBuilder::cuboid(10.0, 10.0, 10.0)
                .mass(mass as Real)
                .restitution(0.3)
                .friction(0.5)
                .build();
            let col_handle = rapier.collider_set.insert_with_parent(collider, handle, &mut rapier.rigid_body_set);
            rapier.body_handles.insert(model_name.clone(), handle);
            rapier.collider_handles.insert(model_name.clone(), col_handle);
        }

        havok.state.rigid_bodies.push(rb);

        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "rigidBody".to_string(),
            name: model_name,
        })))
    }

    fn make_fixed_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let model_name = player.get_datum(&args[0]).string_value()?;
        let is_convex = if args.len() > 1 {
            player.get_datum(&args[1]).int_value()? != 0
        } else {
            true
        };

        // Extract mesh geometry + model transform from W3D scene BEFORE mutably borrowing havok.
        let mesh_data: Option<(Vec<rapier3d_f64::prelude::Vector>, Vec<[u32; 3]>, [f32; 16])> = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
            let havok = match &member.member_type {
                CastMemberType::HavokPhysics(h) => h,
                _ => return Err(ScriptError::new("Not a Havok member".to_string())),
            };
            let w3d_ref = CastMemberRef {
                cast_lib: havok.state.w3d_cast_lib,
                cast_member: havok.state.w3d_cast_member,
            };
            let w3d_member = player.movie.cast_manager.find_member_by_ref(&w3d_ref);
            if let Some(m) = w3d_member {
                if let Some(w3d) = m.member_type.as_shockwave3d() {
                    if let Some(scene) = &w3d.parsed_scene {
                        let node = scene.nodes.iter()
                            .find(|n| n.name.eq_ignore_ascii_case(&model_name));
                        let model_transform = node.map(|n| {
                            // Check runtime transform first, then node transform
                            w3d.runtime_state.node_transforms.get(&n.name)
                                .copied()
                                .unwrap_or(n.transform)
                        }).unwrap_or([1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0]);
                        let res_name = node.map(|n| {
                            if !n.model_resource_name.is_empty() { n.model_resource_name.clone() }
                            else { n.resource_name.clone() }
                        }).unwrap_or_default();
                        if !res_name.is_empty() {
                            // Try CLOD meshes first, then raw meshes
                            let mesh = scene.clod_meshes.get(&res_name).map(|clod_meshes| {
                                let mut vertices = Vec::new();
                                let mut indices = Vec::new();
                                let mut vert_offset = 0u32;
                                for mesh in clod_meshes {
                                    for pos in &mesh.positions {
                                        let (x, y, z) = (pos[0] as f64, pos[1] as f64, pos[2] as f64);
                                        if x.is_finite() && y.is_finite() && z.is_finite() {
                                            vertices.push(rapier3d_f64::prelude::Vector::new(x, y, z));
                                        }
                                    }
                                    let valid_verts = vertices.len() as u32;
                                    for face in &mesh.faces {
                                        let (a, b, c) = (face[0] + vert_offset, face[1] + vert_offset, face[2] + vert_offset);
                                        if a < valid_verts && b < valid_verts && c < valid_verts && a != b && b != c && a != c {
                                            indices.push([a, b, c]);
                                        }
                                    }
                                    vert_offset = valid_verts;
                                }
                                (vertices, indices)
                            }).or_else(|| {
                                scene.raw_meshes.iter().find(|m| m.name.eq_ignore_ascii_case(&res_name)).map(|raw_mesh| {
                                    let vertices: Vec<_> = raw_mesh.positions.iter()
                                        .filter(|p| p[0].is_finite() && p[1].is_finite() && p[2].is_finite())
                                        .map(|pos| rapier3d_f64::prelude::Vector::new(pos[0] as f64, pos[1] as f64, pos[2] as f64))
                                        .collect();
                                    (vertices, raw_mesh.faces.clone())
                                })
                            });
                            match mesh {
                                Some((v, i)) if !v.is_empty() && !i.is_empty() => Some((v, i, model_transform)),
                                _ => None,
                            }
                        } else { None }
                    } else { None }
                } else { None }
            } else { None }
        };

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        let rb = HavokRigidBody::new_fixed(&model_name, is_convex);

        // Create rapier fixed rigid body + collider
        if let Some(ref mut rapier) = havok.state.rapier {
            use rapier3d_f64::prelude::*;
            // Place the fixed body at the model's world position
            let (tx, ty, tz) = mesh_data.as_ref()
                .map(|(_, _, t)| (t[12] as f64, t[13] as f64, t[14] as f64))
                .unwrap_or((0.0, 0.0, 0.0));
            let rapier_rb = RigidBodyBuilder::fixed()
                .translation(Vector::new(tx, ty, tz))
                .build();
            let handle = rapier.rigid_body_set.insert(rapier_rb);

            let collider = if let Some((vertices, indices, _transform)) = mesh_data {
                web_sys::console::log_1(
                    &format!(
                        "Havok makeFixedRigidBody '{}': using trimesh collider ({} verts, {} tris)",
                        model_name, vertices.len(), indices.len()
                    ).into(),
                );
                // trimesh() returns Result in rapier v0.32
                match ColliderBuilder::trimesh(vertices, indices) {
                    Ok(builder) => builder.restitution(0.3).friction(0.5).build(),
                    Err(e) => {
                        web_sys::console::log_1(
                            &format!("Havok: trimesh collider failed for '{}': {:?}, falling back to box", model_name, e).into(),
                        );
                        ColliderBuilder::cuboid(100.0, 100.0, 1.0)
                            .restitution(0.3)
                            .friction(0.5)
                            .build()
                    }
                }
            } else {
                web_sys::console::log_1(
                    &format!(
                        "Havok makeFixedRigidBody '{}': no mesh data, using large box collider",
                        model_name
                    ).into(),
                );
                ColliderBuilder::cuboid(100.0, 100.0, 1.0)
                    .restitution(0.3)
                    .friction(0.5)
                    .build()
            };
            let col_handle = rapier.collider_set.insert_with_parent(collider, handle, &mut rapier.rigid_body_set);
            rapier.body_handles.insert(model_name.clone(), handle);
            rapier.collider_handles.insert(model_name.clone(), col_handle);
        }

        havok.state.rigid_bodies.push(rb);

        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "rigidBody".to_string(),
            name: model_name,
        })))
    }

    fn make_spring(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = player.get_datum(&args[0]).string_value()?;
        let rb_a = player.get_datum(&args[1]).string_value()?;

        let mut spring = HavokSpring::new(&name);
        spring.rigid_body_a = Some(rb_a);

        if args.len() > 2 {
            let arg2 = player.get_datum(&args[2]).clone();
            match &arg2 {
                Datum::String(s) => spring.rigid_body_b = Some(s.clone()),
                Datum::Vector(v) => spring.point_b = *v,
                _ => {
                    let s = arg2.string_value()?;
                    spring.rigid_body_b = Some(s);
                }
            }
        }

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok.state.springs.push(spring);

        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "spring".to_string(),
            name,
        })))
    }

    fn make_linear_dashpot(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = player.get_datum(&args[0]).string_value()?;
        let rb_a = player.get_datum(&args[1]).string_value()?;

        let mut dashpot = HavokLinearDashpot::new(&name);
        dashpot.rigid_body_a = Some(rb_a);

        if args.len() > 2 {
            let arg2 = player.get_datum(&args[2]).clone();
            match &arg2 {
                Datum::String(s) => dashpot.rigid_body_b = Some(s.clone()),
                Datum::Vector(v) => dashpot.point_b = *v,
                _ => {
                    let s = arg2.string_value()?;
                    dashpot.rigid_body_b = Some(s);
                }
            }
        }

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok.state.linear_dashpots.push(dashpot);

        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "linearDashpot".to_string(),
            name,
        })))
    }

    fn make_angular_dashpot(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = player.get_datum(&args[0]).string_value()?;
        let rb_a = player.get_datum(&args[1]).string_value()?;

        let mut dashpot = HavokAngularDashpot::new(&name);
        dashpot.rigid_body_a = Some(rb_a);

        if args.len() > 2 {
            let s = player.get_datum(&args[2]).string_value()?;
            dashpot.rigid_body_b = Some(s);
        }

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok.state.angular_dashpots.push(dashpot);

        Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "angularDashpot".to_string(),
            name,
        })))
    }

    fn delete_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let arg = player.get_datum(&args[0]).clone();
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        // Determine the name of the body to delete
        let rb_name_to_delete: Option<String> = match &arg {
            Datum::String(name) => Some(name.clone()),
            Datum::Int(index) => {
                let idx = (*index as usize).saturating_sub(1);
                if idx < havok.state.rigid_bodies.len() {
                    Some(havok.state.rigid_bodies[idx].name.clone())
                } else { None }
            }
            _ => None,
        };

        // Remove from rapier
        if let Some(ref name) = rb_name_to_delete {
            if let Some(ref mut rapier) = havok.state.rapier {
                if let Some(handle) = rapier.body_handles.remove(name) {
                    rapier.rigid_body_set.remove(
                        handle,
                        &mut rapier.island_manager,
                        &mut rapier.collider_set,
                        &mut rapier.impulse_joint_set,
                        &mut rapier.multibody_joint_set,
                        true,
                    );
                }
                rapier.collider_handles.remove(name);
            }
        }

        // Remove from havok state
        match &arg {
            Datum::String(name) => {
                havok
                    .state
                    .rigid_bodies
                    .retain(|rb| !rb.name.eq_ignore_ascii_case(name));
            }
            Datum::Int(index) => {
                let idx = (*index as usize).saturating_sub(1);
                if idx < havok.state.rigid_bodies.len() {
                    havok.state.rigid_bodies.remove(idx);
                }
            }
            _ => {}
        }
        Ok(DatumRef::Void)
    }

    fn delete_spring(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let arg = player.get_datum(&args[0]).clone();
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        match &arg {
            Datum::String(name) => {
                havok
                    .state
                    .springs
                    .retain(|s| !s.name.eq_ignore_ascii_case(name));
            }
            Datum::Int(index) => {
                let idx = (*index as usize).saturating_sub(1);
                if idx < havok.state.springs.len() {
                    havok.state.springs.remove(idx);
                }
            }
            _ => {}
        }
        Ok(DatumRef::Void)
    }

    fn delete_linear_dashpot(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let arg = player.get_datum(&args[0]).clone();
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        match &arg {
            Datum::String(name) => {
                havok
                    .state
                    .linear_dashpots
                    .retain(|d| !d.name.eq_ignore_ascii_case(name));
            }
            Datum::Int(index) => {
                let idx = (*index as usize).saturating_sub(1);
                if idx < havok.state.linear_dashpots.len() {
                    havok.state.linear_dashpots.remove(idx);
                }
            }
            _ => {}
        }
        Ok(DatumRef::Void)
    }

    fn delete_angular_dashpot(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let arg = player.get_datum(&args[0]).clone();
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        match &arg {
            Datum::String(name) => {
                havok
                    .state
                    .angular_dashpots
                    .retain(|d| !d.name.eq_ignore_ascii_case(name));
            }
            Datum::Int(index) => {
                let idx = (*index as usize).saturating_sub(1);
                if idx < havok.state.angular_dashpots.len() {
                    havok.state.angular_dashpots.remove(idx);
                }
            }
            _ => {}
        }
        Ok(DatumRef::Void)
    }

    fn register_interest(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let rb_name1 = player.get_datum(&args[0]).string_value()?;
        let rb_name2 = player.get_datum(&args[1]).string_value()?;
        let frequency = player.get_datum(&args[2]).to_float()?;
        let threshold = player.get_datum(&args[3]).to_float()?;

        let handler_name = if args.len() > 4 {
            Some(player.get_datum(&args[4]).string_value()?)
        } else {
            None
        };
        let script_instance = if args.len() > 5 {
            Some(args[5].clone())
        } else {
            None
        };

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        havok.state.collision_interests.push(HavokCollisionInterest {
            rb_name1,
            rb_name2,
            frequency,
            threshold,
            handler_name,
            script_instance,
        });

        Ok(DatumRef::Void)
    }

    fn remove_interest(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let rb_name = player.get_datum(&args[0]).string_value()?;
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok
            .state
            .collision_interests
            .retain(|ci| !ci.rb_name1.eq_ignore_ascii_case(&rb_name));
        Ok(DatumRef::Void)
    }

    fn register_step_callback(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let handler_name = player.get_datum(&args[0]).string_value()?;
        let script_instance = args[1].clone();

        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok
            .state
            .step_callbacks
            .push((handler_name, script_instance));
        Ok(DatumRef::Void)
    }

    fn remove_step_callback(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let handler_name = player.get_datum(&args[0]).string_value()?;
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok
            .state
            .step_callbacks
            .retain(|(name, _)| name != &handler_name);
        Ok(DatumRef::Void)
    }

    fn enable_collision(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let rb_a = player.get_datum(&args[0]).string_value()?;
        let rb_b = player.get_datum(&args[1]).string_value()?;
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok.state.disabled_collision_pairs.retain(|(a, b)| {
            !(a.eq_ignore_ascii_case(&rb_a) && b.eq_ignore_ascii_case(&rb_b)
                || a.eq_ignore_ascii_case(&rb_b) && b.eq_ignore_ascii_case(&rb_a))
        });
        Ok(DatumRef::Void)
    }

    fn disable_collision(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let rb_a = player.get_datum(&args[0]).string_value()?;
        let rb_b = player.get_datum(&args[1]).string_value()?;
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok
            .state
            .disabled_collision_pairs
            .push((rb_a, rb_b));
        Ok(DatumRef::Void)
    }

    fn enable_all_collisions(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let rb_name = player.get_datum(&args[0]).string_value()?;
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        havok.state.disabled_collision_pairs.retain(|(a, b)| {
            !a.eq_ignore_ascii_case(&rb_name) && !b.eq_ignore_ascii_case(&rb_name)
        });
        Ok(DatumRef::Void)
    }

    fn disable_all_collisions(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let rb_name = player.get_datum(&args[0]).string_value()?;
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        // Disable collisions with all other rigid bodies
        for rb in &havok.state.rigid_bodies {
            if !rb.name.eq_ignore_ascii_case(&rb_name) {
                havok
                    .state
                    .disabled_collision_pairs
                    .push((rb_name.clone(), rb.name.clone()));
            }
        }
        Ok(DatumRef::Void)
    }
}

/// Build a column-major 4x4 f32 transform from f64 axis-angle rotation + translation.
pub fn axis_angle_to_transform_f64(rot_axis: [f64; 3], rot_angle: f64, pos: [f64; 3]) -> [f32; 16] {
    axis_angle_to_transform(
        rot_axis[0] as f32, rot_axis[1] as f32, rot_axis[2] as f32,
        rot_angle as f32,
        pos[0] as f32, pos[1] as f32, pos[2] as f32,
    )
}

/// Build a column-major 4x4 transform from axis-angle rotation + translation.
fn axis_angle_to_transform(ax: f32, ay: f32, az: f32, angle: f32, px: f32, py: f32, pz: f32) -> [f32; 16] {
    let axis_len = (ax*ax + ay*ay + az*az).sqrt();
    if axis_len < 1e-6 || angle.abs() < 1e-10 {
        return [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            px,  py,  pz,  1.0,
        ];
    }
    let (x, y, z) = (ax / axis_len, ay / axis_len, az / axis_len);
    let c = angle.cos();
    let s = angle.sin();
    let t = 1.0 - c;
    [
        t*x*x + c,   t*x*y + s*z, t*x*z - s*y, 0.0,
        t*x*y - s*z, t*y*y + c,   t*y*z + s*x, 0.0,
        t*x*z + s*y, t*y*z - s*x, t*z*z + c,   0.0,
        px,          py,          pz,          1.0,
    ]
}
