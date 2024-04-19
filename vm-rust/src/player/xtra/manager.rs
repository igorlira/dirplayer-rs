use crate::{
    director::lingo::datum::XtraInstanceId,
    player::{DatumRef, ScriptError},
};

use super::multiuser::{borrow_multiuser_manager_mut, MultiuserXtraManager};

pub fn is_xtra_registered(name: &String) -> bool {
    return name == "Multiuser";
}

pub fn call_xtra_instance_handler(
    xtra_name: &String,
    instance_id: XtraInstanceId,
    handler_name: &String,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    match xtra_name.as_str() {
        "Multiuser" => {
            return MultiuserXtraManager::call_instance_handler(handler_name, instance_id, args)
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
    match xtra_name.as_str() {
        "Multiuser" => {
            return MultiuserXtraManager::call_instance_async_handler(
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
    match xtra_name.as_str() {
        "Multiuser" => MultiuserXtraManager::has_instance_async_handler(handler_name),
        _ => false,
    }
}

pub fn create_xtra_instance(
    xtra_name: &String,
    args: &Vec<DatumRef>,
) -> Result<XtraInstanceId, ScriptError> {
    match xtra_name.as_str() {
        "Multiuser" => Ok(borrow_multiuser_manager_mut(|x| x.create_instance(args))),
        _ => Err(ScriptError::new(format!("Xtra {} not found", xtra_name))),
    }
}
