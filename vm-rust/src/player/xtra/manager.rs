use crate::{
    director::lingo::datum::XtraInstanceId,
    player::{DatumRef, ScriptError},
};

use super::external::{
    external_call_xtra_instance_handler, external_create_xtra_instance,
    external_has_xtra_instance_async_handler, external_is_xtra_registered,
    external_try_call_xtra_static_handler, get_external_xtra_names,
};
use super::fileio::{borrow_fileio_manager_mut, FileIoXtraManager};
use super::multiuser::{borrow_multiuser_manager_mut, MultiuserXtraManager};
use super::xmlparser::{borrow_xmlparser_manager_mut, XmlParserXtraManager};

pub fn is_xtra_registered(name: &str) -> bool {
    // External plugins take precedence over built-ins.
    if external_is_xtra_registered(name) {
        return true;
    }
    let name_lower = name.to_lowercase();
    name == "Multiuser" || name_lower == "xmlparser" || name_lower == "fileio"
}

pub fn get_registered_xtra_names() -> Vec<String> {
    let mut names: Vec<String> = get_external_xtra_names();
    names.extend(["Multiusr", "XmlParser", "FileIO"].iter().map(|s| s.to_string()));
    names
}

/// Dispatch for static (`*`-prefixed) Xtra functions — global handlers that
/// don't require `object me` (e.g. `gsOpenURL`, `baFileExists`, `sysMenuMessageBox`).
/// Returns `Some(result)` if any registered Xtra owns the handler name.
pub fn try_call_xtra_static_handler(
    name: &str,
    args: &Vec<DatumRef>,
) -> Option<Result<DatumRef, ScriptError>> {
    // Check external xtras first.
    if let Some(result) = external_try_call_xtra_static_handler(name, args) {
        return Some(result);
    }
    None
}

pub fn has_xtra_static_async_handler(_name: &str) -> bool {
    false
}

pub async fn call_xtra_static_async_handler(
    name: &str,
    _args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
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
    // External xtras take precedence.
    if external_is_xtra_registered(xtra_name) {
        return external_call_xtra_instance_handler(xtra_name, instance_id, handler_name, args);
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
    // External xtras use their declared async handler info.
    if external_is_xtra_registered(xtra_name) {
        return external_has_xtra_instance_async_handler(xtra_name, handler_name);
    }
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => MultiuserXtraManager::has_instance_async_handler(handler_name),
        "xmlparser" => XmlParserXtraManager::has_instance_async_handler(handler_name),
        "fileio" => FileIoXtraManager::has_instance_async_handler(handler_name),
        _ => false,
    }
}

pub fn create_xtra_instance(
    xtra_name: &str,
    args: &Vec<DatumRef>,
) -> Result<XtraInstanceId, ScriptError> {
    // External xtras take precedence.
    if external_is_xtra_registered(xtra_name) {
        return external_create_xtra_instance(xtra_name, args);
    }
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => Ok(borrow_multiuser_manager_mut(|x| x.create_instance(args))),
        "xmlparser" => Ok(borrow_xmlparser_manager_mut(|x| x.create_instance(args))),
        "fileio" => Ok(borrow_fileio_manager_mut(|x| x.create_instance(args))),
        _ => Err(ScriptError::new(format!("Xtra {} not found", xtra_name))),
    }
}
