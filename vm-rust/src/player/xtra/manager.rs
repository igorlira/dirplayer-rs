use crate::{
    director::lingo::datum::XtraInstanceId,
    player::{DatumRef, ScriptError},
};

use super::budapi::BudApiXtra;
use super::curl::{CurlXtra, CurlXtraManager};
use super::fileio::{borrow_fileio_manager_mut, FileIoXtraManager};
use super::multiuser::{borrow_multiuser_manager_mut, MultiuserXtraManager};
use super::openurl::OpenUrlXtra;
use super::sysmenu::SysMenuXtra;
use super::xmlparser::{borrow_xmlparser_manager_mut, XmlParserXtraManager};

pub fn is_xtra_registered(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    return name == "Multiuser"
        || name_lower == "xmlparser"
        || name_lower == "fileio"
        || name_lower == "curl"
        || name_lower == "openurl"
        || name_lower == "sysmenu"
        || name_lower == "budapi";
}

pub fn get_registered_xtra_names() -> Vec<&'static str> {
    vec![
        "Multiusr",
        "XmlParser",
        "FileIO",
        "Curl",
        "OpenURL",
        "SysMenu",
        "BudAPI",
    ]
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
