use log::{debug, error};
use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        cast_lib::{CastMemberRef, INVALID_CAST_MEMBER_REF},
        datum_formatting::format_datum, ScriptInstanceRef, Score,
        reserve_player_mut, reserve_player_ref, reserve_player_mut_async,
        player_call_script_handler,
        score::{get_sprite_at, concrete_sprite_hit_test}, handlers::datum_handlers::player_call_datum_handler,
        handlers::datum_handlers::script_instance::ScriptInstanceUtils,
        DatumRef, ScriptError, ScriptErrorCode, get_score_sprite_mut, MovieFrameTarget,
        events::{
            player_invoke_static_event, player_wait_available,
            dispatch_event_to_all_behaviors, player_dispatch_event_beginsprite,
            dispatch_system_event_to_timeouts, player_invoke_targeted_event
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
            } else if let Some(cast_datum) = cast_name_or_num {
                // Director returns a valid ref for member(N, castLib) even if the
                // member slot is empty (used by getDynamicSlot pattern).
                // Only for positive member numbers — negative/zero are invalid refs.
                let cast_num = match cast_datum {
                    Datum::String(s) => player.movie.cast_manager.get_cast_by_name(s)
                        .map(|c| c.number as i32),
                    Datum::Int(n) => Some(*n),
                    Datum::CastLib(n) => Some(*n as i32),
                    _ => None,
                };
                if let (Some(cast_num), Ok(member_num)) = (cast_num, member_name_or_num.int_value()) {
                    if member_num > 0 {
                        Ok(player.alloc_datum(Datum::CastMember(CastMemberRef {
                            cast_lib: cast_num,
                            cast_member: member_num,
                        })))
                    } else {
                        Ok(player.alloc_datum(Datum::CastMember(INVALID_CAST_MEMBER_REF)))
                    }
                } else {
                    Ok(player.alloc_datum(Datum::CastMember(INVALID_CAST_MEMBER_REF)))
                }
            } else {
                Ok(player.alloc_datum(Datum::CastMember(INVALID_CAST_MEMBER_REF)))
            }
        })
    }

    pub async fn go(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            // "go" with no args is gotoLoop: go to nearest marker at or before current frame,
            // or frame 1 if no markers exist
            reserve_player_mut(|player| {
                let current = player.movie.current_frame as i32;
                let label_frame = player.movie.score.frame_labels.iter()
                    .rev()
                    .find(|fl| fl.frame_num <= current)
                    .map(|fl| fl.frame_num as u32);
                player.next_frame = Some(label_frame.unwrap_or(1));
            });
            return Ok(DatumRef::Void);
        }
        // If a second argument is provided, it's a movie path: go frame X of movie "path"
        let go_to_movie = if args.len() >= 2 {
            reserve_player_mut(|player| {
                let movie_path = player.get_datum(&args[1]).string_value()?;
                let movie_path = if movie_path.contains(".") {
                    movie_path
                } else {
                    let extension = player.movie.file_name.split('.').last().unwrap_or("dcr");
                    format!("{}.{}", movie_path, extension)
                };

                // Resolve the first arg into a MovieFrameTarget
                let datum = player.get_datum(&args[0]);
                let datum_type = datum.type_enum();
                let target = match datum_type {
                    DatumType::Int => MovieFrameTarget::Frame(datum.int_value()? as u32),
                    DatumType::String => MovieFrameTarget::Label(datum.string_value()?),
                    DatumType::Symbol => MovieFrameTarget::Label(datum.string_value()?),
                    _ => MovieFrameTarget::Default,
                };

                let task_id = player.net_manager.preload_net_thing(movie_path.clone());
                player.pending_goto_net_movie = Some((task_id, target));
                Ok(true)
            })?
        } else {
            false
        };

        if go_to_movie {
            return Ok(DatumRef::Void);
        }

        let mut frame_advanced = false;
        let mut enter_frame = 0;

        let destination_frame: u32 = reserve_player_mut(|player| {
            enter_frame = player.movie.current_frame;

            let datum: &Datum = player.get_datum(&args[0]);
            let datum_type = datum.type_enum();
            use crate::player::format_datum;

            debug!("go() called: current_frame={} datum={}", player.movie.current_frame, format_datum(&args[0], player));

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
                        "next" => {
                            let next_frame = player.movie.current_frame + 1;
                            debug!("🎬 go(#next): {} -> {}", player.movie.current_frame, next_frame);
                            Some(next_frame)
                        },
                        "previous" => Some(if player.movie.current_frame > 1 { player.movie.current_frame - 1 } else { 1 }),
                        "loop" => Some(player.movie.current_frame),
                        _ => player.movie.score.frame_labels
                            .iter()
                            .find(|fl| fl.label.eq_ignore_ascii_case(&symbol))
                            .map(|fl| fl.frame_num as u32),
                    }
                }

                _ => None,
            };

            let frame = match dest {
                Some(f) => f,
                None => {
                    return Err(ScriptError::new("Unsupported or invalid frame label passed to go()".to_string()));
                }
            };

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
                    let mut behaviors = Vec::new();
                    for channel_number in player.active_stage_behavior_channels() {
                        let Some((sprite_num, fallback)) = player
                            .movie
                            .score
                            .channels
                            .get(channel_number)
                            .map(|channel| {
                                (
                                    channel.sprite.number as u32,
                                    channel.sprite.script_instance_list.clone(),
                                )
                            })
                        else {
                            continue;
                        };

                        for behavior_ref in player.get_sprite_script_instance_ids(
                            sprite_num as i16,
                            fallback.as_slice(),
                        ) {
                            if player
                                .allocator
                                .get_script_instance_entry(behavior_ref.id())
                                .is_some_and(|entry| !entry.script_instance.begin_sprite_called)
                            {
                                behaviors.push((behavior_ref, sprite_num));
                            }
                        }
                    }
                    behaviors
                });

                // Initialize behavior default properties
                for (behavior_ref, sprite_num) in &behaviors_to_init {
                    if let Err(err) = Score::initialize_behavior_defaults_async(behavior_ref.clone(), *sprite_num).await {
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
                                        player.allocator.get_script_instance_entry_mut(script_ref.id())
                                    {
                                        entry.script_instance.begin_sprite_called = true;
                                    }
                                }
                            }
                        }
                    }
                });

                // Dispatch beginSprite to any remaining behaviors not handled above
                // (e.g., puppet sprites not in the score's sprite_spans)
                let remaining_behaviors: Vec<ScriptInstanceRef> = reserve_player_mut(|player| {
                    behaviors_to_init.iter()
                        .filter(|(behavior_ref, _)| {
                            player.allocator.get_script_instance_entry(behavior_ref.id())
                                .map_or(false, |entry| !entry.script_instance.begin_sprite_called)
                        })
                        .map(|(behavior_ref, _)| behavior_ref.clone())
                        .collect()
                });

                for behavior_ref in &remaining_behaviors {
                    let receivers = vec![behavior_ref.clone()];
                    let _ = player_invoke_targeted_event(
                        &"beginSprite".to_string(),
                        &vec![],
                        Some(&receivers),
                    ).await;
                }

                if !remaining_behaviors.is_empty() {
                    reserve_player_mut(|player| {
                        for behavior_ref in &remaining_behaviors {
                            if let Some(entry) =
                                player.allocator.get_script_instance_entry_mut(behavior_ref.id())
                            {
                                entry.script_instance.begin_sprite_called = true;
                            }
                        }
                    });
                }

                player_wait_available().await;

                // Note: stepFrame, prepareFrame, and enterFrame are NOT dispatched here.
                // The main frame loop handles those events after go() returns and
                // has_frame_changed_in_go is set. Dispatching them here would cause
                // re-entrant calls (e.g., stepFrame firing timers that call go() again).
            }
        }
        
        if frame_advanced {
            reserve_player_mut(|player| {
                player.has_frame_changed_in_go = true;
            });
        } else {
            // go(the frame) — stay on current frame
            // ONLY set go_same_frame, NOT has_frame_changed_in_go
            reserve_player_mut(|player| {
                player.go_same_frame = true;
                static GC: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let c = GC.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if c < 10 {
                    web_sys::console::log_1(&format!(
                        "[GO-SAME] frame={} dest={} call #{}",
                        player.movie.current_frame, destination_frame, c
                    ).into());
                }
            });
        }

        Ok(DatumRef::Void)
    }

    pub fn puppet_sprite(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let sprite_number = player.get_datum(&args[0]).int_value()?;
            let is_puppet = player.get_datum(&args[1]).int_value()? == 1;

            if !is_puppet {
                // When un-puppeting: if the channel has no score spans at all,
                // clear the sprite state (member, visible, etc.).
                // This matches Director's behavior: un-puppeting in a channel with no
                // score data reverts to score state which is empty for dynamic/pool sprites.
                // Without this, the Lingo clearSpritePool() iteration bug (list mutation
                // during iteration skips ~half the sprites) leaves stuck-puppeted sprites
                // that continue rendering in the next room.
                let channel_has_spans = player.movie.score.sprite_spans
                    .iter()
                    .any(|span| span.channel_number == sprite_number as u32);

                if !channel_has_spans {
                    let sprite = player.movie.score.get_sprite_mut(sprite_number as i16);
                    sprite.puppet = false;
                    sprite.member = None;
                    sprite.visible = true;
                    player.movie.score.invalidate_render_channel_cache();
                    player.refresh_stage_behavior_channel_cache_entry(sprite_number as i16);
                    player.invalidate_active_stage_filmloop_cache();
                    return Ok(DatumRef::Void);
                }
            }

            let sprite = player.movie.score.get_sprite_mut(sprite_number as i16);
            sprite.puppet = is_puppet;
            player.movie.score.invalidate_render_channel_cache();
            player.refresh_stage_behavior_channel_cache_entry(sprite_number as i16);
            player.invalidate_active_stage_filmloop_cache();
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
            let fallback = sprite.script_instance_list.clone();
            let receivers = player.get_sprite_script_instance_ids(
                sprite_num as i16,
                fallback.as_slice(),
            );
            Ok((message.clone(), remaining_args.clone(), receivers))
        })?;

        // sendSprite returns the return value of the first handler that handles the message
        let mut last_return_value = DatumRef::Void;
        let mut handled_by_sprite = false;
        for receiver in receivers {
            let handler_pair = reserve_player_ref(|player| {
                ScriptInstanceUtils::get_script_instance_handler(
                    &message,
                    &receiver,
                    player,
                )
            })?;

            if let Some(handler_ref) = handler_pair {
                match player_call_script_handler(Some(receiver), handler_ref, &remaining_args).await {
                    Ok(scope) => {
                        if !scope.passed {
                            handled_by_sprite = true;
                        }
                        // Capture the return value from the handler
                        if scope.return_value != DatumRef::Void {
                            last_return_value = scope.return_value;
                        }
                    }
                    Err(err) => {
                        if err.code != ScriptErrorCode::Abort {
                            web_sys::console::warn_1(
                                &format!("⚠ sendSprite continuing after error in handler '{}': {}", message, err.message).into()
                            );
                        } else {
                            return Err(err);
                        }
                    }
                }
            }
        }

        if !handled_by_sprite {
            player_invoke_static_event(&message, &remaining_args).await?;
        }

        Ok(last_return_value)
    }

    pub async fn send_all_sprites(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let (message, remaining_args, receivers) = reserve_player_mut(|player| {
            let message = player.get_datum(&args[0]).symbol_value()
                .map_err(|e| ScriptError::new(format!("sendAllSprites: invalid message: {:?}", e)))?;
            let remaining_args = &args[1..].to_vec();

            // Collect receivers from stage score
            let mut receivers: Vec<ScriptInstanceRef> = player.active_stage_script_instance_ids();

            // Also collect receivers from filmloop scores
            let active_filmloops = player.get_active_filmloop_scores();
            for (member_ref, filmloop_frame) in active_filmloops {
                if let Some(filmloop_score) = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .and_then(|member| match &member.member_type {
                        crate::player::cast_member::CastMemberType::FilmLoop(film_loop) => {
                            Some(&film_loop.score)
                        }
                        _ => None,
                    })
                {
                    let filmloop_receivers =
                        filmloop_score.get_active_script_instance_list_for_frame(filmloop_frame);
                    receivers.extend(filmloop_receivers);
                }
            }

            Ok((message.clone(), remaining_args.clone(), receivers))
        })?;
        
        let mut handled_by_sprite = false;
        let mut last_return_value = DatumRef::Void;
        for receiver in receivers {
            let handler_pair = reserve_player_ref(|player| {
                ScriptInstanceUtils::get_script_instance_handler(&message, &receiver, player)
            })?;
            if let Some(handler_ref) = handler_pair {
                match player_call_script_handler(Some(receiver), handler_ref, &remaining_args).await {
                    Ok(scope) => {
                        if !scope.passed {
                            handled_by_sprite = true;
                        }
                        if scope.return_value != DatumRef::Void {
                            last_return_value = scope.return_value;
                        }
                    }
                    Err(err) => {
                        web_sys::console::warn_1(
                            &format!("⚠ sendAllSprites continuing after error in handler: {}", err.message).into()
                        );
                    }
                }
            }
        }

        if !handled_by_sprite {
            player_invoke_static_event(&message, &remaining_args).await?;
        }

        Ok(last_return_value)
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
            let pref_name = player.get_datum(&args[0]).string_value()?;
            let storage = web_sys::window()
                .and_then(|w| w.local_storage().ok().flatten());
            if let Some(storage) = storage {
                let key = format!("dirplayer_pref_{}", pref_name);
                if let Ok(Some(value)) = storage.get_item(&key) {
                    return Ok(player.alloc_datum(Datum::String(value)));
                }
            }
            Ok(DatumRef::Void)
        })
    }

    pub fn set_pref(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let pref_name = player.get_datum(&args[0]).string_value()?;
            let pref_value = player.get_datum(&args[1]).string_value()?;
            let storage = web_sys::window()
                .and_then(|w| w.local_storage().ok().flatten());
            if let Some(storage) = storage {
                let key = format!("dirplayer_pref_{}", pref_name);
                let _ = storage.set_item(&key, &pref_value);
            }
            Ok(DatumRef::Void)
        })
    }

    pub fn go_to_net_page(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Ok(DatumRef::Void);
        }
        let (url, target) = reserve_player_ref(|player| {
            let url = player.get_datum(&args[0]).string_value()?;
            let target = if args.len() > 1 {
                player
                    .get_datum(&args[1])
                    .string_value()
                    .unwrap_or_else(|_| "_blank".to_string())
            } else {
                "_blank".to_string()
            };
            Ok::<(String, String), ScriptError>((url, target))
        })?;

        if let Some(code) = url.strip_prefix("javascript:") {
            // Defer via setTimeout(...,0) so the current WASM call stack
            // fully unwinds before the host-page JS runs. Synchronous
            // callbacks from the host back into WASM (e.g. openMixer
            // touching a wasm_bindgen closure) otherwise trip the
            // "closure invoked recursively or after being dropped" guard.
            let escaped = code.replace('\\', "\\\\").replace('\'', "\\'");
            let wrapper = format!(
                "setTimeout(function(){{try{{eval('{}');}}catch(e){{console.warn('gotoNetPage eval:',e);}}}},0);",
                escaped
            );
            if let Err(e) = js_sys::eval(&wrapper) {
                log::warn!("gotoNetPage: schedule eval failed: {:?}", e);
            }
        } else if let Some(window) = web_sys::window() {
            if let Err(e) = window.open_with_url_and_target(&url, &target) {
                log::warn!("gotoNetPage: window.open failed: {:?}", e);
            }
        }
        Ok(DatumRef::Void)
    }

    pub fn go_to_net_movie(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let raw_url = player.get_datum(&args[0]).string_value()?;

            // Parse URL and extract #fragment marker
            let (fetch_url, target) = if let Some(hash_pos) = raw_url.find('#') {
                let url_part = raw_url[..hash_pos].to_string();
                let fragment = raw_url[hash_pos + 1..].to_string();
                let target = if fragment.is_empty() {
                    MovieFrameTarget::Default
                } else {
                    MovieFrameTarget::Label(fragment)
                };
                (url_part, target)
            } else {
                (raw_url, MovieFrameTarget::Default)
            };

            // Start the network fetch (non-blocking)
            let task_id = player.net_manager.preload_net_thing(fetch_url.clone());

            // Store the pending operation (replaces any previous pending one, cancelling it)
            player.pending_goto_net_movie = Some((task_id, target));

            Ok(player.alloc_datum(Datum::Int(task_id as i32)))
        })
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

        let (has_player_frame_changed, current_frame) = reserve_player_ref(|player| {
            (player.has_player_frame_changed, player.movie.current_frame)
        });

        if already_updating || has_player_frame_changed {
            debug!("🔄 execute_frame_update SKIPPED (already_updating={}, frame_changed={}, frame={})", 
                already_updating, has_player_frame_changed, current_frame);
            return Ok(());  // Exit early if already updating
        }

        player_wait_available().await;

        // Sync all cached scriptInstanceLists back to sprite Vecs.
        // Behaviors added via scriptInstanceList.add() only exist in the cache
        // until synced — this ensures all event dispatch within this frame sees them.
        reserve_player_mut(|player| {
            player.sync_all_script_instance_lists();
        });

        reserve_player_mut(|player| {
            player.movie.score.apply_tween_modifiers(player.movie.current_frame);
        });

        // 1. Send stepFrame to actorList
        let (actor_list_snapshot, mut active_actor_ids, mut actor_list_generation) =
            reserve_player_ref(|player| player.actor_list_stepframe_snapshot());

        for (idx, actor_ref) in actor_list_snapshot.iter().enumerate() {
            let still_active = active_actor_ids.contains(&actor_ref.unwrap());

            if still_active {
                let result =
                    player_call_datum_handler(&actor_ref, &"stepFrame".to_string(), &vec![]).await;

                if let Err(err) = result {
                    if err.code == ScriptErrorCode::Abort {
                        reserve_player_mut(|player| {
                            player.is_in_frame_update = false;
                        });
                        return Err(err);
                    }
                    error!("⚠ stepFrame[{}] error: {}", idx, err.message);
                    reserve_player_mut(|player| {
                        player.on_script_error(&err);
                        player.is_in_frame_update = false;
                    });
                    return Err(err);
                }

                let refreshed_active_ids = reserve_player_ref(|player| {
                    if player.actor_list_generation != actor_list_generation {
                        Some(player.actor_list_active_ids())
                    } else {
                        None
                    }
                });

                if let Some((next_active_actor_ids, next_actor_list_generation)) =
                    refreshed_active_ids
                {
                    active_actor_ids = next_active_actor_ids;
                    actor_list_generation = next_actor_list_generation;
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

        // Skip mid-frame render for performance. Director renders once per
        // frame after enterFrame. The post-enterFrame render below captures
        // the final visual state. If a game needs mid-frame visibility,
        // it can call updateStage() explicitly from prepareFrame.
        // crate::rendering::draw_frame_immediate();

        reserve_player_mut(|player| {
            player.in_enter_frame = true;
        });

        dispatch_event_to_all_behaviors(&"enterFrame".to_string(), &vec![]).await;

        reserve_player_mut(|player| {
            player.in_enter_frame = false;
        });

        // enterFrame handlers are allowed to change the current visual state
        // (camera/model transforms, tunnelDepth, etc.) without calling
        // updateStage(), so flush one more redraw before the frame ends.
        crate::rendering::draw_frame_immediate();

        player_wait_available().await;

        reserve_player_mut(|player| {
            player.is_in_frame_update = false;
        });

        Ok(())
    }

    pub async fn update_stage(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let should_yield = reserve_player_ref(|player| {
            Ok(player.is_yield_safe() || player.command_handler_yielding || player.in_mouse_command)
        })?;

        // Director's updateStage() forces an immediate stage redraw even from
        // inside enterFrame/prepareFrame loops. Yielding to the browser event loop
        // is only needed for busy-wait input handlers.
        reserve_player_mut(|player| { player.stage_dirty = true; });
        crate::rendering::draw_frame_immediate();

        if should_yield {
            // Yield to allow the browser event loop to process pending events
            // (mouse up/move, keyboard, etc.). This is essential for scripts
            // using "repeat while the mouseDown" or similar busy-wait loops.
            // The mouse_up()/mouse_down() WASM exports update movie.mouse_down
            // immediately via reserve_player_mut, so the state is correct when
            // the script resumes. The MouseUp command stays queued and won't
            // dispatch until the current handler finishes.
            async_std::task::sleep(std::time::Duration::from_millis(2)).await;
        }

        Ok(DatumRef::Void)
    }

    pub async fn nothing_async(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let now = js_sys::Date::now();
        let should_yield = reserve_player_mut(|player| {
            // Only count calls inside frame scripts (busy-wait like waitABit).
            // Reset counter when not in a frame script to prevent accumulation
            // across unrelated nothing() calls (catalogue items, downloads, etc.)
            if player.in_frame_script {
                player.nothing_call_count += 1;
            } else {
                player.nothing_call_count = 0;
                return false;
            }
            let many_calls = player.nothing_call_count >= 50;
            let yield_due = now - player.last_nothing_yield_ms >= 16.0;
            many_calls && yield_due
        });

        if should_yield {
            reserve_player_mut(|player| {
                player.nothing_call_count = 0;
                player.last_nothing_yield_ms = now;
            });

            crate::rendering::draw_frame_immediate();

            async_std::task::sleep(std::time::Duration::from_millis(2)).await;
        }

        Ok(DatumRef::Void)
    }

    pub fn rollover(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if !args.is_empty() {
                // rollOver(spriteNum) - returns TRUE if the mouse is over the specified sprite
                let sprite_num = player.get_datum(&args[0]).int_value()?;
                let sprite = player.movie.score.get_sprite(sprite_num as i16);
                if let Some(sprite) = sprite {
                    let hit = concrete_sprite_hit_test(player, sprite, player.mouse_loc.0, player.mouse_loc.1);
                    let result = if hit { 1 } else { 0 };
                    Ok(player.alloc_datum(Datum::Int(result)))
                } else {
                    Ok(player.alloc_datum(Datum::Int(0)))
                }
            } else {
                // the rollOver - returns the sprite number under the mouse
                let sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
                Ok(player.alloc_datum(Datum::Int(sprite.unwrap_or(0) as i32)))
            }
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

    pub fn delay(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let ticks = player.get_datum(&args[0]).int_value()?;
            // Only set delay if not already delaying (prevent reset on every enterFrame)
            if ticks > 0 && player.delay_until.is_none() {
                let delay_ms = (ticks as f64) * (1000.0 / 60.0);
                player.delay_until = Some(
                    chrono::Local::now() + chrono::Duration::milliseconds(delay_ms as i64),
                );
            }
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
