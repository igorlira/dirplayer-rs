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

use std::collections::{HashMap, HashSet, VecDeque};

use futures::channel::oneshot;

use crate::director::lingo::datum::{Datum, DatumType, XtraInstanceId, datum_bool};
use crate::player::{DatumRef, DirPlayer, ScriptError, reserve_player_mut, reserve_player_ref};

use xtra_sdk::Datum as XDatum;
use xtra_sdk::scene3d::{FrameData as SceneFrameData, MeshData as SceneMeshData};
use xtra_sdk::wire;

use super::scene3d;

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
    Scene3dCreate = 8,
    Scene3dUploadMesh = 9,
    Scene3dDropMesh = 10,
    Scene3dUploadTexture = 11,
    Scene3dSubmitFrame = 12,
    Scene3dDestroy = 13,
    CastMemberBytes = 14,
    StageInfo = 15,
    MouseLoc = 16,
    KeyDown = 17,
    SetLingoGlobal = 18,
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
            8 => HostOp::Scene3dCreate,
            9 => HostOp::Scene3dUploadMesh,
            10 => HostOp::Scene3dDropMesh,
            11 => HostOp::Scene3dUploadTexture,
            12 => HostOp::Scene3dSubmitFrame,
            13 => HostOp::Scene3dDestroy,
            14 => HostOp::CastMemberBytes,
            15 => HostOp::StageInfo,
            16 => HostOp::MouseLoc,
            17 => HostOp::KeyDown,
            18 => HostOp::SetLingoGlobal,
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

// ── Pending on-demand loads ──────────────────────────────────────────────
//
// When Lingo executes `new(xtra "X")` and X isn't registered, vm-rust asks
// JS to resolve the name through the registry and load the .wasm. The
// bytecode dispatch awaits a oneshot signal here; JS calls
// `complete_external_xtra_load(name, success)` (exported in `lib.rs`) when
// the load finishes, which fires every receiver waiting on that name.
//
// Multiple concurrent requests for the same name share one fetch — only
// the first requester triggers the JS callback; subsequent ones just
// append a receiver to the queue.

#[derive(Default)]
struct PendingLoad {
    /// One receiver per concurrent requester. When the load finishes
    /// `complete_load` drains the vec and signals each with the result.
    waiters: Vec<oneshot::Sender<bool>>,
}

static mut PENDING_LOADS: Option<HashMap<String, PendingLoad>> = None;

fn pending_loads() -> &'static mut HashMap<String, PendingLoad> {
    unsafe {
        let ptr = &raw mut PENDING_LOADS;
        (*ptr).get_or_insert_with(HashMap::new)
    }
}

/// Ask the host to resolve `name` against its registry and load the
/// plugin. Returns `true` if the load succeeded (the caller can retry
/// `is_registered` and expect success); `false` if no registry entry
/// matched or the load itself failed. Resolves immediately if the name
/// is already registered, so callers can use this as a "ensure loaded"
/// check without a separate fast-path branch.
pub async fn request_xtra_load(name: &str) -> bool {
    let key = name.to_lowercase();
    if is_registered(&key) {
        return true;
    }
    let (tx, rx) = oneshot::channel::<bool>();
    let need_to_request = {
        let map = pending_loads();
        let entry = map.entry(key.clone()).or_default();
        let was_empty = entry.waiters.is_empty();
        entry.waiters.push(tx);
        was_empty
    };
    if need_to_request {
        // Fire-and-forget. JS will call complete_load with the result.
        js_bridge::onRequestXtraLoad(&key);
    }
    rx.await.unwrap_or(false)
}

/// JS calls this (via the `complete_external_xtra_load` wasm-bindgen
/// export in `lib.rs`) once an on-demand load finishes. Drains the
/// waiters for `name` and signals each with `success`.
pub fn complete_load(name: &str, success: bool) {
    let key = name.to_lowercase();
    let entry = pending_loads().remove(&key);
    if let Some(entry) = entry {
        for tx in entry.waiters {
            let _ = tx.send(success);
        }
    }
}

/// Cancel every outstanding on-demand load. Called from `DirPlayer::reset`
/// so leftover futures from a previous movie don't leak across loads.
/// Signals every waiter with `false`, which makes their `request_xtra_load`
/// await resolve to "not loaded" — the in-flight bytecode handler then
/// surfaces the normal "Xtra X not found" error.
pub fn cancel_all_pending_loads() {
    let map = pending_loads();
    let drained: Vec<(String, PendingLoad)> = map.drain().collect();
    for (_name, entry) in drained {
        for tx in entry.waiters {
            let _ = tx.send(false);
        }
    }
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

        /// Tell JS to resolve `name` against the registry and load the
        /// plugin asynchronously. JS calls back via the wasm-bindgen
        /// export `complete_external_xtra_load(name, success)` (see
        /// `lib.rs`) when the load finishes — there is no return value
        /// here; this function is fire-and-forget. Fired by
        /// `request_xtra_load` when an unknown xtra is hit by Lingo.
        pub fn onRequestXtraLoad(name: &str);
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
    pub fn onRequestXtraLoad(_: &str) {}
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

/// Try every registered external xtra as the owner of a **bare** global
/// (no-instance) handler — the shape Groove uses (`InitGroove()`,
/// `MoveObject(...)`). Unlike [`call_static_handler`], the caller doesn't know
/// which xtra owns the name, so we ask each registered plugin
/// `__xtra_has_static_handler(handler)` and dispatch to the first that claims
/// it. Returns `None` when no external plugin owns the handler, so the caller
/// falls through to built-ins.
///
/// Because external plugins are consulted before the built-in Groove fallback
/// in `manager.rs`, a loaded Groove plugin transparently shadows the built-in.
pub fn try_any_static_handler(
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Option<Result<DatumRef, ScriptError>> {
    for name in registered_names() {
        if has_static_handler(&name, handler_name) {
            return call_static_handler(&name, handler_name, args);
        }
    }
    None
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
                log::debug!("[xtra] {}", msg);
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
            wire::encode_return(&XDatum::ByteArray(buf))
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

        // ── 3D scene rendering ────────────────────────────────────────────
        // These arrive mid-Lingo-execution (no GL context in scope), so they
        // only mutate the CPU-side scene store; the webgl2 `XtraSceneRenderer`
        // uploads and composites during the normal draw pass. Note the player
        // lock is free here — `call_static_handler`/`call_instance_handler`
        // release `reserve_player_mut` (in `encode_args_from_refs`) before the
        // JS hop that runs the plugin, so these may re-take it safely.
        HostOp::Scene3dCreate => {
            let tag = match args.first() {
                Some(XDatum::String(s)) => s.clone(),
                _ => return wire::encode_error("scene3d_create: expected String tag"),
            };
            let id = scene3d::with_store_mut(|s| s.create(&tag));
            wire::encode_return(&XDatum::Int(id))
        }
        HostOp::Scene3dUploadMesh => {
            let (scene_id, mesh_id, bytes) = match (args.first(), args.get(1), args.get(2)) {
                (Some(XDatum::Int(s)), Some(XDatum::Int(m)), Some(XDatum::ByteArray(b))) => {
                    (*s, *m as u32, b)
                }
                _ => return wire::encode_error("scene3d_upload_mesh: expected (Int, Int, ByteArray)"),
            };
            match SceneMeshData::from_bytes(bytes) {
                Ok(data) => {
                    scene3d::with_store_mut(|s| s.upload_mesh(scene_id, mesh_id, data));
                    Vec::new()
                }
                Err(e) => wire::encode_error(&format!("scene3d_upload_mesh: bad MeshData: {:?}", e)),
            }
        }
        HostOp::Scene3dDropMesh => {
            let (scene_id, mesh_id) = match (args.first(), args.get(1)) {
                (Some(XDatum::Int(s)), Some(XDatum::Int(m))) => (*s, *m as u32),
                _ => return wire::encode_error("scene3d_drop_mesh: expected (Int, Int)"),
            };
            scene3d::with_store_mut(|s| s.drop_mesh(scene_id, mesh_id));
            Vec::new()
        }
        HostOp::Scene3dUploadTexture => {
            let (scene_id, name, w, h, rgba) = match (
                args.first(), args.get(1), args.get(2), args.get(3), args.get(4),
            ) {
                (
                    Some(XDatum::Int(s)), Some(XDatum::String(n)),
                    Some(XDatum::Int(w)), Some(XDatum::Int(h)), Some(XDatum::ByteArray(b)),
                ) => (*s, n.clone(), *w as u32, *h as u32, b.clone()),
                _ => {
                    return wire::encode_error(
                        "scene3d_upload_texture: expected (Int, String, Int, Int, ByteArray)",
                    );
                }
            };
            scene3d::with_store_mut(|s| s.upload_texture(scene_id, &name, w, h, rgba));
            Vec::new()
        }
        HostOp::Scene3dSubmitFrame => {
            let (scene_id, bytes) = match (args.first(), args.get(1)) {
                (Some(XDatum::Int(s)), Some(XDatum::ByteArray(b))) => (*s, b),
                _ => return wire::encode_error("scene3d_submit_frame: expected (Int, ByteArray)"),
            };
            match SceneFrameData::from_bytes(bytes) {
                Ok(frame) => {
                    let movie_frame =
                        reserve_player_mut(|player| player.movie.current_frame as i32);
                    scene3d::with_store_mut(|s| s.submit_frame(scene_id, frame, movie_frame));
                    Vec::new()
                }
                Err(e) => wire::encode_error(&format!("scene3d_submit_frame: bad FrameData: {:?}", e)),
            }
        }
        HostOp::Scene3dDestroy => {
            let scene_id = match args.first() {
                Some(XDatum::Int(s)) => *s,
                _ => return wire::encode_error("scene3d_destroy: expected Int scene_id"),
            };
            scene3d::with_store_mut(|s| s.destroy(scene_id));
            Vec::new()
        }

        // ── Host state a compute-only plugin reads ────────────────────────
        HostOp::CastMemberBytes => {
            use crate::player::cast_member::CastMemberType;
            let (name, kind) = match (args.first(), args.get(1)) {
                (Some(XDatum::String(n)), Some(XDatum::String(k))) => (n.clone(), k.to_lowercase()),
                _ => return wire::encode_error("cast_member_bytes: expected (String, String)"),
            };
            let bytes = reserve_player_ref(|player| {
                let mref = player.movie.cast_manager.find_member_ref_by_name(&name)?;
                let member = player.movie.cast_manager.find_member_by_ref(&mref)?;
                match (kind.as_str(), &member.member_type) {
                    ("groove3gm", CastMemberType::Groove3gm(m)) => Some(m.data.clone()),
                    _ => None,
                }
            });
            match bytes {
                Some(b) => wire::encode_return(&XDatum::ByteArray(b)),
                None => wire::encode_return(&XDatum::Void),
            }
        }
        HostOp::StageInfo => {
            let (w, h, frame) = reserve_player_ref(|player| {
                (
                    player.movie.rect.width(),
                    player.movie.rect.height(),
                    player.movie.current_frame as i32,
                )
            });
            wire::encode_return(&XDatum::List(vec![
                XDatum::Int(w),
                XDatum::Int(h),
                XDatum::Int(frame),
            ]))
        }
        HostOp::MouseLoc => {
            let (x, y) = reserve_player_ref(|player| player.mouse_loc);
            wire::encode_return(&XDatum::Point(x as f64, y as f64))
        }
        HostOp::KeyDown => {
            let key = match args.first() {
                Some(XDatum::String(s)) => s.clone(),
                _ => return wire::encode_error("key_down: expected String key"),
            };
            let down = reserve_player_ref(|player| player.keyboard_manager.is_key_down(&key));
            wire::encode_return(&XDatum::Bool(down))
        }
        HostOp::SetLingoGlobal => {
            let (name, value) = match (args.first(), args.get(1)) {
                (Some(XDatum::String(n)), Some(v)) => (n.clone(), v.clone()),
                _ => return wire::encode_error("set_lingo_global: expected (String, value)"),
            };
            // Allocate the value into the datum arena, then bind the global.
            let value_ref = xdatum_to_host_datum_ref(&value);
            reserve_player_mut(|player| {
                player.globals.insert(name, value_ref);
            });
            Vec::new()
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
fn host_datum_to_xdatum(d: &Datum, player: &DirPlayer) -> XDatum {
    match d {
        Datum::Void => XDatum::Void,
        Datum::Int(i) => XDatum::Int(*i),
        Datum::Float(f) => XDatum::Float(*f),
        Datum::String(s) => XDatum::String(s.clone()),
        Datum::Symbol(s) => XDatum::Symbol(s.clone()),
        // Container variants recurse, resolving each child DatumRef through
        // the player's datum arena. Groove passes/returns lists, prop-lists,
        // points and rects, so these must survive the boundary intact.
        Datum::List(_, items, _) => XDatum::List(
            items
                .iter()
                .map(|r| host_datum_to_xdatum(player.get_datum(r), player))
                .collect(),
        ),
        Datum::PropList(pairs, _) => XDatum::PropList(
            pairs
                .iter()
                .map(|(k, v)| {
                    (
                        host_datum_to_xdatum(player.get_datum(k), player),
                        host_datum_to_xdatum(player.get_datum(v), player),
                    )
                })
                .collect(),
        ),
        Datum::Point([x, y], _) => XDatum::Point(*x, *y),
        Datum::Rect([l, t, r, b], _) => XDatum::Rect(*l, *t, *r, *b),
        _ => XDatum::Void,
    }
}

/// Convert an SDK-side `XDatum` back to a host `Datum` and allocate it
/// into the player's datum manager, returning a fresh `DatumRef`.
fn xdatum_to_host_datum_ref(d: &XDatum) -> DatumRef {
    reserve_player_mut(|player| {
        let host = xdatum_to_host_datum(d, player);
        player.alloc_datum(host)
    })
}

fn xdatum_to_host_datum(d: &XDatum, player: &mut DirPlayer) -> Datum {
    match d {
        XDatum::Void => Datum::Void,
        XDatum::Int(i) => Datum::Int(*i),
        XDatum::Float(f) => Datum::Float(*f),
        XDatum::String(s) => Datum::String(s.clone()),
        XDatum::Symbol(s) => Datum::Symbol(s.clone()),
        // Container variants recurse: each child is converted and allocated
        // into the player's datum arena first, then the parent references the
        // fresh DatumRefs. Mirrors `host_datum_to_xdatum` in the other
        // direction so Groove's list/prop-list/point/rect returns survive.
        XDatum::List(items) => {
            let refs: VecDeque<DatumRef> = items
                .iter()
                .map(|it| {
                    let child = xdatum_to_host_datum(it, player);
                    player.alloc_datum(child)
                })
                .collect();
            Datum::List(DatumType::List, refs, false)
        }
        XDatum::PropList(pairs) => {
            let entries: VecDeque<(DatumRef, DatumRef)> = pairs
                .iter()
                .map(|(k, v)| {
                    let kd = xdatum_to_host_datum(k, player);
                    let kr = player.alloc_datum(kd);
                    let vd = xdatum_to_host_datum(v, player);
                    let vr = player.alloc_datum(vd);
                    (kr, vr)
                })
                .collect();
            Datum::PropList(entries, false)
        }
        XDatum::Point(x, y) => Datum::Point([*x, *y], 0),
        XDatum::Rect(l, t, r, b) => Datum::Rect([*l, *t, *r, *b], 0),
        // Director booleans are Int(0)/Int(1). Use the helper so a future
        // change to the bool representation only touches one place.
        XDatum::Bool(b) => datum_bool(*b),
        // Byte payloads round-trip into Director as a Latin-1 string
        // (byte b → char b). This matches how multiuser/fileio surface
        // binary data to Lingo: the high-bit-set chars stay distinct,
        // and `string_value()` on the host still yields the original
        // bytes via `c as u8`. Plugins that round-trip raw bytes hand
        // them back to other plugins through `host_env::call_xtra_handler`
        // using ByteArray directly.
        XDatum::ByteArray(b) => {
            Datum::String(b.iter().map(|&byte| byte as char).collect())
        }
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
