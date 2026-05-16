//! PhysX coverage parity tests.
//!
//! All tests are `#[ignore]`d by default. Enable with:
//!   cargo test -p vm-rust --test mod -- --ignored
//! or run a single test by name:
//!   cargo test -p vm-rust --test mod sphere_drops_onto_concave_mesh_floor -- --ignored
//!
//! These cover the integration points wired up in this session:
//!   - Triangle-mesh narrowphase (#concaveShape via `setTriangleMesh`)
//!   - Heightfield terrain (createTerrain / createTerrainDesc)
//!   - SOA solver (verbatim PhysX 3.4 PxsSolverSoa)
//!
//! And re-validate pre-existing functionality (sphere/box/capsule pairs,
//! springs, pinned bodies, RTree integrity, per-tri contact gen).

#![cfg(not(target_arch = "wasm32"))]

use vm_rust::player::cast_member::{
    PhysXBodyType, PhysXConstraint, PhysXConstraintKind, PhysXPhysicsState, PhysXRigidBody,
    PhysXShapeKind, PhysXTerrain,
};
use vm_rust::player::handlers::datum_handlers::cast_member::{
    physx_gu_heightfield::GuHeightField,
    physx_gu_mesh::{box_vs_mesh, capsule_vs_mesh, sphere_vs_mesh, GuTriangleMesh},
    physx_gu_rtree::{LeafTriangles, RTreeAabbCallback, RTreeBuilder},
    physx_native::step_native,
};

// -----------------------------------------------------------------------------
//  Helpers
// -----------------------------------------------------------------------------

/// CCW-from-above floor (normals +Y), N×N quads centered at origin.
fn build_floor_mesh(grid: usize) -> GuTriangleMesh {
    let n = grid;
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity((n + 1) * (n + 1));
    for j in 0..=n {
        for i in 0..=n {
            verts.push([
                i as f32 - n as f32 * 0.5,
                0.0,
                j as f32 - n as f32 * 0.5,
            ]);
        }
    }
    let mut tris: Vec<u32> = Vec::with_capacity(n * n * 6);
    for j in 0..n {
        for i in 0..n {
            let v00 = (j * (n + 1) + i) as u32;
            let v10 = v00 + 1;
            let v01 = ((j + 1) * (n + 1) + i) as u32;
            let v11 = v01 + 1;
            // CCW from above ⇒ +Y normal.
            tris.extend_from_slice(&[v00, v11, v10]);
            tris.extend_from_slice(&[v00, v01, v11]);
        }
    }
    GuTriangleMesh::build(verts, tris)
}

fn make_dynamic_sphere(name: &str, pos: [f64; 3], radius: f64, mass: f64) -> PhysXRigidBody {
    let mut b = PhysXRigidBody::default();
    b.name = name.to_string();
    b.body_type = PhysXBodyType::Dynamic;
    b.shape = PhysXShapeKind::Sphere;
    b.position = pos;
    b.radius = radius;
    b.mass = mass;
    b.friction = 0.5;
    b.restitution = 0.2;
    b
}

fn make_static_box(name: &str, pos: [f64; 3], half_extents: [f64; 3]) -> PhysXRigidBody {
    let mut b = PhysXRigidBody::default();
    b.name = name.to_string();
    b.body_type = PhysXBodyType::Static;
    b.shape = PhysXShapeKind::Box;
    b.position = pos;
    b.half_extents = half_extents;
    b.mass = 0.0;
    b.friction = 0.5;
    b.restitution = 0.2;
    b
}

fn make_static_concave_mesh_body(name: &str, mesh: GuTriangleMesh) -> PhysXRigidBody {
    let mut b = PhysXRigidBody::default();
    b.name = name.to_string();
    b.body_type = PhysXBodyType::Static;
    b.shape = PhysXShapeKind::ConcaveShape;
    b.triangle_mesh = Some(mesh);
    b.mass = 0.0;
    b.friction = 0.5;
    b.restitution = 0.2;
    b
}

fn make_terrain(name: &str, hf: GuHeightField) -> PhysXTerrain {
    PhysXTerrain {
        id: 1,
        name: name.to_string(),
        height_field: hf,
        friction: 0.5,
        restitution: 0.2,
        position: [0.0; 3],
        orientation: [1.0, 0.0, 0.0, 0.0],
    }
}

fn run_seconds(state: &mut PhysXPhysicsState, secs: f64) {
    let dt = 1.0 / 60.0;
    let frames = (secs / dt) as u32;
    for _ in 0..frames {
        step_native(state, dt, 1);
    }
}

// -----------------------------------------------------------------------------
//  Pre-existing pair narrowphase regression checks
// -----------------------------------------------------------------------------

#[test]
#[ignore = "physx parity test — enable with `cargo test -- --ignored`"]
fn sphere_drops_onto_static_box() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let mut box_body = make_static_box("box", [0.0; 3], [2.0, 0.5, 2.0]);
    box_body.restitution = 0.0;
    state.bodies.push(box_body);
    let mut ball = make_dynamic_sphere("ball", [0.0, 5.0, 0.0], 0.5, 1.0);
    ball.restitution = 0.0;
    state.bodies.push(ball);

    run_seconds(&mut state, 3.0);
    let y = state.bodies[1].position[1];
    assert!((y - 1.0).abs() < 0.1, "sphere should rest at y≈1.0, got {y}");
}

#[test]
#[ignore = "physx parity test"]
fn sphere_drops_onto_static_sphere() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let mut ground = PhysXRigidBody::default();
    ground.name = "ground".into();
    ground.body_type = PhysXBodyType::Static;
    ground.shape = PhysXShapeKind::Sphere;
    ground.radius = 5.0;
    ground.mass = 0.0;
    ground.restitution = 0.0;
    state.bodies.push(ground);
    let mut ball = make_dynamic_sphere("ball", [0.0, 8.0, 0.0], 0.5, 1.0);
    ball.restitution = 0.0;
    state.bodies.push(ball);

    run_seconds(&mut state, 3.0);
    let y = state.bodies[1].position[1];
    assert!((y - 5.5).abs() < 0.15, "sphere should rest at y≈5.5, got {y}");
}

#[test]
#[ignore = "physx parity test"]
fn box_drops_onto_static_box() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let mut floor = make_static_box("floor", [0.0; 3], [2.0, 0.5, 2.0]);
    floor.restitution = 0.0;
    state.bodies.push(floor);
    let mut crate_box = PhysXRigidBody::default();
    crate_box.name = "crate".into();
    crate_box.body_type = PhysXBodyType::Dynamic;
    crate_box.shape = PhysXShapeKind::Box;
    crate_box.position = [0.0, 5.0, 0.0];
    crate_box.half_extents = [0.5, 0.5, 0.5];
    crate_box.mass = 1.0;
    crate_box.friction = 0.5;
    crate_box.restitution = 0.0;
    state.bodies.push(crate_box);

    run_seconds(&mut state, 3.0);
    let y = state.bodies[1].position[1];
    assert!((y - 1.0).abs() < 0.1, "box should rest at y≈1.0, got {y}");
}

#[test]
#[ignore = "physx parity test"]
fn sphere_drops_onto_static_capsule() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let mut caps = PhysXRigidBody::default();
    caps.name = "caps".into();
    caps.body_type = PhysXBodyType::Static;
    caps.shape = PhysXShapeKind::Capsule;
    caps.position = [0.0, 0.0, 0.0];
    caps.radius = 0.5;
    caps.half_height = 1.0;
    caps.mass = 0.0;
    caps.restitution = 0.0;
    state.bodies.push(caps);
    let mut ball = make_dynamic_sphere("ball", [0.0, 5.0, 0.0], 0.5, 1.0);
    ball.restitution = 0.0;
    state.bodies.push(ball);

    run_seconds(&mut state, 3.0);
    let y = state.bodies[1].position[1];
    assert!((y - 1.0).abs() < 0.15, "sphere should rest at y≈1.0 on capsule, got {y}");
}

#[test]
#[ignore = "physx parity test"]
fn capsule_drops_onto_static_box() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let mut floor = make_static_box("floor", [0.0; 3], [2.0, 0.5, 2.0]);
    floor.restitution = 0.0;
    state.bodies.push(floor);
    let mut caps = PhysXRigidBody::default();
    caps.name = "caps".into();
    caps.body_type = PhysXBodyType::Dynamic;
    caps.shape = PhysXShapeKind::Capsule;
    caps.position = [0.0, 5.0, 0.0];
    caps.radius = 0.4;
    caps.half_height = 0.6;
    caps.mass = 1.0;
    caps.friction = 0.5;
    caps.restitution = 0.0;
    state.bodies.push(caps);

    run_seconds(&mut state, 3.0);
    let y = state.bodies[1].position[1];
    assert!((y - 0.9).abs() < 0.15, "capsule should rest at y≈0.9 on box, got {y}");
}

#[test]
#[ignore = "physx parity test"]
fn capsule_drops_onto_static_capsule() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let mut ground_caps = PhysXRigidBody::default();
    ground_caps.name = "ground".into();
    ground_caps.body_type = PhysXBodyType::Static;
    ground_caps.shape = PhysXShapeKind::Capsule;
    ground_caps.position = [0.0, 0.0, 0.0];
    ground_caps.radius = 0.5;
    ground_caps.half_height = 1.0;
    ground_caps.mass = 0.0;
    ground_caps.restitution = 0.0;
    state.bodies.push(ground_caps);

    let mut top_caps = PhysXRigidBody::default();
    top_caps.name = "top".into();
    top_caps.body_type = PhysXBodyType::Dynamic;
    top_caps.shape = PhysXShapeKind::Capsule;
    top_caps.position = [0.0, 4.0, 0.0];
    top_caps.radius = 0.5;
    top_caps.half_height = 1.0;
    top_caps.mass = 1.0;
    top_caps.restitution = 0.0;
    state.bodies.push(top_caps);

    run_seconds(&mut state, 3.0);
    let y = state.bodies[1].position[1];
    assert!((y - 1.0).abs() < 0.2, "top capsule should rest at y≈1.0, got {y}");
}

// -----------------------------------------------------------------------------
//  Triangle-mesh narrowphase (#concaveShape) — wired this session
// -----------------------------------------------------------------------------

#[test]
#[ignore = "physx parity test"]
fn sphere_drops_onto_concave_mesh_floor() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    state.bodies.push(make_static_concave_mesh_body("floor", build_floor_mesh(8)));
    state.bodies.push(make_dynamic_sphere("ball", [0.3, 5.0, 0.4], 0.5, 1.0));

    run_seconds(&mut state, 2.0);
    let y = state.bodies[1].position[1];
    assert!((y - 0.5).abs() < 0.05,
            "sphere should rest at y≈0.5 on triangle floor, got {y}");
}

// -----------------------------------------------------------------------------
//  Heightfield terrain — wired this session
// -----------------------------------------------------------------------------

#[test]
#[ignore = "physx parity test"]
fn sphere_drops_onto_flat_heightfield() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let n = 16usize;
    let heights = vec![0.0f32; n * n];
    let hf = GuHeightField::build(
        n, n, heights, 1.0, 1.0, 1.0,
        [-(n as f32) * 0.5, 0.0, -(n as f32) * 0.5],
    );
    state.terrains.push(make_terrain("flat", hf));
    state.bodies.push(make_dynamic_sphere("ball", [0.3, 5.0, 0.4], 0.5, 1.0));

    run_seconds(&mut state, 2.0);
    let y = state.bodies[0].position[1];
    assert!((y - 0.5).abs() < 0.05,
            "sphere should rest at y≈0.5 on flat HF, got {y}");
}

#[test]
#[ignore = "physx parity test"]
fn sphere_slides_down_tilted_heightfield() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let n = 16usize;
    let mut heights = vec![0.0f32; n * n];
    for row in 0..n {
        for col in 0..n {
            heights[row * n + col] = row as f32 * 0.2;
        }
    }
    let hf = GuHeightField::build(
        n, n, heights, 1.0, 1.0, 1.0,
        [-(n as f32) * 0.5, 0.0, -(n as f32) * 0.5],
    );
    let mut terrain = make_terrain("slope", hf);
    terrain.friction = 0.05;
    terrain.restitution = 0.0;
    state.terrains.push(terrain);
    let mut ball = make_dynamic_sphere("ball", [0.0, 5.0, 0.0], 0.5, 1.0);
    ball.friction = 0.05;
    ball.restitution = 0.0;
    state.bodies.push(ball);

    let start_x = state.bodies[0].position[0];
    run_seconds(&mut state, 3.0);
    let dx = state.bodies[0].position[0] - start_x;
    assert!(dx < -0.5, "sphere should slide in -X (down the slope), got dx={dx}");
}

// -----------------------------------------------------------------------------
//  SOA solver parity — wired this session
// -----------------------------------------------------------------------------

#[test]
#[ignore = "physx parity test"]
fn soa_solver_parity_with_aos_solver() {
    fn run_with(use_soa: bool) -> [f64; 3] {
        let mut state = PhysXPhysicsState::default();
        state.initialized = true;
        state.gravity = [0.0, -9.81, 0.0];
        state.use_soa_solver = use_soa;
        state.bodies.push(make_static_box("box", [0.0; 3], [2.0, 0.5, 2.0]));
        state.bodies.push(make_dynamic_sphere("ball", [0.0, 5.0, 0.0], 0.5, 1.0));
        let dt = 1.0 / 60.0;
        for _ in 0..120 { step_native(&mut state, dt, 1); }
        state.bodies[1].position
    }
    let aos = run_with(false);
    let soa = run_with(true);
    let dy = (aos[1] - soa[1]).abs();
    assert!(dy < 0.05,
            "SoA vs AoS parity: AoS y={}, SoA y={}, Δ={}",
            aos[1], soa[1], dy);
}

// -----------------------------------------------------------------------------
//  Director chapter 15 features
// -----------------------------------------------------------------------------

#[test]
#[ignore = "physx parity test"]
fn pinned_body_does_not_move() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    let mut ball = make_dynamic_sphere("pinned", [0.0, 3.0, 0.0], 0.5, 1.0);
    ball.pinned = true;
    let initial_pos = ball.position;
    state.bodies.push(ball);

    run_seconds(&mut state, 1.0);
    let pos = state.bodies[0].position;
    assert!(pos[0] == initial_pos[0] && pos[1] == initial_pos[1] && pos[2] == initial_pos[2],
            "pinned body should not move, got {:?}", pos);
    assert_eq!(state.bodies[0].linear_velocity, [0.0; 3],
               "pinned body should have zero velocity");
}

#[test]
#[ignore = "physx parity test"]
fn hanging_spring_extends_under_gravity() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];

    let mut anchor = PhysXRigidBody::default();
    anchor.id = 1;
    anchor.name = "anchor".into();
    anchor.body_type = PhysXBodyType::Static;
    anchor.shape = PhysXShapeKind::Sphere;
    anchor.radius = 0.1;
    anchor.position = [0.0, 5.0, 0.0];
    anchor.mass = 0.0;
    state.bodies.push(anchor);

    let mass = 1.0f64;
    let mut ball = make_dynamic_sphere("ball", [0.0, 4.0, 0.0], 0.2, mass);
    ball.id = 2;
    ball.restitution = 0.0;
    ball.linear_damping = 1.0;
    state.bodies.push(ball);

    let stiffness = 100.0f64;
    let rest_length = 1.0f64;
    let mut spring = PhysXConstraint::default();
    spring.id = 1;
    spring.name = "rope".into();
    spring.kind = PhysXConstraintKind::Spring;
    spring.body_a = Some(state.bodies[0].id);
    spring.body_b = Some(state.bodies[1].id);
    spring.anchor_a = [0.0; 3];
    spring.anchor_b = [0.0; 3];
    spring.stiffness = stiffness;
    spring.damping = 5.0;
    spring.rest_length = rest_length;
    state.constraints.push(spring);

    run_seconds(&mut state, 5.0);
    let y = state.bodies[1].position[1];
    let expected = 5.0 - (rest_length + mass * 9.81 / stiffness);
    assert!((y - expected).abs() < 0.2,
            "hanging ball should rest at y≈{:.3}, got {y}", expected);
}

#[test]
#[ignore = "physx parity test"]
fn collision_pair_filter_lets_sphere_pass_through_box() {
    let mut state = PhysXPhysicsState::default();
    state.initialized = true;
    state.gravity = [0.0, -9.81, 0.0];
    state.bodies.push(make_static_box("floor", [0.0; 3], [10.0, 0.5, 10.0]));
    state.bodies.push(make_dynamic_sphere("ball", [0.0, 5.0, 0.0], 0.5, 1.0));
    let key = if "ball" < "floor" {
        ("ball".to_string(), "floor".to_string())
    } else {
        ("floor".to_string(), "ball".to_string())
    };
    state.disabled_collision_pairs.insert(key);

    run_seconds(&mut state, 2.0);
    let y = state.bodies[1].position[1];
    assert!(y < -5.0,
            "ball should pass through floor when pair disabled, got y={y}");
}

// -----------------------------------------------------------------------------
//  Pure-data sanity checks
// -----------------------------------------------------------------------------

#[test]
#[ignore = "physx parity test"]
fn rtree_builds_and_traverses() {
    let n = 16usize;
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity((n + 1) * (n + 1));
    for j in 0..=n {
        for i in 0..=n {
            verts.push([i as f32, 0.0, j as f32]);
        }
    }
    let mut tris: Vec<u32> = Vec::with_capacity(n * n * 6);
    for j in 0..n {
        for i in 0..n {
            let v00 = (j * (n + 1) + i) as u32;
            let v10 = v00 + 1;
            let v01 = ((j + 1) * (n + 1) + i) as u32;
            let v11 = v01 + 1;
            tris.extend_from_slice(&[v00, v11, v10]);
            tris.extend_from_slice(&[v00, v01, v11]);
        }
    }
    let tri_count = tris.len() / 3;
    let tree = RTreeBuilder::build(&tris, &verts);
    assert!(tree.num_levels >= 2, "tree should be multi-level");

    struct CountCb<'a> {
        seen: &'a mut Vec<bool>,
        count: usize,
        tri_indices: &'a [u32],
    }
    impl<'a> RTreeAabbCallback for CountCb<'a> {
        fn process(&mut self, leaf_encoded: u32) -> bool {
            let lf = LeafTriangles { data: leaf_encoded | 1 };
            let first = lf.triangle_index();
            for k in 0..lf.nb_triangles() {
                let orig = self.tri_indices[(first + k) as usize] as usize;
                assert!(!self.seen[orig], "tri {} reported twice", orig);
                self.seen[orig] = true;
                self.count += 1;
            }
            true
        }
    }
    let mut seen = vec![false; tri_count];
    let mut cb = CountCb { seen: &mut seen, count: 0, tri_indices: &tree.tri_indices };
    tree.traverse_aabb([-100.0, -100.0, -100.0], [100.0, 100.0, 100.0], &mut cb);
    let count = cb.count;
    assert_eq!(count, tri_count, "world-AABB query should reach every tri");
}

#[test]
#[ignore = "physx parity test"]
fn per_tri_contact_gen_against_flat_mesh() {
    let mesh = build_floor_mesh(8);

    let no_contact = sphere_vs_mesh(&mesh, [0.0, 0.7, 0.0], 0.5, 0.0);
    assert_eq!(no_contact.len(), 0, "no contact when sphere clears the floor");

    let contacts = sphere_vs_mesh(&mesh, [0.0, 0.3, 0.0], 0.5, 0.0);
    assert!(!contacts.is_empty(), "sphere penetrating should produce contacts");
    let avg_sep: f32 = contacts.iter().map(|c| c.separation).sum::<f32>() / contacts.len() as f32;
    assert!((avg_sep - -0.2).abs() < 0.05, "avg sep should be ~-0.2, got {avg_sep}");
    let avg_ny: f32 = contacts.iter().map(|c| c.normal[1]).sum::<f32>() / contacts.len() as f32;
    assert!((avg_ny - 1.0).abs() < 0.1, "normals should be +Y, got {avg_ny}");

    let cc = capsule_vs_mesh(&mesh, [-1.0, 0.3, 0.0], [1.0, 0.3, 0.0], 0.5, 0.0);
    assert!(cc.len() >= 4, "horizontal capsule should hit multiple tris, got {}", cc.len());

    let bc = box_vs_mesh(
        &mesh, [0.0, 0.3, 0.0], [0.5, 0.5, 0.5],
        [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 0.0,
    );
    assert!(!bc.is_empty(), "penetrating box should produce contacts");
    let avg_n_y: f32 = bc.iter().map(|c| c.normal[1]).sum::<f32>() / bc.len() as f32;
    assert!(avg_n_y > 0.5,
            "box-vs-tri avg normal should point predominantly +Y, got {avg_n_y}");
}
