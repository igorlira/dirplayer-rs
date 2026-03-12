use crate::director::lingo::datum::Datum;
use crate::player::script_ref::ScriptInstanceRef;
use super::{VirtualScriptHandler, VirtualScriptRegistry};
use crate::player::{DatumRef, DirPlayer, ScriptError};

pub struct JavascriptProxy;

impl VirtualScriptHandler for JavascriptProxy {
    fn has_handler(&self, name: &str) -> bool {
        matches!(name, "new" | "newJavaScriptProxy" | "JavaScriptProxy" | "call")
    }

    fn call_handler(
        &self,
        player: &mut DirPlayer,
        instance: Option<&ScriptInstanceRef>,
        name: &str,
        _args: &Vec<DatumRef>,
    ) -> Result<Option<DatumRef>, ScriptError> {
        match name {
            "new" | "newJavaScriptProxy" | "JavaScriptProxy" => {
                if let Some(instance_ref) = instance {
                    // Called on an existing instance — return self
                    let datum = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
                    Ok(Some(datum))
                } else {
                    // Called on the class or as a global — create a new instance
                    let script_ref = player
                        .movie
                        .cast_manager
                        .find_member_ref_by_name(&"JavaScriptProxy".to_string())
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
