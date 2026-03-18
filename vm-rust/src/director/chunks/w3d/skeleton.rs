//! Skeleton evaluator: builds bone world matrices from skeleton + motion at a given time.
//! Ported from SkeletonEvaluator.cs.

use super::types::*;

/// Build world matrices for all bones at a given time.
/// Returns column-major matrices (ready for GPU upload).
pub fn build_bone_matrices(
    skeleton: &W3dSkeleton,
    motion: Option<&W3dMotion>,
    time: f32,
) -> Vec<[f32; 16]> {
    let count = skeleton.bones.len();
    let mut local_matrices = Vec::with_capacity(count);
    let mut world_matrices = vec![[0.0f32; 16]; count];

    // Build local matrices from motion tracks or rest pose
    for bone in &skeleton.bones {
        if let Some(mot) = motion {
            if let Some(track) = mot.find_track_by_bone(&bone.name) {
                let kf = track.evaluate(time);
                local_matrices.push(compose_matrix(
                    kf.pos_x, kf.pos_y, kf.pos_z,
                    kf.rot_x, kf.rot_y, kf.rot_z, kf.rot_w,
                    kf.scale_x, kf.scale_y, kf.scale_z,
                ));
                continue;
            }
        }

        // Fall back to rest pose
        local_matrices.push(compose_matrix(
            bone.dir_x, bone.dir_y, bone.dir_z,
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
    rest_matrices.iter().map(|m| invert_matrix(m)).collect()
}

/// Compose a 4x4 column-major matrix from position, quaternion rotation, and scale.
pub fn compose_matrix(
    px: f32, py: f32, pz: f32,
    qx: f32, qy: f32, qz: f32, qw: f32,
    sx: f32, sy: f32, sz: f32,
) -> [f32; 16] {
    // Normalize quaternion
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

/// Invert a 4x4 affine transform matrix (column-major).
fn invert_matrix(m: &[f32; 16]) -> [f32; 16] {
    // For affine (rotation + translation + uniform scale):
    // R^-1 = R^T / scale^2, t^-1 = -R^-1 * t
    let r00 = m[0]; let r01 = m[4]; let r02 = m[8];
    let r10 = m[1]; let r11 = m[5]; let r12 = m[9];
    let r20 = m[2]; let r21 = m[6]; let r22 = m[10];
    let tx = m[12]; let ty = m[13]; let tz = m[14];

    let itx = -(r00 * tx + r01 * ty + r02 * tz);
    let ity = -(r10 * tx + r11 * ty + r12 * tz);
    let itz = -(r20 * tx + r21 * ty + r22 * tz);

    [
        r00, r10, r20, 0.0,
        r01, r11, r21, 0.0,
        r02, r12, r22, 0.0,
        itx, ity, itz, 1.0,
    ]
}
