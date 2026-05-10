//! PhysX (AGEIA) native pipeline — Phase 2.
//!
//! Full canonical PhysX 3.4 simulate pipeline:
//!   1. Integrate velocities (gravity + damping, forward Euler).
//!   2. Apply spring + dashpot forces (legacy NxSpringAndDamperEffector model).
//!   3. Broadphase (O(n²) AABB sweep — sweep-and-prune is later work).
//!   4. Narrowphase: dispatches to verbatim PhysX 3.4 ports in [`super::physx_gu`].
//!   5. Iterative impulse-based velocity solver (Catto-style sequential
//!      impulses, 4 GS iterations matching `NxSceneDesc::solverIterationCount`).
//!   6. Integrate positions + orientations.
//!
//! Convention: the `Gu::contact*` ports use PhysX's "normal: B→A,
//! separation: <0 ⇒ penetrating" — flipped to "normal: A→B, penetration > 0"
//! at the boundary in [`build_contacts`]. The solver's effective-mass /
//! Baumgarte / friction-cone math.

use crate::player::cast_member::{
    PhysXBodyType, PhysXConstraintKind, PhysXPhysicsState, PhysXShapeKind,
};
use super::physx_gu::{
    self as gu, contact_box_box, contact_capsule_capsule, contact_sphere_box, contact_sphere_capsule,
    contact_sphere_sphere, q_integrate, q_rotate, q_rotate_inv, v_add, v_cross, v_dot, v_len_sq,
    v_mul, v_neg, v_sub, GuContactBuffer,
};

/// Pair-data byte for box-box warm-starting (matches PhysX
/// `PxsContactManager::mPairData`). Indexed by (body_i, body_j).
type PairKey = (usize, usize);

/// Per-pair contact data fed into the iterated solver.
struct ContactConstraint {
    body_a: usize,
    body_b: usize,
    /// When true, body_b is treated as a static collider (a terrain).
    /// `body_b` is set to `body_a` (self-pair) and is ignored — the solver
    /// only applies impulses to body_a. This avoids needing to materialize
    /// the terrain as a synthetic rigid body in `state.bodies` just to
    /// satisfy the index-based lookup.
    body_b_is_static_terrain: bool,
    point: [f64; 3],
    normal: [f64; 3],     // A→B convention (post-flip from PhysX's B→A)
    penetration: f64,     // positive when penetrating
    friction: f64,
    restitution: f64,
    // Cached row state set by Prepare.
    eff_mass_n: f64,
    eff_mass_t1: f64,
    eff_mass_t2: f64,
    bias_n: f64,
    tan1: [f64; 3],
    tan2: [f64; 3],
    ra: [f64; 3],
    rb: [f64; 3],
    impulse_n: f64,
    impulse_t1: f64,
    impulse_t2: f64,
}

/// Step the world by `dt` seconds, sub-stepping `sub_steps` times.
/// Mirrors `havok_physics::step_native(state, dt, sub_steps)` and the
/// C# `PxsContext::Simulate(dt)`.
pub fn step_native(state: &mut PhysXPhysicsState, dt: f64, sub_steps: u32) {
    if state.paused || !state.initialized { return; }
    // Clear pending collisions at the start of a tick. The narrowphase
    // appends a (bodyA_id, bodyB_id, points, normals) entry per colliding
    // pair on the LAST substep so Director's notifyCollisions only sees
    // the freshest set.
    state.pending_collisions.clear();
    let n = sub_steps.max(1);
    let h = dt / n as f64;
    for substep in 0..n {
        let is_last = substep == n - 1;
        sub_step(state, h, is_last);
    }
    state.sim_time += dt;
}

/// Canonical (min, max) pair key for collision filter lookups. Mirrors C#'s
/// `World.PairKey` so pairs added via `disableCollision(A,B)` find the same
/// key whether the broadphase iterates (A,B) or (B,A).
fn pair_key_names(a: &str, b: &str) -> (String, String) {
    if a < b { (a.to_string(), b.to_string()) } else { (b.to_string(), a.to_string()) }
}

fn sub_step(state: &mut PhysXPhysicsState, dt: f64, is_last: bool) {
    // ---- 1. Apply gravity + damping to velocities ----
    let g = state.gravity;
    let lin_keep = (1.0 - state.linear_damping * dt).max(0.0);
    let ang_keep = (1.0 - state.angular_damping * dt).max(0.0);
    for body in state.bodies.iter_mut() {
        if matches!(body.body_type, PhysXBodyType::Static) || body.pinned { continue; }
        if body.cached_is_sleeping { continue; }
        if matches!(body.body_type, PhysXBodyType::Dynamic) {
            body.linear_velocity = v_add(body.linear_velocity, v_mul(g, dt));
        }
        let body_lin_keep = (1.0 - body.linear_damping * dt).max(0.0).min(lin_keep.max(1.0));
        let body_ang_keep = (1.0 - body.angular_damping * dt).max(0.0).min(ang_keep.max(1.0));
        body.linear_velocity = v_mul(body.linear_velocity, body_lin_keep.min(lin_keep));
        body.angular_velocity = v_mul(body.angular_velocity, body_ang_keep.min(ang_keep));
    }

    // ---- 2. Apply spring + dashpot forces ----
    apply_spring_forces(state, dt);

    // ---- 3. Broadphase (O(n²) AABB sweep) ----
    let aabbs: Vec<([f64; 3], [f64; 3])> = state.bodies.iter().map(compute_aabb).collect();
    let mut pairs: Vec<PairKey> = Vec::new();
    let global_off = state.all_collisions_disabled;
    for i in 0..state.bodies.len() {
        let bi = &state.bodies[i];
        for j in (i + 1)..state.bodies.len() {
            let bj = &state.bodies[j];
            // No pair if both static.
            if matches!(bi.body_type, PhysXBodyType::Static) && matches!(bj.body_type, PhysXBodyType::Static) {
                continue;
            }
            // Director collision filter (chapter 15): disableCollision and
            // friends — global off, body off, or pair off all skip the pair.
            if global_off { continue; }
            if state.body_collision_disabled.contains(&bi.name) { continue; }
            if state.body_collision_disabled.contains(&bj.name) { continue; }
            let key = pair_key_names(&bi.name, &bj.name);
            if state.disabled_collision_pairs.contains(&key) { continue; }
            if aabb_overlap(&aabbs[i], &aabbs[j]) { pairs.push((i, j)); }
        }
    }

    // ---- 4. Narrowphase ----
    let mut constraints: Vec<ContactConstraint> = Vec::new();
    let mut buffer = GuContactBuffer::new();

    // (4a) Body-vs-terrain pairs. Walked separately from the body×body
    // broadphase since terrains live in `state.terrains` (static-only,
    // not in the bodies Vec). Each terrain is treated as a static collider.
    if !state.terrains.is_empty() {
        for body_idx in 0..state.bodies.len() {
            let body = &state.bodies[body_idx];
            // Skip static-vs-static.
            if matches!(body.body_type, PhysXBodyType::Static) { continue; }
            // Honor body-level callback / collision filter.
            if state.all_collisions_disabled { continue; }
            if state.body_collision_disabled.contains(&body.name) { continue; }
            for terrain_idx in 0..state.terrains.len() {
                let n_before = constraints.len();
                build_terrain_contacts(state, body_idx, terrain_idx, &mut constraints);
                if is_last && constraints.len() > n_before {
                    let body_id = state.bodies[body_idx].id;
                    let terrain = &state.terrains[terrain_idx];
                    let name_a = state.bodies[body_idx].name.clone();
                    let name_b = terrain.name.clone();
                    let key = pair_key_names(&name_a, &name_b);
                    let callbacks_off = state.all_callbacks_disabled
                        || state.body_callback_disabled.contains(&name_a)
                        || state.body_callback_disabled.contains(&name_b)
                        || state.disabled_callback_pairs.contains(&key);
                    if !callbacks_off {
                        let mut points = Vec::with_capacity(constraints.len() - n_before);
                        let mut normals = Vec::with_capacity(constraints.len() - n_before);
                        for c in &constraints[n_before..] {
                            points.push(c.point);
                            normals.push(c.normal);
                        }
                        state.pending_collisions.push((body_id, terrain.id, points, normals));
                    }
                }
            }
        }
    }

    for (i, j) in pairs.iter().copied() {
        let n_before = constraints.len();
        build_contacts(state, i, j, &mut buffer, &mut constraints);
        // Capture contacts for collision-callback dispatch on the last
        // substep only (matches how PhysX flushes contact reports at
        // `PxScene::fetchResults`).
        if is_last && constraints.len() > n_before {
            let body_a_id = state.bodies[i].id;
            let body_b_id = state.bodies[j].id;
            let name_a = state.bodies[i].name.clone();
            let name_b = state.bodies[j].name.clone();
            // Honor pair-callback filter (chapter 15 disableCollisionCallback).
            let key = pair_key_names(&name_a, &name_b);
            let callbacks_off = state.all_callbacks_disabled
                || state.body_callback_disabled.contains(&name_a)
                || state.body_callback_disabled.contains(&name_b)
                || state.disabled_callback_pairs.contains(&key);
            if !callbacks_off {
                let mut points = Vec::with_capacity(constraints.len() - n_before);
                let mut normals = Vec::with_capacity(constraints.len() - n_before);
                for c in &constraints[n_before..] {
                    points.push(c.point);
                    normals.push(c.normal);
                }
                state.pending_collisions.push((body_a_id, body_b_id, points, normals));
            }
        }
    }

    // ---- 5. Constraint setup + solve ----
    let baumgarte = 0.2;
    let slop = 0.005;
    let rest_threshold = 1.0;
    for c in constraints.iter_mut() {
        prepare_constraint(c, &state.bodies, dt, baumgarte, slop, rest_threshold);
    }
    let velocity_iterations = 4;
    if state.use_soa_solver {
        // Verbatim PhysX 3.4 SoA solver path. Body-vs-terrain pairs (which
        // use the static-terrain sentinel and don't have a real body_b) stay
        // on the AoS path; everything else routes through PxsSolverSoa.
        run_soa_solver_step(state, &mut constraints, dt, baumgarte, slop, rest_threshold, velocity_iterations);
    } else {
        for _ in 0..velocity_iterations {
            for c in constraints.iter_mut() {
                solve_velocity(c, &mut state.bodies);
            }
            // Hard linear-joint rows mirror the C# PxsLinearJointConstraint —
            // these run inside the same iteration loop.
            solve_linear_joints(state, dt, baumgarte);
        }
    }

    // ---- 6. Integrate positions / orientations ----
    for body in state.bodies.iter_mut() {
        if matches!(body.body_type, PhysXBodyType::Static) || body.pinned { continue; }
        if body.cached_is_sleeping { continue; }
        body.position = v_add(body.position, v_mul(body.linear_velocity, dt));
        // Orientation: integrate via quaternion in axis-angle storage.
        let q = axisangle_to_quat(body.orientation);
        let q_new = q_integrate(q, body.angular_velocity, dt);
        body.orientation = quat_to_axisangle(q_new);

        // Sleep check.
        let lv = v_len_sq(body.linear_velocity);
        let av = v_len_sq(body.angular_velocity);
        if lv < state.sleep_threshold * state.sleep_threshold
            && av < state.sleep_threshold * state.sleep_threshold {
            body.cached_is_sleeping = true;
        }
    }
}

// ==========================================================================
//  AABB helpers
// ==========================================================================

fn compute_aabb(body: &crate::player::cast_member::PhysXRigidBody) -> ([f64; 3], [f64; 3]) {
    let q = axisangle_to_quat(body.orientation);
    match body.shape {
        PhysXShapeKind::Sphere => {
            let r = [body.radius; 3];
            (v_sub(body.position, r), v_add(body.position, r))
        }
        PhysXShapeKind::Box | PhysXShapeKind::ConvexShape | PhysXShapeKind::ConcaveShape => {
            // Conservative AABB of an oriented box.
            let he = body.half_extents;
            let ex = q_rotate(q, [he[0], 0.0, 0.0]);
            let ey = q_rotate(q, [0.0, he[1], 0.0]);
            let ez = q_rotate(q, [0.0, 0.0, he[2]]);
            let r = [
                ex[0].abs() + ey[0].abs() + ez[0].abs(),
                ex[1].abs() + ey[1].abs() + ez[1].abs(),
                ex[2].abs() + ey[2].abs() + ez[2].abs(),
            ];
            (v_sub(body.position, r), v_add(body.position, r))
        }
        PhysXShapeKind::Capsule => {
            let hh = q_rotate(q, [body.half_height, 0.0, 0.0]);
            let r = [
                hh[0].abs() + body.radius,
                hh[1].abs() + body.radius,
                hh[2].abs() + body.radius,
            ];
            (v_sub(body.position, r), v_add(body.position, r))
        }
    }
}

fn aabb_overlap(a: &([f64; 3], [f64; 3]), b: &([f64; 3], [f64; 3])) -> bool {
    a.1[0] >= b.0[0] && a.0[0] <= b.1[0]
        && a.1[1] >= b.0[1] && a.0[1] <= b.1[1]
        && a.1[2] >= b.0[2] && a.0[2] <= b.1[2]
}

fn axisangle_to_quat(o: [f64; 4]) -> [f64; 4] {
    // Axis-angle (axis.x, axis.y, axis.z, angle_deg) → quat (x, y, z, w).
    let angle_rad = o[3] * std::f64::consts::PI / 180.0;
    let half = angle_rad * 0.5;
    let s = half.sin();
    [o[0] * s, o[1] * s, o[2] * s, half.cos()]
}

fn quat_to_axisangle(q: [f64; 4]) -> [f64; 4] {
    // Quat (x, y, z, w) → axis-angle (axis.x, axis.y, axis.z, angle_deg).
    let w_clamped = q[3].clamp(-1.0, 1.0);
    let angle_rad = 2.0 * w_clamped.acos();
    let s = (1.0 - w_clamped * w_clamped).max(0.0).sqrt();
    if s < 1e-6 {
        [1.0, 0.0, 0.0, 0.0]
    } else {
        let angle_deg = angle_rad * 180.0 / std::f64::consts::PI;
        [q[0] / s, q[1] / s, q[2] / s, angle_deg]
    }
}

// ==========================================================================
//  Narrowphase dispatch — turns Gu* contacts into solver-convention rows.
// ==========================================================================

fn build_contacts(
    state: &mut PhysXPhysicsState,
    i: usize,
    j: usize,
    buffer: &mut GuContactBuffer,
    out: &mut Vec<ContactConstraint>,
) {
    buffer.reset();
    let a = &state.bodies[i];
    let b = &state.bodies[j];
    let qa = axisangle_to_quat(a.orientation);
    let qb = axisangle_to_quat(b.orientation);

    // Dispatch by shape pair. Single-contact pairs return Option<GuContact>;
    // manifold pairs (box-box, capsule-capsule) write directly to `buffer`.
    let single = match (a.shape, b.shape) {
        (PhysXShapeKind::Sphere, PhysXShapeKind::Sphere) =>
            contact_sphere_sphere(a.position, a.radius, b.position, b.radius, 0.0),
        (PhysXShapeKind::Sphere, PhysXShapeKind::Box) =>
            contact_sphere_box(a.position, a.radius, b.half_extents, qb, b.position, 0.0),
        (PhysXShapeKind::Box, PhysXShapeKind::Sphere) => {
            // Reverse: PhysX dispatches sphere-as-shape0; flip the resulting normal.
            contact_sphere_box(b.position, b.radius, a.half_extents, qa, a.position, 0.0)
                .map(|c| gu::GuContact { normal: v_neg(c.normal), ..c })
        }
        (PhysXShapeKind::Sphere, PhysXShapeKind::Capsule) =>
            contact_sphere_capsule(a.position, a.radius, b.position, qb, b.half_height, b.radius, 0.0),
        (PhysXShapeKind::Capsule, PhysXShapeKind::Sphere) => {
            contact_sphere_capsule(b.position, b.radius, a.position, qa, a.half_height, a.radius, 0.0)
                .map(|c| gu::GuContact { normal: v_neg(c.normal), ..c })
        }
        _ => None,
    };
    if let Some(c) = single {
        if c.separation < 0.0 {
            push_contact(out, i, j, a, b, c);
        }
        return;
    }

    // ConcaveShape vs anything: route through the triangle-mesh narrowphase
    // when the body has triangle-mesh data attached. The other body must be
    // a sphere / box / capsule (concave-vs-concave is undefined in PhysX —
    // PhysX returns no contacts for that pair).
    let mesh_route = match (a.shape, b.shape) {
        (PhysXShapeKind::ConcaveShape, _) if a.triangle_mesh.is_some() => Some((i, j, false)),
        (_, PhysXShapeKind::ConcaveShape) if b.triangle_mesh.is_some() => Some((j, i, true)),
        _ => None,
    };
    if let Some((mesh_idx, shape_idx, swapped)) = mesh_route {
        build_mesh_contacts(state, mesh_idx, shape_idx, swapped, out);
        return;
    }

    // Manifold pairs.
    let mut swap_normal = false;
    match (a.shape, b.shape) {
        (PhysXShapeKind::Box, PhysXShapeKind::Box) => {
            let mut pair_data: u8 = 0; // warm-start cache (not yet persistent across frames)
            contact_box_box(buffer, a.half_extents, qa, a.position, b.half_extents, qb, b.position, &mut pair_data, 0.0);
        }
        // Convex pairs route through the hull-vs-hull SAT + Sutherland-Hodgman
        // manifold. Boxes are wrapped as PolygonalBox on the fly so we can
        // mix box / convex / concave-fallback shapes through one path.
        (PhysXShapeKind::ConvexShape, PhysXShapeKind::ConvexShape)
        | (PhysXShapeKind::Box, PhysXShapeKind::ConvexShape)
        | (PhysXShapeKind::ConvexShape, PhysXShapeKind::Box)
        | (PhysXShapeKind::ConcaveShape, PhysXShapeKind::ConvexShape)
        | (PhysXShapeKind::ConvexShape, PhysXShapeKind::ConcaveShape)
        | (PhysXShapeKind::Box, PhysXShapeKind::ConcaveShape)
        | (PhysXShapeKind::ConcaveShape, PhysXShapeKind::Box) => {
            use super::physx_gu_convex as gx;
            let owned_a;
            let poly_a: &gx::PolygonalData = match a.shape {
                PhysXShapeKind::Box => { owned_a = gx::polygonal_box(a.half_extents); &owned_a }
                PhysXShapeKind::ConvexShape | PhysXShapeKind::ConcaveShape => {
                    if let Some(h) = &a.convex_hull { h } else {
                        // Fallback to AABB box hull until a real cooked hull lands.
                        owned_a = gx::polygonal_box(a.half_extents); &owned_a
                    }
                }
                _ => { owned_a = gx::polygonal_box(a.half_extents); &owned_a }
            };
            let owned_b;
            let poly_b: &gx::PolygonalData = match b.shape {
                PhysXShapeKind::Box => { owned_b = gx::polygonal_box(b.half_extents); &owned_b }
                PhysXShapeKind::ConvexShape | PhysXShapeKind::ConcaveShape => {
                    if let Some(h) = &b.convex_hull { h } else {
                        owned_b = gx::polygonal_box(b.half_extents); &owned_b
                    }
                }
                _ => { owned_b = gx::polygonal_box(b.half_extents); &owned_b }
            };
            gx::contact_hull_hull(buffer, poly_a, poly_b, qa, a.position, qb, b.position, 0.0);
        }
        (PhysXShapeKind::Capsule, PhysXShapeKind::Capsule) => {
            contact_capsule_capsule(buffer, a.position, qa, a.half_height, a.radius, b.position, qb, b.half_height, b.radius, 0.0);
        }
        (PhysXShapeKind::Capsule, PhysXShapeKind::Box) => {
            super::physx_gu_capsule_box::contact_capsule_box(
                buffer,
                a.position, qa, a.half_height, a.radius,
                b.position, qb, b.half_extents,
                0.0,
            );
        }
        (PhysXShapeKind::Box, PhysXShapeKind::Capsule) => {
            // PhysX dispatches capsule-as-shape0; pass our box as shape1.
            // Swap argument order so capsule gets the shape0 slot, then
            // post-flip the normal so it lands in our (i=BodyA=Box, j=BodyB=Capsule) frame.
            super::physx_gu_capsule_box::contact_capsule_box(
                buffer,
                b.position, qb, b.half_height, b.radius,
                a.position, qa, a.half_extents,
                0.0,
            );
            swap_normal = true;
        }
        _ => {}
    }
    let body_a = &state.bodies[i];
    let body_b = &state.bodies[j];
    for c in &buffer.contacts {
        if c.separation > 0.0 { continue; }
        let cc = if swap_normal {
            gu::GuContact { normal: v_neg(c.normal), ..*c }
        } else { *c };
        push_contact(out, i, j, body_a, body_b, cc);
    }
}

fn push_contact(
    out: &mut Vec<ContactConstraint>,
    i: usize, j: usize,
    a: &crate::player::cast_member::PhysXRigidBody,
    b: &crate::player::cast_member::PhysXRigidBody,
    c: gu::GuContact,
) {
    // PhysX → solver convention: normal flipped (B→A → A→B), pen = -separation.
    out.push(ContactConstraint {
        body_a: i, body_b: j,
        body_b_is_static_terrain: false,
        point: c.point,
        normal: v_neg(c.normal),
        penetration: -c.separation,
        friction: (a.friction * b.friction).max(0.0).sqrt(),
        restitution: a.restitution.max(b.restitution),
        eff_mass_n: 0.0, eff_mass_t1: 0.0, eff_mass_t2: 0.0,
        bias_n: 0.0,
        tan1: [0.0; 3], tan2: [0.0; 3],
        ra: [0.0; 3], rb: [0.0; 3],
        impulse_n: 0.0, impulse_t1: 0.0, impulse_t2: 0.0,
    });
}

// ==========================================================================
//  Triangle-mesh narrowphase dispatch.
//
//  Routes (sphere | box | capsule) vs ConcaveShape pairs into
//  `physx_gu_mesh::sphere_vs_mesh` / `box_vs_mesh` / `capsule_vs_mesh`. The
//  query shape's pose is transformed into mesh-local space first (PhysX
//  convention: the mesh body acts as shape "B"). Each per-tri contact is
//  pushed as a ContactConstraint with normal flipped to A→B (our solver
//  convention).
// ==========================================================================

fn build_mesh_contacts(
    state: &PhysXPhysicsState,
    mesh_idx: usize, shape_idx: usize, swapped: bool,
    out: &mut Vec<ContactConstraint>,
) {
    use super::physx_gu_mesh as mp;

    let mesh_body = &state.bodies[mesh_idx];
    let shape_body = &state.bodies[shape_idx];
    let Some(mesh) = mesh_body.triangle_mesh.as_ref() else { return; };

    // Transform query shape into mesh-local space.
    let mesh_q = axisangle_to_quat(mesh_body.orientation);
    let mesh_qinv = q_inv(mesh_q);
    let to_local = |p: [f64; 3]| -> [f64; 3] { q_rotate(mesh_qinv, v_sub(p, mesh_body.position)) };
    let to_world_vec = |v: [f64; 3]| -> [f64; 3] { q_rotate(mesh_q, v) };
    let to_world_pt = |p: [f64; 3]| -> [f64; 3] { v_add(q_rotate(mesh_q, p), mesh_body.position) };

    // Helpers to convert f64 ↔ f32 for the per-tri contact gen API (which is
    // f32 to match the actual PhysX surface).
    let f64_to_f32 = |v: [f64; 3]| -> [f32; 3] { [v[0] as f32, v[1] as f32, v[2] as f32] };
    let f32_to_f64 = |v: [f32; 3]| -> [f64; 3] { [v[0] as f64, v[1] as f64, v[2] as f64] };

    let local_contacts: Vec<mp::GuTriContact> = match shape_body.shape {
        PhysXShapeKind::Sphere => {
            let c = f64_to_f32(to_local(shape_body.position));
            mp::sphere_vs_mesh(mesh, c, shape_body.radius as f32, 0.0)
        }
        PhysXShapeKind::Capsule => {
            let q = axisangle_to_quat(shape_body.orientation);
            let p0_w = v_add(shape_body.position, q_rotate(q, [-shape_body.half_height, 0.0, 0.0]));
            let p1_w = v_add(shape_body.position, q_rotate(q, [ shape_body.half_height, 0.0, 0.0]));
            mp::capsule_vs_mesh(mesh, f64_to_f32(to_local(p0_w)), f64_to_f32(to_local(p1_w)),
                                shape_body.radius as f32, 0.0)
        }
        PhysXShapeKind::Box => {
            let c = f64_to_f32(to_local(shape_body.position));
            // Combined rotation: mesh^{-1} * shape ⇒ shape's local axes in mesh-local.
            let q_combined = q_mul(mesh_qinv, axisangle_to_quat(shape_body.orientation));
            let ax = f64_to_f32(q_rotate(q_combined, [1.0, 0.0, 0.0]));
            let ay = f64_to_f32(q_rotate(q_combined, [0.0, 1.0, 0.0]));
            let az = f64_to_f32(q_rotate(q_combined, [0.0, 0.0, 1.0]));
            let he = f64_to_f32(shape_body.half_extents);
            mp::box_vs_mesh(mesh, c, he, ax, ay, az, 0.0)
        }
        _ => return, // ConvexShape / ConcaveShape vs ConcaveShape: undefined per PhysX
    };

    let pair_friction = (mesh_body.friction * shape_body.friction).max(0.0).sqrt();
    let pair_restitution = mesh_body.restitution.max(shape_body.restitution);
    for lc in &local_contacts {
        if lc.separation > 0.0 { continue; }
        let pt_w = to_world_pt(f32_to_f64(lc.point));
        // Gu normal points triangle (B/mesh) → query shape (A). We need solver
        // convention A → B. The shape body in the (out_a, out_b) ordering
        // depends on `swapped` (whether the original pair had mesh as A or B).
        let norm_w = to_world_vec(f32_to_f64(lc.normal)); // mesh → shape (world)
        let solver_normal = if swapped {
            // outA = mesh_idx, outB = shape_idx ⇒ solver wants mesh → shape = norm_w as-is.
            norm_w
        } else {
            // outA = shape_idx, outB = mesh_idx ⇒ solver wants shape → mesh = -norm_w.
            v_neg(norm_w)
        };
        let (out_a, out_b) = if swapped { (mesh_idx, shape_idx) } else { (shape_idx, mesh_idx) };
        out.push(ContactConstraint {
            body_a: out_a, body_b: out_b,
            body_b_is_static_terrain: false,
            point: pt_w,
            normal: solver_normal,
            penetration: -(lc.separation as f64),
            friction: pair_friction,
            restitution: pair_restitution,
            eff_mass_n: 0.0, eff_mass_t1: 0.0, eff_mass_t2: 0.0,
            bias_n: 0.0,
            tan1: [0.0; 3], tan2: [0.0; 3],
            ra: [0.0; 3], rb: [0.0; 3],
            impulse_n: 0.0, impulse_t1: 0.0, impulse_t2: 0.0,
        });
    }
}

/// Sphere/box/capsule vs static heightfield terrain. Mirrors
/// `build_mesh_contacts`. The terrain is treated as PhysX shape "B"
/// (static collider); the dynamic body is shape "A".
fn build_terrain_contacts(
    state: &PhysXPhysicsState,
    body_idx: usize,
    terrain_idx: usize,
    out: &mut Vec<ContactConstraint>,
) {
    use super::physx_gu_heightfield as hf;

    let body = &state.bodies[body_idx];
    let terrain = &state.terrains[terrain_idx];

    // Transform query shape into terrain-local space.
    let terrain_q = axisangle_to_quat(terrain.orientation);
    let terrain_qinv = q_inv(terrain_q);
    let to_local = |p: [f64; 3]| -> [f64; 3] { q_rotate(terrain_qinv, v_sub(p, terrain.position)) };
    let to_world_vec = |v: [f64; 3]| -> [f64; 3] { q_rotate(terrain_q, v) };
    let to_world_pt = |p: [f64; 3]| -> [f64; 3] { v_add(q_rotate(terrain_q, p), terrain.position) };

    let f64_to_f32 = |v: [f64; 3]| -> [f32; 3] { [v[0] as f32, v[1] as f32, v[2] as f32] };
    let f32_to_f64 = |v: [f32; 3]| -> [f64; 3] { [v[0] as f64, v[1] as f64, v[2] as f64] };

    let local_contacts = match body.shape {
        PhysXShapeKind::Sphere => {
            let c = f64_to_f32(to_local(body.position));
            hf::sphere_vs_heightfield(&terrain.height_field, c, body.radius as f32, 0.0)
        }
        PhysXShapeKind::Capsule => {
            let q = axisangle_to_quat(body.orientation);
            let p0_w = v_add(body.position, q_rotate(q, [-body.half_height, 0.0, 0.0]));
            let p1_w = v_add(body.position, q_rotate(q, [ body.half_height, 0.0, 0.0]));
            hf::capsule_vs_heightfield(&terrain.height_field,
                f64_to_f32(to_local(p0_w)), f64_to_f32(to_local(p1_w)),
                body.radius as f32, 0.0)
        }
        PhysXShapeKind::Box => {
            let c = f64_to_f32(to_local(body.position));
            let q_combined = q_mul(terrain_qinv, axisangle_to_quat(body.orientation));
            let ax = f64_to_f32(q_rotate(q_combined, [1.0, 0.0, 0.0]));
            let ay = f64_to_f32(q_rotate(q_combined, [0.0, 1.0, 0.0]));
            let az = f64_to_f32(q_rotate(q_combined, [0.0, 0.0, 1.0]));
            let he = f64_to_f32(body.half_extents);
            hf::box_vs_heightfield(&terrain.height_field, c, he, ax, ay, az, 0.0)
        }
        // Convex / concave bodies hitting a heightfield: not supported in
        // PhysX 3.4's PCM path either (they use convex-vs-HF which we don't
        // ship). Skip silently.
        _ => return,
    };

    let pair_friction = (body.friction * terrain.friction).max(0.0).sqrt();
    let pair_restitution = body.restitution.max(terrain.restitution);

    for lc in &local_contacts {
        if lc.separation > 0.0 { continue; }
        let pt_w = to_world_pt(f32_to_f64(lc.point));
        // Gu normal: triangle (B = terrain) → query shape (A = body).
        let norm_w = to_world_vec(f32_to_f64(lc.normal));
        // Solver convention: A → B = -norm_w.
        let solver_normal = v_neg(norm_w);
        out.push(ContactConstraint {
            body_a: body_idx,
            body_b: body_idx, // self-pair — body_b is ignored (terrain is static)
            body_b_is_static_terrain: true,
            point: pt_w,
            normal: solver_normal,
            penetration: -(lc.separation as f64),
            friction: pair_friction,
            restitution: pair_restitution,
            eff_mass_n: 0.0, eff_mass_t1: 0.0, eff_mass_t2: 0.0,
            bias_n: 0.0,
            tan1: [0.0; 3], tan2: [0.0; 3],
            ra: [0.0; 3], rb: [0.0; 3],
            impulse_n: 0.0, impulse_t1: 0.0, impulse_t2: 0.0,
        });
    }
}

fn q_inv(q: [f64; 4]) -> [f64; 4] { [-q[0], -q[1], -q[2], q[3]] }
fn q_mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

// ==========================================================================
//  Solver — sequential impulses (Catto 2005). Mirrors C#
//  PxsContactConstraint::Prepare / SolveVelocity.
// ==========================================================================

fn static_ghost_body() -> crate::player::cast_member::PhysXRigidBody {
    let mut b = crate::player::cast_member::PhysXRigidBody::default();
    b.body_type = PhysXBodyType::Static;
    b.mass = 0.0;
    b.linear_velocity = [0.0; 3];
    b.angular_velocity = [0.0; 3];
    b
}

fn prepare_constraint(
    c: &mut ContactConstraint,
    bodies: &[crate::player::cast_member::PhysXRigidBody],
    dt: f64, baumgarte: f64, slop: f64, rest_threshold: f64,
) {
    let a = &bodies[c.body_a];
    let ghost; // optional storage for static-terrain ghost body
    let b: &crate::player::cast_member::PhysXRigidBody = if c.body_b_is_static_terrain {
        ghost = static_ghost_body();
        &ghost
    } else {
        &bodies[c.body_b]
    };
    c.ra = v_sub(c.point, a.position);
    // For static-terrain pairs, rb is unused (body_b is treated as a static
    // collider with no velocity contribution). Set to zero for cleanliness.
    c.rb = if c.body_b_is_static_terrain { [0.0; 3] } else { v_sub(c.point, b.position) };
    build_tangent_basis(c.normal, &mut c.tan1, &mut c.tan2);

    c.eff_mass_n = compute_eff_mass(a, b, c.normal, c.ra, c.rb);
    c.eff_mass_t1 = compute_eff_mass(a, b, c.tan1, c.ra, c.rb);
    c.eff_mass_t2 = compute_eff_mass(a, b, c.tan2, c.ra, c.rb);

    let depth = (c.penetration - slop).max(0.0);
    c.bias_n = -(baumgarte / dt) * depth;

    let vn = contact_velocity_along(a, b, c.ra, c.rb, c.normal);
    if vn < -rest_threshold {
        c.bias_n += c.restitution * vn;
    }
}

fn solve_velocity(
    c: &mut ContactConstraint,
    bodies: &mut [crate::player::cast_member::PhysXRigidBody],
) {
    let is_terrain = c.body_b_is_static_terrain;
    let ghost = static_ghost_body();
    let read_ab = |bodies: &[crate::player::cast_member::PhysXRigidBody]| -> (crate::player::cast_member::PhysXRigidBody, crate::player::cast_member::PhysXRigidBody) {
        let a = bodies[c.body_a].clone();
        let b = if is_terrain { ghost.clone() } else { bodies[c.body_b].clone() };
        (a, b)
    };

    // Normal axis.
    let (a_clone, b_clone) = read_ab(bodies);
    let vn = contact_velocity_along(&a_clone, &b_clone, c.ra, c.rb, c.normal) + c.bias_n;
    let mut dlambda = -vn * c.eff_mass_n;
    let old = c.impulse_n;
    c.impulse_n = (old + dlambda).max(0.0);
    dlambda = c.impulse_n - old;
    apply_impulse(bodies, c.body_a, c.body_b, c.normal, c.ra, c.rb, dlambda, is_terrain);

    // Friction (Coulomb cone, two tangent rows).
    let friction_limit = c.friction * c.impulse_n;

    let (a2, b2) = read_ab(bodies);
    let vt1 = contact_velocity_along(&a2, &b2, c.ra, c.rb, c.tan1);
    let dt1 = -vt1 * c.eff_mass_t1;
    let old1 = c.impulse_t1;
    c.impulse_t1 = (old1 + dt1).clamp(-friction_limit, friction_limit);
    apply_impulse(bodies, c.body_a, c.body_b, c.tan1, c.ra, c.rb, c.impulse_t1 - old1, is_terrain);

    let (a3, b3) = read_ab(bodies);
    let vt2 = contact_velocity_along(&a3, &b3, c.ra, c.rb, c.tan2);
    let dt2 = -vt2 * c.eff_mass_t2;
    let old2 = c.impulse_t2;
    c.impulse_t2 = (old2 + dt2).clamp(-friction_limit, friction_limit);
    apply_impulse(bodies, c.body_a, c.body_b, c.tan2, c.ra, c.rb, c.impulse_t2 - old2, is_terrain);
}

fn contact_velocity_along(
    a: &crate::player::cast_member::PhysXRigidBody,
    b: &crate::player::cast_member::PhysXRigidBody,
    ra: [f64; 3], rb: [f64; 3], axis: [f64; 3],
) -> f64 {
    let va = v_add(a.linear_velocity, v_cross(a.angular_velocity, ra));
    let vb = v_add(b.linear_velocity, v_cross(b.angular_velocity, rb));
    v_dot(v_sub(vb, va), axis)
}

fn compute_eff_mass(
    a: &crate::player::cast_member::PhysXRigidBody,
    b: &crate::player::cast_member::PhysXRigidBody,
    axis: [f64; 3], ra: [f64; 3], rb: [f64; 3],
) -> f64 {
    let inv_mass_a = inverse_mass(a);
    let inv_mass_b = inverse_mass(b);
    let mut k = inv_mass_a + inv_mass_b;
    if inv_mass_a > 0.0 {
        let raxn = v_cross(ra, axis);
        let ia = mul_inv_inertia(a, raxn);
        k += v_dot(raxn, ia);
    }
    if inv_mass_b > 0.0 {
        let rbxn = v_cross(rb, axis);
        let ib = mul_inv_inertia(b, rbxn);
        k += v_dot(rbxn, ib);
    }
    if k > 0.0 { 1.0 / k } else { 0.0 }
}

fn inverse_mass(body: &crate::player::cast_member::PhysXRigidBody) -> f64 {
    if matches!(body.body_type, PhysXBodyType::Static) || body.pinned || body.mass <= 0.0 { 0.0 }
    else { 1.0 / body.mass }
}

fn inverse_inertia_diag(body: &crate::player::cast_member::PhysXRigidBody) -> [f64; 3] {
    if inverse_mass(body) == 0.0 { return [0.0; 3]; }
    let m = body.mass;
    match body.shape {
        PhysXShapeKind::Sphere => {
            let r = body.radius;
            let i = 2.0 / 5.0 * m * r * r;
            [1.0 / i, 1.0 / i, 1.0 / i]
        }
        PhysXShapeKind::Box | PhysXShapeKind::ConvexShape | PhysXShapeKind::ConcaveShape => {
            let he = body.half_extents;
            let ix = m / 12.0 * (he[1] * he[1] * 4.0 + he[2] * he[2] * 4.0);
            let iy = m / 12.0 * (he[0] * he[0] * 4.0 + he[2] * he[2] * 4.0);
            let iz = m / 12.0 * (he[0] * he[0] * 4.0 + he[1] * he[1] * 4.0);
            [1.0 / ix, 1.0 / iy, 1.0 / iz]
        }
        PhysXShapeKind::Capsule => {
            let r = body.radius;
            let ix = m * r * r * 0.5;
            let h = body.half_height * 2.0;
            let iyz = m * (3.0 * r * r + h * h) / 12.0;
            [1.0 / ix, 1.0 / iyz, 1.0 / iyz]
        }
    }
}

fn mul_inv_inertia(
    body: &crate::player::cast_member::PhysXRigidBody, v: [f64; 3],
) -> [f64; 3] {
    let q = axisangle_to_quat(body.orientation);
    let inv_diag = inverse_inertia_diag(body);
    let local = q_rotate_inv(q, v);
    let local_scaled = [local[0] * inv_diag[0], local[1] * inv_diag[1], local[2] * inv_diag[2]];
    q_rotate(q, local_scaled)
}

fn apply_impulse(
    bodies: &mut [crate::player::cast_member::PhysXRigidBody],
    ia: usize, ib: usize,
    axis: [f64; 3], ra: [f64; 3], rb: [f64; 3], lambda: f64,
    body_b_is_static_terrain: bool,
) {
    let p = v_mul(axis, lambda);
    if inverse_mass(&bodies[ia]) > 0.0 {
        let inv = inverse_mass(&bodies[ia]);
        let dw = mul_inv_inertia(&bodies[ia], v_cross(ra, p));
        let a = &mut bodies[ia];
        a.linear_velocity = v_sub(a.linear_velocity, v_mul(p, inv));
        a.angular_velocity = v_sub(a.angular_velocity, dw);
    }
    // For static-terrain pairs, body_b is a sentinel (== body_a) — skip the
    // B-side impulse, otherwise we'd double-back the impulse onto body_a.
    if !body_b_is_static_terrain && inverse_mass(&bodies[ib]) > 0.0 {
        let inv = inverse_mass(&bodies[ib]);
        let dw = mul_inv_inertia(&bodies[ib], v_cross(rb, p));
        let b = &mut bodies[ib];
        b.linear_velocity = v_add(b.linear_velocity, v_mul(p, inv));
        b.angular_velocity = v_add(b.angular_velocity, dw);
    }
}

/// Drives one tick through the verbatim PhysX 3.4 SoA solver path
/// (`physx_soa_solver::PxsSolverSoa`). Body-vs-body constraints route
/// through the SoA solver; body-vs-terrain stays on the AoS path so the
/// static-terrain sentinel doesn't need an SoA representation.
fn run_soa_solver_step(
    state: &mut PhysXPhysicsState,
    constraints: &mut Vec<ContactConstraint>,
    dt: f64, baumgarte: f64, _slop: f64, _rest_threshold: f64,
    velocity_iterations: u32,
) {
    use super::physx_soa_solver as soa;

    // Split constraints: terrain pairs run on AoS; the rest go to SoA.
    let mut soa_inputs: Vec<soa::SoaContactInputWithOffsets> = Vec::with_capacity(constraints.len());
    let mut terrain_constraints: Vec<usize> = Vec::new();
    for (idx, c) in constraints.iter().enumerate() {
        if c.body_b_is_static_terrain {
            terrain_constraints.push(idx);
            continue;
        }
        soa_inputs.push(soa::SoaContactInputWithOffsets {
            body_a: c.body_a as u32,
            body_b: c.body_b as u32,
            ra: c.ra, rb: c.rb,
            normal: c.normal,
            penetration: c.penetration,
            friction: c.friction,
            restitution: c.restitution,
        });
    }

    // Build SoA body inputs (one per state body — the SoA solver indexes
    // into this Vec by `body_a` / `body_b` u32).
    let body_inputs: Vec<soa::SoaBodyInput> = state.bodies.iter().map(|b| {
        let inv_mass = inverse_mass(b);
        let inv_iner = if inv_mass > 0.0 { inverse_inertia_diag(b) } else { [0.0; 3] };
        soa::SoaBodyInput {
            linear_velocity: b.linear_velocity,
            angular_velocity: b.angular_velocity,
            inverse_mass: inv_mass,
            inverse_inertia_diag_local: inv_iner,
            orientation: axisangle_to_quat(b.orientation),
        }
    }).collect();

    let mut solver = soa::PxsSolverSoa::default();
    let warm = std::collections::HashMap::new(); // no warm-start cache yet
    solver.build_with_offsets(&body_inputs, &soa_inputs, dt, baumgarte, _slop, _rest_threshold, &warm);

    for _ in 0..velocity_iterations {
        solver.solve_one_iteration(true);

        // Run terrain (AoS) and joint constraints in the same iteration loop.
        // Pull current velocities from the SoA bodies into state.bodies first
        // so the AoS path operates on the same intermediate state.
        for (i, b) in solver.bodies.iter().enumerate() {
            state.bodies[i].linear_velocity = b.linear_velocity;
            state.bodies[i].angular_velocity = b.angular_state;
        }
        for &idx in &terrain_constraints {
            solve_velocity(&mut constraints[idx], &mut state.bodies);
        }
        // Push the AoS-updated velocities back into the SoA buffers so the
        // next SoA iteration sees the post-terrain state.
        for (i, b) in state.bodies.iter().enumerate() {
            solver.bodies[i].linear_velocity = b.linear_velocity;
            solver.bodies[i].angular_state = b.angular_velocity;
        }

        solve_linear_joints(state, dt, baumgarte);

        // After joints (which mutate state.bodies directly), refresh SoA again.
        for (i, b) in state.bodies.iter().enumerate() {
            solver.bodies[i].linear_velocity = b.linear_velocity;
            solver.bodies[i].angular_state = b.angular_velocity;
        }
    }

    // Final write-back from SoA to atoms.
    let (vels, _ws) = solver.write_back();
    for (i, (lin, ang)) in vels.into_iter().enumerate() {
        state.bodies[i].linear_velocity = lin;
        state.bodies[i].angular_velocity = ang;
    }
}

fn build_tangent_basis(n: [f64; 3], t1: &mut [f64; 3], t2: &mut [f64; 3]) {
    let axis = if n[0].abs() < 0.57735 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let cross = v_cross(n, axis);
    let len = v_len_sq(cross).sqrt();
    let normalized = if len > 1e-6 { v_mul(cross, 1.0 / len) } else { cross };
    *t1 = normalized;
    *t2 = v_cross(n, *t1);
}

// ==========================================================================
//  Spring + dashpot forces (legacy NxSpringAndDamperEffector model).
// ==========================================================================

fn apply_spring_forces(state: &mut PhysXPhysicsState, dt: f64) {
    // Snapshot per-body data we'll need.
    let body_count = state.bodies.len();
    let constraint_count = state.constraints.len();
    if constraint_count == 0 { return; }

    for ci in 0..constraint_count {
        let c = &state.constraints[ci];
        if !matches!(c.kind, PhysXConstraintKind::Spring) { continue; }
        let stiffness = c.stiffness;
        let damping = c.damping;
        let rest_length = c.rest_length;
        let anchor_a = c.anchor_a;
        let anchor_b = c.anchor_b;
        let body_a_id = c.body_a;
        let body_b_id = c.body_b;

        // Resolve body indices by id.
        let idx_a = body_a_id.and_then(|id| state.bodies.iter().position(|b| b.id == id));
        let idx_b = body_b_id.and_then(|id| state.bodies.iter().position(|b| b.id == id));
        let world_a = if let Some(i) = idx_a {
            let q = axisangle_to_quat(state.bodies[i].orientation);
            v_add(state.bodies[i].position, q_rotate(q, anchor_a))
        } else { anchor_a };
        let world_b = if let Some(i) = idx_b {
            let q = axisangle_to_quat(state.bodies[i].orientation);
            v_add(state.bodies[i].position, q_rotate(q, anchor_b))
        } else { anchor_b };

        let d = v_sub(world_b, world_a);
        let len_sq = v_len_sq(d);
        if len_sq < 1e-12 { continue; }
        let len = len_sq.sqrt();
        let n = v_mul(d, 1.0 / len);
        let stretch = len - rest_length;

        let v_a = if let Some(i) = idx_a { velocity_at(&state.bodies[i], world_a) } else { [0.0; 3] };
        let v_b = if let Some(i) = idx_b { velocity_at(&state.bodies[i], world_b) } else { [0.0; 3] };
        let v_along = v_dot(v_sub(v_b, v_a), n);
        let f_total = -stiffness * stretch + (-damping * v_along);
        let force = v_mul(n, f_total);

        if let Some(i) = idx_a { apply_force_at(&mut state.bodies[i], world_a, v_neg(force), dt); }
        if let Some(i) = idx_b { apply_force_at(&mut state.bodies[i], world_b, force, dt); }

        // Avoid the unused-warning on body_count when no springs apply.
        let _ = body_count;
    }
}

fn velocity_at(body: &crate::player::cast_member::PhysXRigidBody, world_point: [f64; 3]) -> [f64; 3] {
    let r = v_sub(world_point, body.position);
    v_add(body.linear_velocity, v_cross(body.angular_velocity, r))
}

fn apply_force_at(
    body: &mut crate::player::cast_member::PhysXRigidBody,
    world_point: [f64; 3], force: [f64; 3], dt: f64,
) {
    if matches!(body.body_type, PhysXBodyType::Static) || body.pinned || body.mass <= 0.0 { return; }
    let inv_mass = 1.0 / body.mass;
    let impulse = v_mul(force, dt);
    body.linear_velocity = v_add(body.linear_velocity, v_mul(impulse, inv_mass));
    let r = v_sub(world_point, body.position);
    let torque_impulse = v_cross(r, impulse);
    let inv_diag = inverse_inertia_diag(body);
    let q = axisangle_to_quat(body.orientation);
    let local = q_rotate_inv(q, torque_impulse);
    let local_scaled = [
        local[0] * inv_diag[0],
        local[1] * inv_diag[1],
        local[2] * inv_diag[2],
    ];
    body.angular_velocity = v_add(body.angular_velocity, q_rotate(q, local_scaled));
    body.cached_is_sleeping = false;
}

// ==========================================================================
//  Linear-joint hard constraint (per-iteration row).
// ==========================================================================

fn solve_linear_joints(state: &mut PhysXPhysicsState, dt: f64, baumgarte: f64) {
    let constraint_count = state.constraints.len();
    for ci in 0..constraint_count {
        let c = &state.constraints[ci];
        if !matches!(c.kind, PhysXConstraintKind::LinearJoint) { continue; }
        let body_a_id = c.body_a;
        let body_b_id = c.body_b;
        let anchor_a = c.anchor_a;
        let anchor_b = c.anchor_b;
        let rest_length = c.rest_length;

        let idx_a = body_a_id.and_then(|id| state.bodies.iter().position(|b| b.id == id));
        let idx_b = body_b_id.and_then(|id| state.bodies.iter().position(|b| b.id == id));
        let (Some(ia), Some(ib)) = (idx_a, idx_b) else { continue; };

        let qa = axisangle_to_quat(state.bodies[ia].orientation);
        let qb = axisangle_to_quat(state.bodies[ib].orientation);
        let world_a = v_add(state.bodies[ia].position, q_rotate(qa, anchor_a));
        let world_b = v_add(state.bodies[ib].position, q_rotate(qb, anchor_b));
        let ra = v_sub(world_a, state.bodies[ia].position);
        let rb = v_sub(world_b, state.bodies[ib].position);
        let d = v_sub(world_b, world_a);
        let len_sq = v_len_sq(d);
        let len = len_sq.max(1e-12).sqrt();
        let axis = v_mul(d, 1.0 / len);
        let deviation = len - rest_length;

        let eff = compute_eff_mass(&state.bodies[ia], &state.bodies[ib], axis, ra, rb);
        let bias = -(baumgarte / dt) * deviation;
        let v_along = contact_velocity_along(&state.bodies[ia], &state.bodies[ib], ra, rb, axis);
        let lambda = -(v_along + bias) * eff;
        apply_impulse(&mut state.bodies, ia, ib, axis, ra, rb, lambda, false);
    }
}

// ==========================================================================
//  External hooks (kept for backwards compat with handlers).
// ==========================================================================

/// Wake every dynamic body — used by `SetGravity`.
pub fn wake_all_dynamic(state: &mut PhysXPhysicsState) {
    for body in state.bodies.iter_mut() {
        if matches!(body.body_type, PhysXBodyType::Dynamic) {
            body.cached_is_sleeping = false;
        }
    }
}
