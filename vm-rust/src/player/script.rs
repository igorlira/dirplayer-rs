use std::{cell::RefCell, rc::Rc};

use fxhash::FxHashMap;
use itertools::Itertools;
use log::warn;

use crate::director::{
    chunks::{handler::HandlerDef, script::ScriptChunk},
    enums::ScriptType,
    file::get_variable_multiplier,
    lingo::{datum::Datum, script::ScriptContext},
};

use super::ci_string::{CiStr, CiString};

use super::{
    allocator::{DatumAllocatorTrait, ScriptInstanceAllocatorTrait},
    bytecode::handler_manager::BytecodeHandlerContext,
    cast_lib::{player_cast_lib_set_prop, CastMemberRef},
    datum_formatting::{format_concrete_datum, format_datum},
    handlers::{
        datum_handlers::{
            bitmap::BitmapDatumHandlers, cast_member_ref::CastMemberRefHandlers,
            color::ColorDatumHandlers, int::IntDatumHandlers, list_handlers::ListDatumUtils,
            point::PointDatumHandlers, prop_list::PropListUtils, rect::RectDatumHandlers,
            sound_channel::SoundChannelDatumHandlers, string::StringDatumUtils,
            string_chunk::StringChunkHandlers, symbol::SymbolDatumHandlers,
            timeout::TimeoutDatumHandlers, void::VoidDatumHandlers,
            date::DateDatumHandlers, math::MathDatumHandlers,
            vector::VectorDatumHandlers, xml::XmlDatumHandlers,
            float::FloatDatumHandlers,
            cast_member::shockwave3d::Shockwave3dMemberHandlers,
        },
        types::TypeUtils,
    },
    reserve_player_mut, reserve_player_ref,
    scope::Scope,
    score::{sprite_get_prop, sprite_set_prop},
    script_ref::ScriptInstanceRef,
    stage::{get_stage_prop, set_stage_prop},
    DatumRef, DirPlayer, ScriptError,
};

#[derive(Clone)]
pub struct Script {
    pub member_ref: CastMemberRef,
    pub name: String,
    pub chunk: ScriptChunk,
    pub script_type: ScriptType,
    pub handlers: FxHashMap<CiString, Rc<HandlerDef>>,
    pub handler_names: Vec<String>,
    pub properties: RefCell<FxHashMap<CiString, DatumRef>>,
}

pub type ScriptInstanceId = u32;
pub type ScriptHandlerRefDef<'a> = (CastMemberRef, &'a Rc<HandlerDef>);

pub struct ScriptInstance {
    pub instance_id: ScriptInstanceId,
    pub script: CastMemberRef,
    pub ancestor: Option<ScriptInstanceRef>,
    pub properties: FxHashMap<CiString, DatumRef>,
    pub begin_sprite_called: bool,
}

impl ScriptInstance {
    pub fn new(
        instance_id: ScriptInstanceId,
        script_ref: CastMemberRef,
        script_def: &Script,
        lctx: &ScriptContext,
    ) -> ScriptInstance {
        let mut properties = FxHashMap::default();

        for prop_name_id in &script_def.chunk.property_name_ids {
            let prop_name = lctx
                .names
                .get(*prop_name_id as usize)
                .cloned()
                .unwrap_or_else(|| format!("prop_{}", prop_name_id));

            properties.insert(CiString::from(prop_name), DatumRef::Void);
        }

        ScriptInstance {
            instance_id,
            script: script_ref,
            ancestor: None,
            properties,
            begin_sprite_called: false,
        }
    }
}

impl Script {
    pub fn get_own_handler_ref_at(&self, index: usize) -> Option<ScriptHandlerRef> {
        return self
            .handler_names
            .get(index)
            .map(|x| (self.member_ref.clone(), x.clone()));
    }

    pub fn get_own_handler(&self, name: &str) -> Option<&Rc<HandlerDef>> {
        self.handlers.get(CiStr::new(name))
    }

    pub fn get_own_handler_by_name_id(&self, name_id: u16) -> Option<&Rc<HandlerDef>> {
        self.handlers
            .iter()
            .find(|x| x.1.name_id == name_id)
            .map(|x| x.1)
    }

    pub fn get_handler(&self, name: &str) -> Option<ScriptHandlerRefDef> {
        return self
            .get_own_handler(name)
            .map(|x| (self.member_ref.clone(), x));
    }

    pub fn get_own_handler_ref(&self, name: &str) -> Option<ScriptHandlerRef> {
        return self
            .get_own_handler(name)
            .map(|_| (self.member_ref.clone(), name.to_owned()));
    }
}

pub type ScriptHandlerRef = (CastMemberRef, String);

pub fn script_get_prop_opt(
    player: &mut DirPlayer,
    script_instance_ref: &ScriptInstanceRef,
    prop_name: &str,
) -> Option<DatumRef> {
    // Check virtual script handler first
    match super::virtual_scripts::VirtualScriptRegistry::try_get_instance_prop(player, script_instance_ref, prop_name) {
        Ok(Some(datum_ref)) => return Some(datum_ref),
        Ok(None) | Err(_) => {}
    }

    let script_instance = player.allocator.get_script_instance(&script_instance_ref);

    // Handle special "ancestor" property
    if prop_name == "ancestor" {
        let script_instance = player.allocator.get_script_instance(&script_instance_ref);
        if let Some(ancestor_id) = &script_instance.ancestor {
            return Some(player.alloc_datum(Datum::ScriptInstanceRef(ancestor_id.clone())));
        } else {
            return Some(DatumRef::Void);
        }
    } else if prop_name == "script" {
        let script_instance = player.allocator.get_script_instance(&script_instance_ref);
        return Some(player.alloc_datum(Datum::ScriptRef(script_instance.script.clone())));
    } else if prop_name == "ilk" {
        return Some(player.alloc_datum(Datum::Symbol("instance".to_string())));
    }

    // Try to find the property on the current instance first
    if let Some(prop) = script_instance.properties.get(CiStr::new(prop_name)) {
        return Some(prop.clone());
    }

    // Check ancestor for the property
    if script_instance.ancestor.is_some() {
        let ancestor_ref = script_instance.ancestor.as_ref().unwrap().clone();
        if let Some(result) = script_get_prop_opt(player, &ancestor_ref, prop_name) {
            return Some(result);
        }
    }

    // Fall back to built-in properties if not found in instance or ancestors
    if prop_name == "class" || prop_name == "script" {
        let script_instance = player.allocator.get_script_instance(&script_instance_ref);
        return Some(player.alloc_datum(Datum::ScriptRef(script_instance.script.clone())));
    }

    None
}

pub fn script_get_static_prop(
    player: &mut DirPlayer,
    script_ref: &CastMemberRef,
    prop_name: &str,
) -> Result<DatumRef, ScriptError> {
    let script_rc = player
        .movie
        .cast_manager
        .get_script_by_ref(&script_ref)
        .unwrap();
    let script = script_rc.as_ref();
    let properties = script.properties.borrow();
    if let Some(prop) = properties.get(CiStr::new(prop_name)) {
        Ok(prop.clone())
    } else {
        Err(ScriptError::new(format!(
            "Cannot get static property {} on script {}",
            prop_name, script.name
        )))
    }
}

pub fn script_set_static_prop(
    player: &mut DirPlayer,
    script_ref: &CastMemberRef,
    prop_name: &str,
    value_ref: &DatumRef,
    required: bool,
) -> Result<(), ScriptError> {
    let script_rc = player
        .movie
        .cast_manager
        .get_script_by_ref(&script_ref)
        .unwrap();
    let script = script_rc.as_ref();
    let mut properties = script.properties.borrow_mut();

    if required && !properties.contains_key(CiStr::new(prop_name)) {
        return Err(ScriptError::new(format!(
            "Cannot set static property {} on script {}",
            prop_name, script.name
        )));
    } else {
        properties.insert(CiString::from(prop_name.clone()), value_ref.clone());
        Ok(())
    }
}

pub fn script_get_prop(
    player: &mut DirPlayer,
    script_instance_ref: &ScriptInstanceRef,
    prop_name: &str,
) -> Result<DatumRef, ScriptError> {
    if let Some(prop) = script_get_prop_opt(player, script_instance_ref, prop_name) {
        Ok(prop)
    } else if prop_name.eq_ignore_ascii_case("count") {
        // In Director, .count on a non-list object returns 1
        Ok(player.alloc_datum(Datum::Int(1)))
    } else if prop_name.eq_ignore_ascii_case("spriteNum") {
        // spriteNum is a built-in property for behaviors — if not explicitly set,
        // look up which sprite channel this instance belongs to.
        // Resolve sprite ownership from the live/cached scriptInstanceList.
        let stage_channel_snapshots: Vec<(i16, i32, Vec<ScriptInstanceRef>)> = player
            .movie
            .score
            .channels
            .iter()
            .map(|channel| {
                (
                    channel.sprite.number as i16,
                    channel.sprite.number as i32,
                    channel.sprite.script_instance_list.clone(),
                )
            })
            .collect();
        for (sprite_id, channel_number, fallback) in stage_channel_snapshots {
            let instance_ids = player.get_sprite_script_instance_ids(
                sprite_id,
                fallback.as_slice(),
            );
            if instance_ids.iter().any(|si| si.id() == script_instance_ref.id()) {
                let datum_ref = player.alloc_datum(Datum::Int(channel_number));
                return Ok(datum_ref);
            }
        }
        // Also check the cache — behaviors may be in cache but not in script_instance_list Vec
        Ok(player.alloc_datum(Datum::Int(0)))
    } else {
        // Director silently returns VOID when reading a property that doesn't
        // exist on an instance (or anywhere in its ancestor chain) — many
        // Shockwave movies rely on this, e.g. `repeat with x in me.oItem.someList`
        // where `someList` is only populated in some code paths. Raising a
        // ScriptError here breaks those movies even though they ran fine in
        // original Director. Log once per miss so real typos are still noticeable
        // in the console.
        let script_instance = player.allocator.get_script_instance(&script_instance_ref);
        let valid_props = script_instance.properties.keys().collect_vec();
        warn!(
            "script_get_prop: undefined property '{}' on {} → returning VOID. Valid properties: {}",
            prop_name,
            format_concrete_datum(
                &Datum::ScriptInstanceRef(script_instance_ref.clone()),
                player
            ),
            valid_props.iter().join(", ")
        );
        Ok(DatumRef::Void)
    }
}

pub fn script_set_prop(
    player: &mut DirPlayer,
    script_instance_ref: &ScriptInstanceRef,
    prop_name: &str,
    value_ref: &DatumRef,
    required: bool,
) -> Result<(), ScriptError> {
    // Check virtual script handler first
    match super::virtual_scripts::VirtualScriptRegistry::try_set_instance_prop(player, script_instance_ref, prop_name, value_ref) {
        Ok(Some(())) => return Ok(()),
        Err(e) => return Err(e),
        Ok(None) => {}
    }

    // Try to set the property on the current instance
    let result = {
        if prop_name == "ancestor" {
            let ancestor_id = player
                .allocator
                .get_datum(value_ref)
                .to_script_instance_ref()?
                .clone();
            let script_instance = player
                .allocator
                .get_script_instance_mut(&script_instance_ref);
            script_instance.ancestor = Some(ancestor_id);
            Ok(())
        } else {
            let script_instance = player
                .allocator
                .get_script_instance_mut(&script_instance_ref);
            if let Some(prop) = script_instance.properties.get_mut(CiStr::new(prop_name)) {
                *prop = value_ref.clone();
                Ok(())
            } else {
                Err(ScriptError::new(format!(
                    "Cannot set property {} found on script instance {}",
                    prop_name,
                    format_concrete_datum(
                        &Datum::ScriptInstanceRef(script_instance_ref.clone()),
                        player
                    )
                )))
            }
        }
    };
    // If the property was not found on the current instance, try to set it on the ancestor
    let result = match result {
        Ok(_) => Ok(()),
        Err(_) => {
            let script_instance = player.allocator.get_script_instance(&script_instance_ref);
            if let Some(ancestor_id) = &script_instance.ancestor {
                script_set_prop(player, &ancestor_id.clone(), prop_name, value_ref, true)
            } else {
                Err(ScriptError::new("No ancestor found".to_string()))
            }
        }
    };
    let result = match result {
        Ok(_) => Ok(()),
        Err(err) => {
            if required {
                Err(err)
            } else {
                let script_instance = player
                    .allocator
                    .get_script_instance_mut(&script_instance_ref);
                script_instance
                    .properties
                    .insert(CiString::from(prop_name.to_owned()), value_ref.clone());
                Ok(())
            }
        }
    };

    result.map_err(|err| {
        ScriptError::new(format!(
            "Error setting property {} on script instance {}: {}",
            prop_name,
            format_concrete_datum(
                &Datum::ScriptInstanceRef(script_instance_ref.clone()),
                player
            ),
            err.message
        ))
    })
}

pub fn get_current_scope<'a>(
    player: &'a DirPlayer,
    ctx: &'a BytecodeHandlerContext,
) -> Option<&'a Scope> {
    player.scopes.get(ctx.scope_ref)
}

pub fn get_current_script<'a>(
    player: &'a DirPlayer,
    ctx: &'a BytecodeHandlerContext,
) -> Option<&'a Script> {
    return Some(unsafe { &*ctx.script_ptr });
}

pub fn get_current_handler_def<'a>(
    _: &'a DirPlayer,
    ctx: &'a BytecodeHandlerContext,
) -> &'a HandlerDef {
    return unsafe { &*ctx.handler_def_ptr };
}

pub fn get_current_variable_multiplier(player: &DirPlayer, ctx: &BytecodeHandlerContext) -> u32 {
    let script = get_current_script(player, ctx);
    if let Some(script) = script {
        let cast = player
            .movie
            .cast_manager
            .get_cast(script.member_ref.cast_lib as u32)
            .unwrap();
        return get_variable_multiplier(cast.capital_x, cast.dir_version);
    }
    panic!("No current script found");
}

pub fn get_lctx<'a>(
    player: &'a DirPlayer,
    ctx: &'a BytecodeHandlerContext,
) -> Option<&'a ScriptContext> {
    let script = get_current_script(player, &ctx);
    if let Some(script) = script {
        return get_lctx_for_script(player, script);
    }
    None
}

pub fn get_lctx_for_script<'a>(
    player: &'a DirPlayer,
    script: &'a Script,
) -> Option<&'a ScriptContext> {
    let cast = player
        .movie
        .cast_manager
        .get_cast(script.member_ref.cast_lib as u32)
        .unwrap();
    return cast.lctx.as_ref();
}

pub fn get_name<'a>(
    player: &'a DirPlayer,
    ctx: &'a BytecodeHandlerContext,
    name_id: u16,
) -> Option<&'a String> {
    let lctx = get_lctx(player, ctx);
    if let Some(lctx) = lctx {
        return Some(&lctx.names[name_id as usize]);
    }
    None
}

pub async fn player_set_obj_prop(
    obj_ref: &DatumRef,
    prop_name: &str,
    value_ref: &DatumRef,
) -> Result<(), ScriptError> {
    let (obj_clone, value_clone) = reserve_player_ref(|player| {
        let obj = player.get_datum(obj_ref).to_owned();
        let value = player.get_datum(value_ref).to_owned();
        (obj, value)
    });
    match obj_clone {
        Datum::CastLib(cast_lib) => {
            player_cast_lib_set_prop(cast_lib, prop_name, value_clone).await?;
            Ok(())
        }
        Datum::ScriptInstanceRef(script_instance_ref) => reserve_player_mut(|player| {
            script_set_prop(player, &script_instance_ref, &prop_name, value_ref, false)
        }),
        Datum::SpriteRef(sprite_id) => {
            sprite_set_prop(sprite_id, prop_name, value_clone)
        }
        Datum::CastMember(member_ref) => {
            // TODO should we really pass a clone of the value here?
            CastMemberRefHandlers::set_prop(&member_ref, prop_name, value_clone)
        }
        Datum::Stage => reserve_player_mut(|player| set_stage_prop(player, &prop_name, value_ref)),
        Datum::BitmapRef(bitmap_ref) => reserve_player_mut(|player| {
            BitmapDatumHandlers::set_bitmap_ref_prop(player, bitmap_ref, prop_name, value_ref)
        }),
        Datum::Point(..) => reserve_player_mut(|player| {
            PointDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::TimeoutRef(_) | Datum::TimeoutInstance { .. } | Datum::TimeoutFactory 
            => reserve_player_mut(|player| {
            TimeoutDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::PropList(..) => reserve_player_mut(|player| {
            let key_ref = player.alloc_datum(Datum::Symbol(prop_name.to_owned()));
            PropListUtils::set_prop(
                obj_ref,
                &key_ref,
                value_ref,
                player,
                false,
                prop_name,
            )
        }),
        Datum::Rect(..) => reserve_player_mut(|player| {
            RectDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::StringChunk(..) => reserve_player_mut(|player| {
            StringChunkHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::ColorRef(..) => reserve_player_mut(|player| {
            ColorDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::PlayerRef => {
            reserve_player_mut(|player| player.set_player_prop(prop_name, value_ref))
        }
        Datum::MouseRef => {
            reserve_player_mut(|player| player.set_mouse_prop(prop_name, value_ref))
        }
        Datum::MovieRef => reserve_player_mut(|player| {
            player.set_movie_prop(prop_name, player.get_datum(value_ref).clone())
        }),
        Datum::ScriptRef(script_ref) => reserve_player_mut(|player| {
            script_set_static_prop(player, &script_ref, prop_name, value_ref, false)
        }),
        Datum::XmlRef(_) => reserve_player_mut(|player| {
            XmlDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::DateRef(_) => reserve_player_mut(|player| {
            DateDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::MathRef(_) => reserve_player_mut(|player| {
            MathDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::Vector(..) => reserve_player_mut(|player| {
            VectorDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::SoundChannel(_) => reserve_player_mut(|player| {
            SoundChannelDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::FlashObjectRef(_) => {
            let value_datum = reserve_player_ref(|player| {
                player.get_datum(value_ref).clone()
            });
            crate::player::handlers::datum_handlers::flash_object::FlashObjectDatumHandlers::set_prop(obj_ref, &prop_name, &value_datum)
        }
        Datum::Shockwave3dObjectRef(_) => {
            let value_datum = reserve_player_ref(|player| {
                player.get_datum(value_ref).clone()
            });
            crate::player::handlers::datum_handlers::shockwave3d_object::Shockwave3dObjectDatumHandlers::set_prop(obj_ref, &prop_name, &value_datum)
        }
        Datum::Transform3d(_) => reserve_player_mut(|player| {
            crate::player::handlers::datum_handlers::transform3d::Transform3dDatumHandlers::set_prop(player, obj_ref, &prop_name, value_ref)
        }),
        Datum::HavokObjectRef(_) => {
            crate::player::handlers::datum_handlers::havok_object::HavokObjectDatumHandlers::set_prop(obj_ref, &prop_name, value_ref.clone())
        }
        Datum::Void | Datum::Null => {
            // In Director, setting a property on void/nothing is a no-op (silently ignored)
            // This commonly happens when scripts reference sprites/objects that have been erased
            // or during cleanup when handlers are still being called on partially-destroyed objects
            //
            // Note: The game may have code paths that read uninitialized properties (pLocX, pLocY, etc.)
            // which return Void, and then try to do operations like `obj.loc = obj.loc + point(x,y)`
            // where obj is Void. This is normal Director behavior - it just silently does nothing.
            Ok(())
        }
        _ => reserve_player_ref(|player| {
            Err(ScriptError::new(
                format!(
                    "set_obj_prop was passed an invalid datum: {}",
                    format_datum(obj_ref, &player)
                )
                .to_string(),
            ))
        }),
    }
}

pub fn get_obj_prop(
    player: &mut DirPlayer,
    obj_ref: &DatumRef,
    prop_name: &str,
) -> Result<DatumRef, ScriptError> {
    let obj_clone = player.get_datum(obj_ref).clone();

    // Universal type-check properties (work on any datum type)
    match prop_name {
        "integerp" => {
            let is_int = matches!(obj_clone, Datum::Int(_));
            return Ok(player.alloc_datum(Datum::Int(if is_int { 1 } else { 0 })));
        }
        "floatp" => {
            let is_float = matches!(obj_clone, Datum::Float(_));
            return Ok(player.alloc_datum(Datum::Int(if is_float { 1 } else { 0 })));
        }
        "stringp" => {
            let is_string = matches!(obj_clone, Datum::String(_) | Datum::StringChunk(..));
            return Ok(player.alloc_datum(Datum::Int(if is_string { 1 } else { 0 })));
        }
        "symbolp" => {
            let is_symbol = matches!(obj_clone, Datum::Symbol(_));
            return Ok(player.alloc_datum(Datum::Int(if is_symbol { 1 } else { 0 })));
        }
        "listp" => {
            let is_list = matches!(obj_clone, Datum::List(..) | Datum::PropList(..));
            return Ok(player.alloc_datum(Datum::Int(if is_list { 1 } else { 0 })));
        }
        "objectp" => {
            let is_obj = matches!(obj_clone, Datum::ScriptInstanceRef(_));
            return Ok(player.alloc_datum(Datum::Int(if is_obj { 1 } else { 0 })));
        }
        "voidp" => {
            let is_void = matches!(obj_clone, Datum::Void);
            return Ok(player.alloc_datum(Datum::Int(if is_void { 1 } else { 0 })));
        }
        _ => {}
    }

    match obj_clone {
        Datum::CastLib(cast_lib) => {
            let cast_lib = player.movie.cast_manager.get_cast(cast_lib as u32)?;
            Ok(player.alloc_datum(cast_lib.get_prop(prop_name)?))
        }
        Datum::CastMember(member_ref) => {
            let result = CastMemberRefHandlers::get_prop(player, &member_ref, prop_name)?;
            Ok(player.alloc_datum(result))
        }
        Datum::ScriptInstanceRef(script_instance_id) => {
            script_get_prop(player, &script_instance_id, &prop_name)
        }
        Datum::ScriptRef(script_ref) => script_get_static_prop(player, &script_ref, prop_name),
        Datum::PropList(prop_list, ..) => {
            PropListUtils::get_prop_or_built_in(player, &prop_list, &prop_name)
        }
        Datum::List(_, list, _) => Ok(player.alloc_datum(ListDatumUtils::get_prop(
            &list,
            &prop_name,
            &player.allocator,
        )?)),
        Datum::Stage => {
            let result = get_stage_prop(player, &prop_name)?;
            Ok(player.alloc_datum(result))
        }
        Datum::Rect(..) => {
            Ok(player.alloc_datum(RectDatumHandlers::get_prop(player, obj_ref, &prop_name)?))
        }
        Datum::Point(..) => {
            Ok(player.alloc_datum(PointDatumHandlers::get_prop(player, obj_ref, &prop_name)?))
        }
        Datum::SpriteRef(sprite_id) => {
            let result = sprite_get_prop(player, sprite_id, prop_name)?;
            Ok(player.last_sprite_prop_ref.take()
                .unwrap_or_else(|| player.alloc_datum(result)))
        }
        Datum::BitmapRef(_) => BitmapDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::String(s) => {
            Ok(player.alloc_datum(StringDatumUtils::get_built_in_prop(&s, &prop_name)?))
        }
        Datum::StringChunk(ref source, ref chunk_expr, ref _str_val) => {
            match prop_name {
                "count" => {
                    // Chunk count is `end - start + 1` over the chunk_expr.
                    // For the "whole collection" form produced by
                    // `string.line` etc. that becomes the line/word/item
                    // total. Director's `text.line.count`, `text.word.count`
                    // etc. read off this.
                    let n = (chunk_expr.end - chunk_expr.start + 1).max(0);
                    Ok(player.alloc_datum(Datum::Int(n)))
                }
                "ref" => {
                    // .ref returns the chunk reference itself (a StringChunk datum)
                    Ok(obj_ref.clone())
                }
                "range" => {
                    // .range returns point(startCharPos, endCharPos) — 1-based char positions in the source
                    use crate::player::handlers::datum_handlers::string_chunk::StringChunkUtils;
                    use crate::director::lingo::datum::StringChunkType;

                    let source_str = match source {
                        crate::director::lingo::datum::StringChunkSource::Datum(d) => player.get_datum(d).string_value()?,
                        crate::director::lingo::datum::StringChunkSource::Member(m) => {
                            let member = player.movie.cast_manager.find_member_by_ref(m)
                                .ok_or_else(|| ScriptError::new("Member not found for string chunk range".to_string()))?;
                            if let Some(field) = member.member_type.as_field() {
                                field.text.clone()
                            } else if let Some(text) = member.member_type.as_text() {
                                text.text.clone()
                            } else {
                                return Err(ScriptError::new("Member is not a text/field type".to_string()));
                            }
                        }
                    };

                    let chunk_list = StringChunkUtils::resolve_chunk_list(
                        &source_str,
                        chunk_expr.chunk_type.clone(),
                        chunk_expr.item_delimiter,
                    )?;

                    let (start_idx, end_idx_exclusive) = StringChunkUtils::vm_range_to_host(
                        (chunk_expr.start, chunk_expr.end),
                        chunk_list.len(),
                    );
                    // vm_range_to_host returns exclusive end; convert to inclusive for the loop below
                    let end_idx = if end_idx_exclusive > 0 { end_idx_exclusive - 1 } else { 0 };

                    // Calculate character positions based on chunk type
                    let (char_start, char_end) = match chunk_expr.chunk_type {
                        StringChunkType::Char => {
                            (start_idx as i32 + 1, end_idx_exclusive as i32)
                        }
                        _ => {
                            // For line/word/item, find character positions by summing chunk lengths + delimiters
                            let mut pos = 0usize;
                            let mut result_start = 0usize;
                            let delimiter_len = match chunk_expr.chunk_type {
                                StringChunkType::Line => {
                                    // Detect \r\n vs \r vs \n
                                    if source_str.contains("\r\n") { 2 } else { 1 }
                                }
                                StringChunkType::Item => 1, // delimiter char
                                StringChunkType::Word => 1, // whitespace
                                _ => 1,
                            };
                            for (i, chunk) in chunk_list.iter().enumerate() {
                                if i == start_idx {
                                    result_start = pos;
                                }
                                pos += chunk.chars().count();
                                if i == end_idx {
                                    break;
                                }
                                if i + 1 < chunk_list.len() {
                                    pos += delimiter_len;
                                }
                            }
                            let result_end = pos;
                            (result_start as i32 + 1, result_end as i32)
                        }
                    };

                    Ok(player.alloc_datum(Datum::Point([char_start as f64, char_end as f64], 0)))
                }
                "charSpacing" => {
                    // Read charSpacing from the source member's styled spans, walking the source chain
                    if let Datum::StringChunk(ref source, _, _) = obj_clone {
                        let mut current_source = source.clone();
                        loop {
                            match current_source {
                                crate::director::lingo::datum::StringChunkSource::Member(ref member_ref) => {
                                    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                                        if let Some(text) = member.member_type.as_text() {
                                            return Ok(player.alloc_datum(Datum::Int(text.char_spacing)));
                                        }
                                    }
                                    break;
                                }
                                crate::director::lingo::datum::StringChunkSource::Datum(ref d) => {
                                    let inner = player.get_datum(d).clone();
                                    if let Datum::StringChunk(inner_source, _, _) = inner {
                                        current_source = inner_source;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Int(0)))
                }
                _ if matches!(prop_name.to_ascii_lowercase().as_str(),
                    "fixedlinespace" | "topspacing" | "bottomspacing"
                    | "font" | "fontsize" | "fontstyle"
                    | "color" | "bgcolor" | "alignment") => {
                    // Per-chunk properties — walk to the source member and
                    // resolve the chunk's char range, then look up the
                    // active par_info / styled span. Director exposes
                    // `member.line[N].fixedLineSpace` etc.; without this
                    // branch the StringChunk would fall through to the
                    // string built-in handler and return Void / 0.
                    use crate::player::handlers::datum_handlers::string_chunk::StringChunkHandlers;
                    let resolved = StringChunkHandlers::walk_chunk_to_member_range(player, obj_ref);
                    let Some((member_ref, char_start, _char_end)) = resolved else {
                        return Ok(player.alloc_datum(StringDatumUtils::get_built_in_prop(
                            &obj_clone.string_value()?,
                            &prop_name,
                        )?));
                    };
                    let Some(member) = player.movie.cast_manager.find_member_by_ref(&member_ref) else {
                        return Ok(player.alloc_datum(Datum::Void));
                    };
                    // Field path: lacks per-paragraph data; fall back to
                    // member-level values for the chunk too.
                    if let Some(_field) = member.member_type.as_field() {
                        return Ok(player.alloc_datum(StringDatumUtils::get_built_in_prop(
                            &obj_clone.string_value()?,
                            &prop_name,
                        )?));
                    }
                    let Some(text) = member.member_type.as_text() else {
                        return Ok(player.alloc_datum(Datum::Void));
                    };
                    // Look up the par_info active at the chunk's start
                    // position — line N's first character. par_run.position
                    // values reference text-character offsets, same as
                    // chunk char_start, so a direct walk works.
                    let mut active_idx: Option<u16> = None;
                    let pos = char_start as u32;
                    for run in &text.par_runs {
                        if run.position <= pos {
                            active_idx = Some(run.par_info_index);
                        } else {
                            break;
                        }
                    }
                    let par_info = active_idx
                        .and_then(|idx| text.par_infos.get(idx as usize))
                        .cloned();

                    // Locate the styled span containing the chunk's start
                    // char and snapshot its style fields — `player` is
                    // mutably borrowed below for `alloc_datum` so we can't
                    // hold a reference into the cast member at the same
                    // time.
                    let mut cum = 0usize;
                    let active_span = text.html_styled_spans.iter().find(|span| {
                        let span_chars = span.text.chars().count();
                        let end = cum + span_chars;
                        let hit = pos as usize >= cum && (pos as usize) < end.max(cum + 1);
                        cum = end;
                        hit
                    }).or_else(|| text.html_styled_spans.first());
                    let span_font_face: Option<String> = active_span
                        .and_then(|s| s.style.font_face.clone());
                    let span_font_size: Option<i32> = active_span
                        .and_then(|s| s.style.font_size);
                    let span_bold = active_span.map(|s| s.style.bold).unwrap_or(false);
                    let span_italic = active_span.map(|s| s.style.italic).unwrap_or(false);
                    let span_underline = active_span.map(|s| s.style.underline).unwrap_or(false);
                    let span_color: Option<u32> = active_span.and_then(|s| s.style.color);
                    let member_color = member.color.clone();
                    let member_bg_color = member.bg_color.clone();
                    let member_text_font = text.font.clone();
                    let member_text_font_size = text.font_size as i32;
                    let member_text_alignment = text.alignment.clone();
                    let par_infos_snapshot: Vec<i32> = text.par_infos
                        .iter()
                        .map(|pi| pi.line_spacing)
                        .collect();

                    // Drop the read borrow on `member` / `text` before
                    // touching `player.alloc_datum` (which needs `&mut player`).
                    drop(member);

                    match_ci!(prop_name, {
                        "fixedLineSpace" => {
                            // Per-line line_spacing with the same "0 means
                            // inherit / use document default" fallback the
                            // renderer applies — Director's getter returns
                            // the MAX non-zero line_spacing across the
                            // member's par_infos when this line's own value
                            // is 0. Junkbot v1 level.num: par_infos =
                            // [0, 16, 21, 0]; line[1] resolves to 0 →
                            // fallback → 21 (matches Director).
                            let val = par_info
                                .as_ref()
                                .map(|pi| pi.line_spacing)
                                .filter(|&s| s != 0)
                                .or_else(|| par_infos_snapshot.iter()
                                    .copied()
                                    .filter(|&s| s != 0)
                                    .max())
                                .unwrap_or(0);
                            Ok(player.alloc_datum(Datum::Int(val)))
                        },
                        "topSpacing" => {
                            let val = par_info.as_ref().map(|pi| pi.top_spacing).unwrap_or(0);
                            Ok(player.alloc_datum(Datum::Int(val)))
                        },
                        "bottomSpacing" => {
                            let val = par_info.as_ref().map(|pi| pi.bottom_spacing).unwrap_or(0);
                            Ok(player.alloc_datum(Datum::Int(val)))
                        },
                        "alignment" => {
                            let val = par_info.as_ref().map(|pi| pi.justification).unwrap_or(0);
                            let s = match val {
                                1 => "center".to_string(),
                                2 => "right".to_string(),
                                3 => "justify".to_string(),
                                _ => member_text_alignment,
                            };
                            Ok(player.alloc_datum(Datum::String(s)))
                        },
                        "font" => {
                            let val = span_font_face.unwrap_or(member_text_font);
                            Ok(player.alloc_datum(Datum::String(val)))
                        },
                        "fontSize" => {
                            let val = span_font_size.unwrap_or(member_text_font_size);
                            Ok(player.alloc_datum(Datum::Int(val)))
                        },
                        "fontStyle" => {
                            let mut item_refs = std::collections::VecDeque::new();
                            if span_bold {
                                item_refs.push_back(player.alloc_datum(Datum::Symbol("bold".to_string())));
                            }
                            if span_italic {
                                item_refs.push_back(player.alloc_datum(Datum::Symbol("italic".to_string())));
                            }
                            if span_underline {
                                item_refs.push_back(player.alloc_datum(Datum::Symbol("underline".to_string())));
                            }
                            Ok(player.alloc_datum(Datum::List(crate::director::lingo::datum::DatumType::List, item_refs, false)))
                        },
                        "color" => {
                            let color_ref = if let Some(c) = span_color {
                                crate::player::sprite::ColorRef::Rgb(
                                    ((c >> 16) & 0xFF) as u8,
                                    ((c >> 8) & 0xFF) as u8,
                                    (c & 0xFF) as u8,
                                )
                            } else {
                                member_color
                            };
                            Ok(player.alloc_datum(Datum::ColorRef(color_ref)))
                        },
                        "bgColor" => {
                            Ok(player.alloc_datum(Datum::ColorRef(member_bg_color)))
                        },
                        _ => Ok(player.alloc_datum(StringDatumUtils::get_built_in_prop(
                            &obj_clone.string_value()?,
                            &prop_name,
                        )?)),
                    })
                }
                _ => Ok(player.alloc_datum(StringDatumUtils::get_built_in_prop(
                    &obj_clone.string_value()?,
                    &prop_name,
                )?)),
            }
        }
        Datum::TimeoutRef(_) | Datum::TimeoutInstance { .. } | Datum::TimeoutFactory
             => Ok(TimeoutDatumHandlers::get_prop(player, obj_ref, &prop_name)?),
        Datum::Symbol(_) => SymbolDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::Void => VoidDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::Int(_) => IntDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::Float(_) => FloatDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::ColorRef(_) => ColorDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::PlayerRef => player.get_player_prop(prop_name),
        Datum::MouseRef => player.get_mouse_prop(&prop_name),
        Datum::XmlRef(_) => XmlDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::DateRef(_) => DateDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::MathRef(_) => MathDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::Vector(_) => {
            Ok(player.alloc_datum(VectorDatumHandlers::get_prop(player, obj_ref, prop_name)?))
        }
        Datum::SoundChannel(_) => Ok(player.alloc_datum(SoundChannelDatumHandlers::get_prop(
            player, obj_ref, &prop_name,
        )?)),
        Datum::MovieRef => player.get_movie_prop(prop_name),
        Datum::FlashObjectRef(_) => {
            crate::player::handlers::datum_handlers::flash_object::FlashObjectDatumHandlers::get_prop(obj_ref, &prop_name)
        }
        Datum::Shockwave3dObjectRef(_) => {
            crate::player::handlers::datum_handlers::shockwave3d_object::Shockwave3dObjectDatumHandlers::get_prop(obj_ref, &prop_name)
        }
        Datum::Transform3d(_) => {
            let result = crate::player::handlers::datum_handlers::transform3d::Transform3dDatumHandlers::get_prop(player, obj_ref, &prop_name)?;
            Ok(player.alloc_datum(result))
        }
        Datum::HavokObjectRef(_) => {
            crate::player::handlers::datum_handlers::havok_object::HavokObjectDatumHandlers::get_prop(obj_ref, &prop_name)
        }
        _ => {
            if prop_name == "ilk" {
                let ilk = TypeUtils::get_datum_ilk(&obj_clone)?;
                Ok(player.alloc_datum(Datum::Symbol(ilk.to_string())))
            } else {
                Err(ScriptError::new(
                    format!(
                        "get_obj_prop(\"{}\") was passed an invalid datum: {}",
                        prop_name,
                        format_datum(obj_ref, &player)
                    )
                    .to_string(),
                ))
            }
        }
    }
}
