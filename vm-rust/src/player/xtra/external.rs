// SPDX-License-Identifier: GPL-3.0-only
//
//! Host-side adapter for external Xtras (WASM plugins loaded from URLs).
//!
//! ## Flow
//!
//! ```text
//! Lingo: new(xtra "BobbaXtra")
//!   └─ manager.rs::create_xtra_instance
//!        └─ external::create_instance("BobbaXtra", &args)
//!             └─ JS bridge:  dispatchCreateExternalXtraInstance(name, args_bytes)
//!                  └─ plugin export:  __xtra_create_instance(args_ptr, args_len)
//!                       └─ plugin code, possibly calling back via dx_host_call
//! ```
//!
//! ## Adding a new host capability (the scaling rule)
//!
//! 1. Append a variant to [`HostOp`] (never renumber existing entries).
//! 2. Add the handler arm in [`host_call_dispatch`].
//! 3. Add the matching Rust wrapper in `dirplayer-xtra`'s `host_env.rs`.
//!
//! Three places. No JS-side change required (the JS dispatcher is a pure
//! passthrough that doesn't decode postcard).

use std::collections::{HashMap, HashSet};

use crate::director::lingo::datum::{Datum, XtraInstanceId, datum_bool};
use crate::player::{DatumRef, ScriptError, reserve_player_mut};

use xtra_sdk::Datum as XDatum;
use xtra_sdk::wire;

/// Discriminator for the single `dx_host_call` extern that every plugin
/// imports. Must match `HostOp` in `dirplayer-xtra/src/host_env.rs` and
/// the JS-side passthrough in `src/services/externalXtras.ts`.
///
/// **APPEND-ONLY. Never renumber.** Plugins built against older SDK
/// versions assume these numbers are stable.
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
enum HostOp {
    Log = 1,
    RandomFill = 2,
    StorageGet = 3,
    StorageSet = 4,
    CreateXtraInstance = 5,
    CallXtraHandler = 6,
    DestroyXtraInstance = 7,
}

impl HostOp {
    fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            1 => HostOp::Log,
            2 => HostOp::RandomFill,
            3 => HostOp::StorageGet,
            4 => HostOp::StorageSet,
            5 => HostOp::CreateXtraInstance,
            6 => HostOp::CallXtraHandler,
            7 => HostOp::DestroyXtraInstance,
            _ => return None,
        })
    }
}

// ── Registry ─────────────────────────────────────────────────────────────

/// Set of xtra names (lowercased) currently registered as externally
/// loaded. The JS-side loader calls [`register`] right after a successful
/// `WebAssembly.instantiate`, and the dispatch arms in `manager.rs`
/// consult this set before falling through to built-ins.
static mut REGISTRY: Option<HashSet<String>> = None;

fn registry() -> &'static mut HashSet<String> {
    unsafe {
        let ptr = &raw mut REGISTRY;
        (*ptr).get_or_insert_with(HashSet::new)
    }
}

/// Register an externally-loaded xtra under its name (case-insensitive
/// match against later Lingo calls). Idempotent. Called by JS after
/// each plugin's `__xtra_name()` returns successfully.
pub fn register(name: &str) {
    registry().insert(name.to_lowercase());
}

/// Returns `true` if `name` matches an externally-loaded xtra. Called
/// from `xtra/manager.rs::is_xtra_registered` so external xtras win over
/// built-ins.
pub fn is_registered(name: &str) -> bool {
    registry().contains(&name.to_lowercase())
}

/// Returns all registered external xtra names for `getXtraList()` output.
/// Names are case-lowered; the caller is expected to re-case if needed.
pub fn registered_names() -> Vec<String> {
    registry().iter().cloned().collect()
}

// ── JS bridge (declared in `dirplayer-js-api`) ───────────────────────────

#[cfg(target_arch = "wasm32")]
mod js_bridge {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(module = "dirplayer-js-api")]
    extern "C" {
        /// Calls the plugin's `__xtra_call_static_handler`. Returns the
        /// raw postcard `WireFrame::Return` (or `Error`) bytes, or `None`
        /// if the xtra isn't loaded.
        pub fn dispatchExternalXtraStaticHandler(
            xtra_name: &str,
            handler: &str,
            args: &[u8],
        ) -> Option<Vec<u8>>;

        /// Calls the plugin's `__xtra_call_handler`. Returns the postcard
        /// frame bytes or `None` if the xtra isn't loaded.
        pub fn dispatchExternalXtraInstanceHandler(
            xtra_name: &str,
            instance_id: u32,
            handler: &str,
            args: &[u8],
        ) -> Option<Vec<u8>>;

        /// Calls the plugin's `__xtra_create_instance`. Returns the
        /// postcard frame bytes (whose decoded `Datum::Int` carries the
        /// instance id), or `None` if the xtra isn't loaded.
        pub fn createExternalXtraInstance(xtra_name: &str, args: &[u8]) -> Option<Vec<u8>>;

        /// Calls the plugin's `__xtra_destroy_instance`. No return value.
        pub fn destroyExternalXtraInstance(xtra_name: &str, instance_id: u32);

        /// Returns `1` if the plugin reports the handler as a static
        /// handler. `0` otherwise. Mirrors `__xtra_has_static_handler`.
        pub fn externalXtraHasStaticHandler(xtra_name: &str, handler: &str) -> u32;

        /// Fetch a plugin .wasm from `url`, instantiate it, and register
        /// the xtra. Resolves with the registered xtra name. Used by the
        /// test harness's `player.load_external_xtra(...)` helper —
        /// production code paths normally go through the JS-side
        /// `loadExternalXtras` config loader (localStorage in dev,
        /// init-script in polyfill, etc.).
        #[wasm_bindgen(catch)]
        pub async fn loadExternalXtra(url: &str) -> Result<JsValue, JsValue>;
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod js_bridge {
    // Native-target stubs for `cargo check` / unit tests. The wasm32
    // build is the only target that actually loads plugins.
    pub fn dispatchExternalXtraStaticHandler(
        _: &str, _: &str, _: &[u8],
    ) -> Option<Vec<u8>> { None }
    pub fn dispatchExternalXtraInstanceHandler(
        _: &str, _: u32, _: &str, _: &[u8],
    ) -> Option<Vec<u8>> { None }
    pub fn createExternalXtraInstance(_: &str, _: &[u8]) -> Option<Vec<u8>> { None }
    pub fn destroyExternalXtraInstance(_: &str, _: u32) {}
    pub fn externalXtraHasStaticHandler(_: &str, _: &str) -> u32 { 0 }
}

// ── Lingo-side dispatch (called from manager.rs) ─────────────────────────

/// Returns `true` if the plugin exposes a static handler by this name.
/// Used by manager.rs to decide between static and instance dispatch.
pub fn has_static_handler(xtra_name: &str, handler_name: &str) -> bool {
    js_bridge::externalXtraHasStaticHandler(xtra_name, &handler_name.to_lowercase()) != 0
}

/// Dispatch a static (no-instance) handler on an external xtra. Returns
/// `Some(result)` if the xtra is registered (even if the handler errored
/// — the error is wrapped in the Result); returns `None` if the xtra
/// isn't external-loaded so the caller can fall through to built-ins.
pub fn call_static_handler(
    xtra_name: &str,
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Option<Result<DatumRef, ScriptError>> {
    if !is_registered(xtra_name) {
        return None;
    }
    let payload = encode_args_from_refs(args)?;
    let result_bytes = match js_bridge::dispatchExternalXtraStaticHandler(
        xtra_name,
        &handler_name.to_lowercase(),
        &payload,
    ) {
        Some(b) => b,
        None => {
            return Some(Err(ScriptError::new(format!(
                "External xtra '{}' is registered but dispatch returned None",
                xtra_name
            ))));
        }
    };
    Some(decode_return_to_datum_ref(&result_bytes, xtra_name, handler_name))
}

/// Create an instance on an external xtra. Returns `Some(id)` or
/// `Some(Err(...))`; `None` means the xtra isn't external.
pub fn create_instance(
    xtra_name: &str,
    args: &Vec<DatumRef>,
) -> Option<Result<XtraInstanceId, ScriptError>> {
    if !is_registered(xtra_name) {
        return None;
    }
    let payload = encode_args_from_refs(args)?;
    let result_bytes = match js_bridge::createExternalXtraInstance(xtra_name, &payload) {
        Some(b) => b,
        None => {
            return Some(Err(ScriptError::new(format!(
                "External xtra '{}': createInstance dispatch returned None",
                xtra_name
            ))));
        }
    };
    Some(match wire::decode_return(&result_bytes) {
        Ok(XDatum::Int(id)) => Ok(id as XtraInstanceId),
        Ok(other) => Err(ScriptError::new(format!(
            "External xtra '{}': createInstance returned non-Int datum {:?}",
            xtra_name, other
        ))),
        Err(e) => Err(ScriptError::new(format!(
            "External xtra '{}': createInstance error: {}",
            xtra_name, e
        ))),
    })
}

/// Dispatch an instance handler on an external xtra.
pub fn call_instance_handler(
    xtra_name: &str,
    instance_id: XtraInstanceId,
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Option<Result<DatumRef, ScriptError>> {
    if !is_registered(xtra_name) {
        return None;
    }
    let payload = encode_args_from_refs(args)?;
    let result_bytes = match js_bridge::dispatchExternalXtraInstanceHandler(
        xtra_name,
        instance_id as u32,
        &handler_name.to_lowercase(),
        &payload,
    ) {
        Some(b) => b,
        None => {
            return Some(Err(ScriptError::new(format!(
                "External xtra '{}': instance dispatch returned None",
                xtra_name
            ))));
        }
    };
    Some(decode_return_to_datum_ref(&result_bytes, xtra_name, handler_name))
}

/// Destroy an external xtra instance. Idempotent.
pub fn destroy_instance(xtra_name: &str, instance_id: XtraInstanceId) {
    if !is_registered(xtra_name) {
        return;
    }
    js_bridge::destroyExternalXtraInstance(xtra_name, instance_id as u32);
}

// ── Test-harness plugin loader ──────────────────────────────────────────

/// Fetch a plugin .wasm from `url`, instantiate it, and register the
/// xtra. Returns the registered xtra name on success. Used by e2e tests
/// (`BrowserTestPlayer::load_external_xtra`). Production code paths
/// normally load plugins via the JS-side `loadExternalXtras` configured
/// per-host (localStorage in dev, init-script in polyfill, etc.).
#[cfg(target_arch = "wasm32")]
pub async fn load_for_test(url: &str) -> Result<String, String> {
    match js_bridge::loadExternalXtra(url).await {
        Ok(name_val) => name_val
            .as_string()
            .ok_or_else(|| format!("loadExternalXtra({}): returned non-string name", url)),
        Err(e) => Err(format!(
            "loadExternalXtra({}): {}",
            url,
            e.as_string().unwrap_or_else(|| String::from("(opaque JsValue error)"))
        )),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn load_for_test(_url: &str) -> Result<String, String> {
    Err(String::from("load_for_test is only available on wasm32"))
}

// ── dx_host_call dispatcher (called by JS for every plugin host call) ───

/// Single dispatch entry point for every `dx_host_call` from any plugin.
/// JS reads the args from plugin memory, passes them here, and writes the
/// resulting bytes back into plugin memory.
///
/// Returns the postcard-encoded result frame (`WireFrame::Return` or
/// `WireFrame::Error`) — or an empty `Vec` for the "void" sentinel which
/// lets fire-and-forget ops (like `log`) skip a postcard round-trip.
pub fn host_call_dispatch(op_id: u32, args_bytes: &[u8]) -> Vec<u8> {
    let op = match HostOp::from_u32(op_id) {
        Some(o) => o,
        None => {
            return wire::encode_error(&format!(
                "unknown host op_id {}",
                op_id
            ));
        }
    };
    let args = match wire::decode_args(args_bytes) {
        Ok(a) => a,
        Err(e) => return wire::encode_error(&format!("bad args: {}", e)),
    };
    match op {
        HostOp::Log => {
            if let Some(XDatum::String(msg)) = args.first() {
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(&format!("[xtra] {}", msg).into());
                #[cfg(not(target_arch = "wasm32"))]
                eprintln!("[xtra] {}", msg);
            }
            Vec::new() // void sentinel
        }
        HostOp::RandomFill => {
            let len = match args.first() {
                Some(XDatum::Int(n)) => *n as usize,
                _ => return wire::encode_error("random_fill: expected Int len"),
            };
            // Cap at a reasonable size to avoid DoS via a malicious plugin
            // asking for gigabytes. BobbaXtra's largest call is 32 bytes
            // (machine-id seed); DH ephemeral keys are similar.
            if len > 1 << 20 {
                return wire::encode_error("random_fill: requested length too large");
            }
            let mut buf = vec![0u8; len];
            #[cfg(target_arch = "wasm32")]
            {
                match web_sys::window().and_then(|w| w.crypto().ok()) {
                    Some(crypto) => {
                        if crypto.get_random_values_with_u8_array(&mut buf).is_err() {
                            return wire::encode_error("random_fill: getRandomValues failed");
                        }
                    }
                    None => return wire::encode_error("random_fill: no crypto in window"),
                }
            }
            // Use String as a compact byte container in the postcard payload
            // (avoids per-element overhead of postcard's list encoding).
            // The plugin-side wrapper turns this back into Vec<u8>.
            wire::encode_return(&XDatum::String(unsafe {
                String::from_utf8_unchecked(buf)
            }))
        }
        HostOp::StorageGet => {
            let key = match args.first() {
                Some(XDatum::String(s)) => s.as_str(),
                _ => {
                    return wire::encode_error("storage_get: expected String key");
                }
            };
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(window) = web_sys::window() {
                    if let Ok(Some(storage)) = window.local_storage() {
                        if let Ok(Some(val)) = storage.get_item(key) {
                            return wire::encode_return(&XDatum::String(val));
                        }
                    }
                }
            }
            let _ = key;
            wire::encode_return(&XDatum::Void)
        }
        HostOp::StorageSet => {
            let (key, val) = match (args.first(), args.get(1)) {
                (Some(XDatum::String(k)), Some(XDatum::String(v))) => (k.as_str(), v.as_str()),
                _ => {
                    return wire::encode_error("storage_set: expected (key, val)");
                }
            };
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(window) = web_sys::window() {
                    if let Ok(Some(storage)) = window.local_storage() {
                        match storage.set_item(key, val) {
                            Ok(()) => return Vec::new(),
                            Err(_) => return wire::encode_error("localStorage.setItem failed"),
                        }
                    }
                }
            }
            let _ = (key, val);
            wire::encode_error("storage_set: no localStorage available")
        }
        HostOp::CreateXtraInstance
        | HostOp::CallXtraHandler
        | HostOp::DestroyXtraInstance => {
            // Inter-xtra dispatch is deferred to a later phase. For now
            // we surface a clear error so plugins fail fast instead of
            // hanging.
            wire::encode_error("inter-xtra dispatch not yet implemented")
        }
    }
}

// ── Helpers: Datum <-> DatumRef conversion ───────────────────────────────

/// Encode a `&Vec<DatumRef>` as a postcard `WireFrame::Args` payload.
/// Returns `None` on serialization failure — the caller surfaces this as
/// a None bubble to the dispatch arm.
fn encode_args_from_refs(args: &Vec<DatumRef>) -> Option<Vec<u8>> {
    let xs: Vec<XDatum> = reserve_player_mut(|player| {
        args.iter()
            .map(|r| host_datum_to_xdatum(&player.get_datum(r).clone(), player))
            .collect()
    });
    Some(wire::encode_args(&xs))
}

/// Convert a host-side `Datum` to an SDK-side `XDatum`. The mapping is
/// 1:1 for variants present in both; host-only variants (Color, CastRef,
/// BitmapRef, etc.) become `XDatum::Void` for now — a future revision of
/// the WIT contract can extend the variant set.
///
/// Director represents booleans as `Datum::Int(0)` / `Datum::Int(1)`
/// (there is no separate Bool variant on the host), so we forward Ints
/// as-is. Plugins that want bool semantics can compare to `Int(0)`.
fn host_datum_to_xdatum(d: &Datum, _player: &crate::player::DirPlayer) -> XDatum {
    match d {
        Datum::Void => XDatum::Void,
        Datum::Int(i) => XDatum::Int(*i),
        Datum::Float(f) => XDatum::Float(*f),
        Datum::String(s) => XDatum::String(s.clone()),
        Datum::Symbol(s) => XDatum::Symbol(s.clone()),
        _ => XDatum::Void,
    }
}

/// Convert an SDK-side `XDatum` back to a host `Datum` and allocate it
/// into the player's datum manager, returning a fresh `DatumRef`.
fn xdatum_to_host_datum_ref(d: &XDatum) -> DatumRef {
    reserve_player_mut(|player| {
        let host = xdatum_to_host_datum(d);
        player.alloc_datum(host)
    })
}

fn xdatum_to_host_datum(d: &XDatum) -> Datum {
    match d {
        XDatum::Void => Datum::Void,
        XDatum::Int(i) => Datum::Int(*i),
        XDatum::Float(f) => Datum::Float(*f),
        XDatum::String(s) => Datum::String(s.clone()),
        XDatum::Symbol(s) => Datum::Symbol(s.clone()),
        // Director booleans are Int(0)/Int(1). Use the helper so a future
        // change to the bool representation only touches one place.
        XDatum::Bool(b) => datum_bool(*b),
        _ => Datum::Void,
    }
}

/// Decode a wire-level return frame into a host `Result<DatumRef, ScriptError>`.
fn decode_return_to_datum_ref(
    bytes: &[u8],
    xtra_name: &str,
    handler_name: &str,
) -> Result<DatumRef, ScriptError> {
    match wire::decode_return(bytes) {
        Ok(d) => Ok(xdatum_to_host_datum_ref(&d)),
        Err(e) => Err(ScriptError::new(format!(
            "{}.{}: {}",
            xtra_name, handler_name, e
        ))),
    }
}

// Suppress unused-import warnings when the wasm32 cfg block isn't active.
#[allow(dead_code)]
fn _suppress() {
    let _ = HashMap::<i32, i32>::new();
}
