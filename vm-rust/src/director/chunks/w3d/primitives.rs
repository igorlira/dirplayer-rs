/// Primitive generator parameter parsing (Plane, Box, Sphere, Cylinder).
/// These just store parameters; actual mesh generation is deferred to rendering.

use super::block_reader::W3dBlockReader;

fn log(msg: &str) {
    web_sys::console::log_1(&format!("[W3D] {}", msg).into());
}

#[derive(Clone, Debug)]
pub enum W3dPrimitive {
    Plane {
        name: String,
        width: f32,
        height: f32,
        width_segs: u32,
        height_segs: u32,
    },
    Box {
        name: String,
        width: f32,
        height: f32,
        depth: f32,
        width_segs: u32,
        height_segs: u32,
        depth_segs: u32,
    },
    Sphere {
        name: String,
        radius: f32,
        segments: u32,
    },
    Cylinder {
        name: String,
        height: f32,
        top_radius: f32,
        bottom_radius: f32,
        height_segs: u32,
        radial_segs: u32,
    },
    ParticleSystem {
        name: String,
    },
    Glyph3D {
        name: String,
    },
    PhysicsMesh {
        name: String,
    },
}

pub fn parse_plane(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    let _chain = r.read_u32()?;
    let _flags = r.read_u32()?;
    let width = r.read_f32()?;
    let height = r.read_f32()?;
    let width_segs = r.read_u32()?;
    let height_segs = r.read_u32()?;
    Ok(W3dPrimitive::Plane { name, width, height, width_segs, height_segs })
}

pub fn parse_box(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    let _chain = r.read_u32()?;
    let _flags = r.read_u32()?;
    // 6x U16 face flags
    for _ in 0..6 { r.read_u16()?; }
    let width = r.read_f32()?;
    let height = r.read_f32()?;
    let depth = r.read_f32()?;
    let width_segs = r.read_u32()?;
    let height_segs = r.read_u32()?;
    let depth_segs = r.read_u32()?;
    Ok(W3dPrimitive::Box { name, width, height, depth, width_segs, height_segs, depth_segs })
}

pub fn parse_sphere(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    let _chain = r.read_u32()?;
    let _flags = r.read_u32()?;
    let radius = r.read_f32()?;
    let _start_lat = r.read_f32()?;
    let _end_lat = r.read_f32()?;
    let segments = r.read_u32()?;
    Ok(W3dPrimitive::Sphere { name, radius, segments })
}

pub fn parse_cylinder(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    let _chain = r.read_u32()?;
    let _flags = r.read_u32()?;
    let _top_cap = r.read_u16()?;
    let _bot_cap = r.read_u16()?;
    let height = r.read_f32()?;
    let top_radius = r.read_f32()?;
    let bottom_radius = r.read_f32()?;
    let _start_angle = r.read_f32()?;
    let _end_angle = r.read_f32()?;
    let height_segs = r.read_u32()?;
    let radial_segs = r.read_u32()?;
    Ok(W3dPrimitive::Cylinder { name, height, top_radius, bottom_radius, height_segs, radial_segs })
}

pub fn parse_particle_system(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    Ok(W3dPrimitive::ParticleSystem { name })
}

pub fn parse_glyph_3d(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    // Read glyph parameters if available
    let depth = if r.remaining() >= 4 { r.read_f32().unwrap_or(1.0) } else { 1.0 };
    let bevel = if r.remaining() >= 4 { r.read_f32().unwrap_or(0.0) } else { 0.0 };
    log(&format!("  Glyph3D: \"{}\" depth={:.2} bevel={:.2} ({} bytes remaining)", name, depth, bevel, r.remaining()));
    Ok(W3dPrimitive::Glyph3D { name })
}

/// Extrude a 2D contour into a 3D mesh along the Z axis.
/// contour: list of 2D points forming a closed polygon.
/// depth: extrusion distance along Z.
/// Returns (positions, normals, faces) for the extruded mesh.
pub fn extrude_contour(contour: &[[f32; 2]], depth: f32) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[u32; 3]>) {
    if contour.len() < 3 { return (vec![], vec![], vec![]); }
    let n = contour.len();
    let mut positions = Vec::with_capacity(n * 2 + n * 2);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut faces = Vec::new();

    // Front face vertices (z = 0)
    for p in contour {
        positions.push([p[0], p[1], 0.0]);
        normals.push([0.0, 0.0, -1.0]);
    }
    // Back face vertices (z = depth)
    for p in contour {
        positions.push([p[0], p[1], depth]);
        normals.push([0.0, 0.0, 1.0]);
    }

    // Triangulate front face (ear-clipping for convex approximation)
    for i in 1..n-1 {
        faces.push([0, i as u32 + 1, i as u32]); // front face winding
    }
    // Triangulate back face (reverse winding)
    for i in 1..n-1 {
        faces.push([n as u32, n as u32 + i as u32, n as u32 + i as u32 + 1]);
    }

    // Side faces (quads as 2 triangles each)
    let side_start = positions.len() as u32;
    for i in 0..n {
        let next = (i + 1) % n;
        let p0 = contour[i];
        let p1 = contour[next];

        // Edge normal (perpendicular to edge, pointing outward)
        let dx = p1[0] - p0[0];
        let dy = p1[1] - p0[1];
        let len = (dx*dx + dy*dy).sqrt().max(1e-8);
        let nx = dy / len;
        let ny = -dx / len;

        let v = positions.len() as u32;
        positions.push([p0[0], p0[1], 0.0]);
        positions.push([p1[0], p1[1], 0.0]);
        positions.push([p1[0], p1[1], depth]);
        positions.push([p0[0], p0[1], depth]);
        for _ in 0..4 {
            normals.push([nx, ny, 0.0]);
        }
        faces.push([v, v+1, v+2]);
        faces.push([v, v+2, v+3]);
    }

    (positions, normals, faces)
}

pub fn parse_physics_mesh(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    Ok(W3dPrimitive::PhysicsMesh { name })
}
