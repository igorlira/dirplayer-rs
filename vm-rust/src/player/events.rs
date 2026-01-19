use async_std::channel::Receiver;
use log::{warn, debug};
use std::collections::HashSet;

use crate::{
    console_warn,
    director::lingo::datum::{Datum, VarRef},
    player::{
        handlers::datum_handlers::player_call_datum_handler, player_is_playing, reserve_player_mut,
        Score,
    },
};

use super::{
    cast_lib::CastMemberRef, handlers::datum_handlers::script_instance::ScriptInstanceUtils,
    player_call_script_handler, player_semaphone, reserve_player_ref, script::ScriptInstanceId,
    script_ref::ScriptInstanceRef, DatumRef, ScriptError, ScriptErrorCode, PLAYER_EVENT_TX,
    score::ScoreRef,
};

pub enum PlayerVMEvent {
    Global(String, Vec<DatumRef>),
    Targeted(String, Vec<DatumRef>, Option<Vec<ScriptInstanceRef>>),
    Callback(DatumRef, String, Vec<DatumRef>),
}

pub fn player_dispatch_global_event(handler_name: &String, args: &Vec<DatumRef>) {
    let tx = unsafe { PLAYER_EVENT_TX.clone() }.unwrap();
    tx.try_send(PlayerVMEvent::Global(
        handler_name.to_owned(),
        args.to_owned(),
    ))
    .unwrap();
}

pub fn player_dispatch_callback_event(
    receiver: DatumRef,
    handler_name: &String,
    args: &Vec<DatumRef>,
) {
    let tx = unsafe { PLAYER_EVENT_TX.clone() }.unwrap();
    tx.try_send(PlayerVMEvent::Callback(
        receiver,
        handler_name.to_owned(),
        args.to_owned(),
    ))
    .unwrap();
}

pub fn player_dispatch_targeted_event(
    handler_name: &String,
    args: &Vec<DatumRef>,
    instance_ids: Option<&Vec<ScriptInstanceRef>>,
) {
    let tx = unsafe { PLAYER_EVENT_TX.clone() }.unwrap();
    tx.try_send(PlayerVMEvent::Targeted(
        handler_name.to_owned(),
        args.to_owned(),
        instance_ids.map(|x| x.to_owned()),
    ))
    .unwrap();
}

pub fn player_dispatch_event_to_sprite(
    handler_name: &String,
    args: &Vec<DatumRef>,
    sprite_num: u16,
) {
    let instance_ids = reserve_player_ref(|player| {
        let sprite = player.movie.score.get_sprite(sprite_num as i16);
        if let Some(sprite) = sprite {
            let instance_ids = sprite.script_instance_list.clone();
            Some(instance_ids)
        } else {
            None
        }
    });
    if instance_ids.is_none() {
        return;
    }
    let instance_ids = instance_ids.unwrap();
    let tx = unsafe { PLAYER_EVENT_TX.clone() }.unwrap();
    tx.try_send(PlayerVMEvent::Targeted(
        handler_name.to_owned(),
        args.to_owned(),
        Some(instance_ids),
    ))
    .unwrap();
}

pub async fn player_dispatch_event_to_sprite_targeted(
    handler_name: &String,
    args: &Vec<DatumRef>,
    sprite_num: u16,
) {
    let instance_ids = reserve_player_ref(|player| {
        player
            .movie
            .score
            .get_sprite(sprite_num as i16)
            .map(|sprite| sprite.script_instance_list.clone())
    });
    let Some(instance_ids) = instance_ids else {
        return;
    };

    player_wait_available().await;

     for instance_id in instance_ids {
        player_invoke_targeted_event(
            handler_name,
            args,
            Some(&vec![instance_id].as_ref()),
        ).await;
    }
}

pub async fn player_invoke_event_to_instances(
    handler_name: &String,
    args: &Vec<DatumRef>,
    instance_refs: &Vec<ScriptInstanceRef>,
) -> Result<bool, ScriptError> {
    let recv_instance_handlers = reserve_player_ref(|player| {
        let mut result = vec![];
        for instance_ref in instance_refs {
            let handler_pair = ScriptInstanceUtils::get_script_instance_handler(
                &handler_name,
                instance_ref,
                player,
            )?;
            if let Some(handler_pair) = handler_pair {
                result.push((instance_ref.clone(), handler_pair));
            }
        }
        Ok(result)
    })?;
    
    let mut handled = false;
    for (script_instance_ref, handler_ref) in recv_instance_handlers {
        match player_call_script_handler(Some(script_instance_ref), handler_ref, args).await {
            Ok(scope) => {
                if !scope.passed {
                    handled = true;
                    break;
                }
            }
            Err(err) => {
                // Dump bytecode execution history before the error
                crate::player::bytecode::handler_manager::dump_execution_history_on_error(&err.message);
                // Log the error to console
                web_sys::console::error_1(
                    &format!("⚠ Error in handler '{}': {}", handler_name, err.message).into()
                );
                // Report to player's error handler
                reserve_player_mut(|player| {
                    player.on_script_error(&err);
                });
                // Return the error to caller
                return Err(err);
            }
        }
    }
    
    Ok(handled)
}

pub async fn player_invoke_frame_and_movie_scripts(
    handler_name: &String,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let active_static_scripts = reserve_player_mut(|player| {
        let frame_script = player
            .movie
            .score
            .get_script_in_frame(player.movie.current_frame);
        let movie_scripts = player.movie.cast_manager.get_movie_scripts();
        let movie_scripts = movie_scripts.as_ref().unwrap();

        let mut active_static_scripts: Vec<CastMemberRef> = vec![];
        
        // Frame script first
        if let Some(frame_script) = frame_script {
            let script_ref = CastMemberRef {
                cast_lib: frame_script.cast_lib.into(),
                cast_member: frame_script.cast_member.into(),
            };
            active_static_scripts.push(script_ref);
        }
        
        // Then movie scripts
        for movie_script in movie_scripts {
            active_static_scripts.push(movie_script.member_ref.to_owned());
        }
        
        active_static_scripts
    });

    for script_member_ref in active_static_scripts {
        let has_handler = reserve_player_ref(|player| {
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(&script_member_ref);
            let handler = script.and_then(|x| x.get_handler(handler_name));
            handler.is_some()
        });
        if !has_handler {
            continue;
        }
        
        // NEW: Check if this is the frame script
        let receiver = reserve_player_ref(|player| {
            if player.movie.frame_script_member.as_ref() == Some(&script_member_ref) {
                player.movie.frame_script_instance.clone()
            } else {
                None
            }
        });
        
        let result = player_call_script_handler(
            receiver,  // Changed from None to receiver
            (script_member_ref, handler_name.to_owned()),
            args
        ).await?;

        if !result.passed {
            break;
        }
    }
    Ok(DatumRef::Void)
}

pub async fn player_invoke_targeted_event(
    handler_name: &String,
    args: &Vec<DatumRef>,
    instance_refs: Option<&Vec<ScriptInstanceRef>>,
) -> Result<DatumRef, ScriptError> {
    let handled = match instance_refs {
        Some(instance_refs) => {
            player_invoke_event_to_instances(handler_name, args, instance_refs).await?
        }
        None => false,
    };
    if !handled {
        player_invoke_static_event(handler_name, args).await?;
    }
    Ok(DatumRef::Void)
}

pub async fn player_invoke_static_event(
    handler_name: &String,
    args: &Vec<DatumRef>,
) -> Result<bool, ScriptError> {
    let active_static_scripts = reserve_player_mut(|player| {
        let frame_script = player
            .movie
            .score
            .get_script_in_frame(player.movie.current_frame);
        let movie_scripts = player.movie.cast_manager.get_movie_scripts();
        let movie_scripts = movie_scripts.as_ref().unwrap();
        let mut active_static_scripts: Vec<CastMemberRef> = vec![];
        if let Some(frame_script) = frame_script {
            let script_ref = CastMemberRef {
                cast_lib: frame_script.cast_lib.into(),
                cast_member: frame_script.cast_member.into(),
            };
            active_static_scripts.push(script_ref);
        }
        for movie_script in movie_scripts {
            active_static_scripts.push(movie_script.member_ref.to_owned());
        }
        active_static_scripts
    });

    let mut handled = false;
    for script_member_ref in active_static_scripts {
        let has_handler = reserve_player_ref(|player| {
            let script = player
                .movie
                .cast_manager
                .get_script_by_ref(&script_member_ref);
            let handler = script.and_then(|x| x.get_handler(handler_name));
            handler.is_some()
        });
        if !has_handler {
            continue;
        }

        // NEW: Check if this is the frame script
        let receiver = reserve_player_ref(|player| {
            if player.movie.frame_script_member.as_ref() == Some(&script_member_ref) {
                player.movie.frame_script_instance.clone()
            } else {
                None
            }
        });

        let result = player_call_script_handler(
            receiver,  // Changed from None to receiver
            (script_member_ref, handler_name.to_owned()),
            args
        ).await?;

        if !result.passed {
            handled = true;
            break;
        }
    }
    Ok(handled)
}

pub async fn player_invoke_global_event(
    handler_name: &String,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    // First stage behavior script
    // Then frame behavior script
    // Then movie script
    // If frame is changed during exitFrame, event is no longer propagated
    // TODO find stage behaviors first

    let active_instance_scripts = reserve_player_mut(|player| {
        let mut active_instance_scripts: Vec<ScriptInstanceRef> = vec![];
        active_instance_scripts.extend(player.movie.score.get_active_script_instance_list());
        for global in player.get_hydrated_globals().values() {
            match global {
                Datum::VarRef(VarRef::ScriptInstance(script_instance_ref)) => {
                    active_instance_scripts.push(script_instance_ref.clone());
                }
                Datum::ScriptInstanceRef(script_instance_ref) => {
                    active_instance_scripts.push(script_instance_ref.clone());
                }
                _ => {}
            }
        }

        active_instance_scripts.to_owned()
    });

    let handled =
        player_invoke_event_to_instances(handler_name, args, &active_instance_scripts).await?;
    if handled {
        return Ok(DatumRef::Void);
    }
    player_invoke_static_event(handler_name, args).await?;

    Ok(DatumRef::Void)
}

pub async fn run_event_loop(rx: Receiver<PlayerVMEvent>) {
    warn!("Starting event loop");
    while !rx.is_closed() {
        let item = rx.recv().await.unwrap();
        player_wait_available().await;
        if !player_is_playing().await {
            continue;
        }
        let result = match item {
            PlayerVMEvent::Global(name, args) => player_invoke_global_event(&name, &args).await,
            PlayerVMEvent::Targeted(name, args, instances) => {
                player_invoke_targeted_event(&name, &args, instances.as_ref()).await
            }
            PlayerVMEvent::Callback(receiver, name, args) => {
                player_call_datum_handler(&receiver, &name, &args).await
            }
        };
        match result {
            Err(err) => {
                // TODO ignore error if it's a CancelledException
                // TODO print stack trace
                reserve_player_mut(|player| player.on_script_error(&err));
            }
            _ => {}
        };
    }
    warn!("Event loop stopped!")
}

pub fn player_unwrap_result(result: Result<DatumRef, ScriptError>) -> DatumRef {
    match result {
        Ok(result) => result,
        Err(err) => {
            reserve_player_mut(|player| player.on_script_error(&err));
            DatumRef::Void
        }
    }
}

pub async fn player_dispatch_event_beginsprite(
    handler_name: &String,
    args: &Vec<DatumRef>
) -> Result<Vec<(ScoreRef, u32)>, ScriptError> {
    // Prevent re-entrant beginSprite dispatch (can happen when go() is called during frame update)
    let skip = reserve_player_mut(|player| {
        if player.is_in_beginsprite {
            web_sys::console::warn_1(&format!(
                "Blocking re-entrant beginSprite dispatch"
            ).into());
            return true;
        }
        player.is_in_beginsprite = true;
        false
    });

    if skip {
        return Ok(Vec::new());
    }

    let (mut sprite_instances, mut frame_instances, all_channels) =
        reserve_player_mut(|player| {
            let mut sprite_instances: Vec<(usize, ScriptInstanceRef)> = Vec::new();
            let mut frame_instances: Vec<(usize, ScriptInstanceRef)> = Vec::new();
            let mut all_channels = Vec::new();

            // Collect stage sprites
            let active_channel_numbers: HashSet<u32> = player.movie.score.sprite_spans
                .iter()
                .filter(|span| Score::is_span_in_frame(span, player.movie.current_frame))
                .map(|span| span.channel_number as u32)
                .collect();

            let filtered_channels: Vec<_> = player.movie.score.channels.iter()
                .filter(|channel| !channel.sprite.script_instance_list.is_empty())
                .filter(|channel| channel.sprite.entered)
                .filter(|channel| active_channel_numbers.contains(&(channel.number as u32)))
                .filter(|channel| {
                    channel.sprite.script_instance_list.iter().all(|script_ref| {
                        player
                            .allocator
                            .script_instances
                            .get(&script_ref.id())
                            .map_or(false, |entry| !entry.script_instance.begin_sprite_called)
                    })
                })
                .collect();

            for channel in filtered_channels {
                let instances = channel.sprite.script_instance_list.clone();

                if channel.number == 0 {
                    // Frame behavior (channel 0)
                    frame_instances.extend(
                        instances.into_iter().map(|inst| (channel.number, inst))
                    );
                } else {
                    // Sprite behaviors (channel > 0)
                    sprite_instances.extend(
                        instances.into_iter().map(|inst| (channel.number, inst))
                    );
                }

                all_channels.push((ScoreRef::Stage, channel.number as u32));
            }

            // Collect filmloop sprites
            let active_filmloops = player.get_active_filmloop_scores();
            for (member_ref, filmloop_score) in active_filmloops {
                // Get the filmloop's current frame
                let filmloop_current_frame = match player.movie.cast_manager.find_member_by_ref(&member_ref) {
                    Some(member) => {
                        if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &member.member_type {
                            film_loop.current_frame
                        } else {
                            continue; // Not a filmloop, skip
                        }
                    }
                    None => continue, // Member not found, skip
                };

                let active_filmloop_channels: HashSet<u32> = filmloop_score.sprite_spans
                    .iter()
                    .filter(|span| Score::is_span_in_frame(span, filmloop_current_frame))
                    .map(|span| span.channel_number as u32)
                    .collect();

                let filtered_filmloop_channels: Vec<_> = filmloop_score.channels.iter()
                    .filter(|channel| !channel.sprite.script_instance_list.is_empty())
                    .filter(|channel| channel.sprite.entered)
                    .filter(|channel| active_filmloop_channels.contains(&(channel.number as u32)))
                    .filter(|channel| {
                        channel.sprite.script_instance_list.iter().all(|script_ref| {
                            player
                                .allocator
                                .script_instances
                                .get(&script_ref.id())
                                .map_or(false, |entry| !entry.script_instance.begin_sprite_called)
                        })
                    })
                    .collect();

                for channel in filtered_filmloop_channels {
                    let instances = channel.sprite.script_instance_list.clone();

                    // Filmloop sprites go into sprite_instances (they don't have frame behaviors)
                    if channel.number > 0 {
                        sprite_instances.extend(
                            instances.into_iter().map(|inst| (channel.number, inst))
                        );
                        all_channels.push((ScoreRef::FilmLoop(member_ref.clone()), channel.number as u32));
                    }
                }
            }

            (sprite_instances, frame_instances, all_channels)
        });
    
    if sprite_instances.is_empty() && frame_instances.is_empty() {
        return Ok(Vec::new());
    }
    
    if frame_instances.len() > 0 {
        let _ = player_invoke_frame_and_movie_scripts(
            handler_name,
            args,
        )
        .await;
    }
    
    // Dispatch to sprite behaviors (number > 0)
    for (sprite_number, behavior) in sprite_instances {
        let receivers = vec![behavior.clone()];
        if let Err(err) = player_invoke_targeted_event(handler_name, args, Some(receivers).as_ref()).await {
            web_sys::console::error_1(
                &format!("Error in {} for sprite {}: {}", handler_name, sprite_number, err.message).into()
            );
            reserve_player_mut(|player| {
                player.on_script_error(&err);
            });
        }
    }

    // Reset the re-entrancy guard
    reserve_player_mut(|player| {
        player.is_in_beginsprite = false;
    });

    Ok(all_channels)
}

pub async fn dispatch_event_endsprite(sprite_nums: Vec<u32>) {
    // Legacy function - calls the new implementation with stage score
    dispatch_event_endsprite_for_score(ScoreRef::Stage, sprite_nums).await;
}

pub async fn dispatch_event_endsprite_for_score(score_ref: ScoreRef, sprite_nums: Vec<u32>) {
    // Prevent re-entrant endSprite dispatch (can happen when go() is called during frame update)
    let skip = reserve_player_mut(|player| {
        if player.is_in_endsprite {
            web_sys::console::warn_1(&format!(
                "Blocking re-entrant endSprite dispatch for {} sprites",
                sprite_nums.len()
            ).into());
            return true;
        }
        player.is_in_endsprite = true;
        false
    });

    if skip {
        return;
    }

    let (sprite_tuple, frame_tuple) =
        reserve_player_mut(|player| {
            let mut sprite_tuple = Vec::new();
            let mut frame_tuple = Vec::new();

            // Get the appropriate score based on score_ref
            let score = match &score_ref {
                ScoreRef::Stage => &player.movie.score,
                ScoreRef::FilmLoop(member_ref) => {
                    match player.movie.cast_manager.find_member_by_ref(member_ref) {
                        Some(member) => {
                            if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &member.member_type {
                                &film_loop.score
                            } else {
                                return (sprite_tuple, frame_tuple); // Not a filmloop, return empty
                            }
                        }
                        None => return (sprite_tuple, frame_tuple), // Member not found, return empty
                    }
                }
            };

            for channel in score.channels.iter() {
                // Skip if channel is not active in current frame (for sprite channels)
                if !sprite_nums.contains(&(channel.number as u32)) {
                    continue;
                }

                // Skip channels with no sprite instances
                if channel.sprite.script_instance_list.is_empty() {
                    continue;
                }

                let entry = (
                    channel.sprite.number as u16,
                    channel.sprite.script_instance_list.clone(),
                );

                if channel.number > 0 {
                    // Sprite channels (only those active in current frame)
                    sprite_tuple.push(entry);
                } else {
                    // Frame channel (number == 0, always included)
                    frame_tuple.push(entry);
                }
            }

            (sprite_tuple, frame_tuple)
        });

    // Dispatch to frame behaviors first (number == 0)
    if frame_tuple.len() > 0 {
        let _ = player_invoke_frame_and_movie_scripts(&"endSprite".to_string(), &vec![]).await;
    }

    // Dispatch to sprite behaviors (number > 0)
    for (sprite_num, behaviors) in sprite_tuple {
        for behavior in behaviors {
            let receivers = vec![behavior.clone()];

            if let Err(err) = player_invoke_event_to_instances(
                    &"endSprite".to_string(), &vec![], &receivers
                ).await {
                web_sys::console::error_1(
                    &format!("Error in endSprite for sprite {}: {}", sprite_num, err.message).into()
                );
                reserve_player_mut(|player| {
                    player.on_script_error(&err);
                });
            }
        }
    }

    // Reset the re-entrancy guard
    reserve_player_mut(|player| {
        player.is_in_endsprite = false;
    });
}

pub async fn dispatch_event_to_all_behaviors(
    handler_name: &String,
    args: &Vec<DatumRef>,
) {
    use crate::player::allocator::ScriptInstanceAllocatorTrait;
    use crate::js_api::ascii_safe;
    // Skip event dispatch if we're initializing behavior properties
    let skip = reserve_player_mut(|player| {
        if player.is_initializing_behavior_props {
            web_sys::console::warn_1(&format!(
                "Blocking event '{}' during property initialization",
                handler_name
            ).into());
            return true;
        }
        // Prevent re-entrant event dispatch (this can cause infinite loops)
        if player.is_dispatching_events {
            web_sys::console::warn_1(&format!(
                "Blocking re-entrant event dispatch for '{}'",
                handler_name
            ).into());
            return true;
        }
        player.is_dispatching_events = true;
        false
    });

    if skip {
        return;
    }
    let (sprite_behaviors, frame_behaviors) = reserve_player_mut(|player| {
        let mut sprites = Vec::new();
        let mut frames = Vec::new();

        // Collect stage sprites
        let active_channel_numbers: HashSet<u32> = player.movie.score.sprite_spans
            .iter()
            .filter(|span| Score::is_span_in_frame(span, player.movie.current_frame))
            .map(|span| span.channel_number as u32)
            .collect();
        for channel in player.movie.score.channels.iter() {
            if channel.sprite.script_instance_list.is_empty() || !channel.sprite.entered ||
                !active_channel_numbers.contains(&(channel.number as u32)) {
                continue;
            }
            let behaviors = channel.sprite.script_instance_list.clone();
            if channel.number > 0 {
                sprites.push((channel.number, behaviors));  // Store tuple with channel number
            } else if channel.number == 0 {
                frames.push((channel.number, behaviors));  // Store tuple with channel number
            }
        }

        // Collect filmloop sprites
        let active_filmloops = player.get_active_filmloop_scores();
        for (member_ref, filmloop_score) in active_filmloops {
            // Get the filmloop's current frame
            let filmloop_current_frame = match player.movie.cast_manager.find_member_by_ref(&member_ref) {
                Some(member) => {
                    if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &member.member_type {
                        film_loop.current_frame
                    } else {
                        continue; // Not a filmloop, skip
                    }
                }
                None => continue, // Member not found, skip
            };

            let active_filmloop_channels: HashSet<u32> = filmloop_score.sprite_spans
                .iter()
                .filter(|span| Score::is_span_in_frame(span, filmloop_current_frame))
                .map(|span| span.channel_number as u32)
                .collect();

            for channel in filmloop_score.channels.iter() {
                if channel.sprite.script_instance_list.is_empty() || !channel.sprite.entered ||
                    !active_filmloop_channels.contains(&(channel.number as u32)) {
                    continue;
                }
                let behaviors = channel.sprite.script_instance_list.clone();
                if channel.number > 0 {
                    sprites.push((channel.number, behaviors));  // Store tuple with channel number
                }
            }
        }

        (sprites, frames)
    });
    // Dispatch to sprite behaviors first (channel order)
    for (sprite_number, behaviors) in sprite_behaviors {
        for behavior in behaviors {
            let (script_name, instance_id, scope_count) = reserve_player_ref(|player| {
                let script_instance = player.allocator.get_script_instance(&behavior);
                let name = player.movie.cast_manager
                    .get_script_by_ref(&script_instance.script)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                (name, script_instance.instance_id, player.scope_count)
            });
            debug!(
                "Invoking '{}' on sprite {} behavior '{}' (instance #{}) scope_count {}", 
                handler_name,
                sprite_number,
                ascii_safe(&script_name.to_string()),
                instance_id,
                scope_count
            );
            let receivers = vec![behavior.clone()];

            if let Err(err) = player_invoke_event_to_instances(handler_name, args, &receivers).await {
                web_sys::console::error_1(
                    &format!("Error in {} for sprite {}: {}", handler_name, sprite_number, err.message).into()
                );
                reserve_player_mut(|player| {
                    player.on_script_error(&err);
                });
            }
        }
    }
    // Dispatch event to frame/movie scripts
    let _ = player_invoke_frame_and_movie_scripts(handler_name, args).await;

    // Reset the flag after dispatching
    reserve_player_mut(|player| {
        player.is_dispatching_events = false;
    });
}

pub async fn player_wait_available() {
    player_semaphone().lock().await;
}

/// Dispatch system events to all timeout targets
/// System events include: prepareMovie, startMovie, stopMovie, prepareFrame, exitFrame
pub async fn dispatch_system_event_to_timeouts(
    handler_name: &String,
    args: &Vec<DatumRef>,
) {
    // Get all timeout targets that are currently scheduled
    let timeout_targets = reserve_player_ref(|player| {
        let mut targets = Vec::new();
        for (_timeout_name, timeout) in player.timeout_manager.timeouts.iter() {
            if timeout.is_scheduled {
                targets.push(timeout.target_ref.clone());
            }
        }
        targets
    });

    // Dispatch the event to each timeout target
    for target_ref in timeout_targets {
        let result = player_call_datum_handler(&target_ref, handler_name, args).await;
        if let Err(err) = result {
            // HandlerNotFound is expected when a script doesn't have the event handler
            // (e.g., timeout target script doesn't have prepareFrame or exitFrame).
            // This is normal Director behavior - just silently skip.
            if err.code != ScriptErrorCode::HandlerNotFound {
                // Log actual errors but continue with other timeouts
                web_sys::console::error_1(
                    &format!("⚠ Timeout system event {} error: {}", handler_name, err.message
                ).into());
            }
        }
    }
}
