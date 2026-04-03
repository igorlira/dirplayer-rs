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
    pub mesh_id: u32,
    pub vertices: [[f32; 3]; 3],
    pub uv_coord: [f32; 2],
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

/// Generate a picking ray using the IFX/Director approach.
/// Converts a screen pixel to a camera-space film point, then transforms
/// it to world space using the camera's world matrix. No projection or
/// view matrix inversion needed.
///
/// `width`/`height` are the current viewport (sprite) dimensions.
/// `original_width`/`original_height` are the member's default_rect dimensions
/// (used for distToProj and pixelAspect, matching IFX CIFXView).
pub fn screen_to_ray_shockwave(
    screen_x: f32,
    screen_y: f32,
    width: f32,
    height: f32,
    original_width: f32,
    original_height: f32,
    fov_degrees: f32,
    camera_world_matrix: &[f32; 16],
) -> Ray {
    // IFX WindowToFilm: center-origin, flip Y, project onto film plane
    let half_fov_rad = (fov_degrees * 0.5).to_radians();
    // IFX uses originalHeight for distToProj (field_108)
    let dist_to_proj = (original_height * 0.5) / half_fov_rad.tan();
    // IFX pixelAspect = (currentWidth / currentHeight) / (originalWidth / originalHeight)
    let orig_aspect = if original_height > 0.0 { original_width / original_height } else { 1.0 };
    let pixel_aspect = if orig_aspect > 0.0 { (width / height) / orig_aspect } else { 1.0 };

    let film_x = (screen_x - (width - 1.0) * 0.5) * pixel_aspect;
    let film_y = (height - 1.0) * 0.5 - screen_y;
    let film_z = -dist_to_proj;

    // IFX GenerateRay (perspective): transform film point by camera world matrix
    let world_point = transform_point_4x4(camera_world_matrix, film_x, film_y, film_z);

    // Camera world position = translation column of the world matrix
    let origin = [
        camera_world_matrix[12],
        camera_world_matrix[13],
        camera_world_matrix[14],
    ];

    // Direction = worldPoint - origin (normalize for consistent ray math)
    let dir = normalize([
        world_point[0] - origin[0],
        world_point[1] - origin[1],
        world_point[2] - origin[2],
    ]);

    Ray { origin, direction: dir }
}

/// Test ray against all meshes in a scene, returning hits sorted by distance.
pub fn raycast_scene(
    ray: &Ray,
    scene: &W3dScene,
    max_dist: f32,
) -> Option<RayHit> {
    raycast_scene_multi(ray, scene, max_dist, 1, None, None).into_iter().next()
}

/// Test ray against all meshes in a scene, returning up to max_hits sorted by distance.
/// If node_transforms is provided, meshes are tested in world space using model transforms.
/// If excluded_nodes is provided, nodes in the set are skipped (e.g. invisible models).
pub fn raycast_scene_multi(
    ray: &Ray,
    scene: &W3dScene,
    max_dist: f32,
    max_hits: usize,
    node_transforms: Option<&std::collections::HashMap<String, [f32; 16]>>,
    excluded_nodes: Option<&std::collections::HashSet<String>>,
) -> Vec<RayHit> {
    let mut all_hits: Vec<RayHit> = Vec::new();

    // For each model node, find its mesh data and test
    for node in scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model) {
        // Skip excluded nodes (invisible or detached models)
        if let Some(excluded) = excluded_nodes {
            if excluded.contains(&node.name) { continue; }
        }
        let resource = if !node.model_resource_name.is_empty() {
            &node.model_resource_name
        } else {
            &node.resource_name
        };

        // Get model WORLD transform by accumulating parent chain
        let world_transform = {
            let local = if let Some(nt) = node_transforms {
                nt.get(&node.name).cloned()
                    .or_else(|| {
                        // Case-insensitive fallback
                        nt.iter().find(|(k, _)| k.eq_ignore_ascii_case(&node.name)).map(|(_, v)| *v)
                    })
                    .unwrap_or(node.transform)
            } else {
                node.transform
            };
            // Walk parent chain to accumulate world transform
            let mut result = local;
            let mut current_parent = &node.parent_name;
            for _ in 0..20 {
                if current_parent.is_empty() || current_parent.eq_ignore_ascii_case("World") { break; }
                if let Some(pn) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(current_parent)) {
                    let pt = if let Some(nt) = node_transforms {
                        nt.get(&pn.name).cloned()
                            .or_else(|| nt.iter().find(|(k, _)| k.eq_ignore_ascii_case(&pn.name)).map(|(_, v)| *v))
                            .unwrap_or(pn.transform)
                    } else {
                        pn.transform
                    };
                    result = multiply_4x4(&pt, &result);
                    current_parent = &pn.parent_name;
                } else { break; }
            }
            result
        };
        // Debug: log MainA transform
        if node.name == "MainA" {
            static MA_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if MA_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 1 {
                web_sys::console::log_1(&format!(
                    "[RAYCAST] MainA xform: pos=({:.1},{:.1},{:.1}) X=({:.4},{:.4},{:.4}) Y=({:.4},{:.4},{:.4}) Z=({:.4},{:.4},{:.4}) parent='{}'",
                    world_transform[12], world_transform[13], world_transform[14],
                    world_transform[0], world_transform[1], world_transform[2],
                    world_transform[4], world_transform[5], world_transform[6],
                    world_transform[8], world_transform[9], world_transform[10],
                    node.parent_name
                ).into());
            }
        }
        let inv_transform = invert_4x4(&world_transform);
        let local_ray = Ray {
            origin: transform_point_4x4(&inv_transform, ray.origin[0], ray.origin[1], ray.origin[2]),
            direction: transform_dir_4x4(&inv_transform, ray.direction[0], ray.direction[1], ray.direction[2]),
        };

        // Debug: log MainA sub-mesh info and check floor face on first call
        if node.name == "MainA" {
            static MA_MESH_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if MA_MESH_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 1 {
                if let Some(meshes) = scene.clod_meshes.get(resource.as_str()) {
                    for (mi, m) in meshes.iter().enumerate() {
                        let v0z = if !m.positions.is_empty() { m.positions[0][2] } else { -999.0 };
                        web_sys::console::log_1(&format!(
                            "[RAYCAST-MESH] MainA sub[{}]: {} verts, {} faces, v0z={:.1}",
                            mi, m.positions.len(), m.faces.len(), v0z
                        ).into());
                    }
                    // Check sub[4] face 35 specifically (should be the floor)
                    if meshes.len() > 4 && meshes[4].faces.len() > 35 {
                        let f = &meshes[4].faces[35];
                        let m = &meshes[4];
                        web_sys::console::log_1(&format!(
                            "[FLOOR-FACE] sub[4] face35=[{},{},{}] v{}=({:.1},{:.1},{:.1}) v{}=({:.1},{:.1},{:.1}) v{}=({:.1},{:.1},{:.1})",
                            f[0], f[1], f[2],
                            f[0], m.positions.get(f[0] as usize).map(|p| p[0]).unwrap_or(-1.0),
                                  m.positions.get(f[0] as usize).map(|p| p[1]).unwrap_or(-1.0),
                                  m.positions.get(f[0] as usize).map(|p| p[2]).unwrap_or(-1.0),
                            f[1], m.positions.get(f[1] as usize).map(|p| p[0]).unwrap_or(-1.0),
                                  m.positions.get(f[1] as usize).map(|p| p[1]).unwrap_or(-1.0),
                                  m.positions.get(f[1] as usize).map(|p| p[2]).unwrap_or(-1.0),
                            f[2], m.positions.get(f[2] as usize).map(|p| p[0]).unwrap_or(-1.0),
                                  m.positions.get(f[2] as usize).map(|p| p[1]).unwrap_or(-1.0),
                                  m.positions.get(f[2] as usize).map(|p| p[2]).unwrap_or(-1.0),
                        ).into());
                        // Test ray intersection manually
                        let lo = [3428.0f32, -5878.0, 219.0]; // local ray origin
                        let ld = [0.0f32, 0.0, -1.0]; // local ray dir
                        let p0 = m.positions[f[0] as usize];
                        let p1 = m.positions[f[1] as usize];
                        let p2 = m.positions[f[2] as usize];
                        if let Some((t, _, _)) = ray_triangle_intersect(
                            &Ray { origin: lo, direction: ld }, &p0, &p1, &p2
                        ) {
                            web_sys::console::log_1(&format!("[FLOOR-FACE] ray hit! t={:.2}", t).into());
                        } else {
                            web_sys::console::log_1(&"[FLOOR-FACE] ray MISS!".into());
                        }
                    }
                }
            }
        }

        // Test CLOD meshes
        if let Some(meshes) = scene.clod_meshes.get(resource.as_str()) {
            // Log local ray for MainA sub[4] on first downward ray
            if node.name == "MainA" && ray.direction[2] < -0.9 && ray.direction[0].abs() < 0.1 && ray.origin[2] > 300.0 {
                static LR_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if LR_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 2 {
                    web_sys::console::log_1(&format!(
                        "[LOCAL-RAY] MainA: world_orig=({:.1},{:.1},{:.1}) local_orig=({:.1},{:.1},{:.1}) local_dir=({:.4},{:.4},{:.4})",
                        ray.origin[0], ray.origin[1], ray.origin[2],
                        local_ray.origin[0], local_ray.origin[1], local_ray.origin[2],
                        local_ray.direction[0], local_ray.direction[1], local_ray.direction[2],
                    ).into());
                }
            }
            for (mi, mesh) in meshes.iter().enumerate() {
                // Debug: for MainA sub[4], manually test face 35 inside the real raycast flow
                if node.name == "MainA" && mi == 4 && ray.direction[2] < -0.9 && ray.direction[0].abs() < 0.1 && ray.origin[2] > 300.0 {
                    static F35_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    if F35_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                        let f = &mesh.faces[35];
                        let p0 = mesh.positions[f[0] as usize];
                        let p1 = mesh.positions[f[1] as usize];
                        let p2 = mesh.positions[f[2] as usize];
                        let result = ray_triangle_intersect(&local_ray, &p0, &p1, &p2);
                        web_sys::console::log_1(&format!(
                            "[SUB4-F35] local_orig=({:.1},{:.1},{:.1}) dir=({:.4},{:.4},{:.4}) face=[{},{},{}] hit={:?} nfaces={}",
                            local_ray.origin[0], local_ray.origin[1], local_ray.origin[2],
                            local_ray.direction[0], local_ray.direction[1], local_ray.direction[2],
                            f[0], f[1], f[2], result, mesh.faces.len()
                        ).into());
                    }
                }
                let tc = mesh.tex_coords.first().map(|v| v.as_slice());
                if let Some(mut hit) = raycast_mesh(&local_ray, &mesh.positions, &mesh.normals, &mesh.faces, tc, &node.name, (mi + 1) as u32, max_dist) {
                    // Transform hit position and vertices back to world space
                    hit.position = transform_point_4x4(&world_transform, hit.position[0], hit.position[1], hit.position[2]);
                    hit.normal = transform_dir_4x4(&world_transform, hit.normal[0], hit.normal[1], hit.normal[2]);
                    for v in &mut hit.vertices {
                        *v = transform_point_4x4(&world_transform, v[0], v[1], v[2]);
                    }
                    let dx = hit.position[0] - ray.origin[0];
                    let dy = hit.position[1] - ray.origin[1];
                    let dz = hit.position[2] - ray.origin[2];
                    hit.distance = (dx*dx + dy*dy + dz*dz).sqrt();
                    if hit.distance <= max_dist {
                        all_hits.push(hit);
                    }
                }
            }
        }

        // Test raw meshes
        for (mi, mesh) in scene.raw_meshes.iter().enumerate() {
            if mesh.name == *resource {
                let tc = if !mesh.tex_coords.is_empty() { Some(mesh.tex_coords.as_slice()) } else { None };
                if let Some(mut hit) = raycast_mesh(&local_ray, &mesh.positions, &mesh.normals, &mesh.faces, tc, &node.name, (mi + 1) as u32, max_dist) {
                    hit.position = transform_point_4x4(&world_transform, hit.position[0], hit.position[1], hit.position[2]);
                    hit.normal = transform_dir_4x4(&world_transform, hit.normal[0], hit.normal[1], hit.normal[2]);
                    for v in &mut hit.vertices {
                        *v = transform_point_4x4(&world_transform, v[0], v[1], v[2]);
                    }
                    let dx = hit.position[0] - ray.origin[0];
                    let dy = hit.position[1] - ray.origin[1];
                    let dz = hit.position[2] - ray.origin[2];
                    hit.distance = (dx*dx + dy*dy + dz*dz).sqrt();
                    if hit.distance <= max_dist {
                        all_hits.push(hit);
                    }
                }
            }
        }
    }

    // Sort by distance, take max_hits
    all_hits.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
    all_hits.truncate(max_hits);
    all_hits
}

/// Transform a direction vector (no translation) by a 4x4 matrix
fn transform_dir_4x4(m: &[f32; 16], x: f32, y: f32, z: f32) -> [f32; 3] {
    normalize([
        m[0] * x + m[4] * y + m[8] * z,
        m[1] * x + m[5] * y + m[9] * z,
        m[2] * x + m[6] * y + m[10] * z,
    ])
}

/// Test ray against a single mesh, using BVH acceleration for large meshes.
fn raycast_mesh(
    ray: &Ray,
    positions: &[[f32; 3]],
    _normals: &[[f32; 3]],
    faces: &[[u32; 3]],
    tex_coords: Option<&[[f32; 2]]>,
    model_name: &str,
    mesh_id: u32,
    max_dist: f32,
) -> Option<RayHit> {
    // BVH disabled temporarily - use brute force for all meshes to match C# reference
    // TODO: debug BVH to find why it misses floor faces
    // if faces.len() > 32 {
    //     let mut indices: Vec<usize> = (0..faces.len()).collect();
    //     let bvh = build_bvh(positions, faces, &mut indices);
    //     return raycast_bvh(ray, &bvh, positions, faces, tex_coords, model_name, mesh_id, max_dist);
    // }

    // Brute-force for small meshes
    let mut closest: Option<RayHit> = None;
    for (face_idx, face) in faces.iter().enumerate() {
        let i0 = face[0] as usize;
        let i1 = face[1] as usize;
        let i2 = face[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() { continue; }
        if let Some((t, u, v)) = ray_triangle_intersect(ray, &positions[i0], &positions[i1], &positions[i2]) {
            // No backface culling - Director's modelsUnderRay tests both sides
            let edge1 = sub(positions[i1], positions[i0]);
            let edge2 = sub(positions[i2], positions[i0]);
            let normal = normalize(cross(edge1, edge2));

            if t > 0.0 && t < max_dist {
                if closest.as_ref().map_or(true, |c| t < c.distance) {
                    let pos = [
                        ray.origin[0] + ray.direction[0] * t,
                        ray.origin[1] + ray.direction[1] * t,
                        ray.origin[2] + ray.direction[2] * t,
                    ];
                    let uv = interpolate_uv(tex_coords, i0, i1, i2, u, v);
                    closest = Some(RayHit {
                        model_name: model_name.to_string(),
                        distance: t,
                        position: pos,
                        normal,
                        face_index: face_idx as u32,
                        mesh_id,
                        vertices: [positions[i0], positions[i1], positions[i2]],
                        uv_coord: uv,
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

// ─── AABB BVH for accelerated ray casting ───

struct Aabb {
    min: [f32; 3],
    max: [f32; 3],
}

impl Aabb {
    fn new() -> Self {
        Self {
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        }
    }

    fn expand_point(&mut self, p: &[f32; 3]) {
        for i in 0..3 {
            if p[i] < self.min[i] { self.min[i] = p[i]; }
            if p[i] > self.max[i] { self.max[i] = p[i]; }
        }
    }

    fn merge(&mut self, other: &Aabb) {
        for i in 0..3 {
            if other.min[i] < self.min[i] { self.min[i] = other.min[i]; }
            if other.max[i] > self.max[i] { self.max[i] = other.max[i]; }
        }
    }

    fn largest_axis(&self) -> usize {
        let dx = self.max[0] - self.min[0];
        let dy = self.max[1] - self.min[1];
        let dz = self.max[2] - self.min[2];
        if dx >= dy && dx >= dz { 0 } else if dy >= dz { 1 } else { 2 }
    }

    fn centroid(&self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }

    /// Ray-AABB intersection using the slab method.
    fn ray_intersect(&self, ray: &Ray, max_dist: f32) -> bool {
        let mut tmin = 0.0f32;
        let mut tmax = max_dist;
        for i in 0..3 {
            let inv_d = if ray.direction[i].abs() > 1e-12 { 1.0 / ray.direction[i] } else { 1e12 };
            let mut t0 = (self.min[i] - ray.origin[i]) * inv_d;
            let mut t1 = (self.max[i] - ray.origin[i]) * inv_d;
            if inv_d < 0.0 { std::mem::swap(&mut t0, &mut t1); }
            if t0 > tmin { tmin = t0; }
            if t1 < tmax { tmax = t1; }
            if tmax < tmin { return false; }
        }
        true
    }
}

enum BvhNode {
    Leaf { face_indices: Vec<usize> },
    Inner { bounds: Aabb, left: Box<BvhNode>, right: Box<BvhNode> },
}

/// Build a BVH from face centroids using top-down median split.
fn build_bvh(positions: &[[f32; 3]], faces: &[[u32; 3]], indices: &mut [usize]) -> BvhNode {
    const MAX_LEAF_SIZE: usize = 8;

    if indices.len() <= MAX_LEAF_SIZE {
        return BvhNode::Leaf { face_indices: indices.to_vec() };
    }

    // Compute bounds of all face centroids
    let mut bounds = Aabb::new();
    for &fi in indices.iter() {
        let f = &faces[fi];
        for &vi in f {
            if (vi as usize) < positions.len() {
                bounds.expand_point(&positions[vi as usize]);
            }
        }
    }

    let axis = bounds.largest_axis();

    // Sort by centroid along largest axis
    indices.sort_by(|&a, &b| {
        let ca = face_centroid(positions, &faces[a]);
        let cb = face_centroid(positions, &faces[b]);
        ca[axis].partial_cmp(&cb[axis]).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mid = indices.len() / 2;
    let (left_idx, right_idx) = indices.split_at_mut(mid);

    let left = build_bvh(positions, faces, left_idx);
    let right = build_bvh(positions, faces, right_idx);

    BvhNode::Inner {
        bounds,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn face_centroid(positions: &[[f32; 3]], face: &[u32; 3]) -> [f32; 3] {
    let i0 = face[0] as usize;
    let i1 = face[1] as usize;
    let i2 = face[2] as usize;
    if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
        return [0.0; 3];
    }
    [
        (positions[i0][0] + positions[i1][0] + positions[i2][0]) / 3.0,
        (positions[i0][1] + positions[i1][1] + positions[i2][1]) / 3.0,
        (positions[i0][2] + positions[i1][2] + positions[i2][2]) / 3.0,
    ]
}

/// Raycast against a BVH tree, returning closest hit.
fn raycast_bvh(
    ray: &Ray,
    node: &BvhNode,
    positions: &[[f32; 3]],
    faces: &[[u32; 3]],
    tex_coords: Option<&[[f32; 2]]>,
    model_name: &str,
    mesh_id: u32,
    max_dist: f32,
) -> Option<RayHit> {
    match node {
        BvhNode::Leaf { face_indices } => {
            let mut closest: Option<RayHit> = None;
            for &fi in face_indices {
                let face = &faces[fi];
                let i0 = face[0] as usize;
                let i1 = face[1] as usize;
                let i2 = face[2] as usize;
                if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() { continue; }
                if let Some((t, u, v)) = ray_triangle_intersect(ray, &positions[i0], &positions[i1], &positions[i2]) {
                    let edge1 = sub(positions[i1], positions[i0]);
                    let edge2 = sub(positions[i2], positions[i0]);
                    let normal = normalize(cross(edge1, edge2));
                    if dot(normal, ray.direction) > 0.0 { continue; }

                    let cdist = closest.as_ref().map(|c| c.distance).unwrap_or(max_dist);
                    if t > 0.0 && t < cdist {
                        let pos = [
                            ray.origin[0] + ray.direction[0] * t,
                            ray.origin[1] + ray.direction[1] * t,
                            ray.origin[2] + ray.direction[2] * t,
                        ];
                        let uv = interpolate_uv(tex_coords, i0, i1, i2, u, v);
                        closest = Some(RayHit {
                            model_name: model_name.to_string(),
                            distance: t,
                            position: pos,
                            normal,
                            face_index: fi as u32,
                            mesh_id,
                            vertices: [positions[i0], positions[i1], positions[i2]],
                            uv_coord: uv,
                        });
                    }
                }
            }
            closest
        }
        BvhNode::Inner { bounds, left, right } => {
            if !bounds.ray_intersect(ray, max_dist) {
                return None;
            }
            let hit_left = raycast_bvh(ray, left, positions, faces, tex_coords, model_name, mesh_id, max_dist);
            let new_max = hit_left.as_ref().map(|h| h.distance).unwrap_or(max_dist);
            let hit_right = raycast_bvh(ray, right, positions, faces, tex_coords, model_name, mesh_id, new_max);

            match (hit_left, hit_right) {
                (Some(l), Some(r)) => if l.distance <= r.distance { Some(l) } else { Some(r) },
                (Some(h), None) | (None, Some(h)) => Some(h),
                (None, None) => None,
            }
        }
    }
}

/// Interpolate UV coordinates using barycentric coords (u, v) from ray-triangle intersection.
/// The barycentric weights are: w0 = 1-u-v, w1 = u, w2 = v.
fn interpolate_uv(tex_coords: Option<&[[f32; 2]]>, i0: usize, i1: usize, i2: usize, u: f32, v: f32) -> [f32; 2] {
    if let Some(tc) = tex_coords {
        if i0 < tc.len() && i1 < tc.len() && i2 < tc.len() {
            let w0 = 1.0 - u - v;
            return [
                w0 * tc[i0][0] + u * tc[i1][0] + v * tc[i2][0],
                w0 * tc[i0][1] + u * tc[i1][1] + v * tc[i2][1],
            ];
        }
    }
    [0.0, 0.0]
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
/// Column-major 4x4 matrix multiply: C = A * B
fn multiply_4x4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] =
                a[0 * 4 + row] * b[col * 4 + 0] +
                a[1 * 4 + row] * b[col * 4 + 1] +
                a[2 * 4 + row] * b[col * 4 + 2] +
                a[3 * 4 + row] * b[col * 4 + 3];
        }
    }
    r
}

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
