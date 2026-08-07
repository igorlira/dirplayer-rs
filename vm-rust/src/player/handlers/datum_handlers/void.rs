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
            // Director silently no-ops ANY handler call on a VOID value and
            // returns VOID — it never raises. g349's `on particle` does
            // `g.particles[g.cparticle].spawn(Args)`; between levels
            // `g.particles` is cleared to VOID, so the subscript is VOID and
            // `.spawn(Args)` must be a no-op. Returning VOID here (rather than
            // maintaining a whitelist of handler names) mirrors Director and
            // subsumes the specific cases above.
            _ => {
                log::debug!("Calling handler '{}' on VOID → VOID (Director-lenient no-op)", handler_name);
                Ok(DatumRef::Void)
            },
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        _: &DatumRef,
        prop: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        match prop.as_lower_str() {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol(Symbol::from_str("void")))),
            "count" | "length" => Ok(player.alloc_datum(Datum::Int(0))),
            "x" | "y" | "z" | "magnitude" => Ok(player.alloc_datum(Datum::Float(0.0))),
            "position" | "rotation" | "scale" => Ok(player.alloc_datum(Datum::Vector([0.0, 0.0, 0.0]))),
            "string" => Ok(player.alloc_datum(Datum::String("".to_owned()))),
            "childnodes" => {
                Ok(player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::new(),
                    false,
                )))
            }
            "firstchild" | "lastchild" | "parentnode" | "nextsibling" | "previoussibling" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            "nodename" | "nodevalue" => {
                Ok(player.alloc_datum(Datum::String("".to_owned())))
            }
            "attributes" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            "name" | "type" | "number" | "member"
            | "transform" | "parent" | "shader" | "shaderlist"
            | "visibility" | "visible" | "blend" | "resource"
            | "texture" | "texturelist" | "renderformat"
            | "loch" | "locv" => {
                Ok(player.alloc_datum(Datum::Void))
            }
            "char" | "word" | "line" | "item" => {
                Ok(player.alloc_datum(Datum::String("".to_owned())))
            }
            "count" | "number" => {
                // Director tolerates .count and .number on void, returning 0
                Ok(player.alloc_datum(Datum::Int(0)))
            }
            // Director returns VOID for ANY property read on a VOID value —
            // it never raises. Scripts rely on this: e.g. g349's
            // "horizontal-only background scroll bhv" reads
            // `g.map.we_are_playing` every exitFrame, and once the level ends
            // and `g.map` is cleared to VOID the `> 0` test simply reads VOID
            // (false) and the sprite hides. The typed special-cases above
            // (count/x/string/...) stay because their callers expect a number
            // or string rather than VOID; every other property falls through
            // to VOID, matching Director instead of maintaining a whitelist of
            // custom property names (previously oAvatars/oInfoStand/etc.).
            _ => {
                log::debug!("Reading property '{}' on VOID → VOID (Director-lenient)", prop);
                Ok(player.alloc_datum(Datum::Void))
            }
        }
    }
}
