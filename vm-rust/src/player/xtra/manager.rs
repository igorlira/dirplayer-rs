use crate::{
    director::lingo::datum::XtraInstanceId,
    player::{DatumRef, ScriptError},
};

use super::multiuser::{borrow_multiuser_manager_mut, MultiuserXtraManager};
use super::xmlparser::{borrow_xmlparser_manager_mut, XmlParserXtraManager};

pub fn is_xtra_registered(name: &String) -> bool {
    let name_lower = name.to_lowercase();
    return name == "Multiuser" || name_lower == "xmlparser";
}

pub fn get_registered_xtra_names() -> Vec<&'static str> {
    vec!["Multiusr", "XmlParser"]
}

pub fn call_xtra_instance_handler(
    xtra_name: &String,
    instance_id: XtraInstanceId,
    handler_name: &String,
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
        _ => Err(ScriptError::new(format!(
            "No handler {} found for xtra {} instance #{}",
            handler_name, xtra_name, instance_id
        ))),
    }
}

pub async fn call_xtra_instance_async_handler(
    xtra_name: &String,
    instance_id: XtraInstanceId,
    handler_name: &String,
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
        _ => Err(ScriptError::new(format!(
            "No async handler {} found for xtra {} instance #{}",
            handler_name, xtra_name, instance_id
        ))),
    }
}

pub fn has_xtra_instance_async_handler(
    xtra_name: &String,
    handler_name: &String,
    _instance_id: XtraInstanceId,
) -> bool {
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => MultiuserXtraManager::has_instance_async_handler(handler_name),
        "xmlparser" => XmlParserXtraManager::has_instance_async_handler(handler_name),
        _ => false,
    }
}

pub fn create_xtra_instance(
    xtra_name: &String,
    args: &Vec<DatumRef>,
) -> Result<XtraInstanceId, ScriptError> {
    let xtra_name_lower = xtra_name.to_lowercase();
    match xtra_name_lower.as_str() {
        "multiuser" => Ok(borrow_multiuser_manager_mut(|x| x.create_instance(args))),
        "xmlparser" => Ok(borrow_xmlparser_manager_mut(|x| x.create_instance(args))),
        _ => Err(ScriptError::new(format!("Xtra {} not found", xtra_name))),
    }
}
