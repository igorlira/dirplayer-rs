use crate::{
    director::lingo::datum::XtraInstanceId,
    player::{DatumRef, ScriptError},
};

use super::budapi::BudApiXtra;
use super::curl::{CurlXtra, CurlXtraManager};
use super::external;
use super::fileio::{borrow_fileio_manager_mut, FileIoXtraManager};
use super::multiuser::{borrow_multiuser_manager_mut, MultiuserXtraManager};
use super::openurl::OpenUrlXtra;
use super::sysmenu::SysMenuXtra;
use super::xmlparser::{borrow_xmlparser_manager_mut, XmlParserXtraManager};

pub fn is_xtra_registered(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    // External plugin xtras win over built-ins (per the spec). This is
    // how a user-loaded plugin can shadow a stale built-in.
    if external::is_registered(&name_lower) {
        return true;
    }
    return name == "Multiuser"
        || name_lower == "xmlparser"
        || name_lower == "fileio"
        || name_lower == "curl"
        || name_lower == "openurl"
        || name_lower == "sysmenu"
        || name_lower == "budapi";
}

pub fn get_registered_xtra_names() -> Vec<String> {
    let mut names: Vec<String> = vec![
        "Multiusr".to_string(),
        "XmlParser".to_string(),
        "FileIO".to_string(),
        "Curl".to_string(),
        "OpenURL".to_string(),
        "SysMenu".to_string(),
        "BudAPI".to_string(),
    ];
    // Append any externally loaded xtras. Names are stored lowercased in
    // the external registry; we surface them as-is — Director treats
    // `the xtraList` entries case-insensitively at lookup time anyway.
    names.extend(external::registered_names());
    names
}

/// Dispatch for static (`*`-prefixed) Xtra functions — global handlers that
/// don't require `object me` (e.g. `gsOpenURL`, `baFileExists`, `sysMenuMessageBox`).
/// Returns `Some(result)` if any registered Xtra owns the handler name.
pub fn try_call_xtra_static_handler(
    name: &str,
    args: &Vec<DatumRef>,
) -> Option<Result<DatumRef, ScriptError>> {
    if OpenUrlXtra::has_handler(name) {
        return Some(OpenUrlXtra::call_handler(name, args));
    }
    if SysMenuXtra::has_handler(name) {
        return Some(SysMenuXtra::call_handler(name, args));
    }
    if BudApiXtra::has_handler(name) {
        return Some(BudApiXtra::call_handler(name, args));
    }
    if CurlXtra::has_static_handler(name) {
        return Some(CurlXtra::call_static_handler(name, args));
    }
    // External plugins own bare global handlers too — e.g. the Groove plugin's
    // `InitGroove()`/`MoveObject()`, which games call as bare globals (never
    // `new(xtra "Groove")`). This is the sole path that serves Groove now that
    // the built-in engine is gone.
    if let Some(result) = external::try_any_static_handler(name, args) {
        return Some(result);
    }
    None
}

pub fn has_xtra_static_async_handler(name: &str) -> bool {
    CurlXtra::has_static_async_handler(name)
}

pub async fn call_xtra_static_async_handler(
    name: &str,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    if CurlXtra::has_static_async_handler(name) {
        return CurlXtra::call_static_async_handler(name, args).await;
    }
    Err(ScriptError::new(format!(
        "No async static handler {} found in any Xtra",
        name
    )))
}

pub fn call_xtra_instance_handler(
    xtra_name: &str,
    instance_id: XtraInstanceId,
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    // External plugins first — they shadow built-ins when registered.
    if let Some(result) = external::call_instance_handler(xtra_name, instance_id, handler_name, args) {
        return result;
    }
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => {
            return MultiuserXtraManager::call_instance_handler(handler_name, instance_id, args)
        }
        "xmlparser" => {
            return XmlParserXtraManager::call_instance_handler(handler_name, instance_id, args)
        }
        "fileio" => {
            return FileIoXtraManager::call_instance_handler(handler_name, instance_id, args)
        }
        "curl" => {
            return CurlXtraManager::call_instance_handler(handler_name, instance_id, args)
        }
        _ => Err(ScriptError::new(format!(
            "No handler {} found for xtra {} instance #{}",
            handler_name, xtra_name, instance_id
        ))),
    }
}

pub async fn call_xtra_instance_async_handler(
    xtra_name: &str,
    instance_id: XtraInstanceId,
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => {
            return MultiuserXtraManager::call_instance_async_handler(
                handler_name,
                instance_id,
                args,
            )
            .await
        }
        "xmlparser" => {
            return XmlParserXtraManager::call_instance_async_handler(
                handler_name,
                instance_id,
                args,
            )
            .await
        }
        "fileio" => {
            return FileIoXtraManager::call_instance_async_handler(
                handler_name,
                instance_id,
                args,
            )
            .await
        }
        "curl" => {
            return CurlXtraManager::call_instance_async_handler(
                handler_name,
                instance_id,
                args,
            )
            .await
        }
        _ => Err(ScriptError::new(format!(
            "No async handler {} found for xtra {} instance #{}",
            handler_name, xtra_name, instance_id
        ))),
    }
}

pub fn has_xtra_instance_async_handler(
    xtra_name: &str,
    handler_name: &str,
    _instance_id: XtraInstanceId,
) -> bool {
    // External plugins don't support async handlers in v1 — fall through
    // to built-ins so a same-named built-in still works.
    if external::is_registered(xtra_name) {
        return false;
    }
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => MultiuserXtraManager::has_instance_async_handler(handler_name),
        "xmlparser" => XmlParserXtraManager::has_instance_async_handler(handler_name),
        "fileio" => FileIoXtraManager::has_instance_async_handler(handler_name),
        "curl" => CurlXtraManager::has_instance_async_handler(handler_name),
        _ => false,
    }
}

pub fn create_xtra_instance(
    xtra_name: &str,
    args: &Vec<DatumRef>,
) -> Result<XtraInstanceId, ScriptError> {
    // External plugins first.
    if let Some(result) = external::create_instance(xtra_name, args) {
        return result;
    }
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => Ok(borrow_multiuser_manager_mut(|x| x.create_instance(args))),
        "xmlparser" => Ok(borrow_xmlparser_manager_mut(|x| x.create_instance(args))),
        "fileio" => Ok(borrow_fileio_manager_mut(|x| x.create_instance(args))),
        "curl" => Ok(super::curl::borrow_curl_manager_mut(|x| x.create_instance(args))),
        // OpenURL, SysMenu, BudAPI are static-only — `new` still hands back
        // an opaque instance id for parity with the real Xtras, but the id
        // is never consulted by any handler.
        "openurl" | "sysmenu" | "budapi" => Ok(0),
        _ => Err(ScriptError::new(format!("Xtra {} not found", xtra_name))),
    }
}

/// Async variant of [`create_xtra_instance`] that triggers on-demand
/// loading of unknown external xtras. Called from the Lingo `new(xtra "X")`
/// dispatch (`TypeHandlers::new` → `DatumType::Xtra` arm).
///
/// Flow:
/// 1. Try the sync path. If the xtra is already registered (built-in or
///    previously-loaded external), this returns immediately — no async
///    overhead in the hot path.
/// 2. On miss, await `external::request_xtra_load(name)` which asks JS
///    to resolve the name through the registry and load the .wasm.
/// 3. If JS reports success, retry the sync path — the plugin is now
///    registered. If JS reports failure, surface the normal "not found"
///    error.
///
/// Concurrent `new(xtra "X")` calls for the same X share the same JS
/// fetch (see `external::request_xtra_load`).
pub async fn create_xtra_instance_async(
    xtra_name: &str,
    args: &Vec<DatumRef>,
) -> Result<XtraInstanceId, ScriptError> {
    match create_xtra_instance(xtra_name, args) {
        Ok(id) => Ok(id),
        Err(_) => {
            if external::request_xtra_load(xtra_name).await {
                // Plugin loaded; retry. The retry path goes through the
                // external dispatcher and will succeed (or surface a real
                // create-instance error from the plugin itself).
                create_xtra_instance(xtra_name, args)
            } else {
                Err(ScriptError::new(format!("Xtra {} not found", xtra_name)))
            }
        }
    }
}
