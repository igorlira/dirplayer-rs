//! Skeleton evaluator: builds bone world matrices from skeleton + motion at a given time.
//! Ported from SkeletonEvaluator.cs.

use crate::player::symbols::symbol::Symbol;

use super::types::*;
use std::collections::HashMap;

const TRANSLATION_EPSILON: f32 = 1e-5;

/// The motion Director pre-loads into a skinned model's `bonesPlayer.playList`.
///
/// Director attaches a #bonesPlayer to a skinned model with the rig's own motion
/// ALREADY in the playList, before any script calls `play()`. Games rely on it:
/// Agent Free Ride's `Character2.new` reads
/// `getProp(lChild.bonesPlayer.playList[1], #name)` at construction to learn the
/// clip's name, and if the list starts empty it stores VOID and every later
/// `play(VOID, ...)` is a no-op — the rig then renders in its bind pose forever.
///
/// Pick the rig's own like-named motion, else the first real motion (more than
/// one track, and not the built-in default).
pub fn default_motion_for_model<'a>(scene: &'a W3dScene, model_name: Symbol) -> Option<&'a W3dMotion> {
    let node = scene.nodes.iter().find(|n| n.name == model_name)?;
    let skeleton = scene.skeletons.iter().find(|s| {
        s.bones.len() > 1
            && (s.name == node.resource_name
                || s.name == node.model_resource_name
                || s.name == node.name)
    })?;
    scene.motions.iter()
        .find(|m| m.name == skeleton.name)
        .or_else(|| scene.motions.iter().find(|m| {
            m.tracks.len() > 1
                && !m.name.eq_ignore_ascii_case("DefaultMotion")
                && motion_drives_skeleton(skeleton, m)
        }))
}

/// The motion Director pre-loads into an OBJECT-animated model's
/// `keyframePlayer.playList` — the counterpart of `default_motion_for_model` for
/// models that carry their own keyframe track instead of a skeleton.
///
/// The exporter names an object keyframe clip after the node it drives, with a
/// "-Key" suffix (3ds Max / the W3D exporter's convention: node "tree_c4" ->
/// motion "tree_c4-Key"), and the clip's single track carries the node's name.
/// Match on either, since only the track name is guaranteed.
///
/// Agent Free Ride's `KeyFramed Hierarched Object.InitKeyframeObj` reads
/// `getProp(kObj.keyframePlayer.playList[1], #name)` at construction and plays
/// THAT name for every later animation; falling through to the scene-global
/// legacy motion handed the falling-tree trap the rider's "player" clip.
pub fn keyframe_motion_for_model<'a>(scene: &'a W3dScene, model_name: Symbol) -> Option<&'a W3dMotion> {
    if skeleton_for_model(scene, model_name).is_some() {
        return None;
    }
    let keyed = format!("{}-Key", model_name.as_str());
    scene.motions.iter()
        .find(|m| m.name.as_str().eq_ignore_ascii_case(&keyed))
        .or_else(|| scene.motions.iter().find(|m| {
            m.tracks.len() == 1 && m.tracks[0].bone_name == model_name
        }))
}

/// The rig `model_name` owns, if any — the test for whether Director attaches a
/// #bonesPlayer (skinned) or a #keyframePlayer (object keyframes) to the model.
pub fn skeleton_for_model<'a>(scene: &'a W3dScene, model_name: Symbol) -> Option<&'a W3dSkeleton> {
    let node = scene.nodes.iter().find(|n| n.name == model_name)?;
    scene.skeletons.iter().find(|s| {
        s.bones.len() > 1
            && (s.name == node.resource_name
                || s.name == node.model_resource_name
                || s.name == node.name)
    })
}

/// True when `motion` was authored for `skeleton` — at least one of its tracks
/// names a bone of this rig.
///
/// A member's motion table is scene-global, and a game that clones several rigs
/// plus all their clips into one member has many motions that drive OTHER
/// skeletons. Sampling one of those leaves every bone on its rest TRS (no track
/// name matches), which reads as a plausible pose and is silently wrong.
pub fn motion_drives_skeleton(skeleton: &W3dSkeleton, motion: &W3dMotion) -> bool {
    motion.tracks.iter().any(|t| skeleton.bones.iter().any(|b| b.name == t.bone_name))
}

/// The rig's authored idle, whose frame-0 root is the reference the renderer
/// relativizes a skinned draw by (`[root-relativize]` in `scene3d.rs`).
///
/// Restricted to motions that drive this skeleton's ROOT bone: that is what the
/// caller samples, and it is the authoritative ownership test. AreaZero's level
/// member holds RobotGun, RobotMelee, RobotFrog and RobotTank with every clip
/// cloned in beside them, so an unfiltered "first motion whose name contains
/// idle" hands three of the four rigs a clip that cannot move them — it samples
/// to the rest TRS and reads as a plausible pose.
///
/// Deliberately narrower than `import_root_com_for_skeleton`: falling back to the
/// rig's like-named motion here would start relativizing draws that were never
/// relativized before, and a model whose node does NOT carry the fold (a clone
/// the game positions itself) would then be rotated by the COM. Agent Free Ride
/// 2's rider is exactly that — it sat 90 degrees across its jetski.
pub fn idle_reference_motion<'a>(
    scene: &'a W3dScene,
    skeleton: &W3dSkeleton,
) -> Option<&'a W3dMotion> {
    let root = skeleton.bones.first()?.name;
    let own = || scene.motions.iter()
        .filter(move |m| m.tracks.iter().any(|t| t.bone_name == root));
    own().find(|m| m.name.as_lower_str().contains("idle_rest"))
        .or_else(|| own().find(|m| m.name.as_lower_str().contains("idle")))
}

/// Frame 0 of the motion Director samples to fold a skinned model's biped COM
/// into its model node at import (`apply_root_com_to_model_nodes`).
///
/// `None` means "no clip for this rig in this member" — and the caller now
/// SKIPS the fold entirely in that case, because Director does. Measured with
/// `put` in real Director 11.5: a rig whose reference motion is in its own member
/// is folded (AFR `member(5).model("player")` = (0,0,-90); Rifleman's "enemy"
/// source node = (0,0,-90)), and one whose clips live in a member of their own is
/// NOT (TRECH `member("mech").model("mech")`, AreaZero `member("RobotGun")...`
/// and Backlot `member("onlyguy").model("charachterBiped")` all = (0,0,0)).
/// See `docs/w3d-clone-com-refold-handoff.md` §2c.
///
/// The superseded reasoning, kept because the renderer's STRIP still depends on
/// it — frame 0 of no motion is the skeleton's REST pose, and the claim was that
/// Director folds that too: AreaZero keeps each robot in its own cast member with zero
/// MOTION_BLOCKs and every clip in a member of its own, and without the rest-pose
/// fold `member("RobotGun").model("RobotGun").getWorldTransform()` — the transform
/// the game copies onto every robot it spawns — comes back without the COM.
pub fn import_root_com_motion<'a>(
    scene: &'a W3dScene,
    skeleton: &W3dSkeleton,
) -> Option<&'a W3dMotion> {
    let root = match skeleton.bones.first() { Some(b) => b.name, None => return None };
    idle_reference_motion(scene, skeleton).or_else(|| scene.motions.iter()
        .find(|m| m.name == skeleton.name && m.tracks.iter().any(|t| t.bone_name == root)))
}

pub fn has_meaningful_translation(x: f32, y: f32, z: f32) -> bool {
    x.abs() > TRANSLATION_EPSILON || y.abs() > TRANSLATION_EPSILON || z.abs() > TRANSLATION_EPSILON
}

/// Resolve a bone's local translation: if the candidate is near-zero,
/// fall back to skeleton displacement, then to parent bone length along X.
/// This chains bones end-to-end when displacement is zero.
pub fn resolve_local_translation(skeleton: &W3dSkeleton, bone_idx: usize, cx: f32, cy: f32, cz: f32) -> (f32, f32, f32) {
    if has_meaningful_translation(cx, cy, cz) {
        return (cx, cy, cz);
    }
    let bone = &skeleton.bones[bone_idx];
    if has_meaningful_translation(bone.dir_x, bone.dir_y, bone.dir_z) {
        return (bone.dir_x, bone.dir_y, bone.dir_z);
    }
    if bone.parent_index >= 0 {
        let parent = &skeleton.bones[bone.parent_index as usize];
        if parent.length.abs() > TRANSLATION_EPSILON {
            return (parent.length, 0.0, 0.0);
        }
    }
    (cx, cy, cz)
}

pub fn get_bind_pose(skeleton: &W3dSkeleton, bone_idx: usize) -> W3dKeyframe {
    let bone = &skeleton.bones[bone_idx];
    let (px, py, pz) = resolve_local_translation(skeleton, bone_idx, bone.dir_x, bone.dir_y, bone.dir_z);
    W3dKeyframe {
        time: 0.0,
        pos_x: px,
        pos_y: py,
        pos_z: pz,
        rot_x: bone.rot_x,
        rot_y: bone.rot_y,
        rot_z: bone.rot_z,
        rot_w: bone.rot_w,
        scale_x: 1.0,
        scale_y: 1.0,
        scale_z: 1.0,
    }
}

/// Build world matrices for all bones at a given time.
/// Returns column-major matrices (ready for GPU upload).
/// If root_lock is true, root bone translation is zeroed (character stays in place).
pub fn build_bone_matrices(
    skeleton: &W3dSkeleton,
    motion: Option<&W3dMotion>,
    time: f32,
) -> Vec<[f32; 16]> {
    build_bone_matrices_ex(skeleton, motion, time, false, None)
}

/// Build bone matrices with optional root lock and per-bone manual overrides.
/// `overrides` maps a 0-based bone index to a LOCAL transform set via
/// `bonesPlayer.bone[i].transform` — it replaces the motion/rest local for that
/// bone (its rotation/scale is used and its translation is resolved to the rest
/// length, so a script that sets only a rotation keeps the bone's length).
pub fn build_bone_matrices_ex(
    skeleton: &W3dSkeleton,
    motion: Option<&W3dMotion>,
    time: f32,
    root_lock: bool,
    overrides: Option<&std::collections::HashMap<usize, [f32; 16]>>,
) -> Vec<[f32; 16]> {
    let count = skeleton.bones.len();
    let mut local_matrices = Vec::with_capacity(count);
    let mut world_matrices = vec![[0.0f32; 16]; count];

    // Track which bones have motion data (for world-space vs local-space handling)
    let mut has_motion_track = vec![false; count];

    // Build local matrices from motion tracks or rest pose
    for (bone_idx, bone) in skeleton.bones.iter().enumerate() {
        // Manual per-bone override (bonesPlayer.bone[i].transform = t) takes
        // precedence over the motion. Use its rotation/scale but resolve the
        // translation (zero for a preRotate-only script) to the rest length so
        // the body keeps its shape while the override rotation drives it.
        if let Some(ov) = overrides.and_then(|o| o.get(&bone_idx)) {
            let (px, py, pz) = if root_lock && bone.parent_index < 0 {
                (0.0, 0.0, 0.0)
            } else {
                (ov[12], ov[13], ov[14])
            };
            let mut local = *ov;
            local[12] = px;
            local[13] = py;
            local[14] = pz;
            local_matrices.push(local);
            has_motion_track[bone_idx] = true;
            continue;
        }
        if let Some(mot) = motion {
            if let Some(track) = mot.find_track_by_bone(bone.name) {
                let kf = track.evaluate(time);
                // RAW translation. The parent-tip offset is applied by the world walk
                // below, not folded in here — see the note on bone length there.
                let (px, py, pz) = if root_lock && bone.parent_index < 0 {
                    (0.0, 0.0, 0.0)
                } else {
                    (kf.pos_x, kf.pos_y, kf.pos_z)
                };
                local_matrices.push(compose_matrix(
                    px, py, pz,
                    kf.rot_x, kf.rot_y, kf.rot_z, kf.rot_w,
                    kf.scale_x, kf.scale_y, kf.scale_z,
                ));
                has_motion_track[bone_idx] = true;
                continue;
            }
        }

        // Fall back to the rest pose — raw `dir`, tip offset applied by the world walk.
        local_matrices.push(compose_matrix(
            bone.dir_x, bone.dir_y, bone.dir_z,
            bone.rot_x, bone.rot_y, bone.rot_z, bone.rot_w,
            1.0, 1.0, 1.0,
        ));
    }

    // Walk the parent chain to build world matrices.
    //
    // IFX parents a child at its parent's TIP, not the parent's origin:
    // `IFXCharacter::ForEachNodeTransformed2` stores the node's own transform, then
    // translates by (length, 0, 0) in the node's local frame before recursing into
    // children. So `dir` (and any keyframe position) is an offset measured FROM the
    // parent's tip:
    //
    //     world(child) = world(parent) * T(parentLength.x) * T(dir) * R(rot) * S(scale)
    //
    // Confirmed numerically on the Agent Free Ride biped: 20 of its 31 bones carry
    // dir=(0,0,0) with a large nonzero length, and for the bones that DO have a dir,
    // the sum lands where anatomy requires — L Thigh's dir.x (-24.103) added to its
    // parent Spine's length (13.739) gives -10.364, i.e. exactly one Pelvis length
    // (10.367) below the spine base.
    for i in 0..count {
        let parent = skeleton.bones[i].parent_index;
        if parent < 0 {
            world_matrices[i] = local_matrices[i];
        } else {
            let p = parent as usize;
            let plen = skeleton.bones[p].length;
            let tip = [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                plen, 0.0, 0.0, 1.0,
            ];
            let parent_tip = multiply_matrix(&world_matrices[p], &tip);
            world_matrices[i] = multiply_matrix(&parent_tip, &local_matrices[i]);
        }
    }

    world_matrices
}

/// Build inverse bind matrices (rest pose inverted).
/// These transform from world space back to bone-local space for skinning.
/// The matrix a skinned draw is relativized by, given the model's recorded biped
/// COM fold.
///
/// Shared so the renderer and the RAYCASTER pose a model identically. They used
/// to disagree completely — the raycaster had no skinning at all and intersected
/// the bind pose, so a soldier could only be shot where the T-pose happened to
/// overlap the animated body (its belly), and the hit volume sat sunk into the
/// ground where the bind pose rests.
///
/// Mirrors the renderer's `root_relinv` exactly — recorded fold first, then the
/// rig's authored idle at frame 0, then identity.
pub fn root_relativizer(
    scene: &W3dScene,
    skeleton: &W3dSkeleton,
    model_name: Symbol,
    resource_name: Symbol,
) -> [f32; 16] {
    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let key = [model_name.to_ascii_lowercase(), resource_name.to_ascii_lowercase()]
        .into_iter()
        .find(|k| scene.model_root_com.contains_key(k));
    if let Some(k) = key {
        let r0 = scene.model_root_com[&k];
        return invert_matrix(&r0);
    }
    // Fallback for models whose fold was never recorded: the rig's authored idle,
    // frame 0. Deliberately narrow — widening it relativizes draws that never were.
    match idle_reference_motion(scene, skeleton) {
        Some(im) => {
            let m = build_bone_matrices(skeleton, Some(im), 0.0);
            m.first().map(|r| invert_matrix(r)).unwrap_or(IDENTITY)
        }
        None => IDENTITY,
    }
}

/// Posed bone world matrices in the SAME space the renderer skins in — see
/// `scene3d::setup_skinning_for_resource`. Root translation is stripped when the
/// animation tick carries it on the model node instead, and the result is
/// relativized by the recorded biped COM.
///
/// Scripts pin things to bones through `bonesPlayer.bone[i].worldTransform`, so
/// that value has to agree with what is drawn. Agent Free Ride places its jetpack
/// flames with
///   `parent.getWorldTransform() * bone[spine1].worldTransform * vector(-6, ±12, 16)`
/// and the raw motion matrices still carry the root translation the renderer had
/// already moved onto the model node — counted twice, the flames ended up hundreds
/// of units above the rider instead of at the pack's nozzles.
///
/// `tick_carries_root` is the renderer's condition for handing root motion to the
/// node: this model has a per-model `bonesPlayer` with a motion playing.
pub fn posed_bone_world_matrices(
    scene: &W3dScene,
    skeleton: &W3dSkeleton,
    motion: Option<&W3dMotion>,
    time: f32,
    root_lock: bool,
    tick_carries_root: bool,
    overrides: Option<&HashMap<usize, [f32; 16]>>,
    model_name: Symbol,
) -> Vec<[f32; 16]> {
    let strips_root = !root_lock
        && tick_carries_root
        && motion.map(|m| motion_has_root_translation(skeleton, m)).unwrap_or(false);
    let world = build_bone_matrices_ex(skeleton, motion, time, root_lock || strips_root, overrides);
    let relinv = root_relativizer(scene, skeleton, model_name, skeleton.name);
    world.iter().map(|m| multiply_matrix(&relinv, m)).collect()
}

/// Final per-bone skinning matrices: `root_relinv * world[b] * inv_bind[b]`.
/// This is the transform a skinned vertex is pushed through, so applying it to
/// the mesh gives the geometry that is actually on screen.
pub fn build_skinning_matrices(
    skeleton: &W3dSkeleton,
    motion: Option<&W3dMotion>,
    time: f32,
    root_lock: bool,
    root_relinv: &[f32; 16],
) -> Vec<[f32; 16]> {
    let world = build_bone_matrices_ex(skeleton, motion, time, root_lock, None);
    let inv_bind = build_inverse_bind_matrices(skeleton);
    world.iter().zip(inv_bind.iter())
        .map(|(w, ib)| multiply_matrix(&multiply_matrix(root_relinv, w), ib))
        .collect()
}

/// Skin `positions` with `skin_mats`, weighting each vertex by its bone list.
/// Vertices with no weights are passed through unchanged (rigid geometry welded
/// into a skinned resource).
pub fn skin_positions(
    positions: &[[f32; 3]],
    bone_indices: &[Vec<u32>],
    bone_weights: &[Vec<f32>],
    skin_mats: &[[f32; 16]],
) -> Vec<[f32; 3]> {
    positions.iter().enumerate().map(|(vi, p)| {
        let (Some(idx), Some(wts)) = (bone_indices.get(vi), bone_weights.get(vi)) else {
            return *p;
        };
        let mut acc = [0.0f32; 3];
        let mut total = 0.0f32;
        for (bi, w) in idx.iter().zip(wts.iter()) {
            let Some(m) = skin_mats.get(*bi as usize) else { continue };
            if *w == 0.0 { continue; }
            // Column-major affine transform of a point.
            acc[0] += w * (m[0] * p[0] + m[4] * p[1] + m[8]  * p[2] + m[12]);
            acc[1] += w * (m[1] * p[0] + m[5] * p[1] + m[9]  * p[2] + m[13]);
            acc[2] += w * (m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14]);
            total += w;
        }
        // An unweighted or fully zero-weighted vertex must not collapse to the
        // origin — that would drag stray triangles across the whole model and
        // make the hit volume enormous.
        if total <= 1e-6 { *p } else { [acc[0] / total, acc[1] / total, acc[2] / total] }
    }).collect()
}

pub fn build_inverse_bind_matrices(skeleton: &W3dSkeleton) -> Vec<[f32; 16]> {
    let rest_matrices = build_bone_matrices(skeleton, None, 0.0);
    let inverted: Vec<_> = rest_matrices.iter().map(|m| invert_matrix(m)).collect();
    inverted
}

/// Build world matrices for scene graph nodes using parent-name chaining.
pub fn build_node_world_matrices(nodes: &[W3dNode]) -> HashMap<Symbol, [f32; 16]> {
    fn build_node_world_matrix(
        node: &W3dNode,
        node_map: &HashMap<Symbol, &W3dNode>,
        cache: &mut HashMap<Symbol, [f32; 16]>,
    ) -> [f32; 16] {
        if let Some(world) = cache.get(&node.name) {
            return *world;
        }

        let world = if !node.parent_name.is_empty() {
            if let Some(parent) = node_map.get(&node.parent_name) {
                let parent_world = build_node_world_matrix(parent, node_map, cache);
                multiply_matrix(&parent_world, &node.transform)
            } else {
                node.transform
            }
        } else {
            node.transform
        };

        cache.insert(node.name.clone(), world);
        world
    }

    let node_map: HashMap<Symbol, &W3dNode> = nodes.iter().map(|n| (n.name.clone(), n)).collect();
    let mut cache = HashMap::new();
    for node in nodes {
        build_node_world_matrix(node, &node_map, &mut cache);
    }
    cache
}

/// Convert W3D's authored Z-up basis into the Y-up basis expected by common OBJ/glTF viewers.
pub fn export_basis_transform() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, -1.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Compose a 4x4 column-major matrix from position, quaternion rotation, and scale.
pub fn compose_matrix(
    px: f32, py: f32, pz: f32,
    qx: f32, qy: f32, qz: f32, qw: f32,
    sx: f32, sy: f32, sz: f32,
) -> [f32; 16] {
    // Normalize quaternion (IFX uses column-major right-handed, matching our convention)
    let len = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt();
    let (qx, qy, qz, qw) = if len > 1e-8 {
        (qx / len, qy / len, qz / len, qw / len)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };

    // Rotation matrix from quaternion (column-major layout)
    let xx = qx * qx;
    let yy = qy * qy;
    let zz = qz * qz;
    let xy = qx * qy;
    let xz = qx * qz;
    let yz = qy * qz;
    let wx = qw * qx;
    let wy = qw * qy;
    let wz = qw * qz;

    [
        (1.0 - 2.0 * (yy + zz)) * sx,
        (2.0 * (xy + wz)) * sx,
        (2.0 * (xz - wy)) * sx,
        0.0,
        (2.0 * (xy - wz)) * sy,
        (1.0 - 2.0 * (xx + zz)) * sy,
        (2.0 * (yz + wx)) * sy,
        0.0,
        (2.0 * (xz + wy)) * sz,
        (2.0 * (yz - wx)) * sz,
        (1.0 - 2.0 * (xx + yy)) * sz,
        0.0,
        px,
        py,
        pz,
        1.0,
    ]
}

/// Multiply two 4x4 column-major matrices: result = A * B
fn multiply_matrix(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
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

/// Invert a 4x4 matrix (column-major) using full cofactor expansion.
fn invert_matrix(m: &[f32; 16]) -> [f32; 16] {
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
        // Return identity if singular
        let mut id = [0.0f32; 16];
        id[0] = 1.0; id[5] = 1.0; id[10] = 1.0; id[15] = 1.0;
        return id;
    }
    let inv_det = 1.0 / det;
    for i in 0..16 { inv[i] *= inv_det; }
    inv
}

// ---------------------------------------------------------------------------
// Root motion ("root clearance")
// ---------------------------------------------------------------------------
//
// IFX always strips the root bone's local transform out of the posed hierarchy
// and accumulates it into a persistent root transform held by the modifier.
// `rootLock` then decides only whether the extracted TRANSLATION is added to
// the model NODE's scene-graph transform: false → the model walks through the
// scene, true → it animates in place (see docs/w3d-skeleton-motion-spec.md §1,
// "Root handling").
//
// We used to keep the whole root track inside the skeleton, so a travelling
// clip moved the drawn mesh but left `model.worldPosition` frozen at the spot
// the clip started. Agent Free Ride's end-of-level paraglider is the visible
// cost: `Snowboard Camera`'s #EndScene state re-aims the camera at
// `player_fake.worldPosition` every frame, that node never moved, and the
// boarder flew ~16000 units out of a camera still staring at the launch point.
//
// The two halves below MUST stay in lockstep: whenever `strips_root_translation`
// says yes, `build_bone_matrices_ex` is called with the root translation zeroed
// AND `root_motion_translation`'s value is pushed onto the node.

/// Effective sample time for a bones clip, matching the renderer's clamp/wrap.
pub fn effective_motion_time(
    time: f32,
    start_time: f32,
    end_time: f32,
    looping: bool,
    duration: f32,
) -> f32 {
    let eff_end = if end_time >= 0.0 { end_time.min(duration) } else { duration };
    let eff_start = start_time.min(eff_end);
    let range = eff_end - eff_start;
    if range > 0.0 {
        if looping {
            eff_start + ((time - eff_start) % range + range) % range
        } else {
            time.clamp(eff_start, eff_end)
        }
    } else {
        eff_start
    }
}

/// The root bone's local translation at `time` — the transform IFX hands back
/// through `GetRootClearance`. `None` when the motion has no track for the root
/// bone, i.e. there is no root motion to route anywhere.
pub fn root_motion_translation(
    skeleton: &W3dSkeleton,
    motion: &W3dMotion,
    time: f32,
) -> Option<[f32; 3]> {
    let root = skeleton.bones.first()?;
    if root.parent_index >= 0 {
        return None;
    }
    let track = motion.find_track_by_bone(root.name)?;
    let kf = track.evaluate(time);
    Some([kf.pos_x, kf.pos_y, kf.pos_z])
}

/// Does this clip actually travel? A root track that never leaves the origin
/// (the usual in-place idle/trick) is left alone entirely, so nothing changes
/// for the rigs that do not need this.
pub fn motion_has_root_translation(skeleton: &W3dSkeleton, motion: &W3dMotion) -> bool {
    let root = match skeleton.bones.first() {
        Some(b) if b.parent_index < 0 => b,
        _ => return false,
    };
    let track = match motion.find_track_by_bone(root.name) {
        Some(t) => t,
        None => return false,
    };
    let mut keys = track.keyframes.iter();
    let first = match keys.next() {
        Some(k) => k,
        None => return false,
    };
    keys.any(|k| {
        has_meaningful_translation(
            k.pos_x - first.pos_x,
            k.pos_y - first.pos_y,
            k.pos_z - first.pos_z,
        )
    })
}

/// Where the extracted root translation lands on the model node.
///
/// The skinned draw is `node * inv(R0) * bone`, R0 being the biped COM fold the
/// parser baked into the node (`apply_root_com_to_model_nodes`). Removing a
/// root translation `p` from the bone side therefore has to come back as
/// `inv(R0) * T(p) * R0` on the node side, and for R0 = T(t0)·R0r that product
/// collapses to a pure translation by `R0r⁻¹ · p` — so the draw is unchanged and
/// only the node's reported position moves.
pub fn root_clearance_node_offset(root_relinv: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    // `root_relinv` is inv(R0); its rotation block is already R0r⁻¹.
    [
        root_relinv[0] * p[0] + root_relinv[4] * p[1] + root_relinv[8] * p[2],
        root_relinv[1] * p[0] + root_relinv[5] * p[1] + root_relinv[9] * p[2],
        root_relinv[2] * p[0] + root_relinv[6] * p[1] + root_relinv[10] * p[2],
    ]
}
