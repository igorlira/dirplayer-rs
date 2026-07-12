// SPDX-License-Identifier: GPL-3.0-only
//
//! Host services available to a plugin at runtime.
//!
//! Plugins call ergonomic Rust wrappers (`log`, `random_fill`, …) that
//! funnel through a single generic extern, [`dx_host_call`]. Adding a new
//! capability means: one variant in [`HostOp`], one wrapper here, one
//! `case` in the JS-side dispatcher. **Three places, not five.**
//!
//! ## Wire shape of `dx_host_call`
//!
//! ```text
//! input  : (op_id: u32, args_ptr: u32, args_len: u32)
//!     args are a postcard-encoded `Vec<Datum>` (WireFrame::Args)
//! output : packed u64 = (ret_ptr << 32) | ret_len
//!     bytes are a postcard-encoded WireFrame::Return or WireFrame::Error;
//!     ret_ptr=0 + ret_len=0 means "void return, no error" (skips a
//!     postcard round-trip for fire-and-forget ops like `log`).
//! ```
//!
//! ## Buffer ownership
//!
//! - Plugin → host inputs: borrowed for the duration of the call (the
//!   host MUST NOT retain the pointer past return).
//! - Host → plugin outputs: the host allocated them by calling the
//!   plugin's `__plugin_alloc` and writing into the returned pointer.
//!   The plugin reclaims the buffer with `Vec::from_raw_parts`, which
//!   dealloc's on drop (allocator is shared via `#[global_allocator]`).

use alloc::{string::String, vec, vec::Vec};

use crate::datum::{Datum, InstanceId};
use crate::wire;

// ── The single host-imported extern + the op table ───────────────────────

#[link(wasm_import_module = "dirplayer_xtra_host")]
unsafe extern "C" {
    /// Single entry point through which every host capability is reached.
    /// See [`HostOp`] for the discriminator values and per-op contracts.
    fn dx_host_call(op_id: u32, args_ptr: *const u8, args_len: u32) -> u64;
}

/// Discriminator for [`dx_host_call`]. Plugins never reference these
/// directly — they're an implementation detail of the wrappers below.
/// The JS-side dispatcher and these values are part of the SDK's
/// versioned ABI: append-only, never renumber existing entries.
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum HostOp {
    /// Args: `[Datum::String(msg)]`. Returns void.
    Log = 1,

    /// Args: `[Datum::Int(len)]`. Returns `Datum::ByteArray(bytes)`
    /// containing `len` cryptographically-secure random bytes.
    RandomFill = 2,

    /// Args: `[Datum::String(key)]`. Returns `Datum::String(value)` or
    /// `Datum::Void` if absent.
    StorageGet = 3,

    /// Args: `[Datum::String(key), Datum::String(val)]`. Returns void
    /// on success or `WireFrame::Error` on failure.
    StorageSet = 4,

    /// Args: `[Datum::String(xtra_name), Datum::List(ctor_args)]`.
    /// Returns `Datum::Int(instance_id)`.
    CreateXtraInstance = 5,

    /// Args: `[Datum::String(xtra_name), Datum::Int(instance_id),
    /// Datum::String(handler), Datum::List(call_args)]`. Returns the
    /// handler's `Datum` (or `WireFrame::Error`).
    CallXtraHandler = 6,

    /// Args: `[Datum::String(xtra_name), Datum::Int(instance_id)]`.
    /// Returns void.
    DestroyXtraInstance = 7,

    // ── 3D scene rendering (see `crate::scene3d`) ─────────────────────────

    /// Args: `[Datum::String(tag)]`. Returns `Datum::Int(scene_id)`.
    /// Idempotent per tag — the same tag always maps to the same scene.
    Scene3dCreate = 8,

    /// Args: `[Datum::Int(scene_id), Datum::Int(mesh_id),
    /// Datum::ByteArray(postcard MeshData)]`. Returns void. Re-uploading the
    /// same `mesh_id` replaces its geometry (the deform path).
    Scene3dUploadMesh = 9,

    /// Args: `[Datum::Int(scene_id), Datum::Int(mesh_id)]`. Returns void.
    Scene3dDropMesh = 10,

    /// Args: `[Datum::Int(scene_id), Datum::String(name), Datum::Int(w),
    /// Datum::Int(h), Datum::ByteArray(rgba)]`. Returns void. For
    /// CPU-composed textures; cast-member textures resolve host-side by name.
    Scene3dUploadTexture = 11,

    /// Args: `[Datum::Int(scene_id), Datum::ByteArray(postcard FrameData)]`.
    /// Returns void. The host stores the frame and composites it in its next
    /// draw pass.
    Scene3dSubmitFrame = 12,

    /// Args: `[Datum::Int(scene_id)]`. Returns void. Frees the scene's meshes
    /// and textures.
    Scene3dDestroy = 13,

    // ── Host state a compute-only plugin reads ────────────────────────────

    /// Args: `[Datum::String(member_name), Datum::String(kind)]`. Returns the
    /// raw bytes of the named cast member as `Datum::ByteArray`, or
    /// `Datum::Void` if there's no such member of that `kind`. `kind` gates the
    /// member type (e.g. `"groove3gm"`), so a plugin can't read arbitrary
    /// member data.
    CastMemberBytes = 14,

    /// Args: `[]`. Returns `Datum::List[Int(stage_w), Int(stage_h),
    /// Int(current_frame)]`.
    StageInfo = 15,

    /// Args: `[]`. Returns `Datum::Point(mouseH, mouseV)`.
    MouseLoc = 16,

    /// Args: `[Datum::String(key_name)]`. Returns `Datum::Bool(is_down)`.
    KeyDown = 17,

    /// Args: `[Datum::String(name), Datum::<any>(value)]`. Sets a Lingo global
    /// so scripts can read it (e.g. Groove's `collideX`…`collideTexture`).
    /// Returns void.
    SetLingoGlobal = 18,
}

// ── Low-level glue: call dx_host_call and decode ────────────────────────

/// Pack the args and invoke `dx_host_call`. Returns the raw `Vec<u8>` of
/// the host's postcard-encoded return frame, or `Vec::new()` if the host
/// returned the void sentinel `(0, 0)`. The caller is responsible for
/// decoding via `wire::decode_*`.
fn invoke(op: HostOp, args: &[Datum]) -> Vec<u8> {
    let payload = wire::encode_args(args);
    let packed =
        unsafe { dx_host_call(op as u32, payload.as_ptr(), payload.len() as u32) };
    let ptr = (packed >> 32) as u32;
    let len = packed as u32;
    if ptr == 0 && len == 0 {
        return Vec::new();
    }
    // Safety: the host wrote into a buffer it allocated via the plugin's
    // `__plugin_alloc(len)`, which uses the global allocator. Reclaiming
    // here ensures we don't leak.
    unsafe { Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize) }
}

/// Variant of [`invoke`] that decodes the return as `Datum` (or an
/// error frame). Used by all wrappers whose op_id returns a value.
fn invoke_for_datum(op: HostOp, args: &[Datum]) -> Result<Datum, String> {
    let bytes = invoke(op, args);
    if bytes.is_empty() {
        return Ok(Datum::Void);
    }
    wire::decode_return(&bytes)
}

// ── Ergonomic wrappers ───────────────────────────────────────────────────

/// Write a debug-level log line. Visible in the host's developer console.
#[inline]
pub fn log(msg: &str) {
    invoke(HostOp::Log, &[Datum::String(String::from(msg))]);
}

/// Fill a buffer with cryptographically-secure random bytes.
pub fn random_fill(len: usize) -> Result<Vec<u8>, String> {
    match invoke_for_datum(HostOp::RandomFill, &[Datum::Int(len as i32)])? {
        Datum::ByteArray(b) => Ok(b),
        other => Err(alloc::format!(
            "random_fill: expected ByteArray payload, got {:?}",
            other
        )),
    }
}

/// Read a value from persistent storage (the host's `localStorage`).
pub fn storage_get(key: &str) -> Option<String> {
    match invoke_for_datum(HostOp::StorageGet, &[Datum::String(String::from(key))]) {
        Ok(Datum::String(s)) => Some(s),
        Ok(Datum::Void) | Err(_) => None,
        Ok(other) => {
            log(&alloc::format!(
                "storage_get: unexpected return variant {:?}",
                other
            ));
            None
        }
    }
}

/// Write a value to persistent storage.
pub fn storage_set(key: &str, val: &str) -> Result<(), String> {
    match invoke_for_datum(
        HostOp::StorageSet,
        &[
            Datum::String(String::from(key)),
            Datum::String(String::from(val)),
        ],
    )? {
        Datum::Void => Ok(()),
        other => Err(alloc::format!(
            "storage_set: expected Void on success, got {:?}",
            other
        )),
    }
}

/// Dynamic xtra-to-xtra: create an instance of another loaded xtra.
pub fn create_xtra_instance(xtra_name: &str, args: &[Datum]) -> Result<InstanceId, String> {
    let result = invoke_for_datum(
        HostOp::CreateXtraInstance,
        &[
            Datum::String(String::from(xtra_name)),
            Datum::List(args.to_vec()),
        ],
    )?;
    match result {
        Datum::Int(id) => Ok(id as InstanceId),
        other => Err(alloc::format!(
            "create_xtra_instance: expected Int, got {:?}",
            other
        )),
    }
}

/// Dynamic xtra-to-xtra: invoke a handler on a foreign xtra instance.
pub fn call_xtra_handler(
    xtra_name: &str,
    instance_id: InstanceId,
    handler: &str,
    args: &[Datum],
) -> Result<Datum, String> {
    invoke_for_datum(
        HostOp::CallXtraHandler,
        &[
            Datum::String(String::from(xtra_name)),
            Datum::Int(instance_id as i32),
            Datum::String(String::from(handler)),
            Datum::List(args.to_vec()),
        ],
    )
}

/// Dynamic xtra-to-xtra: destroy a foreign xtra instance.
pub fn destroy_xtra_instance(xtra_name: &str, instance_id: InstanceId) {
    let _ = invoke_for_datum(
        HostOp::DestroyXtraInstance,
        &[
            Datum::String(String::from(xtra_name)),
            Datum::Int(instance_id as i32),
        ],
    );
}

// ── 3D scene rendering wrappers ──────────────────────────────────────────

use crate::scene3d::{FrameData, MeshData};

/// Create (or look up) a host-side 3D scene keyed by `tag`. Idempotent — the
/// same tag returns the same scene id for the plugin's whole lifetime. The
/// returned id addresses the scene in every other `scene3d_*` call.
pub fn scene3d_create(tag: &str) -> Result<i32, String> {
    match invoke_for_datum(HostOp::Scene3dCreate, &[Datum::String(String::from(tag))])? {
        Datum::Int(id) => Ok(id),
        other => Err(alloc::format!("scene3d_create: expected Int, got {:?}", other)),
    }
}

/// Upload (or replace) the geometry of mesh `mesh_id` in `scene_id`. Cheap to
/// call once per shape; re-call with the same id to replace after a deform.
pub fn scene3d_upload_mesh(scene_id: i32, mesh_id: u32, mesh: &MeshData) {
    invoke(
        HostOp::Scene3dUploadMesh,
        &[
            Datum::Int(scene_id),
            Datum::Int(mesh_id as i32),
            Datum::ByteArray(mesh.to_bytes()),
        ],
    );
}

/// Drop mesh `mesh_id` from `scene_id` (releases its host GL buffers lazily).
pub fn scene3d_drop_mesh(scene_id: i32, mesh_id: u32) {
    invoke(
        HostOp::Scene3dDropMesh,
        &[Datum::Int(scene_id), Datum::Int(mesh_id as i32)],
    );
}

/// Upload a CPU-composed RGBA texture under `name`. `rgba` must be
/// `w * h * 4` bytes. Only needed for textures the plugin composes itself;
/// textures that live as movie bitmap cast members resolve host-side by name.
pub fn scene3d_upload_texture(scene_id: i32, name: &str, w: i32, h: i32, rgba: Vec<u8>) {
    invoke(
        HostOp::Scene3dUploadTexture,
        &[
            Datum::Int(scene_id),
            Datum::String(String::from(name)),
            Datum::Int(w),
            Datum::Int(h),
            Datum::ByteArray(rgba),
        ],
    );
}

/// Submit the frame to composite for `scene_id`. Call once per engine step;
/// the host stores it and draws it on its next paint.
pub fn scene3d_submit_frame(scene_id: i32, frame: &FrameData) {
    invoke(
        HostOp::Scene3dSubmitFrame,
        &[Datum::Int(scene_id), Datum::ByteArray(frame.to_bytes())],
    );
}

/// Destroy `scene_id`, freeing its meshes and textures.
pub fn scene3d_destroy(scene_id: i32) {
    invoke(HostOp::Scene3dDestroy, &[Datum::Int(scene_id)]);
}

// ── Host-state wrappers ──────────────────────────────────────────────────

/// Fetch the raw bytes of the cast member named `name`, gated to `kind` (e.g.
/// `"groove3gm"`). Returns `None` if there's no such member of that kind.
pub fn cast_member_bytes(name: &str, kind: &str) -> Option<Vec<u8>> {
    match invoke_for_datum(
        HostOp::CastMemberBytes,
        &[Datum::String(String::from(name)), Datum::String(String::from(kind))],
    ) {
        Ok(Datum::ByteArray(b)) => Some(b),
        _ => None,
    }
}

/// Stage size and current movie frame: `(width, height, current_frame)`.
pub fn stage_info() -> (i32, i32, i32) {
    match invoke_for_datum(HostOp::StageInfo, &[]) {
        Ok(Datum::List(items)) => {
            let g = |i: usize| items.get(i).and_then(|d| d.as_int()).unwrap_or(0);
            (g(0), g(1), g(2))
        }
        _ => (0, 0, 0),
    }
}

/// Current mouse position in stage pixels: `(mouseH, mouseV)`.
pub fn mouse_loc() -> (i32, i32) {
    match invoke_for_datum(HostOp::MouseLoc, &[]) {
        Ok(Datum::Point(x, y)) => (x as i32, y as i32),
        _ => (0, 0),
    }
}

/// Whether the named key is currently held (Director key names / chars).
pub fn key_down(key: &str) -> bool {
    invoke_for_datum(HostOp::KeyDown, &[Datum::String(String::from(key))])
        .ok()
        .and_then(|d| d.as_bool())
        .unwrap_or(false)
}

/// Set a Lingo global `name` to `value` so movie scripts can read it.
pub fn set_lingo_global(name: &str, value: Datum) {
    invoke(
        HostOp::SetLingoGlobal,
        &[Datum::String(String::from(name)), value],
    );
}

// `vec` is used by tests only at present; silence unused-import warning.
#[allow(unused_imports)]
use vec as _vec;
