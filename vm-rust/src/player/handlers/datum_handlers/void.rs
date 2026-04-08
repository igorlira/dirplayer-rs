use std::collections::VecDeque;

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
};

pub struct VoidDatumHandlers {}

impl VoidDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "addAt" | "add" | "append" | "duplicate" | "getAt" | "getOne" | "getLast" | "getFirst"
            | "distanceTo" | "getNormalized" | "normalize" | "crossProduct" | "dotProduct"
            | "cross" | "dot" | "angleBetween" | "getWorldTransform" | "addToWorld" | "removeFromWorld" | "isInWorld" => {
                // Calling these on void should just return void
                Ok(DatumRef::Void)
            }
            "count" => {
                // count(VOID, #items) etc. should return 0
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(0)))
                })
            }
            "getProp" | "getaProp" | "getPropRef" => {
                // getProp(#char, 1, 6) etc. on VOID should return VOID
                Ok(DatumRef::Void)
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for void"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        _: &DatumRef,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        match prop {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("void".to_owned()))),
            "count" | "length" => Ok(player.alloc_datum(Datum::Int(0))),
            "x" | "y" | "z" | "magnitude" => Ok(player.alloc_datum(Datum::Float(0.0))),
            "position" | "rotation" | "scale" => Ok(player.alloc_datum(Datum::Vector([0.0, 0.0, 0.0]))),
            "string" => Ok(player.alloc_datum(Datum::String("".to_owned()))),
            // XML-related properties on Void should return empty/void values
            "childNodes" => {
                // Return empty list for childNodes on void
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::new(),
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
            // String slice operations on VOID should return empty string
            "char" | "word" | "line" | "item" => {
                Ok(player.alloc_datum(Datum::String("".to_owned())))
            }
            "count" | "number" => {
                // Director tolerates .count and .number on void, returning 0
                Ok(player.alloc_datum(Datum::Int(0)))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get property '{}' on VOID - a variable or property that should contain an object is uninitialized",
                prop
            ))),
        }
    }
}
