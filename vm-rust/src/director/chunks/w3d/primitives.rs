/// Primitive generator parameter parsing (Plane, Box, Sphere, Cylinder).
/// These just store parameters; actual mesh generation is deferred to rendering.

use super::block_reader::W3dBlockReader;

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
    Ok(W3dPrimitive::Glyph3D { name })
}

pub fn parse_physics_mesh(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    Ok(W3dPrimitive::PhysicsMesh { name })
}
