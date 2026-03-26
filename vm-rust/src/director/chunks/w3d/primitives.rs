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

fn append_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    tex_coords: &mut Vec<[f32; 2]>,
    faces: &mut Vec<[u32; 3]>,
    quad: [[f32; 3]; 4],
    uv_quad: [[f32; 2]; 4],
    normal: [f32; 3],
) {
    let base = positions.len() as u32;
    positions.extend_from_slice(&quad);
    normals.extend_from_slice(&[normal; 4]);
    tex_coords.extend_from_slice(&uv_quad);
    faces.push([base, base + 2, base + 1]);
    faces.push([base, base + 3, base + 2]);
}

/// Extrude a native-rendered text alpha mask into a 3D glyph mesh.
///
/// This is intentionally pixel-based rather than outline-based: it works for
/// movies that rely on system/native fonts and therefore have no embedded PFR1
/// outline data available to the player.
pub fn extrude_alpha_mask_to_mesh(
    width: u32,
    height: u32,
    rgba: &[u8],
    world_width: f32,
    world_height: f32,
    tunnel_depth: f32,
    bevel_depth: f32,
    bevel_type: u32,
    smoothness: u32,
) -> super::types::ClodDecodedMesh {
    let depth = tunnel_depth.max(1.0);
    if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return super::types::ClodDecodedMesh::default();
    }

    let world_width = world_width.max(1.0);
    let world_height = world_height.max(1.0);
    let pixel_width = world_width / (width as f32).max(1.0);
    let pixel_height = world_height / (height as f32).max(1.0);
    let alpha_threshold = 24u8;

    let idx = |x: u32, y: u32| -> usize { ((y as usize) * (width as usize) + (x as usize)) * 4 + 3 };
    let mut base_mask = vec![false; (width as usize) * (height as usize)];
    for y in 0..height {
        for x in 0..width {
            base_mask[(y as usize) * (width as usize) + (x as usize)] =
                rgba[idx(x, y)] >= alpha_threshold;
        }
    }
    if !base_mask.iter().any(|&p| p) {
        return super::types::ClodDecodedMesh::default();
    }

    let erode_mask = |mask: &[bool]| -> Vec<bool> {
        let mut next = vec![false; mask.len()];
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let i = (y as usize) * (width as usize) + (x as usize);
                if !mask[i] {
                    continue;
                }
                let left = x > 0 && mask[i - 1];
                let right = x + 1 < width as i32 && mask[i + 1];
                let up = y > 0 && mask[i - width as usize];
                let down = y + 1 < height as i32 && mask[i + width as usize];
                next[i] = left && right && up && down;
            }
        }
        next
    };

    let bevel_world = if bevel_type == 0 {
        0.0
    } else {
        bevel_depth.max(0.0).min(depth * 0.5)
    };
    let bevel_step_limit = ((smoothness.max(1) as usize) / 2).clamp(1, 6);
    let mut erosion_levels: Vec<Vec<bool>> = Vec::new();
    if bevel_world > 0.0 {
        let mut current = base_mask.clone();
        for _ in 0..bevel_step_limit {
            let next = erode_mask(&current);
            if !next.iter().any(|&p| p) {
                break;
            }
            erosion_levels.push(next.clone());
            current = next;
        }
    }

    let bevel_steps = erosion_levels.len();
    let mut slab_masks: Vec<Vec<bool>> = Vec::new();
    let mut z_edges: Vec<f32> = vec![0.0];

    if bevel_steps > 0 {
        let step_depth = bevel_world / bevel_steps as f32;
        for level in (0..bevel_steps).rev() {
            slab_masks.push(erosion_levels[level].clone());
            z_edges.push(z_edges.last().copied().unwrap_or(0.0) + step_depth);
        }
    }

    let body_depth = (depth - bevel_world * 2.0).max(0.0);
    if body_depth > 0.0 || slab_masks.is_empty() {
        slab_masks.push(base_mask.clone());
        z_edges.push(z_edges.last().copied().unwrap_or(0.0) + body_depth.max(depth));
    }

    if bevel_steps > 0 {
        let step_depth = bevel_world / bevel_steps as f32;
        for level in 0..bevel_steps {
            slab_masks.push(erosion_levels[level].clone());
            z_edges.push(z_edges.last().copied().unwrap_or(0.0) + step_depth);
        }
    }

    if let Some(last) = z_edges.last_mut() {
        *last = depth;
    }

    let solid = |layer: usize, x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 || layer >= slab_masks.len() {
            return false;
        }
        slab_masks[layer][(y as usize) * (width as usize) + (x as usize)]
    };

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tex_coords = Vec::new();
    let mut faces = Vec::new();

    for layer in 0..slab_masks.len() {
        let z0 = z_edges[layer];
        let z1 = z_edges[layer + 1];
        if z1 <= z0 {
            continue;
        }

        for y in 0..height as i32 {
            for x in 0..width as i32 {
                if !solid(layer, x, y) {
                    continue;
                }

                let x0 = x as f32 * pixel_width;
                let x1 = (x + 1) as f32 * pixel_width;
                let y_top = world_height - (y + 1) as f32 * pixel_height;
                let y_bottom = world_height - y as f32 * pixel_height;
                let u0 = (x as f32 / width as f32).clamp(0.0, 1.0);
                let u1 = ((x + 1) as f32 / width as f32).clamp(0.0, 1.0);
                let v_top = ((y + 1) as f32 / height as f32).clamp(0.0, 1.0);
                let v_bottom = (y as f32 / height as f32).clamp(0.0, 1.0);
                let z0_uv = (1.0 - z0 / depth).clamp(0.0, 1.0);
                let z1_uv = (1.0 - z1 / depth).clamp(0.0, 1.0);

                if layer == 0 || !solid(layer - 1, x, y) {
                    append_quad(
                        &mut positions,
                        &mut normals,
                        &mut tex_coords,
                        &mut faces,
                        [
                            [x0, y_bottom, z0],
                            [x1, y_bottom, z0],
                            [x1, y_top, z0],
                            [x0, y_top, z0],
                        ],
                        [
                            [u0, v_bottom],
                            [u1, v_bottom],
                            [u1, v_top],
                            [u0, v_top],
                        ],
                        [0.0, 0.0, -1.0],
                    );
                }
                if layer + 1 == slab_masks.len() || !solid(layer + 1, x, y) {
                    append_quad(
                        &mut positions,
                        &mut normals,
                        &mut tex_coords,
                        &mut faces,
                        [
                            [x0, y_bottom, z1],
                            [x0, y_top, z1],
                            [x1, y_top, z1],
                            [x1, y_bottom, z1],
                        ],
                        [
                            [u0, v_bottom],
                            [u0, v_top],
                            [u1, v_top],
                            [u1, v_bottom],
                        ],
                        [0.0, 0.0, 1.0],
                    );
                }
                if !solid(layer, x, y - 1) {
                    append_quad(
                        &mut positions,
                        &mut normals,
                        &mut tex_coords,
                        &mut faces,
                        [
                            [x0, y_top, z0],
                            [x1, y_top, z0],
                            [x1, y_top, z1],
                            [x0, y_top, z1],
                        ],
                        [
                            [u0, z0_uv],
                            [u1, z0_uv],
                            [u1, z1_uv],
                            [u0, z1_uv],
                        ],
                        [0.0, -1.0, 0.0],
                    );
                }
                if !solid(layer, x, y + 1) {
                    append_quad(
                        &mut positions,
                        &mut normals,
                        &mut tex_coords,
                        &mut faces,
                        [
                            [x0, y_bottom, z0],
                            [x0, y_bottom, z1],
                            [x1, y_bottom, z1],
                            [x1, y_bottom, z0],
                        ],
                        [
                            [u0, z0_uv],
                            [u0, z1_uv],
                            [u1, z1_uv],
                            [u1, z0_uv],
                        ],
                        [0.0, 1.0, 0.0],
                    );
                }
                if !solid(layer, x - 1, y) {
                    append_quad(
                        &mut positions,
                        &mut normals,
                        &mut tex_coords,
                        &mut faces,
                        [
                            [x0, y_bottom, z0],
                            [x0, y_top, z0],
                            [x0, y_top, z1],
                            [x0, y_bottom, z1],
                        ],
                        [
                            [v_bottom, z0_uv],
                            [v_top, z0_uv],
                            [v_top, z1_uv],
                            [v_bottom, z1_uv],
                        ],
                        [-1.0, 0.0, 0.0],
                    );
                }
                if !solid(layer, x + 1, y) {
                    append_quad(
                        &mut positions,
                        &mut normals,
                        &mut tex_coords,
                        &mut faces,
                        [
                            [x1, y_bottom, z0],
                            [x1, y_bottom, z1],
                            [x1, y_top, z1],
                            [x1, y_top, z0],
                        ],
                        [
                            [v_bottom, z0_uv],
                            [v_bottom, z1_uv],
                            [v_top, z1_uv],
                            [v_top, z0_uv],
                        ],
                        [1.0, 0.0, 0.0],
                    );
                }
            }
        }
    }

    super::types::ClodDecodedMesh {
        name: "Text".to_string(),
        positions,
        normals,
        tex_coords: vec![tex_coords],
        faces,
        diffuse_colors: Vec::new(),
        specular_colors: Vec::new(),
        bone_indices: Vec::new(),
        bone_weights: Vec::new(),
    }
}

pub fn parse_physics_mesh(r: &mut W3dBlockReader) -> Result<W3dPrimitive, String> {
    let name = r.read_ifx_string()?;
    Ok(W3dPrimitive::PhysicsMesh { name })
}

// ─── 3D Text extrusion from PFR glyph outlines ───

use crate::director::chunks::pfr1::types::{OutlineGlyph, PfrCmdType};

/// Flatten a glyph's bezier contours into 2D polylines, scaled and offset.
fn flatten_glyph(
    glyph: &OutlineGlyph,
    scale: f32,
    x_offset: f32,
) -> Vec<Vec<[f32; 2]>> {
    let mut polylines: Vec<Vec<[f32; 2]>> = Vec::new();
    for contour in &glyph.contours {
        let mut pts: Vec<[f32; 2]> = Vec::new();
        let mut cx = 0.0_f32;
        let mut cy = 0.0_f32;
        for cmd in &contour.commands {
            match cmd.cmd_type {
                PfrCmdType::MoveTo => {
                    if pts.len() >= 3 {
                        polylines.push(pts);
                    }
                    pts = Vec::new();
                    cx = cmd.x * scale + x_offset;
                    cy = cmd.y * scale;
                    pts.push([cx, cy]);
                }
                PfrCmdType::LineTo => {
                    cx = cmd.x * scale + x_offset;
                    cy = cmd.y * scale;
                    pts.push([cx, cy]);
                }
                PfrCmdType::CurveTo => {
                    let x1 = cmd.x1 * scale + x_offset;
                    let y1 = cmd.y1 * scale;
                    let x2 = cmd.x2 * scale + x_offset;
                    let y2 = cmd.y2 * scale;
                    let x3 = cmd.x * scale + x_offset;
                    let y3 = cmd.y * scale;
                    // Recursive bezier flattening
                    let mut flat = Vec::new();
                    flatten_bezier_recursive(cx, cy, x1, y1, x2, y2, x3, y3, 0.5, 0, &mut flat);
                    flat.push((x3, y3));
                    for (fx, fy) in flat {
                        pts.push([fx, fy]);
                    }
                    cx = x3;
                    cy = y3;
                }
                PfrCmdType::Close => {
                    // Remove duplicate closing point if present
                    if pts.len() >= 2 {
                        let first = pts[0];
                        let last = pts[pts.len() - 1];
                        if (first[0] - last[0]).abs() < 0.01 && (first[1] - last[1]).abs() < 0.01 {
                            pts.pop();
                        }
                    }
                }
            }
        }
        if pts.len() >= 3 {
            polylines.push(pts);
        }
    }
    polylines
}

fn flatten_bezier_recursive(
    x0: f32, y0: f32, x1: f32, y1: f32,
    x2: f32, y2: f32, x3: f32, y3: f32,
    tolerance: f32, depth: u32, out: &mut Vec<(f32, f32)>,
) {
    if depth > 10 { out.push((x3, y3)); return; }
    let dx = x3 - x0;
    let dy = y3 - y0;
    let d1 = ((x1 - x3) * dy - (y1 - y3) * dx).abs();
    let d2 = ((x2 - x3) * dy - (y2 - y3) * dx).abs();
    let d = d1 + d2;
    let len_sq = dx * dx + dy * dy;
    if d * d <= tolerance * tolerance * len_sq {
        out.push((x3, y3));
        return;
    }
    let mx01 = (x0+x1)*0.5; let my01 = (y0+y1)*0.5;
    let mx12 = (x1+x2)*0.5; let my12 = (y1+y2)*0.5;
    let mx23 = (x2+x3)*0.5; let my23 = (y2+y3)*0.5;
    let mx012 = (mx01+mx12)*0.5; let my012 = (my01+my12)*0.5;
    let mx123 = (mx12+mx23)*0.5; let my123 = (my12+my23)*0.5;
    let mx0123 = (mx012+mx123)*0.5; let my0123 = (my012+my123)*0.5;
    flatten_bezier_recursive(x0,y0, mx01,my01, mx012,my012, mx0123,my0123, tolerance, depth+1, out);
    flatten_bezier_recursive(mx0123,my0123, mx123,my123, mx23,my23, x3,y3, tolerance, depth+1, out);
}

/// Extrude text string into a 3D mesh using PFR glyph outlines.
pub fn extrude_text_to_mesh(
    text: &str,
    glyphs: &std::collections::HashMap<u8, OutlineGlyph>,
    outline_resolution: u16,
    font_size: f32,
    tunnel_depth: f32,
) -> super::types::ClodDecodedMesh {
    let scale = font_size / outline_resolution.max(1) as f32;
    let depth = tunnel_depth.max(1.0);

    // First pass: compute total width for centering
    let mut total_width = 0.0_f32;
    for ch in text.chars() {
        if let Some(glyph) = glyphs.get(&(ch as u8)) {
            total_width += glyph.set_width * scale;
        }
    }
    let x_start = -total_width / 2.0;

    // Second pass: flatten and extrude each character
    let mut all_positions: Vec<[f32; 3]> = Vec::new();
    let mut all_normals: Vec<[f32; 3]> = Vec::new();
    let mut all_faces: Vec<[u32; 3]> = Vec::new();
    let mut x_cursor = x_start;

    for ch in text.chars() {
        let glyph = match glyphs.get(&(ch as u8)) {
            Some(g) => g,
            None => { x_cursor += font_size * 0.5; continue; } // unknown char: advance
        };
        let advance = glyph.set_width * scale;
        if glyph.contours.is_empty() {
            x_cursor += advance;
            continue; // space or blank glyph
        }

        let polylines = flatten_glyph(glyph, scale, x_cursor);
        for poly in &polylines {
            let base = all_positions.len() as u32;
            let (pos, nrm, faces) = extrude_contour(
                &poly.iter().map(|p| [p[0], p[1]]).collect::<Vec<_>>(),
                depth,
            );
            all_positions.extend_from_slice(&pos);
            all_normals.extend_from_slice(&nrm);
            for f in &faces {
                all_faces.push([f[0] + base, f[1] + base, f[2] + base]);
            }
        }
        x_cursor += advance;
    }

    super::types::ClodDecodedMesh {
        name: "Text".to_string(),
        positions: all_positions,
        normals: all_normals,
        tex_coords: Vec::new(),
        faces: all_faces,
        diffuse_colors: Vec::new(),
        specular_colors: Vec::new(),
        bone_indices: Vec::new(),
        bone_weights: Vec::new(),
    }
}
