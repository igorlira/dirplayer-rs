use wasm_bindgen::prelude::*;

use crate::player::{DatumRef, ScriptError};

use super::{
    datum_convert::{args_to_json, json_to_datum_ref},
    ExternalXtra,
};

#[wasm_bindgen(module = "dirplayer-js-api")]
extern "C" {
    /// Returns true if a plugin with this name (case-insensitive) has been loaded.
    pub fn isExternalXtraLoaded(name: &str) -> bool;

    /// Create a new plugin instance. Returns JSON `{"ok":id}` or `{"err":"..."}`.
    pub fn externalXtraCreateInstance(name: &str, args_json: &str) -> String;

    /// Destroy a plugin instance.
    pub fn externalXtraDestroyInstance(name: &str, id: u32);

    /// Call a handler on a plugin instance.
    /// Returns JSON datum or `{"__error":"..."}`.
    pub fn externalXtraCallHandler(
        name: &str,
        id: u32,
        handler_name: &str,
        args_json: &str,
    ) -> String;

    /// Returns true if the named handler is async.
    pub fn externalXtraHasAsyncHandler(name: &str, handler_name: &str) -> bool;

    /// Returns true if the named handler is static (no instance needed).
    pub fn externalXtraHasStaticHandler(name: &str, handler_name: &str) -> bool;

    /// Call a static handler on a plugin.
    /// Returns JSON datum or `{"__error":"..."}`.
    pub fn externalXtraCallStaticHandler(
        name: &str,
        handler_name: &str,
        args_json: &str,
    ) -> String;

    /// Returns a JSON array of loaded external xtra names (lowercase).
    pub fn getLoadedExternalXtraNames() -> String;
}

/// Wraps a JS-loaded external xtra plugin, dispatching all calls through
/// the `dirplayer-js-api` JS bridge.
pub struct JsExternalXtra {
    name: String,
}

impl JsExternalXtra {
    pub fn new(name: String) -> Self {
        JsExternalXtra { name }
    }
}

impl ExternalXtra for JsExternalXtra {
    fn name(&self) -> &str {
        &self.name
    }

    fn create_instance(&mut self, args: &[DatumRef]) -> Result<u32, String> {
        let args_json = args_to_json(args);
        let result = externalXtraCreateInstance(&self.name, &args_json);
        let v: serde_json::Value =
            serde_json::from_str(&result).map_err(|e| format!("create_instance parse: {e}"))?;
        if let Some(err) = v.get("err").and_then(|e| e.as_str()) {
            return Err(err.to_string());
        }
        v.get("ok")
            .and_then(|id| id.as_u64())
            .map(|id| id as u32)
            .ok_or_else(|| format!("create_instance: unexpected response: {result}"))
    }

    fn destroy_instance(&mut self, id: u32) {
        externalXtraDestroyInstance(&self.name, id);
    }

    fn call_handler(
        &mut self,
        id: u32,
        name: &str,
        args: &[DatumRef],
    ) -> Result<DatumRef, String> {
        let args_json = args_to_json(args);
        let result = externalXtraCallHandler(&self.name, id, name, &args_json);
        parse_result_json(&result)
    }

    fn has_async_handler(&self, name: &str) -> bool {
        externalXtraHasAsyncHandler(&self.name, name)
    }

    fn has_static_handler(&self, name: &str) -> bool {
        externalXtraHasStaticHandler(&self.name, name)
    }

    fn call_static_handler(&mut self, name: &str, args: &[DatumRef]) -> Result<DatumRef, String> {
        let args_json = args_to_json(args);
        let result = externalXtraCallStaticHandler(&self.name, name, &args_json);
        parse_result_json(&result)
    }
}

fn parse_result_json(json_str: &str) -> Result<DatumRef, String> {
    let v: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("result parse: {e}"))?;
    if let Some(err) = v.get("__error").and_then(|e| e.as_str()) {
        return Err(err.to_string());
    }
    json_to_datum_ref(json_str)
}
