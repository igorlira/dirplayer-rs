//! PhysX (AGEIA) cast-member Lingo dispatch surface.
//!
//! Property and method names follow the Director Scripting Dictionary
//! chapter 15 (Physics Engine):
//!   - World props: `gravity`, `friction`, `restitution`, `linearDamping`,
//!     `angularDamping`, `contactTolerance`, `sleepThreshold`, `sleepMode`
//!     (#energy / #linearvelocity), `scalingFactor`, `timeStep`,
//!     `timeStepMode` (#equal / #automatic), `subSteps`, `isInitialized`.
//!   - Lifecycle: `init`, `destroy`, `pauseSimulation`, `resumeSimulation`,
//!     `simulate`, `getSimulationTime`.
//!   - Body factories: `createRigidBody`, `createRigidBodyFromProxy`,
//!     `deleteRigidBody`, `getRigidBody`, `getRigidBodies`, `getSleepingBodies`.
//!   - Constraint factories: `createSpring`, `createLinearJoint`,
//!     `createAngularJoint`, `createD6Joint`, `deleteSpring`,
//!     `getSpring`, `getAllSprings`, `getConstraint`, `getAllConstraints`,
//!     `deleteConstraint`.
//!   - Stubs (-7 unavailable): cloth, controller, terrain.

use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, DatumType, PhysXObjectRef},
    player::{
        cast_lib::CastMemberRef,
        cast_member::{
            CastMemberType, PhysXBodyType, PhysXConstraintKind, PhysXRigidBody,
            PhysXShapeKind, PhysXSleepMode, PhysXTimeStepMode,
        },
        reserve_player_mut, DatumRef, ScriptError,
    },
};

pub struct PhysXPhysicsMemberHandlers {}

impl PhysXPhysicsMemberHandlers {
    /// Read a world-level property. Returns an owned `Datum` so the caller
    /// can decide whether to alloc immediately or reuse.
    pub fn get_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        // Scalar props first — no DatumRef alloc needed.
        {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            let s = &physx.state;

            let scalar = match_ci!(prop, {
                "isInitialized" => Some(Datum::Int(if s.initialized { 1 } else { 0 })),
                "gravity" => Some(Datum::Vector(s.gravity)),
                "friction" => Some(Datum::Float(s.friction)),
                "restitution" => Some(Datum::Float(s.restitution)),
                "linearDamping" => Some(Datum::Float(s.linear_damping)),
                "angularDamping" => Some(Datum::Float(s.angular_damping)),
                "contactTolerance" => Some(Datum::Float(s.contact_tolerance)),
                "sleepThreshold" => Some(Datum::Float(s.sleep_threshold)),
                "sleepMode" => Some(Datum::Symbol(match s.sleep_mode {
                    PhysXSleepMode::Energy => "energy",
                    PhysXSleepMode::LinearVelocity => "linearvelocity",
                }.to_string())),
                "scalingFactor" => Some(Datum::Vector(s.scaling_factor)),
                "timeStep" => Some(Datum::Float(s.time_step)),
                "timeStepMode" => Some(Datum::Symbol(match s.time_step_mode {
                    PhysXTimeStepMode::Equal => "equal",
                    PhysXTimeStepMode::Automatic => "automatic",
                }.to_string())),
                "subSteps" => Some(Datum::Int(s.sub_steps as i32)),
                "simulationTime" => Some(Datum::Float(s.sim_time)),
                _ => None,
            });
            if let Some(v) = scalar { return Ok(v); }
        }

        // List-valued props — collect names first, then alloc Vec of refs.
        let (names, list_type): (Vec<String>, &str) = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            let s = &physx.state;
            match_ci!(prop, {
                "rigidBody" => (s.bodies.iter().map(|b| b.name.clone()).collect(), "rigidBody"),
                "spring" => (
                    s.constraints.iter().filter(|c| matches!(c.kind, PhysXConstraintKind::Spring))
                        .map(|c| c.name.clone()).collect(),
                    "spring",
                ),
                "linearJoint" => (
                    s.constraints.iter().filter(|c| matches!(c.kind, PhysXConstraintKind::LinearJoint))
                        .map(|c| c.name.clone()).collect(),
                    "linearJoint",
                ),
                "angularJoint" => (
                    s.constraints.iter().filter(|c| matches!(c.kind, PhysXConstraintKind::AngularJoint))
                        .map(|c| c.name.clone()).collect(),
                    "angularJoint",
                ),
                "d6Joint" => (
                    s.constraints.iter().filter(|c| matches!(c.kind, PhysXConstraintKind::D6Joint))
                        .map(|c| c.name.clone()).collect(),
                    "d6Joint",
                ),
                "constraint" => (
                    s.constraints.iter().map(|c| c.name.clone()).collect(),
                    "constraint",
                ),
                _ => return Err(ScriptError::new(format!(
                    "Cannot get PhysX member property: {}", prop
                ))),
            })
        };

        let items: VecDeque<DatumRef> = names.iter().map(|name| {
            player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib: member_ref.cast_lib,
                cast_member: member_ref.cast_member,
                object_type: list_type.to_string(),
                id: 0,
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
            let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &mut member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            let s = &mut physx.state;

            match_ci!(prop, {
                "gravity" => {
                    if let Datum::Vector(v) = &value { s.gravity = *v; }
                    else { return Err(ScriptError::new("gravity must be a vector".to_string())); }
                    // Wake every dynamic body — matches the C# SetGravity body.
                    super::physx_native::wake_all_dynamic(s);
                    Ok(())
                },
                "friction" => { s.friction = value.to_float()?; Ok(()) },
                "restitution" => { s.restitution = value.to_float()?; Ok(()) },
                "linearDamping" => { s.linear_damping = value.to_float()?; Ok(()) },
                "angularDamping" => { s.angular_damping = value.to_float()?; Ok(()) },
                "contactTolerance" => { s.contact_tolerance = value.to_float()?; Ok(()) },
                "sleepThreshold" => { s.sleep_threshold = value.to_float()?; Ok(()) },
                "sleepMode" => {
                    let sym = match &value {
                        Datum::Symbol(s) => s.to_lowercase(),
                        Datum::String(s) => s.to_lowercase(),
                        _ => return Err(ScriptError::new("sleepMode expects #energy or #linearvelocity".to_string())),
                    };
                    s.sleep_mode = if sym == "linearvelocity" { PhysXSleepMode::LinearVelocity } else { PhysXSleepMode::Energy };
                    Ok(())
                },
                "timeStep" => {
                    let dt = value.to_float()?;
                    if dt > 0.0 { s.time_step = dt; }
                    Ok(())
                },
                "timeStepMode" => {
                    let sym = match &value {
                        Datum::Symbol(s) => s.to_lowercase(),
                        Datum::String(s) => s.to_lowercase(),
                        _ => return Err(ScriptError::new("timeStepMode expects #equal or #automatic".to_string())),
                    };
                    s.time_step_mode = if sym == "automatic" { PhysXTimeStepMode::Automatic } else { PhysXTimeStepMode::Equal };
                    Ok(())
                },
                "subSteps" => {
                    let n = value.int_value()? as u32;
                    s.sub_steps = if n == 0 { 1 } else { n };
                    Ok(())
                },
                _ => Err(ScriptError::new(format!(
                    "Cannot set PhysX member property: {}", prop
                ))),
            })
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
                _ => return Err(ScriptError::new("Cannot call PhysX handler on non-cast-member".to_string())),
            };

            // -- World lifecycle --
            if handler_name.eq_ignore_ascii_case("init") || handler_name.eq_ignore_ascii_case("Initialize") {
                return Self::init(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("destroy") {
                return Self::destroy(player, &member_ref);
            }
            if handler_name.eq_ignore_ascii_case("pauseSimulation") {
                return Self::pause_simulation(player, &member_ref);
            }
            if handler_name.eq_ignore_ascii_case("resumeSimulation") {
                return Self::resume_simulation(player, &member_ref);
            }
            if handler_name.eq_ignore_ascii_case("simulate") {
                return Self::simulate(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("getSimulationTime") {
                return Self::get_simulation_time(player, &member_ref);
            }

            // -- Body factories / lookups --
            if handler_name.eq_ignore_ascii_case("createRigidBody") {
                return Self::create_rigid_body(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("createRigidBodyFromProxy") {
                return Self::create_rigid_body_from_proxy(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("deleteRigidBody") {
                return Self::delete_rigid_body(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("getRigidBody") {
                return Self::get_rigid_body(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("getRigidBodies") {
                return Self::get_rigid_bodies(player, &member_ref, false);
            }
            if handler_name.eq_ignore_ascii_case("getSleepingBodies") || handler_name.eq_ignore_ascii_case("getSleepingRigidBodies") {
                return Self::get_rigid_bodies(player, &member_ref, true);
            }

            // -- Proxy / mesh — Phase 1 stubs, return success / void --
            if handler_name.eq_ignore_ascii_case("createProxyTemplate")
                || handler_name.eq_ignore_ascii_case("addProxyTemplate")
                || handler_name.eq_ignore_ascii_case("loadProxyTemplate")
            {
                return Ok(player.alloc_datum(Datum::Int(0)));
            }

            // -- Constraint factories --
            if handler_name.eq_ignore_ascii_case("createSpring") {
                return Self::create_constraint(player, &member_ref, args, PhysXConstraintKind::Spring);
            }
            if handler_name.eq_ignore_ascii_case("createLinearJoint") {
                return Self::create_constraint(player, &member_ref, args, PhysXConstraintKind::LinearJoint);
            }
            if handler_name.eq_ignore_ascii_case("createAngularJoint") {
                return Self::create_constraint(player, &member_ref, args, PhysXConstraintKind::AngularJoint);
            }
            if handler_name.eq_ignore_ascii_case("createD6Joint") {
                return Self::create_constraint(player, &member_ref, args, PhysXConstraintKind::D6Joint);
            }
            if handler_name.eq_ignore_ascii_case("deleteSpring")
                || handler_name.eq_ignore_ascii_case("deleteConstraint")
                || handler_name.eq_ignore_ascii_case("deleteRigidBodyConstraints")
            {
                return Self::delete_constraint(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("getSpring") {
                return Self::get_constraint_named(player, &member_ref, args, Some(PhysXConstraintKind::Spring));
            }
            if handler_name.eq_ignore_ascii_case("getConstraint") {
                return Self::get_constraint_named(player, &member_ref, args, None);
            }
            if handler_name.eq_ignore_ascii_case("getAllSprings") {
                return Self::get_all_constraints(player, &member_ref, Some(PhysXConstraintKind::Spring));
            }
            if handler_name.eq_ignore_ascii_case("getAllConstraints") {
                return Self::get_all_constraints(player, &member_ref, None);
            }

            // -- Cloth / controller — return -7 (feature unavailable),
            //    matching the AGEIA .o behaviour. --
            if handler_name.eq_ignore_ascii_case("createCloth")
                || handler_name.eq_ignore_ascii_case("deleteCloth")
                || handler_name.eq_ignore_ascii_case("getCloth")
                || handler_name.eq_ignore_ascii_case("getCloths")
                || handler_name.eq_ignore_ascii_case("createClothResource")
                || handler_name.eq_ignore_ascii_case("createController")
                || handler_name.eq_ignore_ascii_case("deleteController")
                || handler_name.eq_ignore_ascii_case("getController")
                || handler_name.eq_ignore_ascii_case("getControllers")
            {
                return Ok(player.alloc_datum(Datum::Int(-7)));
            }

            // -- Terrain (Director chapter 15: createTerrain/createTerrainDesc) --
            if handler_name.eq_ignore_ascii_case("createTerrainDesc") {
                return Self::create_terrain_desc(player, args);
            }
            if handler_name.eq_ignore_ascii_case("createTerrain") {
                return Self::create_terrain(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("deleteTerrain") {
                return Self::delete_terrain(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("getTerrain") {
                return Self::get_terrain(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("getTerrains") {
                return Self::get_terrains(player, &member_ref);
            }

            // -- Collision callbacks (Director chapter 15) --
            if handler_name.eq_ignore_ascii_case("enableCollision") {
                return Self::set_collision_filter(player, &member_ref, args, /*enable*/ true, /*callback*/ false);
            }
            if handler_name.eq_ignore_ascii_case("disableCollision") {
                return Self::set_collision_filter(player, &member_ref, args, false, false);
            }
            if handler_name.eq_ignore_ascii_case("enableCollisionCallback") {
                return Self::set_collision_filter(player, &member_ref, args, true, true);
            }
            if handler_name.eq_ignore_ascii_case("disableCollisionCallback") {
                return Self::set_collision_filter(player, &member_ref, args, false, true);
            }
            if handler_name.eq_ignore_ascii_case("getCollisionDisabledPairs") {
                return Self::get_disabled_pairs(player, &member_ref, /*callback*/ false);
            }
            if handler_name.eq_ignore_ascii_case("getCollisionCallbackDisabledPairs") {
                return Self::get_disabled_pairs(player, &member_ref, true);
            }
            if handler_name.eq_ignore_ascii_case("registerCollisionCallback")
                || handler_name.eq_ignore_ascii_case("registerForCollisions")
            {
                return Self::register_collision_callback(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("removeCollisionCallback")
                || handler_name.eq_ignore_ascii_case("removeCallback")
            {
                return Self::remove_collision_callback(player, &member_ref);
            }
            if handler_name.eq_ignore_ascii_case("notifyCollisions") {
                return Self::notify_collisions(player, &member_ref, args);
            }
            if handler_name.eq_ignore_ascii_case("enableCollisionGroupFlag") {
                // Stub — Director only documents this as "enable group pair";
                // the wrapper has no group state today. Returns 0 success.
                return Ok(player.alloc_datum(Datum::Int(0)));
            }

            // -- Raycast (verbatim Gu::raycast_*) --
            // Director's docs document these as `rayCastClosest` /
            // `rayCastAll`; the AGEIA dynamiks.x32 wrapper exposed them as
            // `getRayCastClosestShape` / `getRayCastAllShapes`. Accept both
            // so movies that follow either spelling work without churn.
            if handler_name.eq_ignore_ascii_case("getRayCastClosestShape")
                || handler_name.eq_ignore_ascii_case("rayCastClosest")
            {
                return Self::raycast(player, &member_ref, args, /*all*/ false);
            }
            if handler_name.eq_ignore_ascii_case("getRayCastAllShapes")
                || handler_name.eq_ignore_ascii_case("rayCastAll")
            {
                return Self::raycast(player, &member_ref, args, true);
            }

            // -- Bounding queries --
            if handler_name.eq_ignore_ascii_case("getBoundingBox") {
                return Self::get_bounding_box(player, &member_ref);
            }
            if handler_name.eq_ignore_ascii_case("getBoundingSphere") {
                return Self::get_bounding_sphere(player, &member_ref);
            }

            // -- Generic getProp / count / getAt -- mirror the Havok pattern --
            if handler_name.eq_ignore_ascii_case("getProp") {
                let prop = player.get_datum(&args[0]).string_value()?;
                let result = Self::get_prop(player, &member_ref, &prop)?;
                return Ok(player.alloc_datum(result));
            }
            if handler_name.eq_ignore_ascii_case("count") {
                let prop = player.get_datum(&args[0]).string_value()?;
                let list_datum = Self::get_prop(player, &member_ref, &prop)?;
                if let Datum::List(_, items, _) = &list_datum {
                    return Ok(player.alloc_datum(Datum::Int(items.len() as i32)));
                }
                return Ok(player.alloc_datum(Datum::Int(0)));
            }
            if handler_name.eq_ignore_ascii_case("getAt") || handler_name.eq_ignore_ascii_case("getPropRef") {
                let prop = player.get_datum(&args[0]).string_value()?;
                let list_datum = Self::get_prop(player, &member_ref, &prop)?;
                if args.len() > 1 {
                    let index = player.get_datum(&args[1]).int_value()?;
                    if let Datum::List(_, items, _) = &list_datum {
                        let idx = (index as usize).saturating_sub(1);
                        if idx < items.len() {
                            return Ok(items[idx].clone());
                        }
                    }
                    return Ok(DatumRef::Void);
                }
                return Ok(player.alloc_datum(list_datum));
            }

            Err(ScriptError::new(format!(
                "No handler {} for PhysX member", handler_name
            )))
        })
    }

    // ===========================================================================
    //  Lifecycle
    // ===========================================================================

    /// init(3dMember, scalingFactor, timeStepMode, timeStep, subStepCount)
    fn init(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let three_d_name = if !args.is_empty() {
            match player.get_datum(&args[0]) {
                Datum::CastMember(r) => {
                    let r = r.to_owned();
                    player.movie.cast_manager.find_member_by_ref(&r)
                        .map(|m| m.name.clone())
                        .unwrap_or_default()
                }
                Datum::String(s) => s.clone(),
                _ => String::new(),
            }
        } else { String::new() };
        let scaling = if args.len() > 1 {
            match player.get_datum(&args[1]) {
                Datum::Vector(v) => *v,
                _ => [1.0, 1.0, 1.0],
            }
        } else { [1.0, 1.0, 1.0] };
        let mode_sym = if args.len() > 2 {
            match player.get_datum(&args[2]) {
                Datum::Symbol(s) | Datum::String(s) => s.to_lowercase(),
                _ => "equal".to_string(),
            }
        } else { "equal".to_string() };
        let time_step = if args.len() > 3 { player.get_datum(&args[3]).to_float().unwrap_or(1.0/60.0) } else { 1.0/60.0 };
        let sub_steps = if args.len() > 4 { player.get_datum(&args[4]).int_value().unwrap_or(1) as u32 } else { 1 };

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let s = &mut physx.state;
        s.three_d_member_name = three_d_name;
        s.scaling_factor = scaling;
        s.time_step_mode = if mode_sym == "automatic" { PhysXTimeStepMode::Automatic } else { PhysXTimeStepMode::Equal };
        if time_step > 0.0 { s.time_step = time_step; }
        s.sub_steps = if sub_steps == 0 { 1 } else { sub_steps };
        s.initialized = true;
        s.paused = false;
        s.sim_time = 0.0;

        Ok(player.alloc_datum(Datum::Int(0)))
    }

    fn destroy(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        physx.state.initialized = false;
        physx.state.bodies.clear();
        physx.state.constraints.clear();
        physx.state.sim_time = 0.0;
        Ok(player.alloc_datum(Datum::Int(0)))
    }

    fn pause_simulation(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        physx.state.paused = true;
        Ok(player.alloc_datum(Datum::Int(0)))
    }

    fn resume_simulation(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        physx.state.paused = false;
        Ok(player.alloc_datum(Datum::Int(0)))
    }

    fn simulate(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // Director docs: simulate() takes no args. The C# port accepts an
        // optional dt for testing — we accept it too.
        let explicit_dt = if !args.is_empty() {
            player.get_datum(&args[0]).to_float().ok()
        } else { None };

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        if !physx.state.initialized { return Ok(player.alloc_datum(Datum::Int(-9))); }
        let dt = match (explicit_dt, physx.state.time_step_mode) {
            (Some(v), _) => v,
            (None, PhysXTimeStepMode::Equal) => physx.state.time_step,
            (None, PhysXTimeStepMode::Automatic) => physx.state.time_step,
        };
        let sub_steps = physx.state.sub_steps;
        super::physx_native::step_native(&mut physx.state, dt, sub_steps);
        Ok(player.alloc_datum(Datum::Int(0)))
    }

    fn get_simulation_time(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let t = physx.state.sim_time;
        Ok(player.alloc_datum(Datum::Float(t)))
    }

    // ===========================================================================
    //  Body factories
    // ===========================================================================

    fn create_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // createRigidBody(rigidBodyName, 3DmodelName, #shape, #type [, #flipNormals])
        if args.len() < 4 {
            return Err(ScriptError::new("createRigidBody expects 4-5 arguments".to_string()));
        }
        let name = player.get_datum(&args[0]).string_value()?;
        let model_name = player.get_datum(&args[1]).string_value()?;
        let shape_sym = match player.get_datum(&args[2]) {
            Datum::Symbol(s) | Datum::String(s) => s.to_lowercase(),
            _ => return Err(ScriptError::new("createRigidBody: shape must be a symbol".to_string())),
        };
        let type_sym = match player.get_datum(&args[3]) {
            Datum::Symbol(s) | Datum::String(s) => s.to_lowercase(),
            _ => return Err(ScriptError::new("createRigidBody: type must be a symbol".to_string())),
        };

        let shape = match shape_sym.as_str() {
            "box" => PhysXShapeKind::Box,
            "sphere" => PhysXShapeKind::Sphere,
            "capsule" => PhysXShapeKind::Capsule,
            "convexshape" | "convex" => PhysXShapeKind::ConvexShape,
            "concaveshape" | "concave" => PhysXShapeKind::ConcaveShape,
            _ => return Err(ScriptError::new(format!("Unknown shape: {}", shape_sym))),
        };
        let body_type = match type_sym.as_str() {
            "static" => PhysXBodyType::Static,
            "dynamic" => PhysXBodyType::Dynamic,
            "kinematic" => PhysXBodyType::Kinematic,
            _ => return Err(ScriptError::new(format!("Unknown body type: {}", type_sym))),
        };

        // Look up the linked 3D model's primitive dimensions (radius for
        // #sphere, half_extents for #box, etc.) so the body has the right
        // shape size. Without this, every body defaulted to radius=1, and
        // sphere-vs-terrain contacts grazed the heightfield with separation
        // ≈ 0 — the solver couldn't generate enough impulse to halt fall,
        // so dynamic bodies sank through the world.
        let (prim_radius, prim_half_extents, prim_half_height, prim_position) = {
            let three_d_member_name = "new"; // ClubMarian's 3D world member
            let _ = three_d_member_name;
            // Walk every Shockwave3D member to find the model by name.
            let mut found = (1.0_f64, [1.0_f64; 3], 1.0_f64, [0.0_f64; 3]);
            for cast in &player.movie.cast_manager.casts {
                for (_, member) in &cast.members {
                    if let crate::player::cast_member::CastMemberType::Shockwave3d(w3d) = &member.member_type {
                        if let Some(scene) = &w3d.parsed_scene {
                            if let Some(node) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&model_name)) {
                                let resource = if !node.model_resource_name.is_empty() {
                                    &node.model_resource_name
                                } else {
                                    &node.resource_name
                                };
                                if let Some(res) = scene.model_resources.get(resource.as_str()) {
                                    let r = res.primitive_radius as f64;
                                    let w = res.primitive_width as f64;
                                    let h = res.primitive_height as f64;
                                    let l = res.primitive_length as f64;
                                    found = (
                                        if r > 0.0 { r } else { 1.0 },
                                        [if w > 0.0 { w * 0.5 } else { 1.0 },
                                         if h > 0.0 { h * 0.5 } else { 1.0 },
                                         if l > 0.0 { l * 0.5 } else { 1.0 }],
                                        // Capsule half-height = half_length minus radius cap (PhysX 3.4 convention).
                                        if l > 0.0 { (l * 0.5 - r).max(0.0) } else { 1.0 },
                                        // Initial body position from the model's local transform [12,13,14].
                                        [node.transform[12] as f64,
                                         node.transform[13] as f64,
                                         node.transform[14] as f64],
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            found
        };

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };

        if physx.state.bodies.iter().any(|b| b.name.eq_ignore_ascii_case(&name)) {
            return Ok(player.alloc_datum(Datum::Void));
        }

        let id = physx.state.next_body_id;
        physx.state.next_body_id = physx.state.next_body_id.wrapping_add(1);
        let mut rb = PhysXRigidBody::default();
        rb.id = id;
        rb.name = name.clone();
        rb.model_name = model_name;
        rb.body_type = body_type;
        rb.shape = shape;
        rb.radius = prim_radius;
        rb.half_extents = prim_half_extents;
        rb.half_height = prim_half_height;
        rb.position = prim_position;
        rb.friction = physx.state.friction;
        rb.restitution = physx.state.restitution;
        rb.linear_damping = physx.state.linear_damping;
        rb.angular_damping = physx.state.angular_damping;
        rb.sleep_threshold = physx.state.sleep_threshold;
        physx.state.bodies.push(rb);

        Ok(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "rigidBody".to_string(),
            id,
            name,
        })))
    }

    fn create_rigid_body_from_proxy(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // createRigidBodyFromProxy(rigidBodyName, 3DmodelName, #type, proxyTemplate [, #flipNormals])
        // Phase 1: ignore the proxy template; treat as createRigidBody with #convexshape.
        if args.len() < 3 {
            return Err(ScriptError::new("createRigidBodyFromProxy expects 3+ arguments".to_string()));
        }
        let name = player.get_datum(&args[0]).string_value()?;
        let model_name = player.get_datum(&args[1]).string_value()?;
        let type_sym = match player.get_datum(&args[2]) {
            Datum::Symbol(s) | Datum::String(s) => s.to_lowercase(),
            _ => return Err(ScriptError::new("createRigidBodyFromProxy: type must be a symbol".to_string())),
        };
        let body_type = match type_sym.as_str() {
            "static" => PhysXBodyType::Static,
            "dynamic" => PhysXBodyType::Dynamic,
            "kinematic" => PhysXBodyType::Kinematic,
            _ => return Err(ScriptError::new(format!("Unknown body type: {}", type_sym))),
        };

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        if physx.state.bodies.iter().any(|b| b.name.eq_ignore_ascii_case(&name)) {
            return Ok(player.alloc_datum(Datum::Void));
        }

        let id = physx.state.next_body_id;
        physx.state.next_body_id = physx.state.next_body_id.wrapping_add(1);
        let mut rb = PhysXRigidBody::default();
        rb.id = id;
        rb.name = name.clone();
        rb.model_name = model_name;
        rb.body_type = body_type;
        rb.shape = PhysXShapeKind::ConvexShape;
        rb.friction = physx.state.friction;
        rb.restitution = physx.state.restitution;
        rb.linear_damping = physx.state.linear_damping;
        rb.angular_damping = physx.state.angular_damping;
        rb.sleep_threshold = physx.state.sleep_threshold;
        physx.state.bodies.push(rb);

        Ok(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "rigidBody".to_string(),
            id,
            name,
        })))
    }

    fn delete_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() { return Ok(player.alloc_datum(Datum::Int(-4))); }
        let name = match player.get_datum(&args[0]) {
            Datum::String(s) => s.clone(),
            Datum::PhysXObjectRef(r) => r.name.clone(),
            _ => return Ok(player.alloc_datum(Datum::Int(-4))),
        };
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let before = physx.state.bodies.len();
        physx.state.bodies.retain(|b| !b.name.eq_ignore_ascii_case(&name));
        let after = physx.state.bodies.len();
        Ok(player.alloc_datum(Datum::Int(if before != after { 0 } else { -8 })))
    }

    fn get_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() { return Ok(player.alloc_datum(Datum::Void)); }
        let name = player.get_datum(&args[0]).string_value()?;
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let body = physx.state.bodies.iter()
            .find(|b| b.name.eq_ignore_ascii_case(&name));
        let id = body.map(|b| b.id);
        let real_name = body.map(|b| b.name.clone());
        match (id, real_name) {
            (Some(id), Some(real_name)) => Ok(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib: member_ref.cast_lib,
                cast_member: member_ref.cast_member,
                object_type: "rigidBody".to_string(),
                id,
                name: real_name,
            }))),
            _ => Ok(player.alloc_datum(Datum::Void)),
        }
    }

    fn get_rigid_bodies(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        only_sleeping: bool,
    ) -> Result<DatumRef, ScriptError> {
        let entries: Vec<(u32, String)> = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            physx.state.bodies.iter()
                .filter(|b| !only_sleeping || b.cached_is_sleeping)
                .map(|b| (b.id, b.name.clone()))
                .collect()
        };
        let items: VecDeque<DatumRef> = entries.into_iter().map(|(id, name)| {
            player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib: member_ref.cast_lib,
                cast_member: member_ref.cast_member,
                object_type: "rigidBody".to_string(),
                id,
                name,
            }))
        }).collect();
        Ok(player.alloc_datum(Datum::List(DatumType::List, items, false)))
    }

    // ===========================================================================
    //  Constraint factories
    // ===========================================================================

    /// Decode a `ConstraintDesc(name, A, B, ptA, ptB, stiff, damp)` arg list.
    /// Director's `ConstraintDesc(...)` builds an opaque value; here we
    /// accept either a propList with those keys or a raw 7-element list.
    fn decode_desc(player: &crate::player::DirPlayer, desc_ref: &DatumRef) -> Option<ConstraintDescDecoded> {
        let d = player.get_datum(desc_ref);
        match d {
            Datum::List(_, items, _) if items.len() >= 7 => {
                let name = match player.get_datum(&items[0]) {
                    Datum::String(s) => s.clone(),
                    Datum::Symbol(s) => s.clone(),
                    _ => return None,
                };
                let body_a = match player.get_datum(&items[1]) {
                    Datum::PhysXObjectRef(r) => Some(r.id),
                    Datum::String(s) => Some(0u32).filter(|_| !s.is_empty()),
                    Datum::Void => None,
                    _ => None,
                };
                let body_b = match player.get_datum(&items[2]) {
                    Datum::PhysXObjectRef(r) => Some(r.id),
                    Datum::Void => None,
                    _ => None,
                };
                let pt_a = match player.get_datum(&items[3]) { Datum::Vector(v) => *v, _ => [0.0; 3] };
                let pt_b = match player.get_datum(&items[4]) { Datum::Vector(v) => *v, _ => [0.0; 3] };
                let stiffness = player.get_datum(&items[5]).to_float().unwrap_or(0.0);
                let damping = player.get_datum(&items[6]).to_float().unwrap_or(0.0);
                Some(ConstraintDescDecoded { name, body_a, body_b, pt_a, pt_b, stiffness, damping })
            }
            Datum::PropList(items, _) => {
                let mut name = String::new();
                let mut body_a: Option<u32> = None;
                let mut body_b: Option<u32> = None;
                let mut pt_a = [0.0; 3];
                let mut pt_b = [0.0; 3];
                let mut stiffness = 0.0;
                let mut damping = 0.0;
                for (k, v) in items.iter() {
                    let key = match player.get_datum(k) {
                        Datum::Symbol(s) | Datum::String(s) => s.to_lowercase(),
                        _ => continue,
                    };
                    let val = player.get_datum(v);
                    match key.as_str() {
                        "name" => if let Datum::String(s) = val { name = s.clone(); },
                        "objecta" | "bodya" | "rigidbodya" => {
                            if let Datum::PhysXObjectRef(r) = val { body_a = Some(r.id); }
                        }
                        "objectb" | "bodyb" | "rigidbodyb" => {
                            if let Datum::PhysXObjectRef(r) = val { body_b = Some(r.id); }
                        }
                        "pointa" | "pca" | "poca" => {
                            if let Datum::Vector(v) = val { pt_a = *v; }
                        }
                        "pointb" | "pcb" | "pocb" => {
                            if let Datum::Vector(v) = val { pt_b = *v; }
                        }
                        "stiffness" => { stiffness = val.to_float().unwrap_or(0.0); }
                        "damping" => { damping = val.to_float().unwrap_or(0.0); }
                        _ => {}
                    }
                }
                Some(ConstraintDescDecoded { name, body_a, body_b, pt_a, pt_b, stiffness, damping })
            }
            _ => None,
        }
    }

    fn create_constraint(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
        kind: PhysXConstraintKind,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new("createConstraint expects at least 1 argument".to_string()));
        }
        let desc = Self::decode_desc(player, &args[0])
            .ok_or_else(|| ScriptError::new("Invalid ConstraintDesc".to_string()))?;
        let extra_length = if args.len() > 1 { player.get_datum(&args[1]).to_float().unwrap_or(0.0) } else { 0.0 };

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };

        if physx.state.constraints.iter().any(|c| c.name.eq_ignore_ascii_case(&desc.name)) {
            return Ok(player.alloc_datum(Datum::Int(-4)));
        }

        let id = physx.state.next_constraint_id;
        physx.state.next_constraint_id = physx.state.next_constraint_id.wrapping_add(1);
        let mut c = crate::player::cast_member::PhysXConstraint::default();
        c.id = id;
        c.name = desc.name.clone();
        c.kind = kind;
        c.body_a = desc.body_a;
        c.body_b = desc.body_b;
        c.anchor_a = desc.pt_a;
        c.anchor_b = desc.pt_b;
        c.stiffness = desc.stiffness;
        c.damping = desc.damping;
        c.rest_length = extra_length;
        physx.state.constraints.push(c);

        let object_type = match kind {
            PhysXConstraintKind::Spring => "spring",
            PhysXConstraintKind::LinearJoint => "linearJoint",
            PhysXConstraintKind::AngularJoint => "angularJoint",
            PhysXConstraintKind::D6Joint => "d6Joint",
        };
        Ok(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: object_type.to_string(),
            id,
            name: desc.name,
        })))
    }

    fn delete_constraint(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() { return Ok(player.alloc_datum(Datum::Int(-4))); }
        let name = match player.get_datum(&args[0]) {
            Datum::String(s) => s.clone(),
            Datum::PhysXObjectRef(r) => r.name.clone(),
            _ => return Ok(player.alloc_datum(Datum::Int(-4))),
        };
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let before = physx.state.constraints.len();
        physx.state.constraints.retain(|c| !c.name.eq_ignore_ascii_case(&name));
        let after = physx.state.constraints.len();
        Ok(player.alloc_datum(Datum::Int(if before != after { 0 } else { -1 })))
    }

    fn get_constraint_named(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
        kind_filter: Option<PhysXConstraintKind>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() { return Ok(player.alloc_datum(Datum::Void)); }
        let name = player.get_datum(&args[0]).string_value()?;
        let entry: Option<(u32, String, PhysXConstraintKind)> = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            physx.state.constraints.iter()
                .find(|c| c.name.eq_ignore_ascii_case(&name) && kind_filter.map_or(true, |k| c.kind == k))
                .map(|c| (c.id, c.name.clone(), c.kind))
        };
        match entry {
            Some((id, real, kind)) => {
                let object_type = match kind {
                    PhysXConstraintKind::Spring => "spring",
                    PhysXConstraintKind::LinearJoint => "linearJoint",
                    PhysXConstraintKind::AngularJoint => "angularJoint",
                    PhysXConstraintKind::D6Joint => "d6Joint",
                };
                Ok(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: object_type.to_string(),
                    id,
                    name: real,
                })))
            }
            None => Ok(player.alloc_datum(Datum::Void)),
        }
    }

    fn get_all_constraints(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        kind_filter: Option<PhysXConstraintKind>,
    ) -> Result<DatumRef, ScriptError> {
        let entries: Vec<(u32, String, PhysXConstraintKind)> = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            physx.state.constraints.iter()
                .filter(|c| kind_filter.map_or(true, |k| c.kind == k))
                .map(|c| (c.id, c.name.clone(), c.kind))
                .collect()
        };
        let items: VecDeque<DatumRef> = entries.into_iter().map(|(id, name, kind)| {
            let object_type = match kind {
                PhysXConstraintKind::Spring => "spring",
                PhysXConstraintKind::LinearJoint => "linearJoint",
                PhysXConstraintKind::AngularJoint => "angularJoint",
                PhysXConstraintKind::D6Joint => "d6Joint",
            };
            player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib: member_ref.cast_lib,
                cast_member: member_ref.cast_member,
                object_type: object_type.to_string(),
                id,
                name,
            }))
        }).collect();
        Ok(player.alloc_datum(Datum::List(DatumType::List, items, false)))
    }

    // ============================================================
    //  Bounding queries
    // ============================================================

    /// `getBoundingBox()` — returns `[minVector, maxVector]`. Director's
    /// docs are loose on the return type; both vectors-as-list and a
    /// 2-element list are seen in real movies. We return a 2-element list
    /// of vectors to match the most common pattern.
    fn get_bounding_box(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef) -> Result<DatumRef, ScriptError> {
        let (lo, hi) = match Self::compute_world_aabb(player, member_ref)? {
            Some(b) => b,
            None => return Ok(player.alloc_datum(Datum::Int(-11))),
        };
        let lo_ref = player.alloc_datum(Datum::Vector(lo));
        let hi_ref = player.alloc_datum(Datum::Vector(hi));
        Ok(player.alloc_datum(Datum::List(DatumType::List, VecDeque::from([lo_ref, hi_ref]), false)))
    }

    /// `getBoundingSphere()` — returns `[center, radius]`.
    fn get_bounding_sphere(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef) -> Result<DatumRef, ScriptError> {
        let (lo, hi) = match Self::compute_world_aabb(player, member_ref)? {
            Some(b) => b,
            None => return Ok(player.alloc_datum(Datum::Int(-11))),
        };
        let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
        let dx = hi[0] - lo[0]; let dy = hi[1] - lo[1]; let dz = hi[2] - lo[2];
        let radius = (dx * dx + dy * dy + dz * dz).sqrt() * 0.5;
        let c_ref = player.alloc_datum(Datum::Vector(center));
        let r_ref = player.alloc_datum(Datum::Float(radius));
        Ok(player.alloc_datum(Datum::List(DatumType::List, VecDeque::from([c_ref, r_ref]), false)))
    }

    /// Walk every body, expand world-space AABB. Mirrors the C#
    /// `World.ExpandBoundsForBody`. Returns None if the world has no bodies.
    fn compute_world_aabb(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef) -> Result<Option<([f64; 3], [f64; 3])>, ScriptError> {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        if physx.state.bodies.is_empty() { return Ok(None); }
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for rb in &physx.state.bodies {
            // Build quat from axis-angle.
            let ang = rb.orientation[3] * std::f64::consts::PI / 180.0 * 0.5;
            let s = ang.sin(); let c = ang.cos();
            let q = [rb.orientation[0] * s, rb.orientation[1] * s, rb.orientation[2] * s, c];
            let (b_lo, b_hi) = Self::body_world_aabb(rb, q);
            for k in 0..3 {
                if b_lo[k] < lo[k] { lo[k] = b_lo[k]; }
                if b_hi[k] > hi[k] { hi[k] = b_hi[k]; }
            }
        }
        Ok(Some((lo, hi)))
    }

    fn body_world_aabb(rb: &PhysXRigidBody, q: [f64; 4]) -> ([f64; 3], [f64; 3]) {
        use super::physx_gu::{q_rotate, v_add, v_sub};
        match rb.shape {
            PhysXShapeKind::Sphere => {
                let r = [rb.radius; 3];
                (v_sub(rb.position, r), v_add(rb.position, r))
            }
            PhysXShapeKind::Capsule => {
                let hh = q_rotate(q, [rb.half_height, 0.0, 0.0]);
                let r = [hh[0].abs() + rb.radius, hh[1].abs() + rb.radius, hh[2].abs() + rb.radius];
                (v_sub(rb.position, r), v_add(rb.position, r))
            }
            _ => {
                let ex = q_rotate(q, [rb.half_extents[0], 0.0, 0.0]);
                let ey = q_rotate(q, [0.0, rb.half_extents[1], 0.0]);
                let ez = q_rotate(q, [0.0, 0.0, rb.half_extents[2]]);
                let r = [
                    ex[0].abs() + ey[0].abs() + ez[0].abs(),
                    ex[1].abs() + ey[1].abs() + ez[1].abs(),
                    ex[2].abs() + ey[2].abs() + ez[2].abs(),
                ];
                (v_sub(rb.position, r), v_add(rb.position, r))
            }
        }
    }

    // ============================================================
    //  Raycast (Director chapter 15)
    // ============================================================

    fn raycast(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef, args: &Vec<DatumRef>, all: bool) -> Result<DatumRef, ScriptError> {
        // Director's documented form is `world.rayCastClosest(origin, direction)`
        // (2 args). The AGEIA Xtra also exposed a 3-arg form with an explicit
        // max distance — accept that for backwards compat. When no distance
        // is supplied, use a large sentinel so practical scenes hit anything
        // along the ray. No-hit / bad-args returns an empty list so scripts
        // that gate on `if rResult <> []` get the documented behaviour.
        let empty = |player: &mut crate::player::DirPlayer| {
            player.alloc_datum(Datum::List(
                crate::director::lingo::datum::DatumType::List,
                VecDeque::new(),
                false,
            ))
        };
        if args.len() < 2 {
            return Ok(empty(player));
        }
        let origin = match player.get_datum(&args[0]) {
            Datum::Vector(v) => *v,
            _ => return Ok(empty(player)),
        };
        let dir = match player.get_datum(&args[1]) {
            Datum::Vector(v) => *v,
            _ => return Ok(empty(player)),
        };
        let distance = if args.len() >= 3 {
            player.get_datum(&args[2]).to_float().unwrap_or(1.0e6)
        } else {
            1.0e6
        };

        // Normalize dir defensively (Director scripts often pass non-unit vectors).
        let dl = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if dl < 1e-10 {
            return Ok(empty(player));
        }
        let dir = [dir[0] / dl, dir[1] / dl, dir[2] / dl];

        // Walk bodies, dispatch by shape.
        let hits = Self::raycast_collect(player, member_ref, origin, dir, distance)?;
        if hits.is_empty() {
            return Ok(empty(player));
        }

        if !all {
            // Closest hit only.
            let mut best = 0usize;
            for i in 1..hits.len() {
                if hits[i].1.distance < hits[best].1.distance { best = i; }
            }
            Self::build_raycast_report(player, member_ref, &hits[best])
        } else {
            // Sorted list of all hits.
            let mut sorted = hits;
            sorted.sort_by(|a, b| a.1.distance.partial_cmp(&b.1.distance).unwrap_or(std::cmp::Ordering::Equal));
            let mut items = VecDeque::new();
            for h in sorted.iter() {
                let r = Self::build_raycast_report(player, member_ref, h)?;
                items.push_back(r);
            }
            Ok(player.alloc_datum(Datum::List(DatumType::List, items, false)))
        }
    }

    /// Walk bodies, run per-shape raycast, return (rb_id, hit) for every hit.
    fn raycast_collect(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        origin: [f64; 3], dir: [f64; 3], distance: f64,
    ) -> Result<Vec<(u32, super::physx_gu_raycast::GuRaycastHit)>, ScriptError> {
        use super::physx_gu_raycast as rc;
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let mut out = Vec::new();
        for rb in &physx.state.bodies {
            let ang = rb.orientation[3] * std::f64::consts::PI / 180.0 * 0.5;
            let s = ang.sin(); let c = ang.cos();
            let q = [rb.orientation[0] * s, rb.orientation[1] * s, rb.orientation[2] * s, c];
            let hit = match rb.shape {
                PhysXShapeKind::Sphere =>
                    rc::raycast_sphere(rb.position, rb.radius, origin, dir, distance),
                PhysXShapeKind::Capsule =>
                    rc::raycast_capsule(rb.position, q, rb.half_height, rb.radius, origin, dir, distance),
                // Box / convex / concave fall through to box AABB until a
                // ray-vs-convex-hull port lands.
                _ => rc::raycast_box(rb.half_extents, q, rb.position, origin, dir, distance),
            };
            if let Some(h) = hit { out.push((rb.id, h)); }
        }
        Ok(out)
    }

    /// Build a Director-style raycast report PropList:
    /// `[#rigidBody: <ref>, #point: vec, #normal: vec, #distance: f]`.
    fn build_raycast_report(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        hit: &(u32, super::physx_gu_raycast::GuRaycastHit),
    ) -> Result<DatumRef, ScriptError> {
        let (rb_id, h) = hit;
        // Look up the body name to attach a PhysXObjectRef.
        let name = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            physx.state.bodies.iter().find(|b| b.id == *rb_id).map(|b| b.name.clone())
        };
        let body_ref = name.map(|n| player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
            cast_lib: member_ref.cast_lib,
            cast_member: member_ref.cast_member,
            object_type: "rigidBody".to_string(),
            id: *rb_id,
            name: n,
        }))).unwrap_or(DatumRef::Void);
        let point = player.alloc_datum(Datum::Vector(h.position));
        let normal = player.alloc_datum(Datum::Vector(h.normal));
        let dist = player.alloc_datum(Datum::Float(h.distance));
        // Director docs (rayCastClosest / rayCastAll): the result is a
        // POSITIONAL list of [rigidBody, contactPoint, contactNormal,
        // distance] — scripts read it as `rResult[1].name`, `rResult[2]`,
        // etc. We previously returned a prop list, which made `rResult[1]`
        // resolve to a (symbol, value) pair / int and broke `.name` access.
        let mut items = VecDeque::new();
        items.push_back(body_ref);
        items.push_back(point);
        items.push_back(normal);
        items.push_back(dist);
        Ok(player.alloc_datum(Datum::List(
            crate::director::lingo::datum::DatumType::List, items, false,
        )))
    }

    // ============================================================
    //  Collision callback dispatch (Director chapter 15)
    // ============================================================

    /// Shared dispatch for enable/disable * Collision/CollisionCallback.
    fn set_collision_filter(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
        enable: bool,
        callback: bool,
    ) -> Result<DatumRef, ScriptError> {
        // Decode 0, 1, or 2 PhysXObjectRef body args.
        let mut names: Vec<String> = Vec::new();
        for a in args.iter().take(2) {
            match player.get_datum(a) {
                Datum::PhysXObjectRef(r) => names.push(r.name.clone()),
                Datum::String(s) => names.push(s.clone()),
                Datum::Void => {} // skip
                _ => {}
            }
        }
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        match (names.len(), enable, callback) {
            // No args → global toggle.
            (0, true,  false) => { physx.state.all_collisions_disabled = false; physx.state.disabled_collision_pairs.clear(); physx.state.body_collision_disabled.clear(); }
            (0, false, false) => { physx.state.all_collisions_disabled = true; physx.state.all_callbacks_disabled = true; }
            (0, true,  true ) => { physx.state.all_callbacks_disabled = false; physx.state.disabled_callback_pairs.clear(); physx.state.body_callback_disabled.clear(); }
            (0, false, true ) => { physx.state.all_callbacks_disabled = true; }
            // One arg → whole-body toggle.
            (1, true,  false) => { physx.state.body_collision_disabled.remove(&names[0]); }
            (1, false, false) => { physx.state.body_collision_disabled.insert(names[0].clone()); physx.state.body_callback_disabled.insert(names[0].clone()); }
            (1, true,  true ) => { physx.state.body_callback_disabled.remove(&names[0]); }
            (1, false, true ) => { physx.state.body_callback_disabled.insert(names[0].clone()); }
            // Two args → pair toggle.
            (_, e, cb) => {
                let key = if names[0] < names[1] { (names[0].clone(), names[1].clone()) } else { (names[1].clone(), names[0].clone()) };
                if !cb {
                    if e { physx.state.disabled_collision_pairs.remove(&key); }
                    else { physx.state.disabled_collision_pairs.insert(key.clone()); physx.state.disabled_callback_pairs.insert(key); }
                } else {
                    if e { physx.state.disabled_callback_pairs.remove(&key); }
                    else { physx.state.disabled_callback_pairs.insert(key); }
                }
            }
        }
        Ok(player.alloc_datum(Datum::Int(0)))
    }

    fn get_disabled_pairs(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef, callback: bool) -> Result<DatumRef, ScriptError> {
        let pairs: Vec<(String, String)> = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            let set = if callback { &physx.state.disabled_callback_pairs } else { &physx.state.disabled_collision_pairs };
            set.iter().cloned().collect()
        };
        let mut items = VecDeque::new();
        for (a, b) in pairs {
            let a_ref = player.alloc_datum(Datum::String(a));
            let b_ref = player.alloc_datum(Datum::String(b));
            let pair = player.alloc_datum(Datum::List(DatumType::List, VecDeque::from([a_ref, b_ref]), false));
            items.push_back(pair);
        }
        Ok(player.alloc_datum(Datum::List(DatumType::List, items, false)))
    }

    /// `registerCollisionCallback(#handler, scriptRef?)`. Stores both fields
    /// on the state; the simulate loop's pending_collisions queue + the
    /// `notifyCollisions` dispatch use them to invoke the Lingo handler.
    fn register_collision_callback(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() { return Ok(player.alloc_datum(Datum::Int(-4))); }
        let handler = match player.get_datum(&args[0]) {
            Datum::Symbol(s) => s.clone(),
            Datum::String(s) => s.clone(),
            _ => return Ok(player.alloc_datum(Datum::Int(-4))),
        };
        let script_ref: Option<DatumRef> = if args.len() >= 2 { Some(args[1].clone()) } else { None };
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        physx.state.collision_callback_handler = Some(handler);
        physx.state.collision_callback_script_ref = script_ref;
        Ok(player.alloc_datum(Datum::Int(0)))
    }

    fn remove_collision_callback(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        physx.state.collision_callback_handler = None;
        physx.state.collision_callback_script_ref = None;
        Ok(player.alloc_datum(Datum::Int(0)))
    }

    /// `notifyCollisions()` — drains the pending_collisions queue and
    /// returns the Director-shaped collision report list. Lingo scripts
    /// then iterate it and dispatch to the registered handler. The
    /// handler-invocation path is async (Lingo handlers can be async),
    /// which is why we return data + leave dispatch to the script side
    /// rather than calling the handler ourselves here.
    fn notify_collisions(player: &mut crate::player::DirPlayer, member_ref: &CastMemberRef, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // Snapshot pending collisions and drain.
        let (cast_lib, cast_member, drained) = {
            let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &mut member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            let mut tmp: Vec<(u32, u32, Vec<[f64; 3]>, Vec<[f64; 3]>)> = Vec::new();
            std::mem::swap(&mut tmp, &mut physx.state.pending_collisions);
            (member_ref.cast_lib, member_ref.cast_member, tmp)
        };

        // Build the collision-report list (Director chapter 15 shape):
        // [
        //   [#objectA: <rb>, #objectB: <rb>, #contactPoints: [..], #contactNormals: [..]],
        //   ...
        // ]
        let mut reports = VecDeque::new();
        for (a_id, b_id, points, normals) in drained {
            // Resolve names (need a re-borrow because we dropped the mut above).
            let (name_a, name_b) = {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                let na = physx.state.bodies.iter().find(|b| b.id == a_id).map(|b| b.name.clone());
                let nb = physx.state.bodies.iter().find(|b| b.id == b_id).map(|b| b.name.clone());
                (na, nb)
            };
            let a_ref = name_a.map(|n| player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib, cast_member, object_type: "rigidBody".to_string(), id: a_id, name: n,
            }))).unwrap_or(DatumRef::Void);
            let b_ref = name_b.map(|n| player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib, cast_member, object_type: "rigidBody".to_string(), id: b_id, name: n,
            }))).unwrap_or(DatumRef::Void);
            let mut pts = VecDeque::new();
            for p in points { pts.push_back(player.alloc_datum(Datum::Vector(p))); }
            let pts_list = player.alloc_datum(Datum::List(DatumType::List, pts, false));
            let mut nms = VecDeque::new();
            for n in normals { nms.push_back(player.alloc_datum(Datum::Vector(n))); }
            let nms_list = player.alloc_datum(Datum::List(DatumType::List, nms, false));
            let key_a = player.alloc_datum(Datum::Symbol("objectA".to_string()));
            let key_b = player.alloc_datum(Datum::Symbol("objectB".to_string()));
            let key_pts = player.alloc_datum(Datum::Symbol("contactPoints".to_string()));
            let key_nms = player.alloc_datum(Datum::Symbol("contactNormals".to_string()));
            let mut props = VecDeque::new();
            props.push_back((key_a, a_ref));
            props.push_back((key_b, b_ref));
            props.push_back((key_pts, pts_list));
            props.push_back((key_nms, nms_list));
            reports.push_back(player.alloc_datum(Datum::PropList(props, false)));
        }
        Ok(player.alloc_datum(Datum::List(DatumType::List, reports, false)))
    }

    // =========================================================================
    //  Terrain (Director chapter 15: createTerrain / createTerrainDesc / ...)
    // =========================================================================

    /// `createTerrainDesc(elevationMatrix, friction, restitution)` — builds an
    /// opaque descriptor consumed by `createTerrain`. We represent it as a
    /// PropList carrying the matrix + scalars; Director scripts treat it as
    /// opaque and pass it back unchanged.
    fn create_terrain_desc(
        player: &mut crate::player::DirPlayer,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Ok(player.alloc_datum(Datum::Int(-1)));
        }
        let matrix_ref = args[0].clone();
        // Validate that arg0 is a list (of lists of numbers). We keep the
        // original Datum reference rather than copying — at createTerrain
        // time we re-decode the matrix from this same reference.
        match player.get_datum(&matrix_ref) {
            Datum::List(_, _, _) => {}
            _ => return Ok(player.alloc_datum(Datum::Int(-1))),
        }
        let friction = if args.len() > 1 {
            player.get_datum(&args[1]).float_value().unwrap_or(0.5) as f64
        } else { 0.5 };
        let restitution = if args.len() > 2 {
            player.get_datum(&args[2]).float_value().unwrap_or(0.0) as f64
        } else { 0.0 };

        let key_matrix = player.alloc_datum(Datum::Symbol("elevationMatrix".to_string()));
        let key_friction = player.alloc_datum(Datum::Symbol("friction".to_string()));
        let key_rest = player.alloc_datum(Datum::Symbol("restitution".to_string()));
        let val_friction = player.alloc_datum(Datum::Float(friction));
        let val_rest = player.alloc_datum(Datum::Float(restitution));
        let mut props = VecDeque::new();
        props.push_back((key_matrix, matrix_ref));
        props.push_back((key_friction, val_friction));
        props.push_back((key_rest, val_rest));
        Ok(player.alloc_datum(Datum::PropList(props, false)))
    }

    /// `createTerrain(name, desc, position, orientation, rowScale, columnScale, heightScale)`
    /// — builds a GuHeightField and stores it as a PhysXTerrain in the world's
    /// `terrains` Vec. Returns a PhysXObjectRef Datum to the new terrain.
    fn create_terrain(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.len() < 7 {
            return Ok(player.alloc_datum(Datum::Int(-1)));
        }
        let name = player.get_datum(&args[0]).string_value()?;

        // Decode descriptor.
        let (matrix_ref, friction, restitution) = match player.get_datum(&args[1]) {
            Datum::PropList(props, _) => {
                let mut matrix_ref = DatumRef::Void;
                let mut fr = 0.5f64;
                let mut rest = 0.0f64;
                for (k, v) in props.iter() {
                    let key = player.get_datum(k).string_value().unwrap_or_default();
                    if key.eq_ignore_ascii_case("elevationMatrix") {
                        matrix_ref = v.clone();
                    } else if key.eq_ignore_ascii_case("friction") {
                        fr = player.get_datum(v).float_value().unwrap_or(0.5) as f64;
                    } else if key.eq_ignore_ascii_case("restitution") {
                        rest = player.get_datum(v).float_value().unwrap_or(0.0) as f64;
                    }
                }
                if matches!(matrix_ref, DatumRef::Void) {
                    return Ok(player.alloc_datum(Datum::Int(-1)));
                }
                (matrix_ref, fr, rest)
            }
            _ => return Ok(player.alloc_datum(Datum::Int(-1))),
        };

        // Decode the elevation matrix — list of lists, row-major.
        let (rows, columns, heights) = match player.get_datum(&matrix_ref) {
            Datum::List(_, row_items, _) => {
                let rows = row_items.len();
                if rows == 0 { return Ok(player.alloc_datum(Datum::Int(-1))); }
                let mut columns = 0usize;
                let mut flat: Vec<f32> = Vec::new();
                for (r_idx, row_ref) in row_items.iter().enumerate() {
                    match player.get_datum(row_ref) {
                        Datum::List(_, col_items, _) => {
                            if r_idx == 0 { columns = col_items.len(); }
                            else if col_items.len() != columns {
                                return Ok(player.alloc_datum(Datum::Int(-1)));
                            }
                            for h in col_items.iter() {
                                let v = player.get_datum(h).float_value().unwrap_or(0.0) as f32;
                                flat.push(v);
                            }
                        }
                        _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                    }
                }
                if columns == 0 { return Ok(player.alloc_datum(Datum::Int(-1))); }
                (rows, columns, flat)
            }
            _ => return Ok(player.alloc_datum(Datum::Int(-1))),
        };

        // Position, orientation, scales.
        let position = match player.get_datum(&args[2]) {
            Datum::Vector(v) => *v,
            _ => [0.0; 3],
        };
        let orientation = match player.get_datum(&args[3]) {
            // Orientation = [vector(ax, ay, az), angle_deg]
            Datum::List(_, items, _) if items.len() == 2 => {
                let axis = match player.get_datum(&items[0]) {
                    Datum::Vector(v) => *v,
                    _ => [1.0, 0.0, 0.0],
                };
                let ang = player.get_datum(&items[1]).float_value().unwrap_or(0.0) as f64;
                [axis[0], axis[1], axis[2], ang]
            }
            _ => [1.0, 0.0, 0.0, 0.0],
        };
        let row_scale = player.get_datum(&args[4]).float_value().unwrap_or(1.0) as f32;
        let column_scale = player.get_datum(&args[5]).float_value().unwrap_or(1.0) as f32;
        let height_scale = player.get_datum(&args[6]).float_value().unwrap_or(1.0) as f32;

        let hf = super::physx_gu_heightfield::GuHeightField::build(
            rows, columns, heights, row_scale, column_scale, height_scale, [0.0; 3],
        );

        // Mutate state to add the terrain.
        let (cast_lib, cast_member, terrain_id) = {
            let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &mut member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            // Reject duplicates.
            if physx.state.terrains.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
                return Ok(player.alloc_datum(Datum::Int(-1)));
            }
            let id = physx.state.next_terrain_id;
            physx.state.next_terrain_id += 1;
            physx.state.terrains.push(crate::player::cast_member::PhysXTerrain {
                id, name: name.clone(),
                height_field: hf,
                friction, restitution,
                position, orientation,
            });
            (member_ref.cast_lib, member_ref.cast_member, id)
        };

        Ok(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
            cast_lib, cast_member,
            object_type: "terrain".to_string(),
            id: terrain_id,
            name,
        })))
    }

    fn delete_terrain(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Ok(player.alloc_datum(Datum::Int(-1)));
        }
        let target_name: String = match player.get_datum(&args[0]) {
            Datum::String(s) => s.clone(),
            Datum::Symbol(s) => s.clone(),
            Datum::PhysXObjectRef(r) if r.object_type == "terrain" => r.name.clone(),
            _ => return Ok(player.alloc_datum(Datum::Int(-1))),
        };
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let n_before = physx.state.terrains.len();
        physx.state.terrains.retain(|t| !t.name.eq_ignore_ascii_case(&target_name));
        let removed = n_before - physx.state.terrains.len();
        Ok(player.alloc_datum(Datum::Int(if removed > 0 { 0 } else { -1 })))
    }

    fn get_terrain(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Ok(player.alloc_datum(Datum::Int(-1)));
        }
        let target_name = player.get_datum(&args[0]).string_value()?;
        let cast_lib = member_ref.cast_lib;
        let cast_member = member_ref.cast_member;
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let found = physx.state.terrains.iter().find(|t| t.name.eq_ignore_ascii_case(&target_name));
        match found {
            Some(t) => Ok(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib, cast_member,
                object_type: "terrain".to_string(),
                id: t.id, name: t.name.clone(),
            }))),
            None => Ok(player.alloc_datum(Datum::Int(-1))),
        }
    }

    fn get_terrains(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
    ) -> Result<DatumRef, ScriptError> {
        let cast_lib = member_ref.cast_lib;
        let cast_member = member_ref.cast_member;
        let names_ids: Vec<(String, u32)> = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            physx.state.terrains.iter().map(|t| (t.name.clone(), t.id)).collect()
        };
        let mut items = VecDeque::new();
        for (name, id) in names_ids {
            items.push_back(player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                cast_lib, cast_member,
                object_type: "terrain".to_string(),
                id, name,
            })));
        }
        Ok(player.alloc_datum(Datum::List(DatumType::List, items, false)))
    }
}

struct ConstraintDescDecoded {
    name: String,
    body_a: Option<u32>,
    body_b: Option<u32>,
    pt_a: [f64; 3],
    pt_b: [f64; 3],
    stiffness: f64,
    damping: f64,
}
