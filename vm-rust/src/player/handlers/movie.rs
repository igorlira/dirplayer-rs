use log::{debug, warn};

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        cast_lib::INVALID_CAST_MEMBER_REF,
        datum_formatting::format_datum, ScriptInstanceRef, Score,
        reserve_player_mut, reserve_player_ref, reserve_player_mut_async,
        score::get_sprite_at, handlers::datum_handlers::player_call_datum_handler,
        DatumRef, ScriptError, get_score_sprite_mut,
        events::{
            player_invoke_event_to_instances, player_invoke_static_event,
            player_invoke_global_event, player_wait_available, player_unwrap_result,
            dispatch_event_to_all_behaviors, player_dispatch_event_beginsprite,
            dispatch_system_event_to_timeouts
        },
    },
    utils::{log_i},
};

pub struct MovieHandlers {}

impl MovieHandlers {
    pub fn puppet_tempo(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            player.movie.puppet_tempo = player.get_datum(&args[0]).int_value()? as u32;
            Ok(DatumRef::Void)
        })
    }

    pub fn script(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let identifier = player.get_datum(&args[0]);
            let formatted_id = format_datum(&args[0], &player);

            let member_ref = match identifier {
                Datum::String(script_name) => Ok(player
                    .movie
                    .cast_manager
                    .find_member_ref_by_name(&script_name)),
                Datum::Int(script_num) => Ok(player
                    .movie
                    .cast_manager
                    .find_member_ref_by_number(*script_num as u32)),
                Datum::CastMember(cast_member_ref) => Ok(Some(cast_member_ref.clone())),
                _ => Err(ScriptError::new(format!(
                    "Invalid identifier for script: {}",
                    formatted_id
                ))), // TODO
            }?;
            let script = member_ref
                .to_owned()
                .and_then(|r| player.movie.cast_manager.get_script_by_ref(&r));

            match script {
                Some(_) => Ok(player.alloc_datum(Datum::ScriptRef(member_ref.unwrap()))),
                None => Err(ScriptError::new(format!(
                    "Script not found {}",
                    formatted_id
                ))),
            }
        })
    }

    pub fn member(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() > 2 {
                return Err(ScriptError::new(
                    "Too many arguments for member".to_string(),
                ));
            }
            let member_name_or_num_ref = args.get(0).unwrap();
            let member_name_or_num = player.get_datum(member_name_or_num_ref);
            if let Datum::CastMember(_) = &member_name_or_num {
                return Ok(member_name_or_num_ref.clone());
            }
            let cast_name_or_num = args.get(1).map(|x| player.get_datum(x));
            let member = player.movie.cast_manager.find_member_ref_by_identifiers(
                member_name_or_num,
                cast_name_or_num,
                &player.allocator,
            )?;
            if let Some(member) = member {
                Ok(player.alloc_datum(Datum::CastMember(member.to_owned())))
            } else {
                Ok(player.alloc_datum(Datum::CastMember(INVALID_CAST_MEMBER_REF)))
            }
        })
    }

    pub async fn go(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let mut frame_advanced = false;
        let mut enter_frame = 0;

        let destination_frame: u32 = reserve_player_mut(|player| {
            enter_frame = player.movie.current_frame;

            let datum: &Datum = player.get_datum(&args[0]);
            let datum_type = datum.type_enum();
            use crate::player::format_datum;

            debug!("Function go() called with datum: {}", format_datum(&args[0], player));

            let dest = match datum_type {
                DatumType::Int => Some(datum.int_value()? as u32),

                DatumType::String => {
                    let label = datum.string_value()?;
                    player.movie.score.frame_labels
                        .iter()
                        .find(|fl| fl.label.eq_ignore_ascii_case(&label))
                        .map(|fl| fl.frame_num as u32)
                }

                DatumType::Symbol => {
                    let symbol = datum.string_value()?;
                    match symbol.as_str() {
                        "next" => Some(player.movie.current_frame + 1),
                        "previous" => Some(player.movie.current_frame.saturating_sub(1).max(1)),
                        "loop" => Some(player.movie.current_frame),
                        _ => player.movie.score.frame_labels
                            .iter()
                            .find(|fl| fl.label.eq_ignore_ascii_case(&symbol))
                            .map(|fl| fl.frame_num as u32),
                    }
                }

                _ => None,
            };

            let frame = dest.ok_or_else(|| {
                ScriptError::new("Unsupported or invalid frame label passed to go()".to_string())
            })?;

            if player.next_frame.is_none() || frame != player.movie.current_frame {
                player.next_frame = Some(frame);

                if frame != enter_frame {
                    frame_advanced = true;
                }
            }

            Ok(frame)
        })?;

        if frame_advanced {
            let mut execute_frame_change = false;

            if enter_frame < destination_frame {
                reserve_player_mut(|player| {
                    player.go_direction = 2; // forwards
                });
                execute_frame_change = true;
            } else if enter_frame > destination_frame {
                reserve_player_mut(|player| {
                    player.go_direction = 1; // backwards
                });
                execute_frame_change = true;
            }

            if execute_frame_change {
                player_wait_available().await;

                // 1. Send endSprite: Frame behaviors -> Sprite behaviors
                let ended_sprite_nums = reserve_player_mut_async(|player| {
                    Box::pin(async move {
                        player.end_all_sprites().await
                    })
                }).await;

                player_wait_available().await;

                reserve_player_mut(|player| {
                    for (score_source, sprite_num) in ended_sprite_nums.iter() {
                        if let Some(sprite) = get_score_sprite_mut(
                            &mut player.movie,
                            &score_source,
                            *sprite_num as i16,
                        ) {
                            sprite.exited = true;
                        }
                    }

                    player.advance_frame();
                    player.movie.frame_script_instance = None;
                    player.begin_all_sprites();

                    // Apply tweening after sprites are initialized
                    player.movie.score.apply_tween_modifiers(player.movie.current_frame);
                });

                player_wait_available().await;

                // 2. Send beginSprite: Frame behaviors -> Sprite behaviors
                // Collect behaviors that need initialization
                let behaviors_to_init: Vec<(ScriptInstanceRef, u32)> = reserve_player_mut(|player| {
                    player
                        .movie
                        .score
                        .channels
                        .iter()
                        .flat_map(|ch| {
                            let sprite_num = ch.sprite.number as u32;
                            ch.sprite.script_instance_list
                                .iter()
                                .filter(|behavior_ref| {
                                    // ONLY initialize behaviors that haven't had beginSprite called
                                    // This means they're NEW this frame
                                    if let Some(entry) = player.allocator.script_instances.get(&behavior_ref.id()) {
                                        !entry.script_instance.begin_sprite_called
                                    } else {
                                        false
                                    }
                                })
                                .map(move |behavior_ref| (behavior_ref.clone(), sprite_num))
                        })
                        .collect()
                });

                // Initialize behavior default properties
                for (behavior_ref, sprite_num) in behaviors_to_init {
                    if let Err(err) = Score::initialize_behavior_defaults_async(behavior_ref, sprite_num).await {
                        web_sys::console::warn_1(
                            &format!("Failed to initialize behavior defaults: {}", err.message).into()
                        );
                    }
                }

                player_wait_available().await;

                let begin_sprite_nums = player_dispatch_event_beginsprite(
                    &"beginSprite".to_string(),
                    &vec![]
                ).await;

                player_wait_available().await;

                reserve_player_mut(|player| {
                    for sprite_list in begin_sprite_nums.iter() {
                        for (score_source, sprite_num) in sprite_list.iter() {
                            if let Some(sprite) = get_score_sprite_mut(
                                &mut player.movie,
                                score_source,
                                *sprite_num as i16,
                            ) {
                                for script_ref in &sprite.script_instance_list {
                                    if let Some(entry) =
                                        player.allocator.script_instances.get_mut(&script_ref.id())
                                    {
                                        entry.script_instance.begin_sprite_called = true;
                                    }
                                }
                            }
                        }
                    }
                });

                player_wait_available().await;

                // 3. Send stepFrame to actorList
                let actor_list_snapshot = reserve_player_ref(|player| {
                    let actor_list_ref = player.globals.get("actorList").unwrap_or(&DatumRef::Void).clone();
                    let actor_list_datum = player.get_datum(&actor_list_ref);
                    match actor_list_datum {
                        Datum::List(_, items, _) => items.clone(),
                        _ => vec![],
                    }
                });

                for (idx, actor_ref) in actor_list_snapshot.iter().enumerate() {
                    let still_active = reserve_player_ref(|player| {
                        let actor_list_ref = player.globals.get("actorList").unwrap_or(&DatumRef::Void).clone();
                        let actor_list_datum = player.get_datum(&actor_list_ref);
                        match actor_list_datum {
                            Datum::List(_, items, _) => items.contains(&actor_ref),
                            _ => false,
                        }
                    });

                    if still_active {
                        let result =
                            player_call_datum_handler(&actor_ref, &"stepFrame".to_string(), &vec![]).await;

                        if let Err(err) = result {
                            web_sys::console::log_1(
                                &format!("⚠ stepFrame[{}] error: {}", idx, err.message).into(),
                            );
                            reserve_player_mut(|player| {
                                player.on_script_error(&err);
                                player.is_in_frame_update = false;
                            });
                            return Err(err);
                        }
                    }
                }

                player_wait_available().await;

                // Prevent re-entrant calls
                let already_updating = reserve_player_mut(|player| {
                    if player.is_in_frame_update {
                        return true;
                    }
                    player.is_in_frame_update = true;
                    false
                });

                if !already_updating {
                    reserve_player_mut(|player| {
                        player.in_prepare_frame = true;
                    });

                    // Relay prepareFrame to timeout targets
                    dispatch_system_event_to_timeouts(&"prepareFrame".to_string(), &vec![]).await;

                    // 4. Send prepareFrame: Sprite behaviors -> Frame behaviors
                    let _ = dispatch_event_to_all_behaviors(&"prepareFrame".to_string(), &vec![]).await;

                    reserve_player_mut(|player| {
                        player.in_prepare_frame = false;
                    });

                    player_wait_available().await;

                    reserve_player_mut(|player| {
                        player.in_enter_frame = true;
                    });

                    // 5. Send enterFrame: Sprite behaviors -> Frame behaviors
                    let _ = dispatch_event_to_all_behaviors(&"enterFrame".to_string(), &vec![]).await;

                    reserve_player_mut(|player| {
                        player.in_enter_frame = false;
                    });

                    player_wait_available().await;

                    reserve_player_mut(|player| {
                        player.is_in_frame_update = false;
                    });
                } else {
                    warn!("Failed to run frame update in go function, already updating");
                }
            }
        }
        
        reserve_player_mut(|player| {
            player.has_frame_changed_in_go = true;
        });

        Ok(DatumRef::Void)
    }

    pub fn puppet_sprite(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let sprite_number = player.get_datum(&args[0]).int_value()?;
            let is_puppet = player.get_datum(&args[1]).int_value()? == 1;
            let sprite = player.movie.score.get_sprite_mut(sprite_number as i16);
            sprite.puppet = is_puppet;
            Ok(DatumRef::Void)
        })
    }

    pub fn sprite(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let sprite_number = player.get_datum(&args[0]).int_value()?;
            Ok(player.alloc_datum(Datum::SpriteRef(sprite_number as i16)))
        })
    }

    pub async fn send_sprite(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let (message, remaining_args, receivers) = reserve_player_mut(|player| {
            let sprite_num = player.get_datum(&args[0]).int_value()
                .map_err(|e| ScriptError::new(format!("sendSprite: invalid sprite number: {:?}", e)))?;
            let message = player.get_datum(&args[1]).symbol_value()
                .map_err(|e| ScriptError::new(format!("sendSprite: invalid message: {:?}", e)))?;
            let remaining_args = &args[2..].to_vec();
            let sprite = player.movie.score.get_sprite(sprite_num as i16)
                .ok_or_else(|| ScriptError::new(format!("sendSprite: sprite {} not found", sprite_num)))?;
            let receivers = sprite.script_instance_list.clone();
            Ok((message.clone(), remaining_args.clone(), receivers))
        })?;
        
        let mut handled_by_sprite = false;
        for receiver in receivers {
            let receivers = vec![receiver];
            match player_invoke_event_to_instances(&message, &remaining_args, &receivers).await {
                Ok(handled) => {
                    handled_by_sprite = handled || handled_by_sprite;
                }
                Err(err) => {
                    // Error already logged by player_invoke_event_to_instances
                    // Continue execution instead of propagating error
                    web_sys::console::warn_1(
                        &format!("⚠ sendSprite continuing after error in handler").into()
                    );
                    // Optionally: break here if you want to stop after first error
                    // For now, continue to match Director behavior
                }
            }
        }
        
        if !handled_by_sprite {
            player_invoke_static_event(&message, &remaining_args).await?;
        }
        
        reserve_player_mut(|player: &mut crate::player::DirPlayer| {
            Ok(player.alloc_datum(Datum::Int(handled_by_sprite as i32)))
        })
    }

    pub async fn send_all_sprites(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // Check for re-entrant sendAllSprites call
        let skip = reserve_player_mut(|player| {
            if player.is_in_send_all_sprites {
                warn!(
                    "Blocking re-entrant sendAllSprites call to prevent infinite recursion"
                );
                return true;
            }
            player.is_in_send_all_sprites = true;
            false
        });

        if skip {
            return Ok(DatumRef::Void);
        }

        let (message, remaining_args, receivers) = reserve_player_mut(|player| {
            let message = player.get_datum(&args[0]).symbol_value()
                .map_err(|e| ScriptError::new(format!("sendAllSprites: invalid message: {:?}", e)))?;
            let remaining_args = &args[1..].to_vec();

            // Collect receivers from stage score
            let mut receivers = player.movie.score.get_active_script_instance_list();

            // Also collect receivers from filmloop scores
            let active_filmloops = player.get_active_filmloop_scores();
            for (_, filmloop_score) in active_filmloops {
                let filmloop_receivers = filmloop_score.get_active_script_instance_list();
                receivers.extend(filmloop_receivers);
            }

            Ok((message.clone(), remaining_args.clone(), receivers))
        })?;
        
        let mut handled_by_sprite = false;
        for receiver in receivers {
            let receivers = vec![receiver];
            match player_invoke_event_to_instances(&message, &remaining_args, &receivers).await {
                Ok(handled) => {
                    handled_by_sprite = handled || handled_by_sprite;
                }
                Err(err) => {
                    // Error already logged by player_invoke_event_to_instances
                    web_sys::console::warn_1(
                        &format!("⚠ sendAllSprites continuing after error in handler").into()
                    );
                    // Continue to next sprite instead of stopping
                }
            }
        }

        if !handled_by_sprite {
            player_invoke_static_event(&message, &remaining_args).await?;
        }

        reserve_player_mut(|player: &mut crate::player::DirPlayer| {
            // Reset the re-entrancy flag
            player.is_in_send_all_sprites = false;
            Ok(player.alloc_datum(Datum::Int(handled_by_sprite as i32)))
        })
    }

    pub fn external_param_count(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let count = player.external_params.len() as i32;
            Ok(player.alloc_datum(Datum::Int(count)))
        })
    }

    pub fn external_param_name(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let datum = player.get_datum(&args[0]);

            // Case 1: argument is a string (lookup by name, case-insensitive)
            if let Ok(key) = datum.string_value() {
                if player
                    .external_params
                    .keys()
                    .any(|k| k.to_lowercase() == key.to_lowercase())
                {
                    return Ok(player.alloc_datum(Datum::String(key)));
                } else {
                    return Ok(player.alloc_datum(Datum::Void));
                }
            }

            // Case 2: argument is an integer (index)
            if let Ok(index) = datum.int_value() {
                if index > 0 && (index as usize) <= player.external_params.len() {
                    if let Some((key, _)) = player.external_params.iter().nth(index as usize - 1) {
                        return Ok(player.alloc_datum(Datum::String(key.clone())));
                    }
                }
                return Ok(player.alloc_datum(Datum::Void));
            }

            // Invalid argument type
            log_i("external_param_name(): invalid argument type, returning Void");
            Ok(player.alloc_datum(Datum::Void))
        })
    }

    pub fn external_param_value(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let datum = player.get_datum(&args[0]);

            // Case 1: argument is a string (lookup by name)
            if let Ok(key) = datum.string_value() {
                if let Some((_k, value)) = player
                    .external_params
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == key.to_lowercase())
                {
                    return Ok(player.alloc_datum(Datum::String(value.clone())));
                } else {
                    return Ok(player.alloc_datum(Datum::Void));
                }
            }

            // Case 2: argument is an integer (index)
            if let Ok(index) = datum.int_value() {
                if index > 0 && (index as usize) <= player.external_params.len() {
                    if let Some((_key, value)) =
                        player.external_params.iter().nth(index as usize - 1)
                    {
                        return Ok(player.alloc_datum(Datum::String(value.clone())));
                    }
                }
                return Ok(player.alloc_datum(Datum::Void));
            }

            // Invalid type
            log_i(&format!(
                "external_param_value(): invalid argument type, returning Void"
            ));
            Ok(player.alloc_datum(Datum::Void))
        })
    }

    pub fn stop_event(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // TODO stop event
        Ok(DatumRef::Void)
    }

    pub fn get_pref(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Return empty string - Lingo code handles the fallback
            Ok(player.alloc_datum(Datum::String("".to_string())))
        })
    }

    pub fn set_pref(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Ok(DatumRef::Void)
    }

    pub fn go_to_net_page(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Ok(DatumRef::Void)
    }

    pub fn pass(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let scope_ref = player.current_scope_ref();
            let scope = player.scopes.get_mut(scope_ref).unwrap();
            scope.passed = true;
            Ok(DatumRef::Void)
        })
    }

    pub async fn execute_frame_update() -> Result<(), ScriptError> {
        player_wait_available().await;

        // Prevent re-entrant calls
        let already_updating = reserve_player_mut(|player| {
            if player.is_in_frame_update {
                return true;
            }
            player.is_in_frame_update = true;
            false
        });

        let has_player_frame_changed = reserve_player_ref(|player| player.has_player_frame_changed);

        if already_updating || has_player_frame_changed {
            return Ok(());  // Exit early if already updating
        }

        player_wait_available().await;

        reserve_player_mut(|player| {
            player.movie.score.apply_tween_modifiers(player.movie.current_frame);
        });

        // 1. Send stepFrame to actorList
        let actor_list_snapshot = reserve_player_ref(|player| {
            let actor_list_ref = player.globals.get("actorList").unwrap_or(&DatumRef::Void).clone();
            let actor_list_datum = player.get_datum(&actor_list_ref);
            match actor_list_datum {
                Datum::List(_, items, _) => items.clone(),
                _ => vec![],
            }
        });

        for (idx, actor_ref) in actor_list_snapshot.iter().enumerate() {
            let still_active = reserve_player_ref(|player| {
                let actor_list_ref = player.globals.get("actorList").unwrap_or(&DatumRef::Void).clone();
                let actor_list_datum = player.get_datum(&actor_list_ref);
                match actor_list_datum {
                    Datum::List(_, items, _) => items.contains(&actor_ref),
                    _ => false,
                }
            });

            if still_active {
                let result =
                    player_call_datum_handler(&actor_ref, &"stepFrame".to_string(), &vec![]).await;

                if let Err(err) = result {
                    web_sys::console::log_1(
                        &format!("⚠ stepFrame[{}] error: {}", idx, err.message).into(),
                    );
                    reserve_player_mut(|player| {
                        player.on_script_error(&err);
                        player.is_in_frame_update = false;
                    });
                    return Err(err);
                }
            }
        }

        player_wait_available().await;

        reserve_player_mut(|player| {
            player.in_prepare_frame = true;
        });

        // Relay prepareFrame to timeout targets
        dispatch_system_event_to_timeouts(&"prepareFrame".to_string(), &vec![]).await;

        dispatch_event_to_all_behaviors(&"prepareFrame".to_string(), &vec![]).await;

        reserve_player_mut(|player| {
            player.in_prepare_frame = false;
        });

        player_wait_available().await;

        reserve_player_mut(|player| {
            player.in_enter_frame = true;
        });

        dispatch_event_to_all_behaviors(&"enterFrame".to_string(), &vec![]).await;

        reserve_player_mut(|player| {
            player.in_enter_frame = false;
        });

        player_wait_available().await;

        reserve_player_mut(|player| {
            player.is_in_frame_update = false;
        });

        Ok(())
    }

    pub async fn update_stage(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let should_yield = reserve_player_ref(|player| {

            debug!("updateStage: handler_stack_depth = {:?}, is_in_frame_update = {}, in_frame_script = {}, in_enter_frame = {}, in_prepare_frame = {}, in_event_dispatch = {}", 
                player.handler_stack_depth,
                player.is_in_frame_update,
                player.in_frame_script,
                player.in_enter_frame,
                player.in_prepare_frame,
                player.in_event_dispatch
            );

            Ok(player.is_yield_safe())
        })?;

        if should_yield {
            // Synchronous render - only works with Canvas2D backend
            reserve_player_mut(|player| {
                crate::rendering::with_canvas2d_renderer_mut(|renderer| {
                    crate::rendering::render_stage_to_bitmap(
                        player,
                        &mut renderer.bitmap,
                        renderer.debug_selected_channel_num,
                    );

                    use wasm_bindgen::Clamped;
                    let bitmap = &renderer.bitmap;
                    if let Ok(image_data) =
                        web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                            Clamped(&bitmap.data[..]),
                            bitmap.width.into(),
                            bitmap.height.into(),
                        )
                    {
                        let _ = renderer.ctx2d.put_image_data(&image_data, 0.0, 0.0);
                    }
                });
            });

            async_std::task::sleep(std::time::Duration::from_millis(2)).await;
        }

        Ok(DatumRef::Void)
    }

    pub fn rollover(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
            Ok(player.alloc_datum(Datum::Int(sprite.unwrap_or(0) as i32)))
        })
    }

    pub fn puppet_sound(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new(
                "puppetSound requires at least 1 argument".to_string(),
            ));
        }
        
        reserve_player_mut(|player| {
            // If only one argument, use channel 1 by default
            let (channel_num, member_ref) = if args.len() == 1 {
                (1, args[0].clone())
            } else {
                let channel = player.get_datum(&args[0]).int_value()?;
                (channel, args[1].clone())
            };
            
            player.puppet_sound(channel_num, member_ref)?;
            Ok(DatumRef::Void)
        })
    }

    pub fn halt(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Stop movie playback
            player.movie.current_frame = 1;
            player.is_playing = false;
            Ok(DatumRef::Void)
        })
    }
}
