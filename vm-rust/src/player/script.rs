use std::{cell::RefCell, rc::Rc};

use fxhash::FxHashMap;
use itertools::Itertools;

use crate::director::{
    chunks::{handler::HandlerDef, script::ScriptChunk},
    enums::ScriptType,
    file::get_variable_multiplier,
    lingo::{datum::Datum, script::ScriptContext},
};

use super::{
    allocator::{DatumAllocatorTrait, ScriptInstanceAllocatorTrait},
    bytecode::handler_manager::BytecodeHandlerContext,
    cast_lib::{player_cast_lib_set_prop, CastMemberRef},
    datum_formatting::{format_concrete_datum, format_datum},
    handlers::datum_handlers::date::DateDatumHandlers,
    handlers::datum_handlers::math::MathDatumHandlers,
    handlers::datum_handlers::vector::VectorDatumHandlers,
    handlers::datum_handlers::xml::XmlDatumHandlers,
    handlers::{
        datum_handlers::{
            bitmap::BitmapDatumHandlers, cast_member_ref::CastMemberRefHandlers,
            color::ColorDatumHandlers, int::IntDatumHandlers, list_handlers::ListDatumUtils,
            point::PointDatumHandlers, prop_list::PropListUtils, rect::RectDatumHandlers,
            sound_channel::SoundChannelDatumHandlers, string::StringDatumUtils,
            string_chunk::StringChunkHandlers, symbol::SymbolDatumHandlers,
            timeout::TimeoutDatumHandlers, void::VoidDatumHandlers,
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
    pub handlers: FxHashMap<String, Rc<HandlerDef>>,
    pub handler_names: Vec<String>,
    pub properties: RefCell<FxHashMap<String, DatumRef>>,
}

pub type ScriptInstanceId = u32;
pub type ScriptHandlerRefDef<'a> = (CastMemberRef, &'a Rc<HandlerDef>);

pub struct ScriptInstance {
    pub instance_id: ScriptInstanceId,
    pub script: CastMemberRef,
    pub ancestor: Option<ScriptInstanceRef>,
    pub properties: FxHashMap<String, DatumRef>,
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

            properties.insert(prop_name, DatumRef::Void);
        }

        ScriptInstance {
            instance_id,
            script: script_ref,
            ancestor: None,
            properties,
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

    pub fn get_own_handler(&self, name: &String) -> Option<&Rc<HandlerDef>> {
        self.handlers.get(&name.to_lowercase())
    }

    pub fn get_own_handler_by_name_id(&self, name_id: u16) -> Option<&Rc<HandlerDef>> {
        self.handlers
            .iter()
            .find(|x| x.1.name_id == name_id)
            .map(|x| x.1)
    }

    pub fn get_handler(&self, name: &String) -> Option<ScriptHandlerRefDef> {
        return self
            .get_own_handler(name)
            .map(|x| (self.member_ref.clone(), x));
    }

    pub fn get_own_handler_ref(&self, name: &String) -> Option<ScriptHandlerRef> {
        return self
            .get_own_handler(name)
            .map(|_| (self.member_ref.clone(), name.clone()));
    }
}

pub type ScriptHandlerRef = (CastMemberRef, String);

pub fn script_get_prop_opt(
    player: &mut DirPlayer,
    script_instance_ref: &ScriptInstanceRef,
    prop_name: &String,
) -> Option<DatumRef> {
    let script_instance = player.allocator.get_script_instance(&script_instance_ref);
    if prop_name == "ancestor" {
        let script_instance = player.allocator.get_script_instance(&script_instance_ref);
        if let Some(ancestor_id) = &script_instance.ancestor {
            Some(player.alloc_datum(Datum::ScriptInstanceRef(ancestor_id.clone())))
        } else {
            Some(DatumRef::Void)
        }
    } else {
        // Try to find the property on the current instance
        let prop_value = script_instance.properties.get(prop_name).map(|x| x.clone());
        if let Some(prop) = prop_value {
            Some(prop)
        } else if script_instance.ancestor.is_some() {
            let ancestor_ref = script_instance.ancestor.as_ref().unwrap().clone();
            script_get_prop_opt(player, &ancestor_ref, prop_name)
        } else {
            None
        }
    }
}

pub fn script_get_static_prop(
    player: &mut DirPlayer,
    script_ref: &CastMemberRef,
    prop_name: &String,
) -> Result<DatumRef, ScriptError> {
    let script_rc = player
        .movie
        .cast_manager
        .get_script_by_ref(&script_ref)
        .unwrap();
    let script = script_rc.as_ref();
    let properties = script.properties.borrow();
    if let Some(prop) = properties.get(prop_name) {
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
    prop_name: &String,
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

    if required && !properties.contains_key(prop_name) {
        return Err(ScriptError::new(format!(
            "Cannot set static property {} on script {}",
            prop_name, script.name
        )));
    } else {
        properties.insert(prop_name.clone(), value_ref.clone());
        Ok(())
    }
}

pub fn script_get_prop(
    player: &mut DirPlayer,
    script_instance_ref: &ScriptInstanceRef,
    prop_name: &String,
) -> Result<DatumRef, ScriptError> {
    if let Some(prop) = script_get_prop_opt(player, script_instance_ref, prop_name) {
        Ok(prop)
    } else {
        let script_instance = player.allocator.get_script_instance(&script_instance_ref);
        let valid_props = script_instance.properties.keys().collect_vec();
        Err(ScriptError::new(format!(
            "Cannot get property {} found on script instance {}. Valid properties are: {}",
            prop_name,
            format_concrete_datum(
                &Datum::ScriptInstanceRef(script_instance_ref.clone()),
                player
            ),
            valid_props.iter().join(", ")
        )))
    }
}

pub fn script_set_prop(
    player: &mut DirPlayer,
    script_instance_ref: &ScriptInstanceRef,
    prop_name: &String,
    value_ref: &DatumRef,
    required: bool,
) -> Result<(), ScriptError> {
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
            if let Some(prop) = script_instance.properties.get_mut(prop_name) {
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
                    .insert(prop_name.to_owned(), value_ref.clone());
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
    prop_name: &String,
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
        Datum::SpriteRef(sprite_id) => sprite_set_prop(sprite_id, prop_name, value_clone),
        Datum::CastMember(member_ref) => {
            // TODO should we really pass a clone of the value here?
            CastMemberRefHandlers::set_prop(&member_ref, prop_name, value_clone)
        }
        Datum::Stage => reserve_player_mut(|player| set_stage_prop(player, &prop_name, value_ref)),
        Datum::BitmapRef(bitmap_ref) => reserve_player_mut(|player| {
            BitmapDatumHandlers::set_bitmap_ref_prop(player, bitmap_ref, prop_name, value_ref)
        }),
        Datum::IntPoint(..) => reserve_player_mut(|player| {
            PointDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::TimeoutRef(_) => reserve_player_mut(|player| {
            TimeoutDatumHandlers::set_prop(player, obj_ref, prop_name, value_ref)
        }),
        Datum::PropList(..) => reserve_player_mut(|player| {
            let key_ref = player.alloc_datum(Datum::Symbol(prop_name.clone()));
            PropListUtils::set_prop(
                obj_ref,
                &key_ref,
                value_ref,
                player,
                true,
                prop_name.clone(),
            )
        }),
        Datum::IntRect(..) => reserve_player_mut(|player| {
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
    prop_name: &String,
) -> Result<DatumRef, ScriptError> {
    let obj_clone = player.get_datum(obj_ref).clone();
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
        Datum::IntRect(..) => {
            Ok(player.alloc_datum(RectDatumHandlers::get_prop(player, obj_ref, &prop_name)?))
        }
        Datum::IntPoint(..) => {
            Ok(player.alloc_datum(PointDatumHandlers::get_prop(player, obj_ref, &prop_name)?))
        }
        Datum::SpriteRef(sprite_id) => {
            let result = sprite_get_prop(player, sprite_id, prop_name)?;
            Ok(player.alloc_datum(result))
        }
        Datum::BitmapRef(_) => BitmapDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::String(s) => {
            Ok(player.alloc_datum(StringDatumUtils::get_built_in_prop(&s, &prop_name)?))
        }
        Datum::StringChunk(..) => Ok(player.alloc_datum(StringDatumUtils::get_built_in_prop(
            &obj_clone.string_value()?,
            &prop_name,
        )?)),
        Datum::TimeoutRef(_) => Ok(TimeoutDatumHandlers::get_prop(player, obj_ref, &prop_name)?),
        Datum::Symbol(_) => SymbolDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::Void => VoidDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::Int(_) => IntDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::ColorRef(_) => ColorDatumHandlers::get_prop(player, obj_ref, &prop_name),
        Datum::PlayerRef => player.get_player_prop(prop_name),
        Datum::XmlRef(_) => XmlDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::DateRef(_) => DateDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::MathRef(_) => MathDatumHandlers::get_prop(player, obj_ref, prop_name),
        Datum::Vector(_) => {
            Ok(player.alloc_datum(VectorDatumHandlers::get_prop(player, obj_ref, prop_name)?))
        }
        Datum::SoundChannel(_) => Ok(player.alloc_datum(SoundChannelDatumHandlers::get_prop(
            player, obj_ref, &prop_name,
        )?)),
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
