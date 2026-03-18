use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

pub struct VoidDatumHandlers {}

impl VoidDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "addAt" | "add" | "append" | "duplicate" | "getAt" | "getOne" | "getLast" | "getFirst" => {
                // Calling these on void should just return void
                Ok(DatumRef::Void)
            }
            "count" => {
                // count(VOID, #items) etc. should return 0
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(0)))
                })
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for void"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        _: &DatumRef,
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        match prop.as_str() {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("void".to_owned()))),
            "count" | "length" => Ok(player.alloc_datum(Datum::Int(0))),
            "string" => Ok(player.alloc_datum(Datum::String("".to_owned()))),
            // XML-related properties on Void should return empty/void values
            "childNodes" => {
                // Return empty list for childNodes on void
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    vec![],
                    false,
                )))
            }
            "firstChild" | "lastChild" | "parentNode" | "nextSibling" | "previousSibling" => {
                // Return void for node navigation on void
                Ok(player.alloc_datum(Datum::Void))
            }
            "nodeName" | "nodeValue" => {
                // Return empty string for node properties on void
                Ok(player.alloc_datum(Datum::String("".to_owned())))
            }
            "attributes" => {
                // Return void for attributes on void
                Ok(player.alloc_datum(Datum::Void))
            }
            // Common properties that scripts may access on VOID results
            // (e.g., out-of-bounds 3D collection access). Director returns VOID silently.
            "name" | "type" | "number" | "member" | "count"
            | "transform" | "parent" | "shader" | "shaderList"
            | "visibility" | "visible" | "blend" | "resource"
            | "texture" | "textureList" | "renderFormat"
            | "position" | "rotation" | "scale"
            | "x" | "y" | "z" | "locH" | "locV" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get property '{}' on VOID - a variable or property that should contain an object is uninitialized",
                prop
            ))),
        }
    }
}
