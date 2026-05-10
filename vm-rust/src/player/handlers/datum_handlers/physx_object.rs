//! PhysX (AGEIA) nested-object dispatch — handles `body.applyForce(...)`,
//! `joint.setStiffness(...)`, etc. Mirrors `havok_object.rs` 1:1.
//!
//! Property and method names follow the Director Scripting Dictionary
//! chapter 15 (Physics Engine). Orientation is `[vector(axis), angleDeg]`
//! as a two-element list.

use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, DatumType, PhysXObjectRef},
    player::{
        cast_lib::CastMemberRef,
        cast_member::{CastMemberType, PhysXBodyType, PhysXConstraintKind, PhysXShapeKind, PhysXSleepMode},
        reserve_player_mut, DatumRef, ScriptError,
    },
};

pub struct PhysXObjectDatumHandlers {}

impl PhysXObjectDatumHandlers {
    pub fn get_prop(obj_ref: &DatumRef, prop_name: &str) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let px_ref = match player.get_datum(obj_ref) {
                Datum::PhysXObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected PhysXObjectRef".to_string())),
            };
            let member_ref = CastMemberRef {
                cast_lib: px_ref.cast_lib,
                cast_member: px_ref.cast_member,
            };
            match px_ref.object_type.as_str() {
                "rigidBody" => Self::get_rigid_body_prop(player, &member_ref, &px_ref.name, prop_name),
                "spring" | "linearJoint" | "angularJoint" | "d6Joint" | "constraint" => {
                    Self::get_constraint_prop(player, &member_ref, &px_ref.object_type, &px_ref.name, prop_name)
                }
                _ => Err(ScriptError::new(format!("Unknown PhysX object type: {}", px_ref.object_type))),
            }
        })
    }

    pub fn set_prop(obj_ref: &DatumRef, prop_name: &str, value: DatumRef) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            let px_ref = match player.get_datum(obj_ref) {
                Datum::PhysXObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected PhysXObjectRef".to_string())),
            };
            let val = player.get_datum(&value).clone();
            let member_ref = CastMemberRef {
                cast_lib: px_ref.cast_lib,
                cast_member: px_ref.cast_member,
            };
            match px_ref.object_type.as_str() {
                "rigidBody" => Self::set_rigid_body_prop(player, &member_ref, &px_ref.name, prop_name, val),
                "spring" | "linearJoint" | "angularJoint" | "d6Joint" | "constraint" => {
                    Self::set_constraint_prop(player, &member_ref, &px_ref.object_type, &px_ref.name, prop_name, val)
                }
                _ => Err(ScriptError::new(format!("Unknown PhysX object type: {}", px_ref.object_type))),
            }
        })
    }

    pub fn call(obj_ref: &DatumRef, handler_name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let px_ref = match player.get_datum(obj_ref) {
                Datum::PhysXObjectRef(r) => r.clone(),
                _ => return Err(ScriptError::new("Expected PhysXObjectRef".to_string())),
            };
            let member_ref = CastMemberRef {
                cast_lib: px_ref.cast_lib,
                cast_member: px_ref.cast_member,
            };
            match px_ref.object_type.as_str() {
                "rigidBody" => Self::call_rigid_body(player, &member_ref, &px_ref.name, handler_name, args),
                "spring" | "linearJoint" | "angularJoint" | "d6Joint" | "constraint" => {
                    Self::call_constraint(player, &member_ref, &px_ref.object_type, &px_ref.name, handler_name, args)
                }
                _ => Err(ScriptError::new(format!(
                    "No handler {} for PhysX {} object", handler_name, px_ref.object_type
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
        // `properties` returns a per-shape prop list — handle before the
        // immutable borrow of the member because we need multiple alloc_datum.
        if prop.eq_ignore_ascii_case("properties") {
            return Self::get_rigid_body_properties_list(player, member_ref, rb_name);
        }
        // `orientation` returns a Lingo list — needs alloc_datum on each
        // element, so handle it before the immutable borrow of the member.
        if prop.eq_ignore_ascii_case("orientation") {
            let (axis, angle) = {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                let rb = physx.state.bodies.iter()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                    .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;
                ([rb.orientation[0], rb.orientation[1], rb.orientation[2]], rb.orientation[3])
            };
            let axis_ref = player.alloc_datum(Datum::Vector(axis));
            let angle_ref = player.alloc_datum(Datum::Float(angle));
            return Ok(player.alloc_datum(Datum::List(
                DatumType::List, VecDeque::from([axis_ref, angle_ref]), false,
            )));
        }

        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let rb = physx.state.bodies.iter()
            .find(|r| r.name.eq_ignore_ascii_case(rb_name))
            .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;

        let result = match_ci!(prop, {
            "name" => Datum::String(rb.name.clone()),
            "model" => Datum::String(rb.model_name.clone()),
            "position" => Datum::Vector(rb.position),
            "linearVelocity" => Datum::Vector(rb.linear_velocity),
            "angularVelocity" => Datum::Vector(rb.angular_velocity),
            "linearMomentum" => Datum::Vector([
                rb.linear_velocity[0] * rb.mass,
                rb.linear_velocity[1] * rb.mass,
                rb.linear_velocity[2] * rb.mass,
            ]),
            "angularMomentum" => Datum::Vector([0.0, 0.0, 0.0]),
            "mass" => Datum::Float(rb.mass),
            "centerOfMass" => Datum::Vector(rb.center_of_mass),
            "friction" => Datum::Float(rb.friction),
            "restitution" => Datum::Float(rb.restitution),
            "linearDamping" => Datum::Float(rb.linear_damping),
            "angularDamping" => Datum::Float(rb.angular_damping),
            "sleepThreshold" => Datum::Float(rb.sleep_threshold),
            "sleepMode" => Datum::Symbol(if rb.sleep_mode == 0 { "energy" } else { "linearvelocity" }.to_string()),
            "userData" => Datum::Int(rb.user_data),
            "shape" => Datum::Symbol(match rb.shape {
                PhysXShapeKind::Box => "box",
                PhysXShapeKind::Sphere => "sphere",
                PhysXShapeKind::Capsule => "capsule",
                PhysXShapeKind::ConvexShape => "convexshape",
                PhysXShapeKind::ConcaveShape => "concaveshape",
            }.to_string()),
            "type" => Datum::Symbol(match rb.body_type {
                PhysXBodyType::Static => "static",
                PhysXBodyType::Dynamic => "dynamic",
                PhysXBodyType::Kinematic => "kinematic",
            }.to_string()),
            // Shape dimensions — Director's `the properties of rb` getter
            // returns a per-shape prop list (chapter 15: `[#radius, #center]`
            // for sphere, `[#length, #width, #height, #center]` for box, etc).
            // We expose the raw fields here for scripts that want them.
            "radius" => Datum::Float(rb.radius),
            "halfHeight" => Datum::Float(rb.half_height),
            "halfExtents" => Datum::Vector(rb.half_extents),
            "isPinned" => Datum::Int(if rb.pinned { 1 } else { 0 }),
            "axisAffinity" => Datum::Int(if rb.axis_affinity { 1 } else { 0 }),
            // `properties` is per-shape — handled outside the simple Datum
            // branch below so we can build a prop list with multiple alloc
            // calls (the borrow on `rb` would block alloc otherwise).
            _ => return Err(ScriptError::new(format!("Unknown rigidBody property: {}", prop))),
        });
        Ok(player.alloc_datum(result))
    }

    /// Director chapter 15: `the properties of rb` — per-shape prop list.
    ///   #box      → [#length, #width, #height, #center]
    ///   #sphere   → [#radius, #center]
    ///   #capsule  → [#radius, #halfHeight, #center]
    ///   #convex   → [#numvertices, #numfaces, #vertexlist, #face]
    ///   #concave  → [#numvertices, #numfaces, #vertexlist, #face]
    fn get_rigid_body_properties_list(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
    ) -> Result<DatumRef, ScriptError> {
        let (shape, half_extents, radius, half_height, center) = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
            let physx = match &member.member_type {
                CastMemberType::PhysXPhysics(p) => p,
                _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
            };
            let rb = physx.state.bodies.iter()
                .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;
            (rb.shape, rb.half_extents, rb.radius, rb.half_height, rb.center_of_mass)
        };
        let mut props = std::collections::VecDeque::new();
        match shape {
            PhysXShapeKind::Box => {
                let k_len = player.alloc_datum(Datum::Symbol("length".to_string()));
                let v_len = player.alloc_datum(Datum::Float(half_extents[0] * 2.0));
                let k_wid = player.alloc_datum(Datum::Symbol("width".to_string()));
                let v_wid = player.alloc_datum(Datum::Float(half_extents[1] * 2.0));
                let k_hei = player.alloc_datum(Datum::Symbol("height".to_string()));
                let v_hei = player.alloc_datum(Datum::Float(half_extents[2] * 2.0));
                let k_ctr = player.alloc_datum(Datum::Symbol("center".to_string()));
                let v_ctr = player.alloc_datum(Datum::Vector(center));
                props.push_back((k_len, v_len));
                props.push_back((k_wid, v_wid));
                props.push_back((k_hei, v_hei));
                props.push_back((k_ctr, v_ctr));
            }
            PhysXShapeKind::Sphere => {
                let k_r = player.alloc_datum(Datum::Symbol("radius".to_string()));
                let v_r = player.alloc_datum(Datum::Float(radius));
                let k_c = player.alloc_datum(Datum::Symbol("center".to_string()));
                let v_c = player.alloc_datum(Datum::Vector(center));
                props.push_back((k_r, v_r));
                props.push_back((k_c, v_c));
            }
            PhysXShapeKind::Capsule => {
                let k_r = player.alloc_datum(Datum::Symbol("radius".to_string()));
                let v_r = player.alloc_datum(Datum::Float(radius));
                let k_h = player.alloc_datum(Datum::Symbol("halfHeight".to_string()));
                let v_h = player.alloc_datum(Datum::Float(half_height));
                let k_c = player.alloc_datum(Datum::Symbol("center".to_string()));
                let v_c = player.alloc_datum(Datum::Vector(center));
                props.push_back((k_r, v_r));
                props.push_back((k_h, v_h));
                props.push_back((k_c, v_c));
            }
            PhysXShapeKind::ConvexShape | PhysXShapeKind::ConcaveShape => {
                // Director docs: [#numvertices, #numfaces, #vertexlist, #face].
                // We populate from the convex_hull if present; otherwise zeros.
                let (nv, nf) = {
                    let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                        .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                    let physx = match &member.member_type {
                        CastMemberType::PhysXPhysics(p) => p,
                        _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                    };
                    let rb = physx.state.bodies.iter()
                        .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                        .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;
                    if let Some(h) = &rb.convex_hull { (h.verts.len() as i32, h.polygons.len() as i32) }
                    else { (0, 0) }
                };
                let k_nv = player.alloc_datum(Datum::Symbol("numvertices".to_string()));
                let v_nv = player.alloc_datum(Datum::Int(nv));
                let k_nf = player.alloc_datum(Datum::Symbol("numfaces".to_string()));
                let v_nf = player.alloc_datum(Datum::Int(nf));
                let k_vl = player.alloc_datum(Datum::Symbol("vertexlist".to_string()));
                let v_vl = player.alloc_datum(Datum::List(DatumType::List, std::collections::VecDeque::new(), false));
                let k_f = player.alloc_datum(Datum::Symbol("face".to_string()));
                let v_f = player.alloc_datum(Datum::List(DatumType::List, std::collections::VecDeque::new(), false));
                props.push_back((k_nv, v_nv));
                props.push_back((k_nf, v_nf));
                props.push_back((k_vl, v_vl));
                props.push_back((k_f, v_f));
            }
        }
        Ok(player.alloc_datum(Datum::PropList(props, false)))
    }

    fn set_rigid_body_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        // `orientation` setter takes [vector(axis), angleDeg] — extract from
        // the Lingo list before the mutable borrow.
        let orient_data: Option<[f64; 4]> = if prop.eq_ignore_ascii_case("orientation") {
            if let Datum::List(_, items, _) = &value {
                if items.len() >= 2 {
                    let axis = match player.get_datum(&items[0]) {
                        Datum::Vector(v) => *v,
                        _ => return Err(ScriptError::new("orientation expects [vector, angle]".to_string())),
                    };
                    let angle = player.get_datum(&items[1]).to_float()?;
                    Some([axis[0], axis[1], axis[2], angle])
                } else { None }
            } else { None }
        } else { None };

        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let rb = physx.state.bodies.iter_mut()
            .find(|r| r.name.eq_ignore_ascii_case(rb_name))
            .ok_or_else(|| ScriptError::new(format!("Rigid body '{}' not found", rb_name)))?;

        match_ci!(prop, {
            "position" => {
                if let Datum::Vector(v) = &value { rb.position = *v; rb.cached_is_sleeping = false; }
                else { return Err(ScriptError::new("Expected vector".to_string())); }
            },
            "orientation" => {
                if let Some(o) = orient_data {
                    if o[0] == 0.0 && o[1] == 0.0 && o[2] == 0.0 {
                        return Err(ScriptError::new("orientation axis cannot be zero".to_string()));
                    }
                    rb.orientation = o;
                    rb.cached_is_sleeping = false;
                }
            },
            "linearVelocity" => {
                if let Datum::Vector(v) = &value { rb.linear_velocity = *v; rb.cached_is_sleeping = false; }
                else { return Err(ScriptError::new("Expected vector".to_string())); }
            },
            "angularVelocity" => {
                if let Datum::Vector(v) = &value { rb.angular_velocity = *v; rb.cached_is_sleeping = false; }
                else { return Err(ScriptError::new("Expected vector".to_string())); }
            },
            "linearMomentum" => {
                if let Datum::Vector(v) = &value {
                    if rb.mass > 0.0 {
                        rb.linear_velocity = [v[0]/rb.mass, v[1]/rb.mass, v[2]/rb.mass];
                        rb.cached_is_sleeping = false;
                    }
                } else { return Err(ScriptError::new("Expected vector".to_string())); }
            },
            "angularMomentum" => {
                // C# stub: validates actor and returns 0 — no-op.
                if !matches!(&value, Datum::Vector(_)) {
                    return Err(ScriptError::new("Expected vector".to_string()));
                }
            },
            "mass" => { rb.mass = value.to_float()?; },
            "centerOfMass" => {
                if let Datum::Vector(v) = &value { rb.center_of_mass = *v; rb.use_center_of_mass = true; }
                else { return Err(ScriptError::new("Expected vector".to_string())); }
            },
            "friction" => { rb.friction = value.to_float()?; },
            "restitution" => { rb.restitution = value.to_float()?; },
            "linearDamping" => { rb.linear_damping = value.to_float()?; },
            "angularDamping" => { rb.angular_damping = value.to_float()?; },
            "sleepThreshold" => { rb.sleep_threshold = value.to_float()?; },
            "sleepMode" => {
                let s = match &value {
                    Datum::Symbol(s) => s.to_lowercase(),
                    Datum::String(s) => s.to_lowercase(),
                    _ => return Err(ScriptError::new("sleepMode expects #energy or #linearvelocity".to_string())),
                };
                rb.sleep_mode = if s == "linearvelocity" { 1 } else { 0 };
            },
            "userData" => {
                rb.user_data = value.int_value().unwrap_or(0);
            },
            // Shape dimension setters — Director derives these from the 3D
            // model bounds; we expose them as direct setters so test scripts
            // (and the dirplayer-rs movie loader, eventually) can populate
            // them without needing the full model-bounds path.
            "radius" => { rb.radius = value.to_float()?; },
            "halfHeight" => { rb.half_height = value.to_float()?; },
            "halfExtents" => {
                if let Datum::Vector(v) = &value { rb.half_extents = *v; }
                else { return Err(ScriptError::new("halfExtents expects a vector".to_string())); }
            },
            // Director chapter 15: isPinned + axisAffinity boolean setters.
            "isPinned" => { rb.pinned = value.int_value()? != 0; },
            "axisAffinity" => { rb.axis_affinity = value.int_value()? != 0; },
            // PhysX/AGEIA `isSleeping` is a live property: setting it true
            // puts the body to sleep (zero velocities + mark cached_is_sleeping)
            // and setting it false wakes the body up. Director scripts use this
            // to force a body active again before applying a one-shot impulse.
            "isSleeping" => {
                let sleep = value.int_value()? != 0;
                rb.cached_is_sleeping = sleep;
                if sleep {
                    rb.linear_velocity = [0.0, 0.0, 0.0];
                    rb.angular_velocity = [0.0, 0.0, 0.0];
                }
            },
            _ => return Err(ScriptError::new(format!("Cannot set rigidBody property: {}", prop))),
        });
        Ok(())
    }

    fn call_rigid_body(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        rb_name: &str,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match_ci!(handler_name, {
            "applyForce" => {
                let force = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                // optional second arg: position vector — Phase 1 ignores torque from offset
                let _pos: Option<[f64; 3]> = if args.len() > 1 {
                    if let Datum::Vector(v) = player.get_datum(&args[1]) { Some(*v) } else { None }
                } else { None };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    if rb.mass > 0.0 && !matches!(rb.body_type, PhysXBodyType::Static) && !rb.pinned {
                        rb.linear_velocity[0] += force[0] / rb.mass;
                        rb.linear_velocity[1] += force[1] / rb.mass;
                        rb.linear_velocity[2] += force[2] / rb.mass;
                    }
                    rb.cached_is_sleeping = false;
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            "applyTorque" => {
                let torque = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.angular_velocity[0] += torque[0];
                    rb.angular_velocity[1] += torque[1];
                    rb.angular_velocity[2] += torque[2];
                    rb.cached_is_sleeping = false;
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            "applyLinearImpulse" => {
                let imp = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let _pos: Option<[f64; 3]> = if args.len() > 1 {
                    if let Datum::Vector(v) = player.get_datum(&args[1]) { Some(*v) } else { None }
                } else { None };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    if rb.mass > 0.0 && !matches!(rb.body_type, PhysXBodyType::Static) && !rb.pinned {
                        rb.linear_velocity[0] += imp[0] / rb.mass;
                        rb.linear_velocity[1] += imp[1] / rb.mass;
                        rb.linear_velocity[2] += imp[2] / rb.mass;
                    }
                    rb.cached_is_sleeping = false;
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            "applyAngularImpulse" => {
                let imp = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.angular_velocity[0] += imp[0];
                    rb.angular_velocity[1] += imp[1];
                    rb.angular_velocity[2] += imp[2];
                    rb.cached_is_sleeping = false;
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            "attemptMoveTo" => {
                let pos = match player.get_datum(&args[0]) { Datum::Vector(v) => *v, _ => return Err(ScriptError::new("Expected vector".to_string())) };
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    // Phase 1: no collision detection, so the move always succeeds.
                    rb.position = pos;
                }
                Ok(player.alloc_datum(Datum::Int(1)))
            },
            "isSleeping" => {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                let sleeping = physx.state.bodies.iter()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                    .map(|r| r.cached_is_sleeping)
                    .unwrap_or(false);
                Ok(player.alloc_datum(Datum::Int(if sleeping { 1 } else { 0 })))
            },
            "putToSleep" => {
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.cached_is_sleeping = true;
                    rb.linear_velocity = [0.0; 3];
                    rb.angular_velocity = [0.0; 3];
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            "wakeUp" => {
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.cached_is_sleeping = false;
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            "getProp" => {
                let prop = player.get_datum(&args[0]).string_value()?;
                Self::get_rigid_body_prop(player, member_ref, rb_name, &prop)
            },
            "setConvexHull" => {
                // setConvexHull(vertList, faceList)
                //   vertList = list of vector(x,y,z)
                //   faceList = list of lists of vertex indices (CW from outside)
                //
                // PhysX-side this would normally come from cooked mesh data
                // via createProxyTemplate / addProxyTemplate. We expose this
                // direct setter so scripts and the eventual movie loader can
                // populate hulls without the cooking pipeline. Returns 0 on
                // success, -1 on malformed input (silently — we don't crash
                // the player if a script passes bad data).
                if args.len() < 2 {
                    return Ok(player.alloc_datum(Datum::Int(-1)));
                }
                let verts: Vec<[f64; 3]> = match player.get_datum(&args[0]) {
                    Datum::List(_, items, _) => {
                        let mut out = Vec::with_capacity(items.len());
                        for it in items.iter() {
                            match player.get_datum(it) {
                                Datum::Vector(v) => out.push(*v),
                                _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                            }
                        }
                        out
                    }
                    _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                };
                let faces: Vec<Vec<usize>> = match player.get_datum(&args[1]) {
                    Datum::List(_, face_items, _) => {
                        let mut out = Vec::with_capacity(face_items.len());
                        for f in face_items.iter() {
                            let face_indices: Vec<usize> = match player.get_datum(f) {
                                Datum::List(_, idx_items, _) => {
                                    let mut row = Vec::with_capacity(idx_items.len());
                                    for ix in idx_items.iter() {
                                        let n = match player.get_datum(ix).int_value() {
                                            Ok(v) if v >= 0 => v as usize,
                                            _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                                        };
                                        row.push(n);
                                    }
                                    row
                                }
                                _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                            };
                            out.push(face_indices);
                        }
                        out
                    }
                    _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                };

                let hull = match super::cast_member::physx_gu_convex::polygonal_convex(verts, &faces) {
                    Some(h) => h,
                    None => return Ok(player.alloc_datum(Datum::Int(-1))),
                };

                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.convex_hull = Some(hull);
                    // Promote the body's shape to ConvexShape if it was a
                    // generic placeholder. Scripts that explicitly want
                    // ConcaveShape are left alone (the dispatch routes both
                    // through the convex narrowphase for now).
                    if !matches!(rb.shape, PhysXShapeKind::ConvexShape | PhysXShapeKind::ConcaveShape) {
                        rb.shape = PhysXShapeKind::ConvexShape;
                    }
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            "setTriangleMesh" => {
                // setTriangleMesh(vertList, triList)
                //   vertList = list of vector(x,y,z)
                //   triList  = flat list of vertex indices, 3 per triangle
                //              OR list of 3-element index lists.
                //
                // Builds a GuTriangleMesh (RTree-indexed) for #concaveShape
                // bodies. Returns 0 on success, -1 on malformed input.
                if args.len() < 2 {
                    return Ok(player.alloc_datum(Datum::Int(-1)));
                }
                let verts: Vec<[f32; 3]> = match player.get_datum(&args[0]) {
                    Datum::List(_, items, _) => {
                        let mut out = Vec::with_capacity(items.len());
                        for it in items.iter() {
                            match player.get_datum(it) {
                                Datum::Vector(v) => out.push([v[0] as f32, v[1] as f32, v[2] as f32]),
                                _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                            }
                        }
                        out
                    }
                    _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                };
                // Parse triangles. Accept either a flat list of 3*N indices
                // or a list of 3-element sub-lists.
                let tris: Vec<u32> = match player.get_datum(&args[1]) {
                    Datum::List(_, items, _) if items.is_empty() => Vec::new(),
                    Datum::List(_, items, _) => {
                        // Peek first element to detect flat vs nested.
                        let first = player.get_datum(&items[0]);
                        let is_nested = matches!(first, Datum::List(_, _, _));
                        let mut out = Vec::with_capacity(items.len() * if is_nested { 3 } else { 1 });
                        if is_nested {
                            for it in items.iter() {
                                match player.get_datum(it) {
                                    Datum::List(_, idx_items, _) => {
                                        if idx_items.len() != 3 {
                                            return Ok(player.alloc_datum(Datum::Int(-1)));
                                        }
                                        for ix in idx_items.iter() {
                                            let n = match player.get_datum(ix).int_value() {
                                                Ok(v) if v >= 0 => v as u32,
                                                _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                                            };
                                            out.push(n);
                                        }
                                    }
                                    _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                                }
                            }
                        } else {
                            for ix in items.iter() {
                                let n = match player.get_datum(ix).int_value() {
                                    Ok(v) if v >= 0 => v as u32,
                                    _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                                };
                                out.push(n);
                            }
                            if out.len() % 3 != 0 {
                                return Ok(player.alloc_datum(Datum::Int(-1)));
                            }
                        }
                        out
                    }
                    _ => return Ok(player.alloc_datum(Datum::Int(-1))),
                };

                // Validate indices.
                let n_verts = verts.len() as u32;
                for &i in &tris { if i >= n_verts { return Ok(player.alloc_datum(Datum::Int(-1))); } }

                let mesh = super::cast_member::physx_gu_mesh::GuTriangleMesh::build(verts, tris);
                let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &mut member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                if let Some(rb) = physx.state.bodies.iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(rb_name))
                {
                    rb.triangle_mesh = Some(mesh);
                    // Promote the body's shape to ConcaveShape so the dispatch
                    // routes through the triangle-mesh narrowphase.
                    rb.shape = PhysXShapeKind::ConcaveShape;
                }
                Ok(player.alloc_datum(Datum::Int(0)))
            },
            _ => Err(ScriptError::new(format!("No handler {} for rigidBody", handler_name))),
        })
    }

    // --- Constraint (spring, linearJoint, angularJoint, d6Joint) ---

    fn get_constraint_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        _object_type: &str,
        name: &str,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let member = player.movie.cast_manager.find_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let c = physx.state.constraints.iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| ScriptError::new(format!("Constraint '{}' not found", name)))?;

        let result = match_ci!(prop, {
            "name" => Datum::String(c.name.clone()),
            "pointA" => Datum::Vector(c.anchor_a),
            "pointB" => Datum::Vector(c.anchor_b),
            "stiffness" => Datum::Float(c.stiffness),
            "damping" => Datum::Float(c.damping),
            "restLength" | "length" => Datum::Float(c.rest_length),
            "type" | "kind" => Datum::Symbol(match c.kind {
                PhysXConstraintKind::Spring => "spring",
                PhysXConstraintKind::LinearJoint => "linearjoint",
                PhysXConstraintKind::AngularJoint => "angularjoint",
                PhysXConstraintKind::D6Joint => "d6joint",
            }.to_string()),
            _ => return Err(ScriptError::new(format!("Unknown constraint property: {}", prop))),
        });
        Ok(player.alloc_datum(result))
    }

    fn set_constraint_prop(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        _object_type: &str,
        name: &str,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        let member = player.movie.cast_manager.find_mut_member_by_ref(member_ref)
            .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
        let physx = match &mut member.member_type {
            CastMemberType::PhysXPhysics(p) => p,
            _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
        };
        let c = physx.state.constraints.iter_mut()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| ScriptError::new(format!("Constraint '{}' not found", name)))?;

        match_ci!(prop, {
            "pointA" => { if let Datum::Vector(v) = &value { c.anchor_a = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } },
            "pointB" => { if let Datum::Vector(v) = &value { c.anchor_b = *v; } else { return Err(ScriptError::new("Expected vector".to_string())); } },
            "stiffness" => { c.stiffness = value.to_float()?; },
            "damping" => { c.damping = value.to_float()?; },
            "restLength" | "length" => { c.rest_length = value.to_float()?; },
            _ => return Err(ScriptError::new(format!("Cannot set constraint property: {}", prop))),
        });
        Ok(())
    }

    fn call_constraint(
        player: &mut crate::player::DirPlayer,
        member_ref: &CastMemberRef,
        object_type: &str,
        name: &str,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // Suppress unused warnings — these arms are referenced in the macro
        let _ = (PhysXSleepMode::Energy, PhysXSleepMode::LinearVelocity);
        match_ci!(handler_name, {
            "getName" => Ok(player.alloc_datum(Datum::String(name.to_string()))),
            "getRigidBodyA" | "getBodyA" => {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                let body_id = physx.state.constraints.iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name))
                    .and_then(|c| c.body_a);
                if let Some(id) = body_id {
                    if let Some(b) = physx.state.bodies.iter().find(|b| b.id == id) {
                        let body_name = b.name.clone();
                        return Ok(player.alloc_datum(Datum::String(body_name)));
                    }
                }
                Ok(player.alloc_datum(Datum::Symbol("none".to_string())))
            },
            "getRigidBodyB" | "getBodyB" => {
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)
                    .ok_or_else(|| ScriptError::new("PhysX member not found".to_string()))?;
                let physx = match &member.member_type {
                    CastMemberType::PhysXPhysics(p) => p,
                    _ => return Err(ScriptError::new("Not a PhysX member".to_string())),
                };
                let body_id = physx.state.constraints.iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name))
                    .and_then(|c| c.body_b);
                if let Some(id) = body_id {
                    if let Some(b) = physx.state.bodies.iter().find(|b| b.id == id) {
                        let body_name = b.name.clone();
                        return Ok(player.alloc_datum(Datum::String(body_name)));
                    }
                }
                Ok(player.alloc_datum(Datum::Symbol("none".to_string())))
            },
            "getProp" => {
                let prop = player.get_datum(&args[0]).string_value()?;
                Self::get_constraint_prop(player, member_ref, object_type, name, &prop)
            },
            _ => Err(ScriptError::new(format!(
                "No handler {} for {} '{}'", handler_name, object_type, name
            ))),
        })
    }
}
