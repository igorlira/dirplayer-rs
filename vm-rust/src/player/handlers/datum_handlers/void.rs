use std::collections::VecDeque;

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, symbols::symbol::Symbol, DatumRef, DirPlayer, ScriptError},
};

pub struct VoidDatumHandlers {}

impl VoidDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // Lingo handlers are case-insensitive, and several upstream paths
        // (e.g. Coke Studios' SF gateway) hand back uninitialized AS objects
        // due to original-game bugs (e.g. setSendOn typo dropping sentOn).
        // Real Shockwave silently no-ops the resulting method calls instead
        // of throwing, so the script keeps running with empty / void output.
        match_ci!(handler_name.as_str(), {
            "addAt" | "add" | "append" | "duplicate" | "getAt" | "getOne" | "getLast" | "getFirst"
            | "distanceTo" | "getNormalized" | "normalize" | "crossProduct" | "dotProduct"
            | "cross" | "dot" | "angleBetween" | "getWorldTransform" | "addToWorld" | "removeFromWorld" | "isInWorld"
            // AS / Lingo Date methods — getters return void, setters are no-ops.
            | "getYear" | "getFullYear" | "getMonth" | "getDate" | "getDay"
            | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds"
            | "getTime" | "getTimezoneOffset"
            | "setYear" | "setFullYear" | "setMonth" | "setDate"
            | "setHours" | "setMinutes" | "setSeconds" | "setMilliseconds"
            | "setTime"
            | "toString" | "toLocaleString" | "toDateString" | "toTimeString"
            | "valueOf" => {
                // Calling these on void should just return void
                Ok(DatumRef::Void)
            },
            "count" => {
                // count(VOID, #items) etc. should return 0
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(0)))
                })
            },
            "getProp" | "getaProp" | "getPropRef" => {
                // getProp(#char, 1, 6) etc. on VOID should return VOID
                Ok(DatumRef::Void)
            },
            // CS Studio.receiveCdStop chains without voidp() guard
            "getAvatar" | "getItemByPossessionId" | "display" => Ok(DatumRef::Void),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for void"
            ))),
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        _: &DatumRef,
        prop: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        match prop.as_str() {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol(Symbol::from_str("void")))),
            "count" | "length" => Ok(player.alloc_datum(Datum::Int(0))),
            "x" | "y" | "z" | "magnitude" => Ok(player.alloc_datum(Datum::Float(0.0))),
            "position" | "rotation" | "scale" => Ok(player.alloc_datum(Datum::Vector([0.0, 0.0, 0.0]))),
            "string" => Ok(player.alloc_datum(Datum::String("".to_owned()))),
            "childNodes" => {
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::new(),
                    false,
                )))
            }
            "firstChild" | "lastChild" | "parentNode" | "nextSibling" | "previousSibling" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            "nodeName" | "nodeValue" => {
                Ok(player.alloc_datum(Datum::String("".to_owned())))
            }
            "attributes" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            "name" | "type" | "number" | "member"
            | "transform" | "parent" | "shader" | "shaderList"
            | "visibility" | "visible" | "blend" | "resource"
            | "texture" | "textureList" | "renderFormat"
            | "locH" | "locV" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            "char" | "word" | "line" | "item" => {
                Ok(player.alloc_datum(Datum::String("".to_owned())))
            }
            "oAvatars" | "oInfoStand" | "oSelectedItem" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get property '{}' on VOID - a variable or property that should contain an object is uninitialized",
                prop
            ))),
        }
    }
}
