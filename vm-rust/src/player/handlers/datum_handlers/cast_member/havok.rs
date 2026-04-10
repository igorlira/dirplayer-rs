use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, DatumType, HavokObjectRef},
    player::{
        cast_lib::CastMemberRef,
        cast_member::{
            CastMemberType, HavokAngularDashpot, HavokCollisionInterest,
            HavokLinearDashpot, HavokRigidBody, HavokSpring,
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
            "collisionList" | "collisionlist" => {
                // Return collision data from last step
                // Format: [[bodyA, bodyB, cx, cy, cz, nx, ny, nz], ...]
                let collisions: Vec<_> = state.collision_list_cache.iter().map(|c| {
                    (c.body_a.clone(), c.body_b.clone(), c.point, c.normal)
                }).collect();
                drop(member);  // Release borrow
                let mut items = VecDeque::new();
                for (na, nb, pt, nm) in collisions {
                    let sub_items: VecDeque<DatumRef> = VecDeque::from([
                        player.alloc_datum(Datum::String(na)),
                        player.alloc_datum(Datum::String(nb)),
                        player.alloc_datum(Datum::Float(pt[0])),
                        player.alloc_datum(Datum::Float(pt[1])),
                        player.alloc_datum(Datum::Float(pt[2])),
                        player.alloc_datum(Datum::Float(nm[0])),
                        player.alloc_datum(Datum::Float(nm[1])),
                        player.alloc_datum(Datum::Float(nm[2])),
                    ]);
                    items.push_back(player.alloc_datum(Datum::List(DatumType::List, sub_items, false)));
                }
                return Ok(Datum::List(DatumType::List, items, false));
            }
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
                        // Gravity stored in state.gravity — used by havok_physics::step_native
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

    /// Run physics step and return callback info for async invocation.
    /// Returns: (step_result, step_callbacks, collision_callbacks)
    /// Step callbacks: Vec<(handler_name, script_instance, sub_dt)>
    /// Collision callbacks: Vec<(handler_name, script_instance, collision_info_datum)>
    pub fn step_with_callbacks(
        datum: &DatumRef,
        args: &Vec<DatumRef>,
    ) -> Result<(DatumRef, Vec<(String, DatumRef, f64)>, Vec<(String, DatumRef, DatumRef)>), ScriptError> {
        reserve_player_mut(|player| {
            let member_ref = match player.get_datum(datum) {
                Datum::CastMember(r) => r.to_owned(),
                _ => return Err(ScriptError::new("Cannot call Havok handler on non-cast-member".to_string())),
            };

            // Run the physics step
            let step_result = Self::step(player, &member_ref, args)?;

            // Collect step callback and collision interest info as raw data first,
            // then allocate datums afterward (to avoid borrow conflicts with player).
            let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
            let havok = member.and_then(|m| match &m.member_type {
                CastMemberType::HavokPhysics(h) => Some(h),
                _ => None,
            });

            let mut step_cbs: Vec<(String, DatumRef, f64)> = Vec::new();
            // Raw collision data: (handler, instance, body_a, body_b, point, normal)
            let mut raw_collisions: Vec<(String, DatumRef, String, String, [f64;3], [f64;3])> = Vec::new();

            if let Some(havok) = havok {
                // Step callbacks
                let time_step = havok.state.time_step;
                let sub_steps = havok.state.sub_steps;
                let sub_dt = {
                    let time_inc = if !args.is_empty() {
                        player.get_datum(&args[0]).to_float().unwrap_or(time_step)
                    } else {
                        time_step
                    };
                    let num_sub = if args.len() > 1 {
                        player.get_datum(&args[1]).int_value().unwrap_or(sub_steps)
                    } else {
                        sub_steps
                    };
                    if num_sub > 0 { time_inc / num_sub as f64 } else { time_inc }
                };
                for (handler, instance) in &havok.state.step_callbacks {
                    step_cbs.push((handler.clone(), instance.clone(), sub_dt));
                }

                // Collision interests matched against cached contacts
                for contact in &havok.state.collision_list_cache {
                    for interest in &havok.state.collision_interests {
                        if interest.handler_name.is_none() || interest.script_instance.is_none() { continue; }
                        let matches = if interest.rb_name2 == "#all" || interest.rb_name2 == "all" {
                            contact.body_a.eq_ignore_ascii_case(&interest.rb_name1)
                                || contact.body_b.eq_ignore_ascii_case(&interest.rb_name1)
                        } else {
                            (contact.body_a.eq_ignore_ascii_case(&interest.rb_name1) && contact.body_b.eq_ignore_ascii_case(&interest.rb_name2))
                            || (contact.body_a.eq_ignore_ascii_case(&interest.rb_name2) && contact.body_b.eq_ignore_ascii_case(&interest.rb_name1))
                        };
                        if matches {
                            raw_collisions.push((
                                interest.handler_name.clone().unwrap(),
                                interest.script_instance.clone().unwrap(),
                                contact.body_a.clone(), contact.body_b.clone(),
                                contact.point, contact.normal,
                            ));
                        }
                    }
                }
            }
            drop(member);

            // Now allocate datums (requires mutable player, no longer borrowing havok)
            let mut collision_cbs = Vec::new();
            for (handler, instance, ba, bb, pt, nm) in raw_collisions {
                let ba_r = player.alloc_datum(Datum::String(ba));
                let bb_r = player.alloc_datum(Datum::String(bb));
                let cx = player.alloc_datum(Datum::Float(pt[0]));
                let cy = player.alloc_datum(Datum::Float(pt[1]));
                let cz = player.alloc_datum(Datum::Float(pt[2]));
                let nx = player.alloc_datum(Datum::Float(nm[0]));
                let ny = player.alloc_datum(Datum::Float(nm[1]));
                let nz = player.alloc_datum(Datum::Float(nm[2]));
                let info = player.alloc_datum(Datum::List(
                    DatumType::List,
                    VecDeque::from([ba_r, bb_r, cx, cy, cz, nx, ny, nz]),
                    false,
                ));
                collision_cbs.push((handler, instance, info));
            }

            Ok((step_result, step_cbs, collision_cbs))
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

        // Read existing rigid body names and model transforms from the W3D scene
        let (model_names, model_transforms): (Vec<String>, std::collections::HashMap<String, [f32; 16]>) = {
            let w3d_member = player
                .movie
                .cast_manager
                .find_member_by_ref(&w3d_ref);
            if let Some(m) = w3d_member {
                if let Some(w3d) = m.member_type.as_shockwave3d() {
                    if let Some(scene) = &w3d.parsed_scene {
                        let names = scene.nodes.iter().map(|n| n.name.clone()).collect();
                        let transforms: std::collections::HashMap<String, [f32; 16]> = scene.nodes.iter()
                            .map(|n| (n.name.to_lowercase(), n.transform))
                            .collect();
                        (names, transforms)
                    } else {
                        (Vec::new(), std::collections::HashMap::new())
                    }
                } else {
                    (Vec::new(), std::collections::HashMap::new())
                }
            } else {
                (Vec::new(), std::collections::HashMap::new())
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

        // Load HKE meshes into native collision system (for havok_physics.rs)
        if !havok.state.hke_data.is_empty() {
            let hke = super::hke_parser::parse_hke(&havok.state.hke_data);
            let inv_scale = if scale.abs() > 1e-10 { 1.0 / scale } else { 1.0 };
            havok.state.collision_meshes.clear();

            for mesh in &hke.meshes {
                if mesh.vertices.is_empty() || mesh.triangles.is_empty() { continue; }
                let model_xform = model_transforms.get(&mesh.name.to_lowercase());
                let vertices: Vec<[f64; 3]> = mesh.vertices.iter()
                    .map(|v| {
                        let lx = v[0] as f64 * inv_scale;
                        let ly = v[1] as f64 * inv_scale;
                        let lz = v[2] as f64 * inv_scale;
                        if let Some(t) = model_xform {
                            let wx = t[0] as f64*lx + t[4] as f64*ly + t[8] as f64*lz + t[12] as f64;
                            let wy = t[1] as f64*lx + t[5] as f64*ly + t[9] as f64*lz + t[13] as f64;
                            let wz = t[2] as f64*lx + t[6] as f64*ly + t[10] as f64*lz + t[14] as f64;
                            [wx, wy, wz]
                        } else {
                            [lx, ly, lz]
                        }
                    })
                    .collect();
                let mut cmesh = super::havok_physics::CollisionMesh {
                    name: mesh.name.clone(),
                    vertices,
                    triangles: mesh.triangles.clone(),
                    aabb_min: [0.0; 3],
                    aabb_max: [0.0; 3],
                };
                cmesh.compute_aabb();
                havok.state.collision_meshes.push(cmesh);
            }
            web_sys::console::log_1(&format!(
                "Native Havok: loaded {} collision meshes ({} total triangles)",
                havok.state.collision_meshes.len(),
                havok.state.collision_meshes.iter().map(|m| m.triangles.len()).sum::<usize>()
            ).into());
        }

        web_sys::console::log_1(
            &format!(
                "Havok initialized: tolerance={}, scale={}, w3d_models={}, hke_colliders={}",
                tolerance, scale, model_names.len(), havok.state.collision_meshes.len()
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

        // Use native Havok physics (replaces Rapier)
        super::havok_physics::step_native(&mut havok.state, time_increment, num_sub_steps);

        // Detect ground Z on first step if not set
        if havok.state.ground_z < -1e10 {
            // Use the first raycast hit z = 330.70 (known from Director data)
            // TODO: compute from actual HKE mesh geometry
            havok.state.ground_z = 330.70;
        }

        // Diagnostic logging
        {
            static STEP_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = STEP_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 20 || (n % 60 == 0) {
                for rb in &havok.state.rigid_bodies {
                    if rb.is_fixed || rb.name.starts_with("hke_") { continue; }
                    web_sys::console::log_1(&format!(
                        "[HAVOK-NATIVE {}] '{}': pos=({:.1},{:.1},{:.1}) vel=({:.1},{:.1},{:.1}) spd={:.1}",
                        n, rb.name, rb.position[0], rb.position[1], rb.position[2],
                        rb.linear_velocity[0], rb.linear_velocity[1], rb.linear_velocity[2],
                        (rb.linear_velocity[0]*rb.linear_velocity[0]+rb.linear_velocity[1]*rb.linear_velocity[1]+rb.linear_velocity[2]*rb.linear_velocity[2]).sqrt(),
                    ).into());
                }
            }
        }

        // Collect W3D sync data using quaternion-based transform builder
        let w3d_cast_lib = havok.state.w3d_cast_lib;
        let w3d_cast_member = havok.state.w3d_cast_member;
        let sync_data: Vec<(String, [f32; 16])> = havok.state.rigid_bodies.iter()
            .filter(|rb| !rb.is_fixed && rb.active)
            .map(|rb| {
                let t = super::havok_physics::build_sync_transform(
                    rb.position, rb.orientation, rb.center_of_mass,
                );
                (rb.name.clone(), t)
            })
            .collect();

        // Clear forces (already done in step_native, but ensure clean state)

        // Drop havok borrow, sync to W3D
        drop(member);
        let w3d_ref = CastMemberRef { cast_lib: w3d_cast_lib, cast_member: w3d_cast_member };
        if let Some(w3d_member) = player.movie.cast_manager.find_mut_member_by_ref(&w3d_ref) {
            if let Some(w3d) = w3d_member.member_type.as_shockwave3d_mut() {
                for (name, t) in &sync_data {
                    if t.iter().any(|v| !v.is_finite()) { continue; }
                    w3d.runtime_state.node_transforms.insert(name.clone(), *t);
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

        // Read model's initial transform + mesh bounding box + vertices/faces from W3D scene
        let (initial_position, mesh_half_extents, mesh_vertices, mesh_faces) = {
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
                    let pos = w3d.runtime_state.node_transforms
                        .get(&model_name)
                        .map(|t| [t[12] as f64, t[13] as f64, t[14] as f64])
                        .unwrap_or([0.0; 3]);

                    // Collect vertices + triangle indices from every CLOD submesh,
                    // offsetting face indices so they remain valid in the merged buffer.
                    let node = w3d.parsed_scene.as_ref()
                        .and_then(|s| s.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&model_name)));
                    let res_name = node.map(|n| {
                        if !n.model_resource_name.is_empty() { &n.model_resource_name }
                        else { &n.resource_name }
                    });
                    let (half_ext, verts, faces) = res_name
                        .and_then(|rn| w3d.parsed_scene.as_ref().and_then(|s| s.clod_meshes.get(rn.as_str())))
                        .map(|meshes| {
                            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
                            let mut all_verts: Vec<[f64; 3]> = Vec::new();
                            let mut all_faces: Vec<[u32; 3]> = Vec::new();
                            for mesh in meshes {
                                let offset = all_verts.len() as u32;
                                for p in &mesh.positions {
                                    for i in 0..3 { mn[i] = mn[i].min(p[i]); mx[i] = mx[i].max(p[i]); }
                                    all_verts.push([p[0] as f64, p[1] as f64, p[2] as f64]);
                                }
                                for f in &mesh.faces {
                                    all_faces.push([f[0] + offset, f[1] + offset, f[2] + offset]);
                                }
                            }
                            let he = [(mx[0]-mn[0]) as f64 / 2.0, (mx[1]-mn[1]) as f64 / 2.0, (mx[2]-mn[2]) as f64 / 2.0];
                            (he, all_verts, all_faces)
                        })
                        .unwrap_or(([10.0, 10.0, 10.0], Vec::new(), Vec::new()));

                    (pos, half_ext, verts, faces)
                } else {
                    ([0.0; 3], [10.0, 10.0, 10.0], Vec::new(), Vec::new())
                }
            } else {
                ([0.0; 3], [10.0, 10.0, 10.0], Vec::new(), Vec::new())
            }
        };

        // Compute unit inertia from the actual mesh geometry.
        // This matches Havok's InertialTensorComputer pipeline (PPC 0x5d3c0).
        // Fall back to an AABB-box approximation if the mesh is unavailable or degenerate.
        let unit_inertia = crate::player::handlers::datum_handlers::cast_member::havok_physics
            ::compute_polyhedron_unit_inertia(&mesh_vertices, &mesh_faces)
            .map(|(ui, _com, _vol)| ui)
            .unwrap_or_else(|| crate::player::handlers::datum_handlers::cast_member::havok_physics
                ::box_unit_inertia(mesh_half_extents));

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
        rb.inertia_half_extents = mesh_half_extents;
        rb.unit_inertia_tensor = unit_inertia;
        // Apply mass → finalise inertia / inverseInertia (PPC setMass 0x4c930)
        crate::player::handlers::datum_handlers::cast_member::havok_physics::recompute_body_inertia(
            mass,
            rb.unit_inertia_tensor,
            &mut rb.inertia_tensor,
            &mut rb.inverse_inertia_tensor,
            &mut rb.inverse_mass,
        );

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
