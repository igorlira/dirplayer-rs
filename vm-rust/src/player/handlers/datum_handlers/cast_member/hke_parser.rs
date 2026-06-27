/// Parser for Havok binary HKE scene exports used by the Director Xtra.
///
/// Structure:
///   1. Small world header (name, version, worldScale)
///   2. Collision mesh records (marker A9 EE 9F 01 45 30)
///   3. Binary tail with subspace, rigid body, primitive, and action records
///
/// Rigid body properties (mass, restitution, friction, etc.) are in the tail,
/// identified by 4-byte tokens followed by their values.
/// Mass is stored per-primitive; total body mass = sum of primitive masses.

use log::debug;

pub struct HkeCollisionMesh {
    pub name: String,
    pub entry_type: u16,
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// A collision primitive attached to a rigid body.
#[derive(Clone, Debug)]
pub enum HkePrimitiveKind {
    /// References a triangle mesh by name (an ENTRY mesh, possibly shared).
    Mesh { mesh_name: String },
    /// Infinite plane: local `normal` (unit) at signed `dist` from origin.
    Plane { normal: [f32; 3], dist: f32 },
    /// Sphere of `radius` (Havok metres).
    Sphere { radius: f32 },
}

#[derive(Clone, Debug)]
pub struct HkePrimitive {
    pub kind: HkePrimitiveKind,
    /// Primitive-local translation relative to the body origin (Havok metres).
    pub local_translation: [f32; 3],
    pub mass: f32,
}

/// Per-body properties parsed from the HKE tail section.
#[derive(Clone, Debug)]
pub struct HkeBodyProps {
    pub name: String,
    pub total_mass: f32,
    pub restitution: Option<f32>,
    pub static_friction: Option<f32>,
    pub dynamic_friction: Option<f32>,
    pub translation: Option<[f32; 3]>,
    /// Body orientation as axis-angle: `[angle_radians, axisX, axisY, axisZ]`.
    pub orientation: Option<[f32; 4]>,
    /// Initial linear velocity (Havok metres/s).
    pub linear_velocity: Option<[f32; 3]>,
    /// Initial angular velocity (radians/s — scale-independent, NOT × worldScale).
    pub angular_velocity: Option<[f32; 3]>,
    pub active: Option<bool>,
    /// `COLLISIONS_DISABLED` — when true the body is NOT registered with the
    /// collision detector (it still integrates under gravity/forces but never
    /// collides). From `Import`: `if (!collisions_disabled) addRigidBody(...)`.
    pub collisions_disabled: Option<bool>,
    /// `DISPLACEMENT` — the authored CENTER OF MASS (Havok metres). `Import`
    /// writes it to the body's COM slot *after* computeMassProperties, so it
    /// overrides the geometric centre of mass.
    pub displacement: Option<[f32; 3]>,
    /// `CASTS_SHADOWS` — render metadata (display-body shadow); not used by physics.
    pub casts_shadows: Option<bool>,
    pub primitives: Vec<HkePrimitive>,
}

/// Drag action parsed from HKE tail.
#[derive(Clone, Debug)]
pub struct HkeDragAction {
    pub linear_drag: f32,
    pub angular_drag: f32,
}

/// A cable / point-to-point constraint between two bodies (e.g. the hanging
/// lamp). `pivot_*` are body-local attachment points (Havok metres).
#[derive(Clone, Debug)]
pub struct HkeCable {
    pub body_a: String,
    pub pivot_a: [f32; 3],
    pub body_b: String,
    pub pivot_b: [f32; 3],
    pub length: f32,
}

pub struct HkeWorld {
    pub world_name: String,
    pub world_scale: f32,
    /// Collision tolerance authored in the modeler and stored in the HKE subspace
    /// (`havok.tolerance`). The Xtra uses it as the "objects are touching" distance,
    /// which also sets how far above a surface a resting body floats. `initialize()`
    /// uses it when the movie doesn't pass an explicit tolerance argument.
    pub tolerance: Option<f32>,
    pub gravity: Option<[f32; 3]>,
    pub drag: Option<HkeDragAction>,
    pub meshes: Vec<HkeCollisionMesh>,
    pub bodies: Vec<HkeBodyProps>,
    pub cables: Vec<HkeCable>,
}

// --- Markers and tokens ---
//
// HKE field tokens are the ELF/PJW hash of the field's keyword string. In the
// binary HKE every value is preceded by its 4-byte little-endian token id, and
// the field order is fixed per record type. `elf_hash` below reproduces the
// hash; the test module asserts every constant equals elf_hash(its keyword), so
// these ids are exact rather than approximate.

/// HKE token hash (ELF/PJW). `token_id = elf_hash(KEYWORD)`.
pub fn elf_hash(s: &str) -> u32 {
    let mut h: u32 = 0;
    for &c in s.as_bytes() {
        h = h.wrapping_mul(16).wrapping_add(c as u32);
        let g = h & 0xF000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

const ENTRY_MARKER: [u8; 6] = [0xA9, 0xEE, 0x9F, 0x01, 0x45, 0x30];
const ENTRY_SEPARATOR: [u8; 8] = [0xEF, 0xCD, 0xAB, 0x12, 0xC9, 0x0A, 0xA3, 0x0E];

const HEADER_VERSION_TOKEN: [u8; 4] = [0x6E, 0x7E, 0xA7, 0x0A];
const HEADER_WORLD_SCALE_TOKEN: [u8; 4] = [0x05, 0x33, 0x1B, 0x0A];

// Rigid body markers (3 variants: full 16-byte, short 12-byte, tiny 4-byte)
const RIGID_BODY_MARKER: [u8; 16] = [
    0xEF, 0xCD, 0xAB, 0x12, 0x85, 0xCB, 0x34, 0x08,
    0x89, 0x1A, 0x47, 0x0F, 0x99, 0x77, 0xE3, 0x03,
];
const SHORT_RIGID_BODY_MARKER: [u8; 12] = [
    0x85, 0xCB, 0x34, 0x08, 0x89, 0x1A, 0x47, 0x0F,
    0x99, 0x77, 0xE3, 0x03,
];
const TINY_RIGID_BODY_MARKER: [u8; 4] = [0x99, 0x77, 0xE3, 0x03];
const TINY_RIGID_BODY_PREFIX: [u8; 8] = [
    0x85, 0xCB, 0x34, 0x08, 0x89, 0x1A, 0x47, 0x0F,
];

// Primitive markers
const PRIMITIVE_MESH_MARKER: [u8; 8] = [0x55, 0x8D, 0xFA, 0x07, 0x85, 0x54, 0x73, 0x01];
const PRIMITIVE_SPHERE_MARKER: [u8; 8] = [0x55, 0x8D, 0xFA, 0x07, 0xC5, 0x6B, 0x12, 0x06];
const PRIMITIVE_PLANE_MARKER: [u8; 8] = [0x55, 0x8D, 0xFA, 0x07, 0x65, 0x90, 0x88, 0x08];

// Action markers
const DRAG_ACTION_MARKER: [u8; 16] = [
    0xEF, 0xCD, 0xAB, 0x12, 0x8E, 0x06, 0x3B, 0x02,
    0x7E, 0x34, 0x31, 0x07, 0x77, 0xB8, 0x04, 0x00,
];
const DEACTIVATOR_ACTION_MARKER: [u8; 16] = [
    0xEF, 0xCD, 0xAB, 0x12, 0x9E, 0x30, 0x5C, 0x03,
    0x7E, 0x34, 0x31, 0x07, 0xC2, 0x3E, 0x46, 0x0B,
];
const SUBSPACE_MARKER: [u8; 4] = [0x95, 0x05, 0xC6, 0x00];

// Property tokens (4 bytes each, followed by value). Keyword in the trailing
// comment is the verified elf_hash() pre-image (see the test module).
const RESTITUTION_TOKEN: [u8; 4] = [0xF9, 0x6E, 0xC7, 0x08]; // "ELLASTICITY" (sic)
const STATIC_FRICTION_TOKEN: [u8; 4] = [0x0E, 0x67, 0x55, 0x00]; // "STATIC_FRICTION"
const DYNAMIC_FRICTION_TOKEN: [u8; 4] = [0xAE, 0xA1, 0xC1, 0x00]; // "DYNAMIC_FRICTION"
const TRANSLATION_TOKEN: [u8; 4] = [0x0E, 0xEC, 0x5F, 0x08]; // "TRANSLATION"
const ACTIVE_TOKEN: [u8; 4] = [0xA5, 0x8E, 0x58, 0x04]; // "ACTIVE"
const PRIMITIVE_MASS_TOKEN: [u8; 4] = [0x83, 0x16, 0x05, 0x00]; // "MASS"
const ORIENTATION_TOKEN: [u8; 4] = [0x4E, 0x89, 0x86, 0x04]; // "ROTATION" [angle, axisX, axisY, axisZ]
const LINEAR_VELOCITY_TOKEN: [u8; 4] = [0xD9, 0x4A, 0xA5, 0x0C]; // "LINEAR_VELOCITY" [vx, vy, vz] m/s
const ANGULAR_VELOCITY_TOKEN: [u8; 4] = [0xF9, 0x58, 0x11, 0x04]; // "ANGULAR_VELOCITY" [wx, wy, wz] rad/s
const PLANE_NORMAL_TOKEN: [u8; 4] = [0x25, 0x06, 0x55, 0x00]; // "PLANE" [nx, ny, nz, dist]
const SPHERE_RADIUS_TOKEN: [u8; 4] = [0xA3, 0x8E, 0x65, 0x05]; // "RADIUS"

// Per-body fields read from the HKE rigid-body record (Export*::Import order).
// Token ids derived from elf_hash() and checked in the test module.
// COLLISIONS_DISABLED gates whether the body is registered for collision;
// DISPLACEMENT is the authored centre of mass; CASTS_SHADOWS is render-only.
const COLLISIONS_DISABLED_TOKEN: [u8; 4] = [0x24, 0x0D, 0xF8, 0x08]; // "COLLISIONS_DISABLED" bool
const DISPLACEMENT_TOKEN: [u8; 4] = [0x34, 0x9D, 0xF8, 0x01]; // "DISPLACEMENT" vec3 (= centre of mass)
const CASTS_SHADOWS_TOKEN: [u8; 4] = [0xC3, 0x2D, 0xAD, 0x00]; // "CASTS_SHADOWS" bool

// Subspace tokens
const SUBSPACE_GRAVITY_TOKEN: [u8; 4] = [0xD9, 0xAE, 0x66, 0x0C];
const SUBSPACE_TOLERANCE_TOKEN: [u8; 4] = [0x35, 0x1B, 0xA6, 0x00];

// Drag action tokens
const DRAG_LINEAR_TOKEN: [u8; 4] = [0xC7, 0x74, 0x33, 0x06];
const DRAG_ANGULAR_TOKEN: [u8; 4] = [0x57, 0x5C, 0x21, 0x02];

pub fn parse_hke(data: &[u8]) -> HkeWorld {
    let mut world = HkeWorld {
        world_name: String::new(),
        world_scale: 0.0254,
        tolerance: None,
        gravity: None,
        drag: None,
        meshes: Vec::new(),
        bodies: Vec::new(),
        cables: Vec::new(),
    };

    // Parse header
    if data.len() > 5 {
        let mut pos = 5;
        world.world_name = read_null_string(data, &mut pos);
    }
    if let Some(p) = find_bytes(data, &HEADER_WORLD_SCALE_TOKEN, 0) {
        if p + 8 <= data.len() {
            world.world_scale = read_f32(data, p + 4);
        }
    }

    // Parse collision meshes
    let mut tail_start = 0;
    let mut search_pos = 0;
    while search_pos < data.len().saturating_sub(ENTRY_MARKER.len()) {
        let marker_pos = match find_bytes(data, &ENTRY_MARKER, search_pos) {
            Some(pos) => pos,
            None => break,
        };

        if let Some((mesh, end_pos)) = parse_mesh_entry(data, marker_pos) {
            world.meshes.push(mesh);
            tail_start = end_pos;
            search_pos = end_pos;
        } else {
            search_pos = marker_pos + ENTRY_MARKER.len();
        }
    }

    // Parse tail section (rigid bodies, primitives, actions)
    parse_tail(data, tail_start, &mut world);

    // Parse cable / point-to-point constraints (the hanging lamp).
    parse_cables(data, &mut world);

    // Log results
    let movable: Vec<&str> = world.bodies.iter()
        .filter(|b| b.total_mass > 0.0)
        .map(|b| b.name.as_str())
        .collect();
    debug!(
        "HKE parsed: {} meshes, {} bodies ({} movable: {:?}), worldScale={}",
        world.meshes.len(), world.bodies.len(), movable.len(), movable, world.world_scale
    );

    world
}

fn parse_mesh_entry(data: &[u8], marker_pos: usize) -> Option<(HkeCollisionMesh, usize)> {
    let mut pos = marker_pos + ENTRY_MARKER.len();

    if pos + 2 > data.len() { return None; }
    let entry_type = u16::from_le_bytes(data[pos..pos + 2].try_into().ok()?);
    pos += 2;

    let name = read_null_string(data, &mut pos);
    if name.is_empty() { return None; }

    if pos + 4 > data.len() { return None; }
    let vert_count = read_u32(data, pos) as usize;
    pos += 4;
    if vert_count > 100_000 { return None; }

    let vert_bytes = vert_count * 12;
    if pos + vert_bytes > data.len() { return None; }
    let mut vertices = Vec::with_capacity(vert_count);
    for i in 0..vert_count {
        let off = pos + i * 12;
        let x = read_f32(data, off);
        let y = read_f32(data, off + 4);
        let z = read_f32(data, off + 8);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() { return None; }
        vertices.push([x, y, z]);
    }
    pos += vert_bytes;

    if pos + 4 > data.len() { return None; }
    let tri_count = read_u32(data, pos) as usize;
    pos += 4;
    if tri_count > 500_000 { return None; }

    let idx_bytes = tri_count * 12;
    if pos + idx_bytes > data.len() { return None; }
    let mut triangles = Vec::with_capacity(tri_count);
    for i in 0..tri_count {
        let off = pos + i * 12;
        let a = read_u32(data, off);
        let b = read_u32(data, off + 4);
        let c = read_u32(data, off + 8);
        if a as usize >= vert_count || b as usize >= vert_count || c as usize >= vert_count {
            return None;
        }
        triangles.push([a, b, c]);
    }
    pos += idx_bytes;

    // Skip separator if present
    if match_bytes(data, pos, &ENTRY_SEPARATOR) {
        pos += ENTRY_SEPARATOR.len();
    }

    Some((HkeCollisionMesh { name, entry_type, vertices, triangles }, pos))
}

/// Parse the tail section after all collision meshes.
/// Contains subspace, rigid body, primitive, and action records.
fn parse_tail(data: &[u8], start: usize, world: &mut HkeWorld) {
    let mut pos = start;

    while pos < data.len() {
        // Skip entry separators
        if match_bytes(data, pos, &ENTRY_SEPARATOR) {
            pos += ENTRY_SEPARATOR.len();
            continue;
        }

        // Subspace marker — parse gravity
        if match_bytes(data, pos, &SUBSPACE_MARKER) {
            pos += SUBSPACE_MARKER.len();
            let _name = read_null_string(data, &mut pos);
            let end = find_next_top_level_marker(data, pos).unwrap_or(data.len());
            let payload = &data[pos..end];
            if world.gravity.is_none() {
                world.gravity = try_read_vec3_after_token(payload, &SUBSPACE_GRAVITY_TOKEN);
            }
            if world.tolerance.is_none() {
                world.tolerance = try_read_f32_after_token(payload, &SUBSPACE_TOLERANCE_TOKEN);
            }
            pos = end;
            continue;
        }

        // Full rigid body marker (16 bytes)
        if match_bytes(data, pos, &RIGID_BODY_MARKER) {
            parse_rigid_body(data, &mut pos, RIGID_BODY_MARKER.len(), world);
            continue;
        }

        // Short rigid body marker (12 bytes) — must NOT be preceded by separator prefix
        if match_short_rb_marker(data, pos) {
            parse_rigid_body(data, &mut pos, SHORT_RIGID_BODY_MARKER.len(), world);
            continue;
        }

        // Tiny rigid body marker (4 bytes)
        if match_tiny_rb_marker(data, pos) {
            parse_rigid_body(data, &mut pos, TINY_RIGID_BODY_MARKER.len(), world);
            continue;
        }

        // Drag action — parse linear/angular drag
        if match_bytes(data, pos, &DRAG_ACTION_MARKER) {
            pos += DRAG_ACTION_MARKER.len();
            let _name = read_null_string(data, &mut pos);
            let end = find_next_top_level_marker(data, pos).unwrap_or(data.len());
            let payload = &data[pos..end];
            if world.drag.is_none() {
                let linear = try_read_f32_after_token(payload, &DRAG_LINEAR_TOKEN).unwrap_or(0.0);
                let angular = try_read_f32_after_token(payload, &DRAG_ANGULAR_TOKEN).unwrap_or(0.0);
                world.drag = Some(HkeDragAction { linear_drag: linear, angular_drag: angular });
            }
            pos = end;
            continue;
        }

        // Deactivator action
        if match_bytes(data, pos, &DEACTIVATOR_ACTION_MARKER) {
            pos += DEACTIVATOR_ACTION_MARKER.len();
            let _name = read_null_string(data, &mut pos);
            if let Some(next) = find_next_top_level_marker(data, pos) {
                pos = next;
            } else {
                break;
            }
            continue;
        }

        pos += 1;
    }
}

// Cable / point-to-point constraint tokens.
const CABLE_MARKER: [u8; 8] = [0x7E, 0x34, 0x31, 0x07, 0x47, 0x90, 0xA7, 0x05];
const CABLE_BODY_REF: [u8; 3] = [0x9F, 0x73, 0x04]; // followed by body name
const CABLE_PIVOT_A: [u8; 4] = [0x61, 0x3A, 0x3E, 0x05]; // "a:" + vec3
const CABLE_PIVOT_B: [u8; 4] = [0x62, 0x3A, 0x3E, 0x05]; // "b:" + vec3
const CABLE_LENGTH: [u8; 4] = [0xBE, 0x26, 0xCC, 0x0E];

/// Parse every cable constraint record. Each holds two body references
/// (`9F 73 04` + name), an "a:"/"b:" pivot vec3, and a length scalar.
fn parse_cables(data: &[u8], world: &mut HkeWorld) {
    let mut search = 0;
    while let Some(m) = find_bytes(data, &CABLE_MARKER, search) {
        let mut pos = m + CABLE_MARKER.len();
        let _name = read_null_string(data, &mut pos);
        // Bound the record at the next cable marker (or end).
        let end = find_bytes(data, &CABLE_MARKER, pos).unwrap_or(data.len());
        let seg = &data[pos..end];

        // Two body references, in order (A then B).
        let mut body_a = String::new();
        let mut body_b = String::new();
        let mut bsearch = 0;
        while let Some(bi) = find_bytes(seg, &CABLE_BODY_REF, bsearch) {
            let mut np = bi + CABLE_BODY_REF.len();
            let name = read_null_string(seg, &mut np);
            if body_a.is_empty() { body_a = name; } else if body_b.is_empty() { body_b = name; }
            bsearch = bi + CABLE_BODY_REF.len();
        }

        let pivot_a = try_read_vec3_after_token(seg, &CABLE_PIVOT_A).unwrap_or([0.0; 3]);
        let pivot_b = try_read_vec3_after_token(seg, &CABLE_PIVOT_B).unwrap_or([0.0; 3]);
        let length = try_read_f32_after_token(seg, &CABLE_LENGTH).unwrap_or(0.0);

        if !body_b.is_empty() {
            world.cables.push(HkeCable { body_a, pivot_a, body_b, pivot_b, length });
        }
        search = end;
    }
}

fn parse_rigid_body(data: &[u8], pos: &mut usize, marker_len: usize, world: &mut HkeWorld) {
    *pos += marker_len;
    let name = read_null_string(data, pos);

    // Find end of body payload (before next body/primitive/action marker)
    let body_end = find_next_body_boundary(data, *pos).unwrap_or(data.len());
    let payload = &data[*pos..body_end];

    let mut body = HkeBodyProps {
        name,
        total_mass: 0.0,
        restitution: try_read_f32_after_token(payload, &RESTITUTION_TOKEN),
        static_friction: try_read_f32_after_token(payload, &STATIC_FRICTION_TOKEN),
        dynamic_friction: try_read_f32_after_token(payload, &DYNAMIC_FRICTION_TOKEN),
        translation: try_read_vec3_after_token(payload, &TRANSLATION_TOKEN),
        orientation: try_read_vec4_after_token(payload, &ORIENTATION_TOKEN),
        linear_velocity: try_read_vec3_after_token(payload, &LINEAR_VELOCITY_TOKEN),
        angular_velocity: try_read_vec3_after_token(payload, &ANGULAR_VELOCITY_TOKEN),
        active: try_read_bool_after_token(payload, &ACTIVE_TOKEN),
        collisions_disabled: try_read_bool_after_token(payload, &COLLISIONS_DISABLED_TOKEN),
        displacement: try_read_vec3_after_token(payload, &DISPLACEMENT_TOKEN),
        casts_shadows: try_read_bool_after_token(payload, &CASTS_SHADOWS_TOKEN),
        primitives: Vec::new(),
    };

    *pos = body_end;

    // Parse child primitives (geometry + mass).
    while *pos < data.len() {
        let kind_marker = if match_bytes(data, *pos, &PRIMITIVE_MESH_MARKER) {
            Some((PRIMITIVE_MESH_MARKER.len(), 0u8))
        } else if match_bytes(data, *pos, &PRIMITIVE_SPHERE_MARKER) {
            Some((PRIMITIVE_SPHERE_MARKER.len(), 1u8))
        } else if match_bytes(data, *pos, &PRIMITIVE_PLANE_MARKER) {
            Some((PRIMITIVE_PLANE_MARKER.len(), 2u8))
        } else {
            None
        };
        let (marker_len, kind_id) = match kind_marker { Some(v) => v, None => break };

        if let Some(prim) = parse_primitive(data, pos, marker_len, kind_id) {
            body.total_mass += prim.mass;
            body.primitives.push(prim);
        }
    }

    world.bodies.push(body);
}

/// Parse a single collision primitive: name, transform, geometry, mass.
/// `kind_id`: 0 = mesh, 1 = sphere, 2 = plane.
fn parse_primitive(data: &[u8], pos: &mut usize, marker_len: usize, kind_id: u8) -> Option<HkePrimitive> {
    *pos += marker_len;
    let name = read_null_string(data, pos);

    let prim_end = find_next_primitive_boundary(data, *pos).unwrap_or(data.len());
    let payload = &data[*pos..prim_end];
    let mass = try_read_f32_after_token(payload, &PRIMITIVE_MASS_TOKEN).unwrap_or(0.0);
    let local_translation = try_read_vec3_after_token(payload, &TRANSLATION_TOKEN).unwrap_or([0.0; 3]);

    let kind = match kind_id {
        1 => {
            let radius = try_read_f32_after_token(payload, &SPHERE_RADIUS_TOKEN).unwrap_or(0.0);
            HkePrimitiveKind::Sphere { radius }
        }
        2 => {
            let p = try_read_vec4_after_token(payload, &PLANE_NORMAL_TOKEN).unwrap_or([0.0, 0.0, 1.0, 0.0]);
            HkePrimitiveKind::Plane { normal: [p[0], p[1], p[2]], dist: p[3] }
        }
        _ => HkePrimitiveKind::Mesh { mesh_name: name },
    };

    *pos = prim_end;
    Some(HkePrimitive { kind, local_translation, mass })
}

// --- Token readers ---

fn try_read_f32_after_token(data: &[u8], token: &[u8; 4]) -> Option<f32> {
    let idx = find_bytes(data, token, 0)?;
    if idx + 4 + 4 <= data.len() {
        Some(read_f32(data, idx + 4))
    } else {
        None
    }
}

fn try_read_bool_after_token(data: &[u8], token: &[u8; 4]) -> Option<bool> {
    let idx = find_bytes(data, token, 0)?;
    if idx + 4 + 1 <= data.len() {
        Some(data[idx + 4] != 0)
    } else {
        None
    }
}

fn try_read_vec4_after_token(data: &[u8], token: &[u8; 4]) -> Option<[f32; 4]> {
    let idx = find_bytes(data, token, 0)?;
    if idx + 4 + 16 <= data.len() {
        Some([
            read_f32(data, idx + 4),
            read_f32(data, idx + 8),
            read_f32(data, idx + 12),
            read_f32(data, idx + 16),
        ])
    } else {
        None
    }
}

fn try_read_vec3_after_token(data: &[u8], token: &[u8; 4]) -> Option<[f32; 3]> {
    let idx = find_bytes(data, token, 0)?;
    if idx + 4 + 12 <= data.len() {
        Some([
            read_f32(data, idx + 4),
            read_f32(data, idx + 8),
            read_f32(data, idx + 12),
        ])
    } else {
        None
    }
}

// --- Marker matching helpers ---

fn match_short_rb_marker(data: &[u8], pos: usize) -> bool {
    if !match_bytes(data, pos, &SHORT_RIGID_BODY_MARKER) { return false; }
    // Must NOT be preceded by separator prefix (EF CD AB 12)
    if pos >= 4 && data[pos-4] == 0xEF && data[pos-3] == 0xCD && data[pos-2] == 0xAB && data[pos-1] == 0x12 {
        return false;
    }
    true
}

fn match_tiny_rb_marker(data: &[u8], pos: usize) -> bool {
    if !match_bytes(data, pos, &TINY_RIGID_BODY_MARKER) { return false; }
    // Must NOT be preceded by the longer prefix
    if pos >= TINY_RIGID_BODY_PREFIX.len() && match_bytes(data, pos - TINY_RIGID_BODY_PREFIX.len(), &TINY_RIGID_BODY_PREFIX) {
        return false;
    }
    // Next byte must be printable ASCII (start of name)
    let name_start = pos + TINY_RIGID_BODY_MARKER.len();
    if name_start >= data.len() || !data[name_start].is_ascii_graphic() { return false; }
    true
}

/// Find the offset of the next top-level marker (rigid body, action, subspace).
fn find_next_top_level_marker(data: &[u8], start: usize) -> Option<usize> {
    let markers: &[&[u8]] = &[
        &RIGID_BODY_MARKER, &SHORT_RIGID_BODY_MARKER, &TINY_RIGID_BODY_MARKER,
        &DRAG_ACTION_MARKER, &DEACTIVATOR_ACTION_MARKER, &SUBSPACE_MARKER,
    ];
    find_next_any_marker(data, start, markers)
}

/// Find offset of next body/primitive boundary marker.
fn find_next_body_boundary(data: &[u8], start: usize) -> Option<usize> {
    let markers: &[&[u8]] = &[
        &PRIMITIVE_MESH_MARKER, &PRIMITIVE_SPHERE_MARKER, &PRIMITIVE_PLANE_MARKER,
        &RIGID_BODY_MARKER, &SHORT_RIGID_BODY_MARKER, &TINY_RIGID_BODY_MARKER,
        &DRAG_ACTION_MARKER, &DEACTIVATOR_ACTION_MARKER,
    ];
    find_next_any_marker(data, start, markers)
}

/// Find offset of next primitive boundary marker.
fn find_next_primitive_boundary(data: &[u8], start: usize) -> Option<usize> {
    let markers: &[&[u8]] = &[
        &PRIMITIVE_MESH_MARKER, &PRIMITIVE_SPHERE_MARKER, &PRIMITIVE_PLANE_MARKER,
        &RIGID_BODY_MARKER, &SHORT_RIGID_BODY_MARKER, &TINY_RIGID_BODY_MARKER,
        &DRAG_ACTION_MARKER, &DEACTIVATOR_ACTION_MARKER,
    ];
    find_next_any_marker(data, start, markers)
}

fn find_next_any_marker(data: &[u8], start: usize, markers: &[&[u8]]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for marker in markers {
        if let Some(pos) = find_bytes(data, marker, start) {
            best = Some(best.map_or(pos, |b: usize| b.min(pos)));
        }
    }
    best
}

// --- Low-level helpers ---

fn read_f32(data: &[u8], pos: usize) -> f32 {
    f32::from_le_bytes(data[pos..pos + 4].try_into().unwrap_or([0; 4]))
}

fn read_u32(data: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap_or([0; 4]))
}

fn read_null_string(data: &[u8], pos: &mut usize) -> String {
    let start = *pos;
    while *pos < data.len() && data[*pos] != 0 { *pos += 1; }
    let s = std::str::from_utf8(&data[start..*pos]).unwrap_or("").to_string();
    if *pos < data.len() { *pos += 1; } // skip null
    s
}

fn match_bytes(data: &[u8], offset: usize, pattern: &[u8]) -> bool {
    if offset + pattern.len() > data.len() { return false; }
    data[offset..offset + pattern.len()] == *pattern
}

fn find_bytes(data: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || data.len() < needle.len() + start { return None; }
    for i in start..=data.len() - needle.len() {
        if data[i..i + needle.len()] == *needle {
            return Some(i);
        }
    }
    None
}
