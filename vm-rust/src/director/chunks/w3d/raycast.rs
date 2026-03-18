//! Ray casting utilities for 3D picking (modelUnderLoc, modelUnderRay).

use super::types::*;

pub struct Ray {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
}

pub struct RayHit {
    pub model_name: String,
    pub distance: f32,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub face_index: u32,
}

/// Unproject a screen point to a world-space ray.
/// screen_x, screen_y are in [0, width] x [0, height].
/// Returns (ray_origin, ray_direction).
pub fn screen_to_ray(
    screen_x: f32,
    screen_y: f32,
    width: f32,
    height: f32,
    view_matrix: &[f32; 16],
    projection_matrix: &[f32; 16],
) -> Ray {
    // Convert screen coords to NDC (-1 to 1)
    let ndc_x = (2.0 * screen_x / width) - 1.0;
    let ndc_y = 1.0 - (2.0 * screen_y / height); // flip Y

    // Inverse projection * inverse view
    let inv_proj = invert_4x4(projection_matrix);
    let inv_view = invert_4x4(view_matrix);

    // Near point in clip space → NDC → view space → world space
    let near_clip = transform_point_4x4(&inv_proj, ndc_x, ndc_y, -1.0);
    let far_clip = transform_point_4x4(&inv_proj, ndc_x, ndc_y, 1.0);

    let near_world = transform_point_4x4(&inv_view, near_clip[0], near_clip[1], near_clip[2]);
    let far_world = transform_point_4x4(&inv_view, far_clip[0], far_clip[1], far_clip[2]);

    let dir = normalize([
        far_world[0] - near_world[0],
        far_world[1] - near_world[1],
        far_world[2] - near_world[2],
    ]);

    Ray {
        origin: near_world,
        direction: dir,
    }
}

/// Test ray against all meshes in a scene, returning the closest hit.
pub fn raycast_scene(
    ray: &Ray,
    scene: &W3dScene,
    max_dist: f32,
) -> Option<RayHit> {
    let mut closest: Option<RayHit> = None;

    // Test against CLOD meshes
    for (resource_name, meshes) in &scene.clod_meshes {
        // Find the model node that uses this resource
        let model_name = scene.nodes.iter()
            .find(|n| n.node_type == W3dNodeType::Model && (n.model_resource_name == *resource_name || n.resource_name == *resource_name))
            .map(|n| n.name.clone())
            .unwrap_or_else(|| resource_name.clone());

        for mesh in meshes {
            if let Some(hit) = raycast_mesh(ray, &mesh.positions, &mesh.normals, &mesh.faces, &model_name, max_dist) {
                if closest.as_ref().map_or(true, |c| hit.distance < c.distance) {
                    closest = Some(hit);
                }
            }
        }
    }

    // Test against raw meshes
    for mesh in &scene.raw_meshes {
        if let Some(hit) = raycast_mesh(ray, &mesh.positions, &mesh.normals, &mesh.faces, &mesh.name, max_dist) {
            if closest.as_ref().map_or(true, |c| hit.distance < c.distance) {
                closest = Some(hit);
            }
        }
    }

    closest
}

/// Test ray against a single mesh
fn raycast_mesh(
    ray: &Ray,
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    faces: &[[u32; 3]],
    model_name: &str,
    max_dist: f32,
) -> Option<RayHit> {
    let mut closest: Option<RayHit> = None;

    for (face_idx, face) in faces.iter().enumerate() {
        let i0 = face[0] as usize;
        let i1 = face[1] as usize;
        let i2 = face[2] as usize;

        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }

        let v0 = positions[i0];
        let v1 = positions[i1];
        let v2 = positions[i2];

        if let Some((t, _u, _v)) = ray_triangle_intersect(ray, &v0, &v1, &v2) {
            if t > 0.0 && t < max_dist {
                if closest.as_ref().map_or(true, |c| t < c.distance) {
                    let pos = [
                        ray.origin[0] + ray.direction[0] * t,
                        ray.origin[1] + ray.direction[1] * t,
                        ray.origin[2] + ray.direction[2] * t,
                    ];

                    // Compute face normal
                    let edge1 = sub(v1, v0);
                    let edge2 = sub(v2, v0);
                    let normal = normalize(cross(edge1, edge2));

                    closest = Some(RayHit {
                        model_name: model_name.to_string(),
                        distance: t,
                        position: pos,
                        normal,
                        face_index: face_idx as u32,
                    });
                }
            }
        }
    }

    closest
}

/// Möller–Trumbore ray-triangle intersection.
/// Returns (t, u, v) if intersection found.
fn ray_triangle_intersect(
    ray: &Ray,
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
) -> Option<(f32, f32, f32)> {
    let edge1 = sub(*v1, *v0);
    let edge2 = sub(*v2, *v0);

    let h = cross(ray.direction, edge2);
    let a = dot(edge1, h);

    if a.abs() < 1e-8 {
        return None; // Parallel
    }

    let f = 1.0 / a;
    let s = sub(ray.origin, *v0);
    let u = f * dot(s, h);

    if u < 0.0 || u > 1.0 {
        return None;
    }

    let q = cross(s, edge1);
    let v = f * dot(ray.direction, q);

    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let t = f * dot(edge2, q);
    if t > 1e-6 {
        Some((t, u, v))
    } else {
        None
    }
}

// ─── Vector math helpers ───

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-8 { [v[0] / len, v[1] / len, v[2] / len] } else { [0.0, 0.0, 1.0] }
}

fn transform_point_4x4(m: &[f32; 16], x: f32, y: f32, z: f32) -> [f32; 3] {
    // Column-major matrix * point with perspective divide
    let w = m[3] * x + m[7] * y + m[11] * z + m[15];
    let w = if w.abs() > 1e-8 { w } else { 1.0 };
    [
        (m[0] * x + m[4] * y + m[8] * z + m[12]) / w,
        (m[1] * x + m[5] * y + m[9] * z + m[13]) / w,
        (m[2] * x + m[6] * y + m[10] * z + m[14]) / w,
    ]
}

/// General 4x4 matrix inverse (column-major)
fn invert_4x4(m: &[f32; 16]) -> [f32; 16] {
    let mut inv = [0.0f32; 16];

    inv[0] = m[5]*m[10]*m[15] - m[5]*m[11]*m[14] - m[9]*m[6]*m[15] + m[9]*m[7]*m[14] + m[13]*m[6]*m[11] - m[13]*m[7]*m[10];
    inv[4] = -m[4]*m[10]*m[15] + m[4]*m[11]*m[14] + m[8]*m[6]*m[15] - m[8]*m[7]*m[14] - m[12]*m[6]*m[11] + m[12]*m[7]*m[10];
    inv[8] = m[4]*m[9]*m[15] - m[4]*m[11]*m[13] - m[8]*m[5]*m[15] + m[8]*m[7]*m[13] + m[12]*m[5]*m[11] - m[12]*m[7]*m[9];
    inv[12] = -m[4]*m[9]*m[14] + m[4]*m[10]*m[13] + m[8]*m[5]*m[14] - m[8]*m[6]*m[13] - m[12]*m[5]*m[10] + m[12]*m[6]*m[9];
    inv[1] = -m[1]*m[10]*m[15] + m[1]*m[11]*m[14] + m[9]*m[2]*m[15] - m[9]*m[3]*m[14] - m[13]*m[2]*m[11] + m[13]*m[3]*m[10];
    inv[5] = m[0]*m[10]*m[15] - m[0]*m[11]*m[14] - m[8]*m[2]*m[15] + m[8]*m[3]*m[14] + m[12]*m[2]*m[11] - m[12]*m[3]*m[10];
    inv[9] = -m[0]*m[9]*m[15] + m[0]*m[11]*m[13] + m[8]*m[1]*m[15] - m[8]*m[3]*m[13] - m[12]*m[1]*m[11] + m[12]*m[3]*m[9];
    inv[13] = m[0]*m[9]*m[14] - m[0]*m[10]*m[13] - m[8]*m[1]*m[14] + m[8]*m[2]*m[13] + m[12]*m[1]*m[10] - m[12]*m[2]*m[9];
    inv[2] = m[1]*m[6]*m[15] - m[1]*m[7]*m[14] - m[5]*m[2]*m[15] + m[5]*m[3]*m[14] + m[13]*m[2]*m[7] - m[13]*m[3]*m[6];
    inv[6] = -m[0]*m[6]*m[15] + m[0]*m[7]*m[14] + m[4]*m[2]*m[15] - m[4]*m[3]*m[14] - m[12]*m[2]*m[7] + m[12]*m[3]*m[6];
    inv[10] = m[0]*m[5]*m[15] - m[0]*m[7]*m[13] - m[4]*m[1]*m[15] + m[4]*m[3]*m[13] + m[12]*m[1]*m[7] - m[12]*m[3]*m[5];
    inv[14] = -m[0]*m[5]*m[14] + m[0]*m[6]*m[13] + m[4]*m[1]*m[14] - m[4]*m[2]*m[13] - m[12]*m[1]*m[6] + m[12]*m[2]*m[5];
    inv[3] = -m[1]*m[6]*m[11] + m[1]*m[7]*m[10] + m[5]*m[2]*m[11] - m[5]*m[3]*m[10] - m[9]*m[2]*m[7] + m[9]*m[3]*m[6];
    inv[7] = m[0]*m[6]*m[11] - m[0]*m[7]*m[10] - m[4]*m[2]*m[11] + m[4]*m[3]*m[10] + m[8]*m[2]*m[7] - m[8]*m[3]*m[6];
    inv[11] = -m[0]*m[5]*m[11] + m[0]*m[7]*m[9] + m[4]*m[1]*m[11] - m[4]*m[3]*m[9] - m[8]*m[1]*m[7] + m[8]*m[3]*m[5];
    inv[15] = m[0]*m[5]*m[10] - m[0]*m[6]*m[9] - m[4]*m[1]*m[10] + m[4]*m[2]*m[9] + m[8]*m[1]*m[6] - m[8]*m[2]*m[5];

    let det = m[0]*inv[0] + m[1]*inv[4] + m[2]*inv[8] + m[3]*inv[12];
    if det.abs() < 1e-10 {
        return [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
    }

    let inv_det = 1.0 / det;
    for i in 0..16 {
        inv[i] *= inv_det;
    }
    inv
}
