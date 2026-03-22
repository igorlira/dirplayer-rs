//! Skeleton evaluator: builds bone world matrices from skeleton + motion at a given time.
//! Ported from SkeletonEvaluator.cs.

use super::types::*;

const TRANSLATION_EPSILON: f32 = 1e-5;

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
    build_bone_matrices_ex(skeleton, motion, time, false)
}

/// Build bone matrices with optional root lock.
pub fn build_bone_matrices_ex(
    skeleton: &W3dSkeleton,
    motion: Option<&W3dMotion>,
    time: f32,
    root_lock: bool,
) -> Vec<[f32; 16]> {
    let count = skeleton.bones.len();
    let mut local_matrices = Vec::with_capacity(count);
    let mut world_matrices = vec![[0.0f32; 16]; count];

    // Track which bones have motion data (for world-space vs local-space handling)
    let mut has_motion_track = vec![false; count];

    // Build local matrices from motion tracks or rest pose
    for (bone_idx, bone) in skeleton.bones.iter().enumerate() {
        if let Some(mot) = motion {
            if let Some(track) = mot.find_track_by_bone(&bone.name) {
                let kf = track.evaluate(time);
                // Resolve translation: use parent bone length if displacement is zero
                let (px, py, pz) = if root_lock && bone.parent_index < 0 {
                    (0.0, 0.0, 0.0)
                } else {
                    resolve_local_translation(skeleton, bone_idx, kf.pos_x, kf.pos_y, kf.pos_z)
                };
                let sx = if kf.scale_x.abs() < 0.01 { 1.0 } else { kf.scale_x };
                let sy = if kf.scale_y.abs() < 0.01 { 1.0 } else { kf.scale_y };
                let sz = if kf.scale_z.abs() < 0.01 { 1.0 } else { kf.scale_z };
                local_matrices.push(compose_matrix(
                    px, py, pz,
                    kf.rot_x, kf.rot_y, kf.rot_z, kf.rot_w,
                    sx, sy, sz,
                ));
                has_motion_track[bone_idx] = true;
                continue;
            }
        }

        // Fall back to rest pose with resolved translation
        let (rx, ry, rz) = resolve_local_translation(skeleton, bone_idx, bone.dir_x, bone.dir_y, bone.dir_z);
        local_matrices.push(compose_matrix(
            rx, ry, rz,
            bone.rot_x, bone.rot_y, bone.rot_z, bone.rot_w,
            1.0, 1.0, 1.0,
        ));
    }

    // Walk parent chain to build world matrices
    for i in 0..count {
        let parent = skeleton.bones[i].parent_index;
        if parent < 0 {
            world_matrices[i] = local_matrices[i];
        } else {
            world_matrices[i] = multiply_matrix(&world_matrices[parent as usize], &local_matrices[i]);
        }
    }

    world_matrices
}

/// Build inverse bind matrices (rest pose inverted).
/// These transform from world space back to bone-local space for skinning.
pub fn build_inverse_bind_matrices(skeleton: &W3dSkeleton) -> Vec<[f32; 16]> {
    let rest_matrices = build_bone_matrices(skeleton, None, 0.0);
    let inverted: Vec<_> = rest_matrices.iter().map(|m| invert_matrix(m)).collect();
    inverted
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
