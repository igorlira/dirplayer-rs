use crate::{director::lingo::datum::Datum, player::symbols::symbol::Symbol};
use crate::player::script_ref::ScriptInstanceRef;
use super::{VirtualScriptHandler, VirtualScriptRegistry};
use crate::player::{DatumRef, DirPlayer, ScriptError};

pub struct JavascriptProxy;

impl VirtualScriptHandler for JavascriptProxy {
    fn has_handler(&self, name: Symbol) -> bool {
        matches!(name.as_lower_str(), "new" | "newjavascriptproxy" | "javascriptproxy" | "call")
    }

    fn call_handler(
        &self,
        player: &mut DirPlayer,
        instance: Option<&ScriptInstanceRef>,
        name: Symbol,
        _args: &Vec<DatumRef>,
    ) -> Result<Option<DatumRef>, ScriptError> {
        match name.as_lower_str() {
            "new" | "newjavascriptproxy" | "javascriptproxy" => {
                if let Some(instance_ref) = instance {
                    // Called on an existing instance — return self
                    let datum = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
                    Ok(Some(datum))
                } else {
                    // Called on the class or as a global — create a new instance
                    // Virtual scripts have no CastMember, so resolve through the
                    // registry rather than the movie's cast (see
                    // `VirtualScriptRegistry::register`).
                    let script_ref = VirtualScriptRegistry::find_by_name(player, "JavaScriptProxy")
                        .ok_or_else(|| ScriptError::new("JavaScriptProxy script not found".to_string()))?;
                    let (_instance_ref, datum_ref) =
                        VirtualScriptRegistry::create_instance(player, &script_ref);
                    Ok(Some(datum_ref))
                }
            }
            "call" => {
                // No-op, return self
                if let Some(instance_ref) = instance {
                    let datum = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
                    Ok(Some(datum))
                } else {
                    Ok(Some(DatumRef::Void))
                }
            }
            _ => Ok(None),
        }
    }
}
