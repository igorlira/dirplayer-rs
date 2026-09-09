use std::collections::VecDeque;
use log::error;
use crate::{
    director::lingo::datum::{Datum, DatumType, datum_bool},
    player::{
        DatumRef, ScriptError, ScriptErrorCode, allocator::ScriptInstanceAllocatorTrait, cast_lib::CastMemberRef, player_call_script_handler, player_handle_scope_return, reserve_player_mut, reserve_player_ref, script::{ScriptInstance, get_lctx_for_script}, script_ref::ScriptInstanceRef, symbols::{builtin::BuiltInSymbol, symbol::Symbol}
    },
};
pub struct ScriptDatumHandlers {}

impl ScriptDatumHandlers {
    pub fn has_async_handler(obj_ref: &DatumRef, name: Symbol) -> bool {
        match name.as_lower_str() {
            "new" => true,
            // `birth` on a ScriptRef is the Director 6 constructor (see `birth`).
            "birth" => true,
            "rawnew" => false,
            "handler" => false,
            "handlers" => false,
            "count" => false,
            _ => {
                reserve_player_ref(|player| {
                    if let Datum::ScriptRef(script_ref) = player.get_datum(obj_ref) {
                        if let Some(script_rc) =
                            player.movie.cast_manager.get_script_by_ref(script_ref)
                        {
                            if script_rc.get_own_handler(name).is_some() {
                                return true;
                            }
                        }
                        if crate::player::virtual_scripts::VirtualScriptRegistry::has_script_handler(player, script_ref, name) {
                            return true;
                        }
                    }
                    false
                })
            }
        }
    }

    pub async fn call_async(
        obj_ref: &DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_lower_str() {
            "new" => Self::new(obj_ref, args).await,
            "birth" => Self::birth(obj_ref, args).await,
            "rawnew" => Self::raw_new(obj_ref),
            _ => {
                // Try to call a handler defined in the script itself
                let handler_ref = reserve_player_ref(|player| {
                    let script_ref = match player.get_datum(obj_ref) {
                        Datum::ScriptRef(script_ref) => script_ref.clone(),
                        _ => return Err(ScriptError::new("Expected script reference".to_string())),
                    };
                    Ok::<_, ScriptError>((script_ref, handler_name.to_owned()))
                })?;

                // Check if the script actually has this handler
                let has_handler = reserve_player_ref(|player| {
                    if let Datum::ScriptRef(script_ref) = player.get_datum(obj_ref) {
                        if let Some(script_rc) =
                            player.movie.cast_manager.get_script_by_ref(script_ref)
                        {
                            let script = script_rc.as_ref();
                            return script.get_own_handler(handler_name).is_some();
                        }
                    }
                    false
                });

                if !has_handler {
                    let virtual_result = reserve_player_mut(|player| {
                        let script_ref = match player.get_datum(obj_ref) {
                            Datum::ScriptRef(script_ref) => script_ref.clone(),
                            _ => return Ok(None),
                        };
                        crate::player::virtual_scripts::VirtualScriptRegistry::try_call_handler(player, &script_ref, None, handler_name, args)
                    });
                    match virtual_result {
                        Ok(Some(result)) => return Ok(result),
                        Err(e) => return Err(e),
                        Ok(None) => {}
                    }

                    return Err(ScriptError::new_code(
                        ScriptErrorCode::HandlerNotFound,
                        format!("No handler {} for script datum", handler_name),
                    ));
                }

                // Call with no receiver (None) - the script itself becomes "me"
                let result = player_call_script_handler(None, handler_ref, args).await?;
                Ok(result.return_value)
            }
        }
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_lower_str() {
            "rawnew" => Self::raw_new(datum),
            "handler" => Self::handler(datum, args),
            "handlers" => Self::handlers(datum, args),
            "count" => Self::count(datum, args),
            // A movie script's static properties are addressable through the
            // script reference (`g.levellist = []`, `g.levellist.add(...)`).
            // getPropRef returns the property's shared DatumRef so in-place list
            // mutation persists, mirroring the ScriptInstance handler.
            "getprop" | "getpropref" | "getaprop" => reserve_player_mut(|player| {
                let script_ref = match player.get_datum(datum) {
                    Datum::ScriptRef(s) => s.clone(),
                    _ => return Err(ScriptError::new("Expected script reference".to_string())),
                };
                let prop_name = player.get_datum(&args[0]).string_value()?;
                let prop_ref =
                    crate::player::script::script_get_static_prop(player, &script_ref, Symbol::from_str(&prop_name))?;
                if args.len() >= 2 {
                    // `g.prop[index]` — the bytecode passes (script, #prop, index),
                    // so index into the property value (e.g. list element). Without
                    // this the whole property was returned, ignoring the index.
                    crate::player::handlers::types::TypeUtils::get_sub_prop(
                        &prop_ref, &args[1], player,
                    )
                } else {
                    Ok(prop_ref)
                }
            }),
            "setprop" | "setaprop" => reserve_player_mut(|player| {
                let script_ref = match player.get_datum(datum) {
                    Datum::ScriptRef(s) => s.clone(),
                    _ => return Err(ScriptError::new("Expected script reference".to_string())),
                };
                let prop_name = player.get_datum(&args[0]).string_value()?;
                if args.len() >= 3 {
                    // `g.prop[index] = value`
                    let prop_ref = crate::player::script::script_get_static_prop(
                        player, &script_ref, Symbol::from_str(&prop_name),
                    )?;
                    crate::player::handlers::types::TypeUtils::set_sub_prop(
                        &prop_ref, &args[1], &args[2], player,
                    )?;
                    Ok(args[2].clone())
                } else {
                    crate::player::script::script_set_static_prop(
                        player, &script_ref, Symbol::from_str(&prop_name), &args[1], false,
                    )?;
                    Ok(args[1].clone())
                }
            }),
            _ => Err(ScriptError::new(format!(
                "no handler {handler_name} for script datum"
            ))),
        }
    }

    /// `script.count(#prop)` — number of items in a static property that is a
    /// list, matching `ScriptInstanceHandlers::count`.
    /// No-arg `script.count()` is the number of static properties (Director
    /// `count(object)` for a non-list object is 1 if there are none).
    pub fn count(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let script_ref = match player.get_datum(datum) {
                Datum::ScriptRef(s) => s.clone(),
                _ => return Err(ScriptError::new("Expected script reference".to_string())),
            };
            if args.is_empty() {
                let n = player
                    .movie
                    .cast_manager
                    .get_script_by_ref(&script_ref)
                    .map(|s| s.properties.borrow().len() as i32)
                    .unwrap_or(1)
                    .max(1);
                return Ok(player.alloc_datum(Datum::Int(n)));
            }
            let prop_name = Symbol::from_str(&player.get_datum(&args[0]).string_value()?);
            let prop_value = crate::player::script::script_get_static_prop(player, &script_ref, prop_name)?;
            let prop_value_datum = player.get_datum(&prop_value);
            let count = match prop_value_datum {
                Datum::List(_, list, _) => list.len(),
                Datum::PropList(prop_list, ..) => prop_list.len(),
                Datum::Void => 0,
                other => {
                    return Err(ScriptError::new(format!(
                        "Cannot count non-list property {} (type {})",
                        prop_name.as_str(),
                        other.type_str()
                    )))
                }
            };
            Ok(player.alloc_datum(Datum::Int(count as i32)))
        })
    }

    pub fn handlers(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let script_ref = match player.get_datum(datum) {
                Datum::ScriptRef(script_ref) => script_ref,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot get handlers of non-script".to_string(),
                    ))
                }
            };
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .unwrap();
            let handler_names = script.handler_names.clone();
            let handler_name_datums: VecDeque<_> = handler_names
                .iter()
                .map(|name| player.alloc_datum(Datum::Symbol(name.clone())))
                .collect();
            Ok(player.alloc_datum(Datum::List(DatumType::List, handler_name_datums, false)))
        })
    }

    pub fn handler(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let name = player.get_datum(&args[0]).symbol_value()?;
            let script_ref = match player.get_datum(datum) {
                Datum::ScriptRef(script_ref) => script_ref,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot create new instance of non-script".to_string(),
                    ))
                }
            };
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .unwrap();
            let own_handler = script.get_own_handler(name);
            Ok(player.alloc_datum(datum_bool(own_handler.is_some())))
        })
    }

    pub fn create_script_instance(script_ref: &CastMemberRef) -> Result<(ScriptInstanceRef, DatumRef), ScriptError> {
        reserve_player_mut(|player| {
            let instance_id = player.allocator.get_free_script_instance_id();
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .ok_or_else(|| ScriptError::new(format!("Script not found: {:?}", script_ref)))?;

            let lctx_opt = get_lctx_for_script(player, script);

            if let Some(lctx) = lctx_opt {
                let lctx_ptr: *const crate::director::lingo::script::ScriptContext = lctx as *const _;
                let instance = ScriptInstance::new(
                    instance_id,
                    script_ref.to_owned(),
                    script,
                    unsafe { &*lctx_ptr },
                );
                let instance_ref = player.allocator.alloc_script_instance(instance);
                let datum_ref = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
                Ok((instance_ref, datum_ref))
            } else {
                Ok(crate::player::virtual_scripts::VirtualScriptRegistry::create_instance(player, script_ref))
            }
        })
    }

    fn create_uninit_instance(datum: &DatumRef) -> Result<(CastMemberRef, ScriptInstanceRef, DatumRef), ScriptError> {
        let script_ref = reserve_player_mut(|player| {
            let script_ref = match player.get_datum(datum) {
                Datum::ScriptRef(script_ref) => script_ref,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot create new instance of non-script".to_string(),
                    ))
                }
            };

            Ok(script_ref.clone())
        })?;

        let (script_instance_ref, datum_ref) = match Self::create_script_instance(&script_ref) {
            Ok((instance_ref, datum_ref)) => (instance_ref, datum_ref),
            Err(e) => {
                error!("Failed to create script instance: {}", e.message);
                return Err(e); // Return the error
            }
        };

        Ok((script_ref, script_instance_ref, datum_ref))
    }

    pub fn raw_new(datum: &DatumRef) -> Result<DatumRef, ScriptError> {
        Ok(Self::create_uninit_instance(datum)?.2)
    }

    pub async fn new(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Self::construct(datum, args, "new").await
    }

    /// Director 6 `birth(script, args)` — the pre-`new` constructor idiom. It is
    /// exactly `new` except it runs the instance's `on birth me, args` handler
    /// instead of `on new`. The 11.5 Scripting Dictionary dropped `birth`, so we
    /// mirror the documented `new` / parent-script contract: a FRESH instance
    /// with its OWN property storage per call. (100s-Marios births 200 marios
    /// via `birth(script "MarioScript", 3+i)`; without a fresh instance each
    /// call, all same-script marios shared one `sn`/`timr`/`pipe` and only ~4
    /// sprites were ever driven.)
    pub async fn birth(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Self::construct(datum, args, "birth").await
    }

    /// Shared constructor for `new`/`birth`: allocate a fresh instance, then run
    /// its `ctor` handler (`on new` / `on birth`) with the instance as `me`.
    async fn construct(datum: &DatumRef, args: &Vec<DatumRef>, ctor: &str) -> Result<DatumRef, ScriptError> {
        let (script_ref, script_instance_ref, datum_ref) = Self::create_uninit_instance(datum)?;

        let (new_handler_ref, expected_param_count, script_name) =
            reserve_player_mut(|player| {
                let script = player
                    .movie
                    .cast_manager
                    .get_script_by_ref(&script_ref)
                    .unwrap();
                let new_handler_ref = script.get_own_handler_ref(Symbol::from_str(&ctor.to_string()));

                let param_count = if let Some(_) = &new_handler_ref {
                    let handler_def = script.get_own_handler(Symbol::from_str(&ctor.to_string())).unwrap();
                    handler_def.argument_name_ids.len()
                } else {
                    0
                };

                Ok((
                    new_handler_ref,
                    param_count,
                    script.name.clone(),
                ))
            })?;

        let virtual_new_result = reserve_player_mut(|player| {
            crate::player::virtual_scripts::VirtualScriptRegistry::try_call_handler(player, &script_ref, Some(&script_instance_ref), Symbol::from_str(ctor), args)
        });
        match virtual_new_result {
            Ok(Some(_)) => return Ok(datum_ref),
            Err(e) => return Err(e),
            Ok(None) => {}
        }

        if let Some(new_handler_ref) = new_handler_ref {
            let mut padded_args = args.clone();
            while padded_args.len() < expected_param_count {
                padded_args.push(DatumRef::Void);
            }

            let result_scope =
                match player_call_script_handler(Some(script_instance_ref), new_handler_ref, &padded_args)
                    .await
                {
                    Ok(scope) => scope,
                    Err(err) => {
                        error!("❌ Error in {}.{}(): {}", script_name, ctor, err.message);
                        return Err(err);
                    }
                };

            player_handle_scope_return(&result_scope);
            // Director's `new()` returns the new child instance. The `on new`
            // handler conventionally ends with `return me`, but if it falls off
            // the end without returning a value (VOID), Director still returns the
            // instance — NOT VOID. Only an explicit non-void return overrides.
            // (SpongeBob "JellyFishin'" nav object: `on new me, targetMovie`
            // has no `return me`, so navMovieObj was VOID and gotoExitPage /
            // gotoMainMovieAgain dispatched on Void.)
            if matches!(result_scope.return_value, DatumRef::Void) {
                return Ok(datum_ref);
            }
            return Ok(result_scope.return_value);
        } else {
            return Ok(datum_ref);
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod script_count_tests {
    use super::*;
    use crate::{
        director::{chunks::script::ScriptChunk, enums::ScriptType, lingo::datum::DatumType},
        player::{
            cast_lib::{cast_member_ref, CastLib, CastLibState},
            script::Script,
            symbols::symbol_table::init_symbol_table,
            testing::{run_test, TestPlayer},
        },
    };
    use fxhash::FxHashMap;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    fn empty_cast(number: u32) -> CastLib {
        CastLib {
            name: String::new(),
            file_name: String::new(),
            number,
            is_external: false,
            state: CastLibState::Loaded,
            lctx: None,
            members: FxHashMap::default(),
            scripts: FxHashMap::default(),
            name_symbols: Vec::new(),
            preload_mode: 0,
            capital_x: false,
            dir_version: 0,
            palette_id_offset: 0,
            name_index: RefCell::new(None),
            font_table: HashMap::new(),
        }
    }

    fn empty_script_chunk() -> ScriptChunk {
        ScriptChunk {
            script_number: 1,
            literals: vec![],
            handlers: vec![],
            property_name_ids: vec![],
            property_defaults: HashMap::new(),
        }
    }

    fn install_script(player: &mut crate::player::DirPlayer, name: &str) -> (CastMemberRef, DatumRef) {
        let member_ref = cast_member_ref(1, 1);
        let script = Script {
            member_ref: member_ref.clone(),
            name: name.to_string(),
            chunk: empty_script_chunk(),
            script_type: ScriptType::Movie,
            handlers: FxHashMap::default(),
            handler_names_raw: vec![],
            handler_names: vec![],
            properties: RefCell::new(FxHashMap::default()),
        };
        let mut cast = empty_cast(1);
        cast.scripts.insert(1, Rc::new(script));
        player.movie.cast_manager.casts.push(cast);
        let datum = player.alloc_datum(Datum::ScriptRef(member_ref.clone()));
        (member_ref, datum)
    }

    #[test]
    fn no_arg_count_is_at_least_one() {
        init_symbol_table();
        run_test(async {
            let _p = TestPlayer::new();
            reserve_player_mut(|player| {
                let (_r, datum) = install_script(player, "globals");
                let n = ScriptDatumHandlers::count(&datum, &vec![]).unwrap();
                match player.get_datum(&n) {
                    Datum::Int(1) => {}
                    other => panic!("expected Int(1), got {}", other.type_str()),
                }
            });
        });
    }

    #[test]
    fn no_arg_count_is_the_number_of_static_properties() {
        init_symbol_table();
        run_test(async {
            let _p = TestPlayer::new();
            reserve_player_mut(|player| {
                let (member_ref, datum) = install_script(player, "globals");
                let a = player.alloc_datum(Datum::Int(1));
                let b = player.alloc_datum(Datum::Int(2));
                crate::player::script::script_set_static_prop(
                    player,
                    &member_ref,
                    Symbol::from_str("one"),
                    &a,
                    false,
                )
                .unwrap();
                crate::player::script::script_set_static_prop(
                    player,
                    &member_ref,
                    Symbol::from_str("two"),
                    &b,
                    false,
                )
                .unwrap();
                let n = ScriptDatumHandlers::count(&datum, &vec![]).unwrap();
                match player.get_datum(&n) {
                    Datum::Int(2) => {}
                    other => panic!("expected Int(2), got {}", other.type_str()),
                }
            });
        });
    }

    #[test]
    fn count_of_a_list_property_is_the_list_length() {
        init_symbol_table();
        run_test(async {
            let _p = TestPlayer::new();
            reserve_player_mut(|player| {
                let (member_ref, datum) = install_script(player, "globals");
                let items = VecDeque::from([
                    player.alloc_datum(Datum::Int(1)),
                    player.alloc_datum(Datum::Int(2)),
                    player.alloc_datum(Datum::Int(3)),
                ]);
                let list = player.alloc_datum(Datum::List(DatumType::List, items, false));
                crate::player::script::script_set_static_prop(
                    player,
                    &member_ref,
                    Symbol::from_str("levellist"),
                    &list,
                    false,
                )
                .unwrap();
                let prop = player.alloc_datum(Datum::Symbol(Symbol::from_str("levellist")));
                let n = ScriptDatumHandlers::count(&datum, &vec![prop]).unwrap();
                match player.get_datum(&n) {
                    Datum::Int(3) => {}
                    other => panic!("expected Int(3), got {}", other.type_str()),
                }
            });
        });
    }

    #[test]
    fn count_of_a_void_property_is_zero() {
        init_symbol_table();
        run_test(async {
            let _p = TestPlayer::new();
            reserve_player_mut(|player| {
                let (member_ref, datum) = install_script(player, "globals");
                crate::player::script::script_set_static_prop(
                    player,
                    &member_ref,
                    Symbol::from_str("empty"),
                    &DatumRef::Void,
                    false,
                )
                .unwrap();
                let prop = player.alloc_datum(Datum::Symbol(Symbol::from_str("empty")));
                let n = ScriptDatumHandlers::count(&datum, &vec![prop]).unwrap();
                match player.get_datum(&n) {
                    Datum::Int(0) => {}
                    other => panic!("expected Int(0), got {}", other.type_str()),
                }
            });
        });
    }
}
