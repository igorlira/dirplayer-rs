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

// `vec` is used by tests only at present; silence unused-import warning.
#[allow(unused_imports)]
use vec as _vec;
