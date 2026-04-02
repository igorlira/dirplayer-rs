use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, DatumType, HavokObjectRef},
    player::{
        cast_lib::CastMemberRef,
        cast_member::CastMemberType,
        reserve_player_mut, DatumRef, ScriptError,
    },
};

pub struct HavokObjectDatumHandlers {}

impl HavokObjectDatumHandlers {
    pub fn get_prop(obj_ref: &DatumRef, prop_name: &str) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let hk_ref = match player.get_datum(obj_ref) {
                Datum::HavokObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected HavokObjectRef".to_string())),
            };
            let member_ref = CastMemberRef {
                cast_lib: hk_ref.cast_lib,
                cast_member: hk_ref.cast_member,
            };
            match hk_ref.object_type.as_str() {
                "rigidBody" => Self::get_rigid_body_prop(player, &member_ref, &hk_ref.name, prop_name),
                "spring" => Self::get_spring_prop(player, &member_ref, &hk_ref.name, prop_name),
                "linearDashpot" => Self::get_linear_dashpot_prop(player, &member_ref, &hk_ref.name, prop_name),
                "angularDashpot" => Self::get_angular_dashpot_prop(player, &member_ref, &hk_ref.name, prop_name),
                "corrector" => Self::get_corrector_prop(player, &member_ref, &hk_ref.name, prop_name),
                _ => Err(ScriptError::new(format!("Unknown Havok object type: {}", hk_ref.object_type))),
            }
        })
    }

    pub fn set_prop(obj_ref: &DatumRef, prop_name: &str, value: DatumRef) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            let hk_ref = match player.get_datum(obj_ref) {
                Datum::HavokObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected HavokObjectRef".to_string())),
            };
            let val = player.get_datum(&value).clone();
            let member_ref = CastMemberRef {
                cast_lib: hk_ref.cast_lib,
                cast_member: hk_ref.cast_member,
            };
            match hk_ref.object_type.as_str() {
                "rigidBody" => Self::set_rigid_body_prop(player, &member_ref, &hk_ref.name, prop_name, val),
                "spring" => Self::set_spring_prop(player, &member_ref, &hk_ref.name, prop_name, val),
                "linearDashpot" => Self::set_linear_dashpot_prop(player, &member_ref, &hk_ref.name, prop_name, val),
                "angularDashpot" => Self::set_angular_dashpot_prop(player, &member_ref, &hk_ref.name, prop_name, val),
                "corrector" => Self::set_corrector_prop(player, &member_ref, &hk_ref.name, prop_name, val),
                _ => Err(ScriptError::new(format!("Unknown Havok object type: {}", hk_ref.object_type))),
            }
        })
    }

    pub fn call(obj_ref: &DatumRef, handler_name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let hk_ref = match player.get_datum(obj_ref) {
                Datum::HavokObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected HavokObjectRef".to_string())),
            };
            let member_ref = CastMemberRef {
                cast_lib: hk_ref.cast_lib,
                cast_member: hk_ref.cast_member,
            };
            match hk_ref.object_type.as_str() {
                "rigidBody" => Self::call_rigid_body(player, &member_ref, &hk_ref.name, handler_name, args),
                "spring" | "linearDashpot" | "angularDashpot" => {
                    Self::call_constraint(player, &member_ref, &hk_ref.object_type, &hk_ref.name, handler_name, args)
                }
                _ => Err(ScriptError::new(format!(
                    "No handler {} for Havok {} object", handler_name, hk_ref.object_type
                ))),
            }
        })
    }

    // --- Rigid Body ---

    fn get_rigid_body_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        // Read all needed values with an immutable borrow first
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let rb = havok.state.rigid_bodies.iter()
            .find(|r| r.name.eq_ignore_ascii_case(rb_name))
            .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;

        let result = match prop {
            "name" => Datum::String(rb.name.clone()),
            "position" => Datum::Vector(rb.position),
            "centerOfMass" | "centerofmass" => Datum::Vector(rb.center_of_mass),
            "rotation" => {
                let axis = rb.rotation_axis;
                let angle = rb.rotation_angle;
                // Drop borrow before alloc
                let axis_ref = player.alloc_datum(Datum::Vector(axis));
                let angle_ref = player.alloc_datum(Datum::Float(angle));
                return Ok(player.alloc_datum(Datum::List(DatumType::List, VecDeque::from([axis_ref, angle_ref]), false)));
            }
            "mass" => Datum::Float(rb.mass),
            "restitution" => Datum::Float(rb.restitution),
            "friction" => Datum::Float(rb.friction),
            "active" => Datum::Int(if rb.active { 1 } else { 0 }),
            "pinned" => Datum::Int(if rb.pinned { 1 } else { 0 }),
            "linearVelocity" | "linearvelocity" => Datum::Vector(rb.linear_velocity),
            "angularVelocity" | "angularvelocity" => Datum::Vector(rb.angular_velocity),
            "linearMomentum" | "linearmomentum" => Datum::Vector(rb.linear_momentum),
            "angularMomentum" | "angularmomentum" => Datum::Vector(rb.angular_momentum),
            "force" => Datum::Vector(rb.force),
            "torque" => Datum::Vector(rb.torque),
            "corrector" => {
                // Drop borrow via return
                return Ok(player.alloc_datum(Datum::HavokObjectRef(HavokObjectRef {
                    cast_lib: member_ref.cast_lib,
                    cast_member: member_ref.cast_member,
                    object_type: "corrector".to_string(),
                    name: rb_name.to_string(),
                })));
            }
            _ => return Err(ScriptError::new(format!("Unknown rigidBody property: {}", prop))),
        };
        // Borrow on member/havok/rb is dropped here since result is an owned Datum
        Ok(player.alloc_datum(result))
    }

    fn set_rigid_body_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        // For rotation, we need to extract list item values via player.get_datum
        // before borrowing the member mutably.
        if prop == "rotation" {
            let (axis, angle) = if let Datum::List(_, items, _) = &value {
                if items.len() >= 2 {
                    let axis = match player.get_datum(&items[0]) {
                        Datum::Vector(v) => *v,
                        _ => return Err(ScriptError::new("Expected vector for rotation axis".to_string())),
                    };
                    let angle = player.get_datum(&items[1]).to_float()?;
                    (Some(axis), Some(angle))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
            if let (Some(axis), Some(angle)) = (axis, angle) {
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                {
                    let rb = havok.state.rigid_bodies.iter_mut()
                        .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                        .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;
                    rb.rotation_axis = axis;
                    rb.rotation_angle = angle;
                }
                // Sync rotation to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            let axis_len = (axis[0]*axis[0] + axis[1]*axis[1] + axis[2]*axis[2]).sqrt();
                            if axis_len > 1e-10 {
                                let scaled_axis = Vector::new(axis[0], axis[1], axis[2]).normalize() * angle;
                                body.set_rotation(rotation_from_angle(scaled_axis), true);
                            }
                        }
                    }
                }
            }
            return Ok(());
        }

        // Extract rotation data from list BEFORE mutable borrow (to satisfy borrow checker)
        let rotation_data: Option<([f64; 3], f64)> = if prop == "rotation" {
            if let Datum::List(_, items, _) = &value {
                if items.len() >= 2 {
                    let axis = if let Datum::Vector(v) = player.get_datum(&items[0]) { Some(*v) } else { None };
                    let angle = player.get_datum(&items[1]).to_float().unwrap_or(0.0);
                    axis.map(|a| (a, angle))
                } else { None }
            } else { None }
        } else { None };

        // Read W3D refs BEFORE mutably borrowing havok (for position sync)
        let (w3d_cast_lib, w3d_cast_member) = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
            match &member.member_type {
                CastMemberType::HavokPhysics(h) => (h.state.w3d_cast_lib, h.state.w3d_cast_member),
                _ => return Err(ScriptError::new("Not a Havok member".to_string())),
            }
        };

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };

        // Track whether W3D sync is needed
        let mut needs_w3d_sync = false;

        // Update the HavokRigidBody fields
        {
            let rb = havok.state.rigid_bodies.iter_mut()
                .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;

            match prop {
                "position" => { if let Datum::Vector(v) = &value { rb.position = *v; needs_w3d_sync = true; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
                "rotation" => {
                    if let Some((axis, angle)) = rotation_data {
                        rb.rotation_axis = axis;
                        rb.rotation_angle = angle;
                        needs_w3d_sync = true;
                    }
                }
                "mass" => { rb.mass = value.to_float()?; }
                "restitution" => { rb.restitution = value.to_float()?; }
                "friction" => { rb.friction = value.to_float()?; }
                "active" => { rb.active = value.int_value()? != 0; }
                "pinned" => { rb.pinned = value.int_value()? != 0; }
                "linearVelocity" | "linearvelocity" => { if let Datum::Vector(v) = &value { rb.linear_velocity = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
                "angularVelocity" | "angularvelocity" => { if let Datum::Vector(v) = &value { rb.angular_velocity = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
                "linearMomentum" | "linearmomentum" => { if let Datum::Vector(v) = &value { rb.linear_momentum = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
                "angularMomentum" | "angularmomentum" => { if let Datum::Vector(v) = &value { rb.angular_momentum = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
                _ => return Err(ScriptError::new(format!("Cannot set rigidBody property: {}", prop))),
            }
        }

        // Collect sync data after property update
        let sync_data = if needs_w3d_sync {
            havok.state.rigid_bodies.iter()
                .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                .map(|rb| (rb.position, rb.rotation_axis, rb.rotation_angle))
        } else { None };

        // rb borrow is dropped -- now sync relevant properties to rapier
        if let Some(ref mut rapier) = havok.state.rapier {
            if let Some(handle) = rapier.body_handles.get(rb_name) {
                if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                    use rapier3d_f64::prelude::*;
                    match prop {
                        "position" => {
                            if let Datum::Vector(v) = &value {
                                body.set_translation(Vector::new(v[0], v[1], v[2]), true);
                            }
                        }
                        "linearVelocity" | "linearvelocity" => {
                            if let Datum::Vector(v) = &value {
                                body.set_linvel(Vector::new(v[0], v[1], v[2]), true);
                            }
                        }
                        "angularVelocity" | "angularvelocity" => {
                            if let Datum::Vector(v) = &value {
                                body.set_angvel(Vector::new(v[0], v[1], v[2]), true);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Sync rigid body position+rotation to W3D model transform
        if let Some((pos, rot_axis, rot_angle)) = sync_data {
            let w3d_ref = CastMemberRef { cast_lib: w3d_cast_lib, cast_member: w3d_cast_member };
            if let Some(w3d_member) = player.movie.cast_manager.find_mut_member_by_ref(&w3d_ref) {
                if let Some(w3d) = w3d_member.member_type.as_shockwave3d_mut() {
                    let t = crate::player::handlers::datum_handlers::cast_member::havok::axis_angle_to_transform_f64(
                        rot_axis, rot_angle, pos,
                    );
                    w3d.runtime_state.node_transforms.insert(rb_name.to_string(), t);
                }
            }
        }

        Ok(())
    }

    fn call_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "applyForce" | "applyforce" => {
                let force = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                {
                    static FORCE_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    let n = FORCE_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if n < 5 || (n % 300 == 0) {
                        let mag = (force[0]*force[0] + force[1]*force[1] + force[2]*force[2]).sqrt();
                        web_sys::console::log_1(&format!(
                            "[HAVOK-FORCE] applyForce on '{}': ({:.1},{:.1},{:.1}) mag={:.1}",
                            rb_name, force[0], force[1], force[2], mag
                        ).into());
                    }
                }
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.force[0] += force[0];
                    rb.force[1] += force[1];
                    rb.force[2] += force[2];
                }
                // Forward to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.add_force(Vector::new(force[0], force[1], force[2]), true);
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "applyForceAtPoint" | "applyforceatpoint" => {
                let force = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let point = match player.get_datum(&args[1]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    // Add linear force
                    rb.force[0] += force[0];
                    rb.force[1] += force[1];
                    rb.force[2] += force[2];
                    // Compute torque = cross(point - (position + centerOfMass), force)
                    let r = [
                        point[0] - rb.position[0] - rb.center_of_mass[0],
                        point[1] - rb.position[1] - rb.center_of_mass[1],
                        point[2] - rb.position[2] - rb.center_of_mass[2],
                    ];
                    rb.torque[0] += r[1]*force[2] - r[2]*force[1];
                    rb.torque[1] += r[2]*force[0] - r[0]*force[2];
                    rb.torque[2] += r[0]*force[1] - r[1]*force[0];
                }
                Ok(DatumRef::Void)
            }
            "applyImpulse" | "applyimpulse" => {
                let impulse = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    if rb.mass > 0.0 {
                        rb.linear_velocity[0] += impulse[0] / rb.mass;
                        rb.linear_velocity[1] += impulse[1] / rb.mass;
                        rb.linear_velocity[2] += impulse[2] / rb.mass;
                    }
                }
                // Forward to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.apply_impulse(Vector::new(impulse[0], impulse[1], impulse[2]), true);
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "applyImpulseAtPoint" | "applyimpulseatpoint" => {
                let impulse = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let point = match player.get_datum(&args[1]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    if rb.mass > 0.0 {
                        rb.linear_velocity[0] += impulse[0] / rb.mass;
                        rb.linear_velocity[1] += impulse[1] / rb.mass;
                        rb.linear_velocity[2] += impulse[2] / rb.mass;
                    }
                }
                // Forward to rapier with point application
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.apply_impulse_at_point(
                                Vector::new(impulse[0], impulse[1], impulse[2]),
                                Vector::new(point[0], point[1], point[2]),
                                true,
                            );
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "applyTorque" | "applytorque" => {
                let torque = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.torque[0] += torque[0];
                    rb.torque[1] += torque[1];
                    rb.torque[2] += torque[2];
                }
                // Forward to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.add_torque(Vector::new(torque[0], torque[1], torque[2]), true);
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "applyAngularImpulse" | "applyangularimpulse" => {
                let impulse = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.angular_velocity[0] += impulse[0];
                    rb.angular_velocity[1] += impulse[1];
                    rb.angular_velocity[2] += impulse[2];
                }
                // Forward to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.apply_torque_impulse(Vector::new(impulse[0], impulse[1], impulse[2]), true);
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "attemptMoveTo" | "attemptmoveto" => {
                let pos = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                // rotation is a list [axis_vector, angle_float] - extract before mut borrow
                let rotation = if args.len() > 1 {
                    let rot = player.get_datum(&args[1]).clone();
                    if let Datum::List(_, items, _) = &rot {
                        if items.len() >= 2 {
                            let axis = match player.get_datum(&items[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector for rotation axis".to_string())) };
                            let angle = player.get_datum(&items[1]).to_float()?;
                            Some((axis, angle))
                        } else { None }
                    } else { None }
                } else { None };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.position = pos;
                    if let Some((axis, angle)) = rotation {
                        rb.rotation_axis = axis;
                        rb.rotation_angle = angle;
                    }
                }
                // Sync position to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.set_translation(Vector::new(pos[0], pos[1], pos[2]), true);
                            if let Some((axis, angle)) = rotation {
                                let axis_len = (axis[0]*axis[0] + axis[1]*axis[1] + axis[2]*axis[2]).sqrt();
                                if axis_len > 1e-10 {
                                    let scaled_axis = Vector::new(axis[0], axis[1], axis[2]).normalize() * angle;
                                    body.set_rotation(rotation_from_angle(scaled_axis), true);
                                }
                            }
                        }
                    }
                }
                // Always return TRUE (move succeeded)
                Ok(player.alloc_datum(Datum::Int(1)))
            }
            "interpolatingMoveTo" | "interpolatingmoveto" => {
                let pos = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.position = pos;
                }
                // Sync position to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.set_translation(Vector::new(pos[0], pos[1], pos[2]), true);
                        }
                    }
                }
                // Return 1.0 (fully moved)
                Ok(player.alloc_datum(Datum::Float(1.0)))
            }
            "correctorMoveTo" | "correctormoveto" => {
                let pos = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.position = pos;
                }
                // Sync position to rapier
                if let Some(ref mut rapier) = havok.state.rapier {
                    if let Some(handle) = rapier.body_handles.get(rb_name) {
                        if let Some(body) = rapier.rigid_body_set.get_mut(*handle) {
                            use rapier3d_f64::prelude::*;
                            body.set_translation(Vector::new(pos[0], pos[1], pos[2]), true);
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "shiftCenterOfMass" | "shiftcenterofmass" => {
                let offset = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                if let Some(rb) = havok.state.rigid_bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.center_of_mass[0] += offset[0];
                    rb.center_of_mass[1] += offset[1];
                    rb.center_of_mass[2] += offset[2];
                }
                Ok(DatumRef::Void)
            }
            "getProp" => {
                let prop = player.get_datum(&args[0]).string_value()?;
                Self::get_rigid_body_prop(player, member_ref, rb_name, &prop)
            }
            _ => Err(ScriptError::new(format!("No handler {} for rigidBody", handler_name))),
        }
    }

    // --- Spring ---

    fn get_spring_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        spring_name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let spring = havok.state.springs.iter()
            .find(|s| s.name.eq_ignore_ascii_case(spring_name))
            .ok_or_else(|| ScriptError::new(format!("Spring '{}' not found", spring_name)))?;

        let result = match prop {
            "name" => Datum::String(spring.name.clone()),
            "pointA" | "pointa" => Datum::Vector(spring.point_a),
            "pointB" | "pointb" => Datum::Vector(spring.point_b),
            "restLength" | "restlength" => Datum::Float(spring.rest_length),
            "elasticity" => Datum::Float(spring.elasticity),
            "damping" => Datum::Float(spring.damping),
            "onCompression" | "oncompression" => Datum::Int(if spring.on_compression { 1 } else { 0 }),
            "onExtension" | "onextension" => Datum::Int(if spring.on_extension { 1 } else { 0 }),
            _ => return Err(ScriptError::new(format!("Unknown spring property: {}", prop))),
        };
        Ok(player.alloc_datum(result))
    }

    fn set_spring_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        spring_name: &str,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let spring = havok.state.springs.iter_mut()
            .find(|s| s.name.eq_ignore_ascii_case(spring_name))
            .ok_or_else(|| ScriptError::new(format!("Spring '{}' not found", spring_name)))?;

        match prop {
            "pointA" | "pointa" => { if let Datum::Vector(v) = &value { spring.point_a = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
            "pointB" | "pointb" => { if let Datum::Vector(v) = &value { spring.point_b = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
            "restLength" | "restlength" => { spring.rest_length = value.to_float()?; }
            "elasticity" => { spring.elasticity = value.to_float()?; }
            "damping" => { spring.damping = value.to_float()?; }
            "onCompression" | "oncompression" => { spring.on_compression = value.int_value()? != 0; }
            "onExtension" | "onextension" => { spring.on_extension = value.int_value()? != 0; }
            _ => return Err(ScriptError::new(format!("Cannot set spring property: {}", prop))),
        }
        Ok(())
    }

    // --- LinearDashpot ---

    fn get_linear_dashpot_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let dp = havok.state.linear_dashpots.iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| ScriptError::new(format!("LinearDashpot '{}' not found", name)))?;

        let result = match prop {
            "name" => Datum::String(dp.name.clone()),
            "pointA" | "pointa" => Datum::Vector(dp.point_a),
            "pointB" | "pointb" => Datum::Vector(dp.point_b),
            "strength" => Datum::Float(dp.strength),
            "damping" => Datum::Float(dp.damping),
            _ => return Err(ScriptError::new(format!("Unknown linearDashpot property: {}", prop))),
        };
        Ok(player.alloc_datum(result))
    }

    fn set_linear_dashpot_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        name: &str,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let dp = havok.state.linear_dashpots.iter_mut()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| ScriptError::new(format!("LinearDashpot '{}' not found", name)))?;

        match prop {
            "pointA" | "pointa" => { if let Datum::Vector(v) = &value { dp.point_a = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
            "pointB" | "pointb" => { if let Datum::Vector(v) = &value { dp.point_b = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } }
            "strength" => { dp.strength = value.to_float()?; }
            "damping" => { dp.damping = value.to_float()?; }
            _ => return Err(ScriptError::new(format!("Cannot set linearDashpot property: {}", prop))),
        }
        Ok(())
    }

    // --- AngularDashpot ---

    fn get_angular_dashpot_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let dp = havok.state.angular_dashpots.iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| ScriptError::new(format!("AngularDashpot '{}' not found", name)))?;

        let result = match prop {
            "name" => Datum::String(dp.name.clone()),
            "damping" => Datum::Float(dp.damping),
            "strength" => Datum::Float(dp.strength),
            "rotation" => {
                let axis = dp.rotation_axis;
                let angle = dp.rotation_angle;
                // Drop borrow before alloc
                let axis_ref = player.alloc_datum(Datum::Vector(axis));
                let angle_ref = player.alloc_datum(Datum::Float(angle));
                return Ok(player.alloc_datum(Datum::List(DatumType::List, VecDeque::from([axis_ref, angle_ref]), false)));
            }
            _ => return Err(ScriptError::new(format!("Unknown angularDashpot property: {}", prop))),
        };
        Ok(player.alloc_datum(result))
    }

    fn set_angular_dashpot_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        name: &str,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        // For rotation, extract list item values before borrowing member mutably
        if prop == "rotation" {
            let (axis, angle) = if let Datum::List(_, items, _) = &value {
                if items.len() >= 2 {
                    let axis = match player.get_datum(&items[0]) {
                        Datum::Vector(v) => *v,
                        _ => return Err(ScriptError::new("Expected vector for rotation axis".to_string())),
                    };
                    let angle = player.get_datum(&items[1]).to_float()?;
                    (Some(axis), Some(angle))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
            if let (Some(axis), Some(angle)) = (axis, angle) {
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                let dp = havok.state.angular_dashpots.iter_mut()
                    .find(|d| d.name.eq_ignore_ascii_case(name))
                    .ok_or_else(|| ScriptError::new(format!("AngularDashpot '{}' not found", name)))?;
                dp.rotation_axis = axis;
                dp.rotation_angle = angle;
            }
            return Ok(());
        }

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let dp = havok.state.angular_dashpots.iter_mut()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| ScriptError::new(format!("AngularDashpot '{}' not found", name)))?;

        match prop {
            "damping" => { dp.damping = value.to_float()?; }
            "strength" => { dp.strength = value.to_float()?; }
            _ => return Err(ScriptError::new(format!("Cannot set angularDashpot property: {}", prop))),
        }
        Ok(())
    }

    // --- Corrector ---

    fn get_corrector_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let rb = havok.state.rigid_bodies.iter()
            .find(|r| r.name.eq_ignore_ascii_case(rb_name))
            .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;
        let c = &rb.corrector;

        let result = match prop {
            "enabled" => Datum::Int(if c.enabled { 1 } else { 0 }),
            "threshold" => Datum::Float(c.threshold),
            "multiplier" => Datum::Float(c.multiplier),
            "level" => Datum::Int(c.level),
            "maxTries" | "maxtries" => Datum::Int(c.max_tries),
            "maxDistance" | "maxdistance" => Datum::Float(c.max_distance),
            _ => return Err(ScriptError::new(format!("Unknown corrector property: {}", prop))),
        };
        Ok(player.alloc_datum(result))
    }

    fn set_corrector_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
        let havok = match &mut member.member_type {
            CastMemberType::HavokPhysics(h) => h,
            _ => return Err(ScriptError::new("Not a Havok member".to_string())),
        };
        let rb = havok.state.rigid_bodies.iter_mut()
            .find(|r| r.name.eq_ignore_ascii_case(rb_name))
            .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;
        let c = &mut rb.corrector;

        match prop {
            "enabled" => { c.enabled = value.int_value()? != 0; }
            "threshold" => { c.threshold = value.to_float()?; }
            "multiplier" => { c.multiplier = value.to_float()?; }
            "level" => { c.level = value.int_value()?; }
            "maxTries" | "maxtries" => { c.max_tries = value.int_value()?; }
            "maxDistance" | "maxdistance" => { c.max_distance = value.to_float()?; }
            _ => return Err(ScriptError::new(format!("Cannot set corrector property: {}", prop))),
        }
        Ok(())
    }

    // --- Shared constraint methods (spring, linearDashpot, angularDashpot) ---

    fn call_constraint(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        object_type: &str,
        name: &str,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "setRigidBodyA" | "setrigidbodya" => {
                let rb_name = player.get_datum(&args[0]).string_value()?;
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                match object_type {
                    "spring" => {
                        if let Some(s) = havok.state.springs.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
                            s.rigid_body_a = Some(rb_name);
                        }
                    }
                    "linearDashpot" => {
                        if let Some(d) = havok.state.linear_dashpots.iter_mut().find(|d| d.name.eq_ignore_ascii_case(name)) {
                            d.rigid_body_a = Some(rb_name);
                        }
                    }
                    "angularDashpot" => {
                        if let Some(d) = havok.state.angular_dashpots.iter_mut().find(|d| d.name.eq_ignore_ascii_case(name)) {
                            d.rigid_body_a = Some(rb_name);
                        }
                    }
                    _ => {}
                }
                Ok(DatumRef::Void)
            }
            "setRigidBodyB" | "setrigidbodyb" => {
                let rb_name_str = player.get_datum(&args[0]).string_value()?;
                let rb_val = if rb_name_str.eq_ignore_ascii_case("none") { None } else { Some(rb_name_str) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &mut member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                match object_type {
                    "spring" => {
                        if let Some(s) = havok.state.springs.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name)) {
                            s.rigid_body_b = rb_val;
                        }
                    }
                    "linearDashpot" => {
                        if let Some(d) = havok.state.linear_dashpots.iter_mut().find(|d| d.name.eq_ignore_ascii_case(name)) {
                            d.rigid_body_b = rb_val;
                        }
                    }
                    "angularDashpot" => {
                        if let Some(d) = havok.state.angular_dashpots.iter_mut().find(|d| d.name.eq_ignore_ascii_case(name)) {
                            d.rigid_body_b = rb_val;
                        }
                    }
                    _ => {}
                }
                Ok(DatumRef::Void)
            }
            "getRigidBodyA" | "getrigidbodya" => {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                let rb_name = match object_type {
                    "spring" => havok.state.springs.iter().find(|s| s.name.eq_ignore_ascii_case(name)).and_then(|s| s.rigid_body_a.clone()),
                    "linearDashpot" => havok.state.linear_dashpots.iter().find(|d| d.name.eq_ignore_ascii_case(name)).and_then(|d| d.rigid_body_a.clone()),
                    "angularDashpot" => havok.state.angular_dashpots.iter().find(|d| d.name.eq_ignore_ascii_case(name)).and_then(|d| d.rigid_body_a.clone()),
                    _ => None,
                };
                match rb_name {
                    Some(n) => Ok(player.alloc_datum(Datum::String(n))),
                    None => Ok(player.alloc_datum(Datum::Symbol("none".to_string()))),
                }
            }
            "getRigidBodyB" | "getrigidbodyb" => {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("Havok member not found".to_string()))?;
                let havok = match &member.member_type {
                    CastMemberType::HavokPhysics(h) => h,
                    _ => return Err(ScriptError::new("Not a Havok member".to_string())),
                };
                let rb_name = match object_type {
                    "spring" => havok.state.springs.iter().find(|s| s.name.eq_ignore_ascii_case(name)).and_then(|s| s.rigid_body_b.clone()),
                    "linearDashpot" => havok.state.linear_dashpots.iter().find(|d| d.name.eq_ignore_ascii_case(name)).and_then(|d| d.rigid_body_b.clone()),
                    "angularDashpot" => havok.state.angular_dashpots.iter().find(|d| d.name.eq_ignore_ascii_case(name)).and_then(|d| d.rigid_body_b.clone()),
                    _ => None,
                };
                match rb_name {
                    Some(n) => Ok(player.alloc_datum(Datum::String(n))),
                    None => Ok(player.alloc_datum(Datum::Symbol("none".to_string()))),
                }
            }
            "getProp" => {
                let prop = player.get_datum(&args[0]).string_value()?;
                match object_type {
                    "spring" => Self::get_spring_prop(player, member_ref, name, &prop),
                    "linearDashpot" => Self::get_linear_dashpot_prop(player, member_ref, name, &prop),
                    "angularDashpot" => Self::get_angular_dashpot_prop(player, member_ref, name, &prop),
                    _ => Err(ScriptError::new(format!("Unknown constraint type: {}", object_type))),
                }
            }
            _ => Err(ScriptError::new(format!(
                "No handler {} for {} '{}'", handler_name, object_type, name
            ))),
        }
    }
}
