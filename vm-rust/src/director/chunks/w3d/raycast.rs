//! Ray casting utilities for 3D picking (modelUnderLoc, modelUnderRay).

use log::debug;

use crate::player::symbols::{builtin::BuiltInSymbol, symbol::Symbol};

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
/// Pick ray for an ORTHOGRAPHIC camera (`camera.projection = #orthographic`).
///
/// Under a perspective camera every ray fans out from the camera position; under
/// an orthographic one they are all parallel to the camera's forward axis and it
/// is the ORIGIN that slides across the film plane. Feeding an orthographic
/// camera through `screen_to_ray_shockwave` therefore aims almost every pixel
/// somewhere it should not go, and picking misses entirely — AreaZero's menu is
/// drawn by an orthographic camera, so none of its buttons could be hit.
///
/// Matches the renderer's projection exactly: `orthoHeight` world units span the
/// viewport vertically and the horizontal extent follows the viewport aspect, so
/// one world unit maps to `height / ortho_height` pixels.
pub fn screen_to_ray_orthographic(
    screen_x: f32,
    screen_y: f32,
    width: f32,
    height: f32,
    original_width: f32,
    original_height: f32,
    ortho_height: f32,
    camera_world_matrix: &[f32; 16],
) -> Ray {
    // Same film-plane convention as the perspective path: centre origin, Y up.
    //
    // NO pixel-aspect correction here, unlike the perspective path. The renderer
    // builds the orthographic frustum as `half_w = half_h * (width / height)`, so
    // world-units-per-pixel is `ortho_height / height` on BOTH axes and the
    // member's original (default_rect) aspect never enters. Applying the
    // perspective path's `pixel_aspect` skewed X by that ratio — with a 320x240
    // member on an 800x450 stage it displaced the pick by 4/3, so a click at
    // x=305 probed x=274 and the menu buttons never registered.
    let _ = (original_width, original_height);
    let film_x = screen_x - (width - 1.0) * 0.5;
    let film_y = (height - 1.0) * 0.5 - screen_y;

    // Pixels -> world units. `ortho_height` spans the viewport vertically.
    let units_per_px = if height > 0.0 { ortho_height / height } else { 1.0 };

    // Origin: the film point itself, placed on the camera's near plane.
    let origin = transform_point_4x4(
        camera_world_matrix,
        film_x * units_per_px,
        film_y * units_per_px,
        0.0,
    );

    // Direction: the camera's forward axis (-Z in camera space), rotation only.
    let fwd = transform_point_4x4(camera_world_matrix, 0.0, 0.0, -1.0);
    let dir = normalize([
        fwd[0] - camera_world_matrix[12],
        fwd[1] - camera_world_matrix[13],
        fwd[2] - camera_world_matrix[14],
    ]);

    Ray { origin, direction: dir }
}

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
    // IFX WindowToFilm: center-origin, flip Y, project onto film plane.
    // Use the VIEWPORT height for distToProj so film coords and projection
    // distance are in the same pixel space. The original_height was used by
    // IFX for the member's internal resolution, but screen_x/screen_y are in
    // viewport pixels — mixing the two scales produces wrong ray angles.
    let half_fov_rad = (fov_degrees * 0.5).to_radians();
    let dist_to_proj = (height * 0.5) / half_fov_rad.tan();

    // NO pixel-aspect correction — the same correction the orthographic path
    // above had to drop, for the same reason. The renderer builds the frustum as
    // `perspective(fov_y, sprite_width / sprite_height)`, i.e. the camera's
    // fieldOfView is VERTICAL and the horizontal extent follows the viewport
    // aspect; the member's original (default_rect) aspect never enters it.
    // Scaling film_x by `(width/height) / original_aspect` therefore skewed the
    // pick ray horizontally by exactly that ratio whenever a member's authored
    // rect and its sprite disagreed. Bottle Rocket is a 320x240 member on a
    // 750x405 sprite, so picking ran at 0.72x the rendered width: its fuse is
    // drawn at x=392..396 but was pickable only at x=388..390 — no overlap at
    // all, so clicking the fuse could never launch the rocket.
    //
    // With this gone the ray is the exact inverse of the render projection:
    // film_x / dist_to_proj == x_ndc * (width/height) * tan(fov/2).
    let _ = (original_width, original_height);
    let film_x = screen_x - (width - 1.0) * 0.5;
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
    raycast_scene_multi(ray, scene, max_dist, 1, None, None, None, None).into_iter().next()
}

/// Local-space AABB of one model resource's geometry, or `None` when it has no
/// vertices. Computed once per resource per ray cast and reused across every
/// node that instances it — `#maxDistance` culling must not cost a pass over
/// the geometry per node.
fn resource_local_aabb(scene: &W3dScene, resource: &Symbol) -> Option<([f32; 3], [f32; 3])> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut any = false;
    {
        let mut visit = |positions: &[[f32; 3]]| {
            for p in positions {
                for i in 0..3 {
                    if p[i] < min[i] { min[i] = p[i]; }
                    if p[i] > max[i] { max[i] = p[i]; }
                }
                any = true;
            }
        };
        if let Some(meshes) = scene.clod_meshes.get(resource) {
            for mesh in meshes {
                visit(&mesh.positions);
            }
        }
        for mesh in scene.raw_meshes.iter().filter(|m| m.name == *resource) {
            visit(&mesh.positions);
        }
    }
    if any { Some((min, max)) } else { None }
}

/// World-space bounding sphere of a local AABB placed by `world_transform`:
/// the eight transformed corners, their midpoint as the centre and the farthest
/// corner as the radius. A corner-derived sphere is never tighter than the true
/// one, and for `#maxDistance` looser only means MORE models are considered —
/// which the ray then rejects on its own.
fn aabb_world_sphere(
    (min, max): ([f32; 3], [f32; 3]),
    world_transform: &[f32; 16],
) -> ([f32; 3], f32) {
    let mut corners = [[0.0f32; 3]; 8];
    for (i, c) in corners.iter_mut().enumerate() {
        let x = if i & 1 == 0 { min[0] } else { max[0] };
        let y = if i & 2 == 0 { min[1] } else { max[1] };
        let z = if i & 4 == 0 { min[2] } else { max[2] };
        *c = transform_point_4x4(world_transform, x, y, z);
    }
    let mut lo = corners[0];
    let mut hi = corners[0];
    for c in &corners[1..] {
        for i in 0..3 {
            if c[i] < lo[i] { lo[i] = c[i]; }
            if c[i] > hi[i] { hi[i] = c[i]; }
        }
    }
    let center = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let radius = ((hi[0] - center[0]).powi(2)
        + (hi[1] - center[1]).powi(2)
        + (hi[2] - center[2]).powi(2))
        .sqrt();
    (center, radius)
}

/// Test ray against all meshes in a scene, returning up to max_hits sorted by distance.
/// If node_transforms is provided, meshes are tested in world space using model transforms.
/// If excluded_nodes is provided, nodes in the set are skipped (e.g. invisible models).
pub fn raycast_scene_multi(
    ray: &Ray,
    scene: &W3dScene,
    max_dist: f32,
    max_hits: usize,
    node_transforms: Option<&std::collections::HashMap<Symbol, [f32; 16]>>,
    excluded_nodes: Option<&std::collections::HashSet<Symbol>>,
    included_nodes: Option<&std::collections::HashSet<Symbol>>,
    // Per-model animation state: (model, skeleton) -> (motion name, time, rootLock).
    // Supplied by the player, which owns the bonesPlayer state the renderer draws
    // from; passing it as a closure keeps this module free of player types.
    // `None` disables skinned raycasting and tests bind-pose geometry.
    anim: Option<&dyn Fn(Symbol, Symbol) -> Option<(Option<Symbol>, f32, bool)>>,
) -> Vec<RayHit> {
    let mut all_hits: Vec<RayHit> = Vec::new();

    // Progressive tightening. `modelsUnderRay` returns the NEAREST `max_hits`
    // models, so once that many are in hand nothing beyond the current worst can
    // survive — the bound can shrink to it and prune the rest of the scene.
    //
    // This matters because `maxDistance` is optional in Director and therefore
    // UNBOUNDED by default (measured: a ray with no maxDistance returns a hit at
    // distance 499921). Without tightening, an unbounded ray tests every triangle
    // of every model in the member: level 2 of Agent Free Ride went from ~97 to
    // ~198 ms/frame purely from that.
    //
    // `max_dist` is `#maxDistance`, which is NOT a cutoff on the intersection.
    // Director 11.5 Scripting Dictionary, `modelsUnderRay`:
    //
    //   maxDistance — "The maximum distance from the world position specified by
    //   locationVector. If a MODEL'S BOUNDING SPHERE is within the maximum
    //   distance specified, THAT MODEL IS INCLUDED. If the bounding sphere is in
    //   range, then it may contain polygons in range and thus might be
    //   intersected."
    //
    // So it selects MODELS by their bounding sphere and then intersects their
    // polygons with no distance limit at all — a hit may come back far beyond
    // maxDistance. Treating it as a hit-distance cutoff broke thehillshaveeyes:
    // `_controller_FPS.checkFloor` casts straight down with `#maxDistance: 100`
    // to seat the player on `L_C_floor`, but the mine floor under the spawn is
    // ~530 units below. Director includes the model (the ray starts well inside
    // its 3378-unit bounding sphere), returns the hit at 530, and the player
    // stands on the ground; clamped to 100 the ray found nothing and the movie —
    // which has no gravity, only this snap — left the player floating ~480 units
    // above the mine for the whole game.
    //
    // The hit-distance bound therefore starts UNBOUNDED and is only ever
    // tightened by the progressive pruning below.
    let mut work_max = f32::INFINITY;

    // Name -> node index, built once per call. The world transform of each model
    // is accumulated by walking its parent chain, and each level did
    // `scene.nodes.iter().find(|n| n.name == parent)` — a linear scan of every
    // node in the scene. For N nodes at depth d that is O(N^2 * d) symbol
    // comparisons per ray, before a single triangle is touched.
    //
    // Scoped to this call rather than cached on the scene: nodes are added and
    // removed at runtime (clones, removeFromWorld), and a stale index would
    // resolve a parent to the wrong node.
    let node_index: std::collections::HashMap<Symbol, usize> = scene
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name, i))
        .collect();

    // Local AABB per model resource, filled on demand for the `#maxDistance`
    // cull below. Scoped to the call for the same reason `node_index` is: the
    // scene's geometry changes at runtime.
    let mut local_aabb_cache: std::collections::HashMap<Symbol, Option<([f32; 3], [f32; 3])>> =
        std::collections::HashMap::new();

    // For each model node, find its mesh data and test
    for node in scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model) {
        // Skip excluded nodes (invisible or detached models)
        if let Some(excluded) = excluded_nodes {
            if excluded.contains(&node.name) { continue; }
        }
        // #modelList whitelist (modelsUnderRay/Loc optionsList): when provided, only
        // models whose name is in the list are tested; all others are ignored even if
        // under the ray, per the Director 11.5 Scripting Dictionary. `None` means the
        // caller omitted the list entirely => no restriction. A provided-but-EMPTY list
        // includes nothing and therefore matches nothing — the caller decides that, so
        // do NOT re-interpret an empty set as "test all" here.
        if let Some(included) = included_nodes {
            // Hash lookup, not a linear scan. This is a `HashSet<Symbol>` — the
            // `excluded` test above already uses `contains` — but the whitelist
            // was being walked entry by entry with case-insensitive STRING
            // compares, making the filter cost
            // `all_nodes x modelList_len x name_length` per ray.
            //
            // Agent Free Ride passes `#modelList` from its culling manager on
            // every hover-point raycast (4 per vehicle, per frame, plus
            // character physics, shadows, bonuses and the track manager), so
            // this ran constantly.
            //
            // `contains` stays case-insensitive for free: `intern` lowercases,
            // so symbol identity is already case-folded and equality is a spur
            // compare.
            if !included.contains(&node.name) {
                continue;
            }
        }
        let resource = if !node.model_resource_name.is_empty() {
            &node.model_resource_name
        } else {
            &node.resource_name
        };

        // Get model WORLD transform by accumulating parent chain
        let world_transform = {
            // No linear fallback: the map is keyed by `Symbol` and the old
            // `.or_else(|| nt.iter().find(|(k, _)| **k == node.name))` compared
            // symbols — the very same equality the hash lookup uses — so it could
            // never find anything `get` missed. All it did was scan the whole map
            // on every MISS, which is the common case (most nodes carry no
            // override). Symbol identity is already case-insensitive: `intern`
            // lowercases, so `get` is the case-insensitive lookup.
            let local = if let Some(nt) = node_transforms {
                nt.get(&node.name).cloned().unwrap_or(node.transform)
            } else {
                node.transform
            };
            // Walk parent chain to accumulate world transform
            let mut result = local;
            let mut current_parent = &node.parent_name;
            for _ in 0..20 {
                if current_parent.is_empty() || *current_parent == BuiltInSymbol::World { break; }
                if let Some(pn) = node_index.get(current_parent).map(|&i| &scene.nodes[i]) {
                    let pt = if let Some(nt) = node_transforms {
                        nt.get(&pn.name).cloned().unwrap_or(pn.transform)
                    } else {
                        pn.transform
                    };
                    result = multiply_4x4(&pt, &result);
                    current_parent = &pn.parent_name;
                } else { break; }
            }
            result
        };
        // (A one-shot `MainA` transform debug log lived here. `node.name ==
        // BuiltInSymbol::MainA` goes through `PartialEq<BuiltInSymbol>`, which
        // calls `into_builtin()` — a `spur_to_builtin` hash lookup — so it cost
        // one hash lookup per model per ray forever, and in any scene without a
        // `MainA` node it could never fire at all. Removed rather than gated;
        // `git log` has it if the FinalDrive mirror work needs it again.)
        let inv_transform = invert_4x4(&world_transform);
        let local_ray = Ray {
            origin: transform_point_4x4(&inv_transform, ray.origin[0], ray.origin[1], ray.origin[2]),
            direction: transform_dir_4x4(&inv_transform, ray.direction[0], ray.direction[1], ray.direction[2]),
        };
        // `transform_dir_4x4` NORMALISES, so a scaled model's local parametric
        // distance is not the world distance and a world-space bound cannot be
        // handed to the local mesh test. Tighten only for unit-scale models —
        // exact there — and leave scaled ones on the caller's original bound,
        // exactly as before. The world-space filter below uses `work_max`
        // unconditionally, which is always valid.
        let unit_scale = {
            let l = |a: usize, b: usize, c: usize| {
                (world_transform[a] * world_transform[a]
                    + world_transform[b] * world_transform[b]
                    + world_transform[c] * world_transform[c]).sqrt()
            };
            (l(0, 1, 2) - 1.0).abs() < 1e-3
                && (l(4, 5, 6) - 1.0).abs() < 1e-3
                && (l(8, 9, 10) - 1.0).abs() < 1e-3
        };
        let node_max = if unit_scale { work_max } else { f32::INFINITY };

        // `#maxDistance` model cull: skip the whole model when its world-space
        // bounding sphere is farther than maxDistance from the ray's origin.
        // This is the only thing maxDistance does, and it is also what keeps an
        // unbounded ray affordable — a far model costs one sphere test instead
        // of a pass over its triangles. Runs before the skinning below so a
        // culled model costs no pose either; the AABB is the BIND pose, which
        // is what the sphere's generous corner-derived radius is there to
        // absorb.
        if max_dist.is_finite() {
            let aabb = *local_aabb_cache
                .entry(*resource)
                .or_insert_with(|| resource_local_aabb(scene, resource));
            if let Some(aabb) = aabb {
                let (center, radius) = aabb_world_sphere(aabb, &world_transform);
                let dx = center[0] - ray.origin[0];
                let dy = center[1] - ray.origin[1];
                let dz = center[2] - ray.origin[2];
                let to_center = (dx * dx + dy * dy + dz * dz).sqrt();
                if to_center - radius > max_dist {
                    continue;
                }
            }
        }
        // Skinned models are tested against their POSED geometry. `anim` supplies
        // the same (motion, time, rootLock) the renderer is drawing with, so the
        // hit volume tracks the body instead of the bind pose. `None` (no rig, or
        // a caller that passed no animation state) falls through to the raw mesh,
        // which is correct for rigid geometry.
        let skin_pose: Option<Vec<[f32; 16]>> = anim.and_then(|a| {
            let skeleton = scene.skeletons.iter()
                .find(|s| s.name == *resource && s.bones.len() > 1)?;
            let (motion, time, root_lock) = a(node.name, skeleton.name)?;
            let relinv = super::skeleton::root_relativizer(scene, skeleton, node.name, *resource);
            Some(super::skeleton::build_skinning_matrices(
                skeleton,
                motion.and_then(|m| scene.motions.iter().find(|x| x.name == m)),
                time,
                root_lock,
                &relinv,
            ))
        });

        // Handedness of this node's world transform. The ray is tested in LOCAL
        // space against a normal built as cross(e1,e2) from the local winding, but
        // the front/back test below is a statement about WORLD space. A transform
        // with a negative determinant is a MIRROR (the side-mirror-one-wheel-mesh
        // authoring trick), and a reflection reverses the effective winding — so a
        // triangle that faces the ray in world space faces away from it in local
        // space, and the cull keeps the wrong side.
        //
        // FinalDrive's car has exactly this: two of its four wheels (wheec, wheelb)
        // are mirrored instances. Without compensation, its startup hover rays
        // returned the NEAR face of those wheels where Director returns the FAR one
        // (10.4/5.4 vs 21.2/23.3), turning Director's pitch kick into a roll kick.
        //
        // Column-major upper-3x3 (cols 0/1/2), det = c0 · (c1 × c2). This is a
        // no-op for every non-mirrored model (det > 0), so ordinary geometry is
        // byte-identical — including the #front box behaviour the cull is tuned for.
        let cull_flip = {
            let c0 = [world_transform[0], world_transform[1], world_transform[2]];
            let c1 = [world_transform[4], world_transform[5], world_transform[6]];
            let c2 = [world_transform[8], world_transform[9], world_transform[10]];
            dot(c0, cross(c1, c2)) < 0.0
        };

        // Debug: log MainA sub-mesh info and check floor face on first call
        if node.name == BuiltInSymbol::MainA {
            static MA_MESH_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if MA_MESH_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 1 {
                if let Some(meshes) = scene.clod_meshes.get(resource) {
                    for (mi, m) in meshes.iter().enumerate() {
                        let v0z = if !m.positions.is_empty() { m.positions[0][2] } else { -999.0 };
                        debug!(
                            "[RAYCAST-MESH] MainA sub[{}]: {} verts, {} faces, v0z={:.1}",
                            mi, m.positions.len(), m.faces.len(), v0z
                        );
                    }
                    // Check sub[4] face 35 specifically (should be the floor)
                    if meshes.len() > 4 && meshes[4].faces.len() > 35 {
                        let f = &meshes[4].faces[35];
                        let m = &meshes[4];
                        debug!(
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
                        );
                        // Test ray intersection manually
                        let lo = [3428.0f32, -5878.0, 219.0]; // local ray origin
                        let ld = [0.0f32, 0.0, -1.0]; // local ray dir
                        let p0 = m.positions[f[0] as usize];
                        let p1 = m.positions[f[1] as usize];
                        let p2 = m.positions[f[2] as usize];
                        if let Some((t, _, _)) = ray_triangle_intersect(
                            &Ray { origin: lo, direction: ld }, &p0, &p1, &p2
                        ) {
                            debug!("[FLOOR-FACE] ray hit! t={:.2}", t);
                        } else {
                            debug!("[FLOOR-FACE] ray MISS!");
                        }
                    }
                }
            }
        }

        // Test CLOD meshes
        if let Some(meshes) = scene.clod_meshes.get(resource) {
            // Log local ray for MainA sub[4] on first downward ray
            if node.name == BuiltInSymbol::MainA && ray.direction[2] < -0.9 && ray.direction[0].abs() < 0.1 && ray.origin[2] > 300.0 {
                static LR_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if LR_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 2 {
                    debug!(
                        "[LOCAL-RAY] MainA: world_orig=({:.1},{:.1},{:.1}) local_orig=({:.1},{:.1},{:.1}) local_dir=({:.4},{:.4},{:.4})",
                        ray.origin[0], ray.origin[1], ray.origin[2],
                        local_ray.origin[0], local_ray.origin[1], local_ray.origin[2],
                        local_ray.direction[0], local_ray.direction[1], local_ray.direction[2],
                    );
                }
            }
            for (mi, mesh) in meshes.iter().enumerate() {
                // Debug: for MainA sub[4], manually test face 35 inside the real raycast flow
                if node.name == BuiltInSymbol::MainA && mi == 4 && ray.direction[2] < -0.9 && ray.direction[0].abs() < 0.1 && ray.origin[2] > 300.0 {
                    static F35_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                    if F35_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
                        let f = &mesh.faces[35];
                        let p0 = mesh.positions[f[0] as usize];
                        let p1 = mesh.positions[f[1] as usize];
                        let p2 = mesh.positions[f[2] as usize];
                        let result = ray_triangle_intersect(&local_ray, &p0, &p1, &p2);
                        debug!(
                            "[SUB4-F35] local_orig=({:.1},{:.1},{:.1}) dir=({:.4},{:.4},{:.4}) face=[{},{},{}] hit={:?} nfaces={}",
                            local_ray.origin[0], local_ray.origin[1], local_ray.origin[2],
                            local_ray.direction[0], local_ray.direction[1], local_ray.direction[2],
                            f[0], f[1], f[2], result, mesh.faces.len()
                        );
                    }
                }
                let tc = mesh.tex_coords.first().map(|v| v.as_slice());
                // Pose the mesh before intersecting it. A skinned model's drawn
                // geometry is `skin_mat * vertex`, so testing the raw (BIND) mesh
                // hit a T-pose standing wherever the bind pose rests — for
                // Rifleman's soldiers that is sunk into the ground, and only the
                // belly, which barely moves relative to the skeleton root,
                // overlapped the animated body enough to be shootable.
                let posed = skin_pose.as_ref().and_then(|mats| {
                    if mesh.bone_indices.is_empty() { return None; }
                    Some(super::skeleton::skin_positions(
                        &mesh.positions, &mesh.bone_indices, &mesh.bone_weights, mats,
                    ))
                });
                let positions = posed.as_deref().unwrap_or(&mesh.positions);
                if let Some(mut hit) = raycast_mesh(&local_ray, positions, &mesh.normals, &mesh.faces, tc, node.name, (mi + 1) as u32, node_max, cull_flip) {
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
                    if hit.distance <= work_max {
                        all_hits.push(hit);
                    }
                }
            }
        }

        // Test raw meshes
        for (mi, mesh) in scene.raw_meshes.iter().enumerate() {
            if mesh.name == *resource {
                let tc = if !mesh.tex_coords.is_empty() { Some(mesh.tex_coords.as_slice()) } else { None };
                if let Some(mut hit) = raycast_mesh(&local_ray, &mesh.positions, &mesh.normals, &mesh.faces, tc, node.name, (mi + 1) as u32, node_max, cull_flip) {
                    hit.position = transform_point_4x4(&world_transform, hit.position[0], hit.position[1], hit.position[2]);
                    hit.normal = transform_dir_4x4(&world_transform, hit.normal[0], hit.normal[1], hit.normal[2]);
                    for v in &mut hit.vertices {
                        *v = transform_point_4x4(&world_transform, v[0], v[1], v[2]);
                    }
                    let dx = hit.position[0] - ray.origin[0];
                    let dy = hit.position[1] - ray.origin[1];
                    let dz = hit.position[2] - ray.origin[2];
                    hit.distance = (dx*dx + dy*dy + dz*dz).sqrt();
                    if hit.distance <= work_max {
                        all_hits.push(hit);
                    }
                }
            }
        }

        // Enough hits in hand: shrink the bound to the current worst so the
        // remaining models are pruned by distance instead of triangle-tested.
        if max_hits > 0 && all_hits.len() >= max_hits {
            all_hits.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));
            all_hits.truncate(max_hits);
            if let Some(worst) = all_hits.last() {
                work_max = work_max.min(worst.distance);
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
    _tex_coords: Option<&[[f32; 2]]>,
    model_name: Symbol,
    mesh_id: u32,
    max_dist: f32,
    // True when the node's world transform is a mirror (negative determinant),
    // which reverses the effective winding — see raycast_scene_multi.
    cull_flip: bool,
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
            let edge1 = sub(positions[i1], positions[i0]);
            let edge2 = sub(positions[i2], positions[i0]);
            let normal = normalize(cross(edge1, edge2));

            // Front-face culling. Director's modelsUnderRay returns ONLY front-face entries
            // (where the ray enters a surface), never back-faces/exits — confirmed against
            // Director 11.5: a ray cast from INSIDE a box returns no hit for that box, even
            // with visibility #both. dirplayer previously tested both sides, so a Rasterwerks
            // bot's collision/LOS rays hit the exit faces of its own body/proxy and of any
            // geometry it stood inside → phantom near-hits → stuck navigation (constant
            // re-path/mismatch), no line-of-sight to the player (never fires), and rockets
            // detonating on adjacent geometry (self-splash). Skip a face the ray is exiting
            // (its normal points along the ray direction).
            if (dot(normal, ray.direction) >= 0.0) != cull_flip {
                continue;
            }
            // Director reports isectNormal pointing OUT of the surface the ray
            // enters, i.e. opposing the ray. Under a mirror the local winding is
            // reversed, so the raw cross(e1,e2) normal comes out negated relative
            // to that convention — flip it back. (Verified against Director on
            // FinalDrive's mirrored wheels: same face, same distance, but our
            // normal was the exact negation of Director's.)
            let normal = if cull_flip { [-normal[0], -normal[1], -normal[2]] } else { normal };

            if t > 0.0 && t < max_dist {
                if closest.as_ref().map_or(true, |c| t < c.distance) {
                    let pos = [
                        ray.origin[0] + ray.direction[0] * t,
                        ray.origin[1] + ray.direction[1] * t,
                        ray.origin[2] + ray.direction[2] * t,
                    ];
                    // Director 11.5 Scripting Dictionary, modelsUnderLoc /
                    // modelsUnderRay: "#uvCoord is a property list with
                    // properties #u and #v that represent the u and v
                    // BARYCENTRIC coordinates of the face." Not the interpolated
                    // texture coordinate this used to hand back — a different
                    // quantity, which can be negative or outside [0,1].
                    //
                    // Scripts reconstruct the hit point from it, and that only
                    // works with barycentrics. Burnin' Rubber 3's GetAlphaPixel
                    // (the alpha gate every #alpha 3D button is clicked through)
                    // does exactly that against the face's own UV triangle:
                    //     tUVector = (tLocB - tLocA) * tUVCoord.u
                    //     tVVector = (tLocC - tLocA) * tUVCoord.V
                    //     tPos     = tLocA + tUVector + tVVector
                    //     tAlpha   = tImage.extractAlpha().getPixel(tPos)
                    // A + u(B-A) + v(C-A) IS the barycentric reconstruction of
                    // the hit point. Fed texture UVs, tPos landed off the image,
                    // getPixel raised, and the raise took the whole enterFrame
                    // with it — so no alpha-tested button ever lit up.
                    let uv = [u, v];
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
    // True when the node's world transform is a mirror (negative determinant),
    // which reverses the effective winding — see raycast_scene_multi.
    cull_flip: bool,
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
                    if (dot(normal, ray.direction) > 0.0) != cull_flip { continue; }

                    let cdist = closest.as_ref().map(|c| c.distance).unwrap_or(max_dist);
                    if t > 0.0 && t < cdist {
                        let pos = [
                            ray.origin[0] + ray.direction[0] * t,
                            ray.origin[1] + ray.direction[1] * t,
                            ray.origin[2] + ray.direction[2] * t,
                        ];
                        // Barycentric — see the note in raycast_mesh.
                        let uv = [u, v];
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
            let hit_left = raycast_bvh(ray, left, positions, faces, tex_coords, model_name, mesh_id, max_dist, cull_flip);
            let new_max = hit_left.as_ref().map(|h| h.distance).unwrap_or(max_dist);
            let hit_right = raycast_bvh(ray, right, positions, faces, tex_coords, model_name, mesh_id, new_max, cull_flip);

            match (hit_left, hit_right) {
                (Some(l), Some(r)) => if l.distance <= r.distance { Some(l) } else { Some(r) },
                (Some(h), None) | (None, Some(h)) => Some(h),
                (None, None) => None,
            }
        }
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
