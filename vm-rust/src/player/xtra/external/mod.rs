pub mod datum_convert;
pub mod js_bridge;

use crate::{
    director::lingo::datum::XtraInstanceId,
    player::{DatumRef, ScriptError},
};

pub use js_bridge::JsExternalXtra;

/// Mirrors the WIT `xtra-plugin` interface.  Each loaded external WASM plugin
/// gets one implementation of this trait registered in the global registry.
pub trait ExternalXtra: Send {
    fn name(&self) -> &str;
    fn create_instance(&mut self, args: &[DatumRef]) -> Result<u32, String>;
    fn destroy_instance(&mut self, id: u32);
    fn call_handler(&mut self, id: u32, name: &str, args: &[DatumRef])
        -> Result<DatumRef, String>;
    fn has_async_handler(&self, name: &str) -> bool;
    fn has_static_handler(&self, name: &str) -> bool;
    fn call_static_handler(&mut self, name: &str, args: &[DatumRef]) -> Result<DatumRef, String>;
}

// ── Global registry ───────────────────────────────────────────────────────────

static mut EXTERNAL_XTRA_REGISTRY: Option<Vec<Box<dyn ExternalXtra>>> = None;

pub fn init_external_xtra_registry() {
    unsafe {
        EXTERNAL_XTRA_REGISTRY = Some(Vec::new());
    }
}

pub fn register_external_xtra(xtra: Box<dyn ExternalXtra>) {
    unsafe {
        if let Some(registry) = EXTERNAL_XTRA_REGISTRY.as_mut() {
            registry.push(xtra);
        }
    }
}

pub fn is_external_xtra_registered(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    unsafe {
        EXTERNAL_XTRA_REGISTRY
            .as_ref()
            .map(|r| r.iter().any(|x| x.name().to_lowercase() == name_lower))
            .unwrap_or(false)
    }
}

/// Run a closure against the external xtra with the given name (mutably).
/// Returns None if not found.
pub fn with_external_xtra_mut<F, R>(name: &str, f: F) -> Option<R>
where
    F: FnOnce(&mut dyn ExternalXtra) -> R,
{
    let name_lower = name.to_lowercase();
    unsafe {
        let registry = EXTERNAL_XTRA_REGISTRY.as_mut()?;
        let xtra = registry
            .iter_mut()
            .find(|x| x.name().to_lowercase() == name_lower)?;
        Some(f(xtra.as_mut()))
    }
}

pub fn get_external_xtra_names() -> Vec<String> {
    unsafe {
        EXTERNAL_XTRA_REGISTRY
            .as_ref()
            .map(|r| r.iter().map(|x| x.name().to_string()).collect())
            .unwrap_or_default()
    }
}

// ── Public dispatch API (mirrors xtra/manager.rs signature style) ─────────────

pub fn external_is_xtra_registered(name: &str) -> bool {
    is_external_xtra_registered(name)
}

pub fn external_create_xtra_instance(
    xtra_name: &str,
    args: &Vec<DatumRef>,
) -> Result<XtraInstanceId, ScriptError> {
    with_external_xtra_mut(xtra_name, |x| x.create_instance(args))
        .ok_or_else(|| ScriptError::new(format!("External xtra '{}' not found", xtra_name)))?
        .map_err(|e| ScriptError::new(e))
}

pub fn external_call_xtra_instance_handler(
    xtra_name: &str,
    instance_id: XtraInstanceId,
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    with_external_xtra_mut(xtra_name, |x| {
        x.call_handler(instance_id, handler_name, args)
    })
    .ok_or_else(|| ScriptError::new(format!("External xtra '{}' not found", xtra_name)))?
    .map_err(|e| ScriptError::new(e))
}

pub fn external_has_xtra_instance_async_handler(
    xtra_name: &str,
    handler_name: &str,
) -> bool {
    unsafe {
        EXTERNAL_XTRA_REGISTRY
            .as_ref()
            .and_then(|r| {
                let name_lower = xtra_name.to_lowercase();
                r.iter().find(|x| x.name().to_lowercase() == name_lower)
            })
            .map(|x| x.has_async_handler(handler_name))
            .unwrap_or(false)
    }
}

pub fn external_try_call_xtra_static_handler(
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Option<Result<DatumRef, ScriptError>> {
    unsafe {
        let registry = EXTERNAL_XTRA_REGISTRY.as_mut()?;
        for xtra in registry.iter_mut() {
            if xtra.has_static_handler(handler_name) {
                let result = xtra
                    .call_static_handler(handler_name, args)
                    .map_err(|e| ScriptError::new(e));
                return Some(result);
            }
        }
        None
    }
}
