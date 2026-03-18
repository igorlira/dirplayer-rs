use std::collections::HashMap;

use async_std::{channel::Receiver, task::spawn_local};
use chrono::Local;
use log::{warn, debug};
use manual_future::ManualFuture;
use url::Url;

use crate::{
    console_warn,
    director::lingo::datum::{Datum, TimeoutRef},
    js_api::JsApi,
    player::PLAYER_OPT,
    utils::{log_i, ToHexString},
};

use super::{
    allocator::ScriptInstanceAllocatorTrait,
    cast_lib::CastMemberRef,
    cast_member::CastMemberType,
    datum_ref::DatumRef,
    events::{
        player_dispatch_callback_event, player_dispatch_event_to_sprite,
        player_dispatch_movie_callback, player_dispatch_targeted_event, player_wait_available,
        player_dispatch_event_to_sprite_targeted, player_invoke_frame_and_movie_scripts,
    },
    font::player_load_system_font,
    keyboard_events::{player_key_down, player_key_up},
    player_alloc_datum, player_call_script_handler, player_dispatch_global_event,
    player_is_playing, reserve_player_mut, reserve_player_ref,
    score::{concrete_sprite_hit_test, get_concrete_sprite_rect, get_sprite_at, is_active_sprite},
    script_ref::ScriptInstanceRef,
    PlayerVMExecutionItem, ScriptError, ScriptReceiver, PLAYER_TX,
};

#[allow(dead_code)]
pub enum PlayerVMCommand {
    LoadMovieFromFile(String, bool),
    SetExternalParams(HashMap<String, String>),
    SetBasePath(String),
    SetMoviePathOverride(String),
    SetSystemFontPath(String),
    SetStageSize(u32, u32),
    TimeoutTriggered(TimeoutRef),
    PrintMemberBitmapHex(CastMemberRef),
    MouseDown((i32, i32)),
    MouseUp((i32, i32)),
    MouseMove((i32, i32)),
    KeyDown(String, u16),
    KeyUp(String, u16),
    TriggerAlertHook,
    // Flash-to-Lingo callback mechanism
    TriggerFlashCallback {
        sprite_num: i32,
        handler_name: String,
        args: Vec<DatumRef>,
    },
    TriggerLingoCallbackOnScript {
        cast_lib: i32,
        cast_member: i32,
        handler_name: String,
        args: Vec<DatumRef>,
    },
    SetLingoScriptProperty {
        cast_lib: i32,
        cast_member: i32,
        prop_name: String,
        value: DatumRef,
    },
}

pub fn _format_player_cmd(command: &PlayerVMCommand) -> String {
    match command {
        PlayerVMCommand::LoadMovieFromFile(path, autoplay) => format!("LoadMovieFromFile({}, {})", path, autoplay),
        PlayerVMCommand::SetExternalParams(params) => {
            format!("SetExternalParams({:?})", params.keys().collect::<Vec<_>>())
        }
        PlayerVMCommand::SetBasePath(path) => format!("SetBasePath({})", path),
        PlayerVMCommand::SetMoviePathOverride(path) => format!("SetMoviePathOverride({})", path),
        PlayerVMCommand::SetSystemFontPath(path) => format!("SetSystemFontPath({})", path),
        PlayerVMCommand::SetStageSize(width, height) => {
            format!("SetStageSize({}, {})", width, height)
        }
        PlayerVMCommand::TimeoutTriggered(timeout_ref) => {
            format!("TimeoutTriggered({})", timeout_ref)
        }
        PlayerVMCommand::PrintMemberBitmapHex(..) => "PrintMemberBitmapHex(..)".to_string(),
        PlayerVMCommand::MouseDown((x, y)) => format!("MouseDown({}, {})", x, y),
        PlayerVMCommand::MouseUp((x, y)) => format!("MouseUp({}, {})", x, y),
        PlayerVMCommand::MouseMove((x, y)) => format!("MouseMove({}, {})", x, y),
        PlayerVMCommand::KeyDown(key, ..) => format!("KeyDown({})", key),
        PlayerVMCommand::KeyUp(key, ..) => format!("KeyUp({})", key),
        PlayerVMCommand::TriggerAlertHook => "TriggerAlertHook".to_string(),
        PlayerVMCommand::TriggerFlashCallback { sprite_num, handler_name, .. } => {
            format!("TriggerFlashCallback(sprite: {}, handler: {})", sprite_num, handler_name)
        }
        PlayerVMCommand::TriggerLingoCallbackOnScript { cast_lib, cast_member, handler_name, .. } => {
            format!("TriggerLingoCallbackOnScript(cast_lib: {}, cast_member: {}, handler: {})", cast_lib, cast_member, handler_name)
        }
        PlayerVMCommand::SetLingoScriptProperty { cast_lib, cast_member, prop_name, .. } => {
            format!("SetLingoScriptProperty(cast_lib: {}, cast_member: {}, prop: {})", cast_lib, cast_member, prop_name)
        }
    }
}

/// Check if a movie callback script (mouseDownScript, mouseUpScript, etc.)
/// contains actual executable content. Comments like "--nothing" are stored
/// but don't block event propagation in Director.
fn has_executable_callback(callback: &Option<ScriptReceiver>) -> bool {
    match callback {
        Some(ScriptReceiver::ScriptText(text)) => {
            let trimmed = text.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--")
        }
        Some(_) => true, // ScriptInstance or Script refs are always executable
        None => false,
    }
}

pub async fn run_command_loop(rx: Receiver<PlayerVMExecutionItem>) {
    warn!("Starting command loop");

    while !rx.is_closed() {
        let item = rx.recv().await.unwrap();
        let result = run_player_command(item.command).await;
        match result {
            Ok(result) => {
                if let Some(completer) = item.completer {
                    completer.complete(Ok(result)).await;
                }
            }
            Err(err) => {
                if err.code == super::ScriptErrorCode::Abort {
                    // abort is a normal control flow mechanism, not an error
                    if let Some(completer) = item.completer {
                        completer.complete(Ok(DatumRef::Void)).await;
                    }
                } else {
                    // TODO ignore error if it's a CancelledException
                    // TODO print stack trace
                    reserve_player_mut(|player| player.on_script_error(&err));
                    if let Some(completer) = item.completer {
                        completer.complete(Err(err)).await;
                    }
                }
            }
        }
    }
    warn!("Command loop stopped!")
}

pub fn player_dispatch(command: PlayerVMCommand) {
    if let Some(tx) = unsafe { PLAYER_TX.clone() } {
        if let Err(e) = tx.try_send(PlayerVMExecutionItem {
            command,
            completer: None,
        }) {
            // The channel is closed or full
            eprintln!("Failed to send command to player: {:?}", e);
        }
    } else {
        eprintln!("PLAYER_TX not initialized");
    }
}

#[allow(dead_code)]
pub async fn player_dispatch_async(command: PlayerVMCommand) -> Result<DatumRef, ScriptError> {
    let tx = unsafe { PLAYER_TX.clone() }.unwrap();
    let (future, completer) = ManualFuture::new();
    let item = PlayerVMExecutionItem {
        command,
        completer: Some(completer),
    };
    tx.send(item).await.unwrap();
    future.await
}

pub async fn run_player_command(command: PlayerVMCommand) -> Result<DatumRef, ScriptError> {
    player_wait_available().await;
    match command {
        PlayerVMCommand::SetExternalParams(params) => {
            reserve_player_mut(|player| {
                player.external_params = params;
            });
        }
        PlayerVMCommand::SetBasePath(path) => {
            reserve_player_mut(|player| {
                player.net_manager.set_base_path(Url::parse(&path).unwrap());
            });
        }
        PlayerVMCommand::SetMoviePathOverride(path) => {
            reserve_player_mut(|player| {
                player.movie_path_override = if path.is_empty() { None } else { Some(path) };
            });
        }
        PlayerVMCommand::SetSystemFontPath(path) => {
            console_warn!("Loading system font: {}", path);
            player_load_system_font(&path).await;
        }
        PlayerVMCommand::LoadMovieFromFile(file_path, autoplay) => {
            let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
            player.load_movie_from_file(&file_path).await;
            if autoplay {
                player.play();
            }
        }
        PlayerVMCommand::SetStageSize(width, height) => {
            reserve_player_mut(|player| {
                player.stage_size = (width, height);
            });
        }
        PlayerVMCommand::TimeoutTriggered(timeout_ref) => {
            let (is_found, is_playing, is_script_paused, target_ref, handler_name, timeout_name) =
                reserve_player_mut(|player| {
                    if let Some(timeout) = player.timeout_manager.get_timeout(&timeout_ref) {
                        let is_playing = player.is_playing;
                        let is_script_paused = player.is_script_paused;
                        (
                            true,
                            is_playing,
                            is_script_paused,
                            timeout.target_ref.clone(),
                            timeout.handler.to_owned(),
                            timeout.name.to_owned(),
                        )
                    } else {
                        (
                            false,
                            false,
                            false,
                            DatumRef::Void,
                            "".to_string(),
                            "".to_string(),
                        )
                    }
                });
            if !is_found {
                warn!("Timeout triggered but not found: {}", timeout_ref);
                return Ok(DatumRef::Void);
            }
            if !is_playing || is_script_paused {
                // TODO how to handle is_script_paused?
                warn!("Timeout triggered but not playing");
                return Ok(DatumRef::Void);
            }
            let ref_datum = player_alloc_datum(Datum::TimeoutRef(timeout_name));
            let args = vec![ref_datum];
            if target_ref != DatumRef::Void {
                player_dispatch_callback_event(target_ref, &handler_name, &args);
            } else {
                player_dispatch_global_event(&handler_name, &args);
            }
        }
        PlayerVMCommand::PrintMemberBitmapHex(member_ref) => {
            reserve_player_ref(|player| {
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let bitmap = member.member_type.as_bitmap().unwrap();
                let bitmap = player.bitmap_manager.get_bitmap(bitmap.image_ref).unwrap();
                let bitmap = &bitmap.data;
                warn!("Bitmap hex: {}", bitmap.to_hex_string());
            });
        }
        PlayerVMCommand::MouseDown((x, y)) => {
            if !player_is_playing().await {
                return Ok(DatumRef::Void);
            }
            // In Director, mouseDownScript intercepts BEFORE sprites get the event.
            // Only block when it contains executable content (not just a comment).
            // Comments like "--nothing" are stored but don't block propagation.
            let mouse_down_script_active = reserve_player_ref(|player| {
                has_executable_callback(&player.movie.mouse_down_script)
            });
            if mouse_down_script_active {
                reserve_player_mut(|player| {
                    let now = Local::now().timestamp_millis().abs();
                    let is_double_click = (now - player.last_mouse_down_time) < 500;
                    player.mouse_loc = (x, y);
                    player.movie.mouse_down = true;
                    player.movie.click_loc = (x, y);
                    player.is_double_click = is_double_click;
                    player.last_mouse_down_time = now;
                });
                player_dispatch_movie_callback("mouseDown").await?;
                return Ok(DatumRef::Void);
            }

            // Use scripted=true so only sprites with scripts (behavior or cast member)
            // are detected. Non-scripted sprites (decorations, overlays) are skipped,
            // matching Director behavior.
            reserve_player_mut(|player| {
                let now = Local::now().timestamp_millis().abs();
                let is_double_click = (now - player.last_mouse_down_time) < 500;
                player.mouse_loc = (x, y);
                player.movie.mouse_down = true;
                player.movie.click_loc = (x, y);
                player.is_double_click = is_double_click;
                player.last_mouse_down_time = now;

                // "the clickOn" should return the topmost sprite at the click point
                // regardless of whether it has a script — use unscripted lookup.
                let any_sprite = get_sprite_at(player, x, y, false);
                if let Some(sprite_number) = any_sprite {
                    player.click_on_sprite = sprite_number as i16;
                    // Capture drag offset for moveable sprites (so sprite doesn't jump to cursor)
                    if let Some(sprite) = player.movie.score.get_sprite(sprite_number as i16) {
                        if sprite.moveable {
                            player.drag_offset = (sprite.loc_h - x, sprite.loc_v - y);
                        }
                    }
                    let sprite = player.movie.score.get_sprite(sprite_number as i16);
                    let sprite_member = sprite
                        .and_then(|x| x.member.as_ref())
                        .and_then(|x| player.movie.cast_manager.find_member_by_ref(&x));
                    if let Some(sprite_member) = sprite_member {
                        match &sprite_member.member_type {
                            CastMemberType::Field(field_member) => {
                                if field_member.editable {
                                    player.keyboard_focus_sprite = sprite_number as i16;
                                }
                            }
                            _ => {}
                        }
                    }

                    // Toggle hilite for button members on mouseDown
                    if let Some(sprite) = player.movie.score.get_sprite(sprite_number as i16) {
                        if let Some(member_ref) = sprite.member.clone() {
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                match &mut member.member_type {
                                    CastMemberType::Button(button) => {
                                        match button.button_type {
                                            crate::player::cast_member::ButtonType::PushButton => {
                                                button.hilite = true;
                                            }
                                            crate::player::cast_member::ButtonType::CheckBox => {
                                                button.hilite = !button.hilite;
                                            }
                                            crate::player::cast_member::ButtonType::RadioButton => {
                                                button.hilite = true;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                } else {
                    player.click_on_sprite = 0;
                }

                // For event dispatch targeting, use scripted lookup —
                // only sprites with behaviors or cast member scripts receive mouseDown.
                let scripted_sprite = get_sprite_at(player, x, y, true);
                if let Some(sprite_number) = scripted_sprite {
                    player.mouse_down_sprite = sprite_number as i16;
                } else {
                    player.mouse_down_sprite = -1;
                }
            });

            // Temporarily clear ALL is_yield_safe() flags so that updateStage()
            // called from within mouseDown handlers will render (but not yield).
            // MouseDown commands can be processed at any .await point in the frame
            // loop, where multiple flags may be true simultaneously (e.g.
            // is_in_frame_update AND in_enter_frame during enterFrame dispatch).
            // Set in_mouse_command so the frame loop skips frame updates/advancement
            // and updateStage renders without sleeping (preventing re-entrant event
            // dispatch and timing issues with mouseUp processing).
            let saved_yield_flags = reserve_player_mut(|player| {
                let saved = (
                    player.is_in_frame_update,
                    player.in_frame_script,
                    player.in_enter_frame,
                    player.in_prepare_frame,
                    player.in_event_dispatch,
                    player.in_mouse_command,
                );
                player.is_in_frame_update = false;
                player.in_frame_script = false;
                player.in_enter_frame = false;
                player.in_prepare_frame = false;
                player.in_event_dispatch = false;
                player.in_mouse_command = true;
                saved
            });

            // Dispatch to sprite behaviors if the sprite has any, otherwise
            // fall through to frame/movie scripts per Director's propagation chain.
            let sprite_with_behaviors = reserve_player_ref(|player| {
                if player.mouse_down_sprite > 0 {
                    let sprite = player.movie.score.get_sprite(player.mouse_down_sprite);
                    let has_behaviors = sprite.map_or(false, |s| !s.script_instance_list.is_empty())
                        || player.script_instance_list_cache.get(&player.mouse_down_sprite)
                            .map_or(false, |cached_ref| {
                                matches!(player.get_datum(cached_ref), Datum::List(_, items, _) if !items.is_empty())
                            });
                    if has_behaviors {
                        return Some(player.mouse_down_sprite as u16);
                    }
                }
                None
            });

            if let Some(sprite_num) = sprite_with_behaviors {
                player_dispatch_event_to_sprite_targeted(
                    &"mouseDown".to_string(),
                    &vec![],
                    sprite_num,
                ).await;
            } else {
                player_invoke_frame_and_movie_scripts(
                    &"mouseDown".to_string(),
                    &vec![]
                ).await?;
            }

            // Execute cast member script if it exists
            let cast_member_script_call = reserve_player_mut(|player| {
                if player.mouse_down_sprite <= 0 {
                    return None;
                }

                let sprite = player.movie.score.get_sprite(player.mouse_down_sprite)?;
                let member_ref = sprite.member.as_ref()?;
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)?;

                // First check for member behavior script (stored in member_script_ref)
                if let Some(script_ref) = member.get_member_script_ref() {
                    debug!(
                        "Cast member '{}' has behavior script (cast_lib={}, member={}), executing mouseDown",
                        member.name, script_ref.cast_lib, script_ref.cast_member
                    );

                    if let Some(script) = player.movie.cast_manager.get_script_by_ref(script_ref) {
                        if let Some(handler) = script.get_own_handler_ref(&"mouseDown".to_string()) {
                            return Some((None, handler, vec![]));
                        }
                    }
                }

                // Fallback: check for script_id and get directly from lctx.scripts
                let script_id = member.get_script_id()?;

                debug!(
                    "Cast member '{}' has script {}, getting from lctx.scripts for mouseDown",
                    member.name, script_id
                );

                let script = {
                    let cast_lib = player.movie.cast_manager.get_cast_mut(member_ref.cast_lib as u32);
                    cast_lib.get_behavior_script_from_lctx(script_id)
                };

                let script = match script {
                    Some(s) => {
                       debug!("Behavior script {} found for mouseDown", script_id);
                        s
                    }
                    None => {
                        debug!("Behavior script {} NOT FOUND in lctx.scripts for mouseDown", script_id);
                        return None;
                    }
                };

                let handler = script.get_own_handler_ref(&"mouseDown".to_string())?;

                Some((None, handler, vec![]))
            });

            let mut handler_err: Option<ScriptError> = None;
            if let Some((receiver, handler, args)) = cast_member_script_call {
                if let Err(e) = player_call_script_handler(receiver, handler, &args).await {
                    handler_err = Some(e);
                }
            }

            // Dispatch mouseDownScript last as well (for cases where it was set
            // during sprite handler execution, e.g. not set at start of event)
            if handler_err.is_none() {
                if let Err(e) = player_dispatch_movie_callback("mouseDown").await {
                    handler_err = Some(e);
                }
            }

            // Restore all is_yield_safe() flags and in_mouse_command
            // MUST happen even on error to prevent skip_frame getting stuck
            reserve_player_mut(|player| {
                player.is_in_frame_update = saved_yield_flags.0;
                player.in_frame_script = saved_yield_flags.1;
                player.in_enter_frame = saved_yield_flags.2;
                player.in_prepare_frame = saved_yield_flags.3;
                player.in_event_dispatch = saved_yield_flags.4;
                player.in_mouse_command = saved_yield_flags.5;
            });

            if let Some(e) = handler_err {
                return Err(e);
            }
            return Ok(DatumRef::Void);
        }
        PlayerVMCommand::MouseUp((x, y)) => {
            if !player_is_playing().await {
                return Ok(DatumRef::Void);
            }
            // In Director, mouseUpScript intercepts BEFORE sprites get the event.
            let mouse_up_script_active = reserve_player_ref(|player| {
                has_executable_callback(&player.movie.mouse_up_script)
            });
            if mouse_up_script_active {
                reserve_player_mut(|player| {
                    player.mouse_loc = (x, y);
                    player.movie.mouse_down = false;
                    player.mouse_down_sprite = -1;
                });
                player_dispatch_movie_callback("mouseUp").await?;
                reserve_player_mut(|player| {
                    player.is_double_click = false;
                });
                return Ok(DatumRef::Void);
            }

            // Update mouse state and determine which sprite to notify
            let result = reserve_player_mut(|player| {
                player.mouse_loc = (x, y);
                player.movie.mouse_down = false;
                let sprite_num_to_notify = player.mouse_down_sprite;

                // Reset hilite for push buttons on mouseUp
                if player.mouse_down_sprite > 0 {
                    if let Some(sprite) = player.movie.score.get_sprite(player.mouse_down_sprite) {
                        if let Some(member_ref) = sprite.member.clone() {
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let CastMemberType::Button(button) = &mut member.member_type {
                                    if button.button_type == crate::player::cast_member::ButtonType::PushButton {
                                        button.hilite = false;
                                    }
                                }
                            }
                        }
                    }
                }

                let sprite = if player.mouse_down_sprite > 0 {
                    player.movie.score.get_sprite(player.mouse_down_sprite)
                } else {
                    None
                };
                player.mouse_down_sprite = -1;
                if let Some(sprite) = sprite {
                    let is_inside = concrete_sprite_hit_test(player, sprite, x, y);
                    Some((sprite.script_instance_list.clone(), is_inside, sprite_num_to_notify))
                } else {
                    None
                }
            });
            let is_inside = result.as_ref().map(|x| x.1).unwrap_or(true);
            let event_name = if is_inside {
                "mouseUp"
            } else {
                "mouseUpOutSide"
            };

            // Temporarily clear ALL is_yield_safe() flags so that updateStage()
            // called from within mouseUp handlers will render (but not yield).
            // Same rationale as mouseDown: multiple flags may be true when
            // the mouseUp command is processed at a frame loop .await point.
            // Set in_mouse_command to prevent re-entrant frame updates and
            // make updateStage synchronous (render-only, no sleep).
            let saved_yield_flags = reserve_player_mut(|player| {
                let saved = (
                    player.is_in_frame_update,
                    player.in_frame_script,
                    player.in_enter_frame,
                    player.in_prepare_frame,
                    player.in_event_dispatch,
                    player.in_mouse_command,
                );
                player.is_in_frame_update = false;
                player.in_frame_script = false;
                player.in_enter_frame = false;
                player.in_prepare_frame = false;
                player.in_event_dispatch = false;
                player.in_mouse_command = true;
                saved
            });

            // Dispatch to the sprite that originally received mouseDown,
            // or fall through to frame/movie scripts if no sprite was involved.
            let dispatched_to_sprite = if let Some((_, _, sprite_num)) = result.as_ref() {
                if *sprite_num > 0 {
                    player_dispatch_event_to_sprite_targeted(
                        &event_name.to_string(),
                        &vec![],
                        *sprite_num as u16,
                    ).await;
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !dispatched_to_sprite {
                player_invoke_frame_and_movie_scripts(
                    &event_name.to_string(),
                    &vec![]
                ).await;
            }

            // Execute cast member script using the ORIGINAL sprite that had mouseDown,
            // consistent with Director behavior
            let cast_member_script_call = reserve_player_mut(|player| {
                let sprite_num_to_notify = result.as_ref().map(|r| r.2).unwrap_or(-1);
                if sprite_num_to_notify <= 0 {
                    return None;
                }

                let sprite = player.movie.score.get_sprite(sprite_num_to_notify)?;
                let member_ref = sprite.member.as_ref()?;
                let member = player.movie.cast_manager.find_member_by_ref(member_ref)?;

                let handler_name = if is_inside { "mouseUp" } else { "mouseUpOutSide" };

                // First check for member behavior script (stored in member_script_ref)
                if let Some(script_ref) = member.get_member_script_ref() {
                    if let Some(script) = player.movie.cast_manager.get_script_by_ref(script_ref) {
                        if let Some(handler) = script.get_own_handler_ref(&handler_name.to_string()) {
                            return Some((None, handler, vec![]));
                        }
                    }
                }

                // Fallback: check for script_id and get directly from lctx.scripts
                let script_id = member.get_script_id()?;

                let script = {
                    let cast_lib = player.movie.cast_manager.get_cast_mut(member_ref.cast_lib as u32);
                    cast_lib.get_behavior_script_from_lctx(script_id)
                };

                let script = script?;
                let handler = script.get_own_handler_ref(&handler_name.to_lowercase())?;

                // Try to get the handler
                let handler = script.get_own_handler_ref(&handler_name);
                
                // ADD THIS CHECK:
                if handler.is_none() {
                    debug!("⚠️  Handler '{}' NOT FOUND in script {}", handler_name, script_id);
                    return None;
                }
                
                debug!("✓ Handler '{}' found!", handler_name);
                
                Some((None, handler.unwrap(), vec![]))
            });

            let mut handler_err: Option<ScriptError> = None;
            if let Some((receiver, handler, args)) = cast_member_script_call {
                if let Err(e) = player_call_script_handler(receiver, handler, &args).await {
                    handler_err = Some(e);
                }
            }

            if handler_err.is_none() {
                if let Err(e) = player_dispatch_movie_callback("mouseUp").await {
                    handler_err = Some(e);
                }
            }

            // Restore all is_yield_safe() flags and command_handler_yielding
            // MUST happen even on error to prevent skip_frame getting stuck
            reserve_player_mut(|player| {
                player.is_in_frame_update = saved_yield_flags.0;
                player.in_frame_script = saved_yield_flags.1;
                player.in_enter_frame = saved_yield_flags.2;
                player.in_prepare_frame = saved_yield_flags.3;
                player.in_event_dispatch = saved_yield_flags.4;
                player.in_mouse_command = saved_yield_flags.5;
                player.is_double_click = false;
            });

            if let Some(e) = handler_err {
                return Err(e);
            }
            return Ok(DatumRef::Void);
        }
        PlayerVMCommand::MouseMove((x, y)) => {
            if !player_is_playing().await {
                return Ok(DatumRef::Void);
            }
            let (sprite_num, hovered_sprite) = reserve_player_mut(|player| {
                player.mouse_loc = (x, y);

                // Drag moveable sprites (use click_on_sprite, not mouse_down_sprite,
                // since moveable sprites don't need scripts to be draggable)
                if player.movie.mouse_down && player.click_on_sprite > 0 {
                    let drag_sprite_num = player.click_on_sprite;
                    let (off_x, off_y) = player.drag_offset;
                    let sprite = player.movie.score.get_sprite_mut(drag_sprite_num);
                    if sprite.moveable {
                        let mut new_h = x + off_x;
                        let mut new_v = y + off_y;

                        // Apply constraint bounds
                        let constraint_num = sprite.constraint;
                        if constraint_num > 0 {
                            // Constrain to the bounding rect of the constraint sprite
                            if let Some(constraint_sprite) = player.movie.score.get_sprite(constraint_num as i16) {
                                let bounds = get_concrete_sprite_rect(player, constraint_sprite);
                                new_h = new_h.max(bounds.left).min(bounds.right);
                                new_v = new_v.max(bounds.top).min(bounds.bottom);
                            }
                        } else {
                            // Constrain to stage
                            let stage = &player.movie.rect;
                            new_h = new_h.max(stage.left).min(stage.right);
                            new_v = new_v.max(stage.top).min(stage.bottom);
                        }

                        let sprite = player.movie.score.get_sprite_mut(drag_sprite_num);
                        sprite.loc_h = new_h;
                        sprite.loc_v = new_v;
                    }
                }

                let hovered_sprite = player.hovered_sprite;
                let sprite_num = get_sprite_at(player, x, y, false);
                if let Some(sprite_num) = sprite_num {
                    player.hovered_sprite = Some(sprite_num as i16);
                }
                (sprite_num, hovered_sprite)
            });
            if let Some(sprite_num) = sprite_num {
                let hovered_sprite = hovered_sprite.unwrap_or(-1);
                if hovered_sprite != sprite_num as i16 {
                    if hovered_sprite != -1 {
                        player_dispatch_event_to_sprite(
                            &"mouseLeave".to_string(),
                            &vec![],
                            hovered_sprite as u16,
                        )
                    }
                    player_dispatch_event_to_sprite(
                        &"mouseEnter".to_string(),
                        &vec![],
                        sprite_num as u16,
                    );
                } else {
                    player_dispatch_event_to_sprite(
                        &"mouseWithin".to_string(),
                        &vec![],
                        sprite_num as u16,
                    );
                }
            }
        }
        PlayerVMCommand::KeyDown(key, code) => {
            // Set command_handler_yielding so that:
            // 1. updateStage() always yields (bypasses is_yield_safe check),
            //    letting the browser process keyUp events during repeat-while-
            //    keyPressed loops (like the Hook script's movement).
            // 2. The frame loop skips frame updates and advancement to avoid
            //    running frame scripts that would corrupt the shared scope stack.
            reserve_player_mut(|player| {
                player.command_handler_yielding = true;
            });

            let result = player_key_down(key, code).await;

            reserve_player_mut(|player| {
                player.command_handler_yielding = false;
            });

            return result;
        }
        PlayerVMCommand::KeyUp(key, code) => {
            return player_key_up(key, code).await;
        }
        PlayerVMCommand::TriggerAlertHook => {
            let call_params = reserve_player_mut(|player| {
                let arg_list = vec![
                    player.alloc_datum(Datum::String("Script Error".to_string())),
                    player
                        .alloc_datum(Datum::String("An error occurred in the script".to_string())),
                ];
                if let Some(alert_hook) = &player.movie.alert_hook {
                    match alert_hook {
                        ScriptReceiver::ScriptInstance(instance_ref) => {
                            let script_instance =
                                player.allocator.get_script_instance(&instance_ref);
                            let script = player
                                .movie
                                .cast_manager
                                .get_script_by_ref(&script_instance.script)
                                .unwrap();
                            let handler = script.get_own_handler_ref(&"alertHook".to_string());
                            if let Some(handler) = handler {
                                Some((Some(instance_ref.clone()), handler, arg_list))
                            } else {
                                None
                            }
                        }
                        ScriptReceiver::Script(script_ref) => {
                            let script = player
                                .movie
                                .cast_manager
                                .get_script_by_ref(&script_ref)
                                .unwrap();
                            let handler = script.get_own_handler_ref(&"alertHook".to_string());
                            if let Some(handler) = handler {
                                Some((None, handler, arg_list))
                            } else {
                                None
                            }
                        }
                        ScriptReceiver::ScriptText(text) => {
                            // For mouseDownScript, you'd compile and execute the text
                            // But for alertHook specifically, you'd look for an alertHook handler
                            // in the compiled script
                            
                            // Option 1: Compile the script text on-the-fly
                            // This requires your Lingo compiler to be accessible
                            
                            // Option 2: For simple cases like "--nothing", just ignore it
                            if text.trim().starts_with("--") || text.trim().is_empty() {
                                // It's a comment or empty, do nothing
                                None
                            } else {
                                // TODO: Compile and execute the script text
                                // You'll need to:
                                // 1. Parse the text as Lingo code
                                // 2. Compile it to bytecode
                                // 3. Execute it with the given arguments
                                
                                // For now, log a warning and skip
                                warn!("Warning: Script text execution not yet implemented for alertHook: {}", text);
                                None
                            }
                        }
                    }
                } else {
                    None
                }
            });
            if let Some((receiver, handler, args)) = call_params {
                player_call_script_handler(receiver, handler, &args).await?;
            }
        }
        PlayerVMCommand::TriggerFlashCallback { sprite_num, handler_name, args } => {
            // Find the sprite and its script instances, call the matching handler
            let call_params = reserve_player_mut(|player| {
                if let Some(sprite) = player.movie.score.get_sprite(sprite_num as i16) {
                    for script_instance_ref in &sprite.script_instance_list {
                        let script_instance = player.allocator.get_script_instance(script_instance_ref);
                        if let Some(script) = player.movie.cast_manager.get_script_by_ref(&script_instance.script) {
                            if let Some(handler_ref) = script.get_own_handler_ref(&handler_name) {
                                return Some((
                                    Some(script_instance_ref.clone()),
                                    handler_ref,
                                    args.clone(),
                                ));
                            }
                        }
                    }
                }
                None
            });

            if let Some((receiver, handler, args)) = call_params {
                player_call_script_handler(receiver, handler, &args).await?;
            }
        }
        PlayerVMCommand::TriggerLingoCallbackOnScript { cast_lib, cast_member, handler_name, args } => {
            use super::handlers::datum_handlers::script_instance::ScriptInstanceDatumHandlers;

            let call_params = reserve_player_mut(|player| {
                for (script_instance_id, script_instance_entry) in player.allocator.script_instances.iter() {
                    let script_instance = &script_instance_entry.script_instance;

                    if script_instance.script.cast_lib == cast_lib &&
                       script_instance.script.cast_member == cast_member
                    {
                        if let Some(script) = player.movie.cast_manager.get_script_by_ref(&script_instance.script) {
                            if let Some(handler_ref) = script.get_own_handler_ref(&handler_name) {
                                let script_instance_ref = ScriptInstanceRef::from_id(
                                    script_instance_id as u32,
                                    script_instance_entry.ref_count.get()
                                );
                                return Some((
                                    script_instance_ref,
                                    handler_ref,
                                    args.clone(),
                                ));
                            }
                        }
                    }
                }
                None
            });

            if let Some((receiver, handler, args)) = call_params {
                let receiver_datum = player_alloc_datum(Datum::ScriptInstanceRef(receiver));
                let _ = ScriptInstanceDatumHandlers::call_async(
                    &receiver_datum,
                    &handler.1,
                    &args
                ).await;
            }
        }
        PlayerVMCommand::SetLingoScriptProperty { cast_lib, cast_member, prop_name, value } => {
            use super::script::script_set_prop;

            reserve_player_mut(|player| {
                let mut matching_instances = Vec::new();

                for (instance_id, instance_entry) in player.allocator.script_instances.iter() {
                    let instance = &instance_entry.script_instance;
                    if instance.script.cast_lib == cast_lib && instance.script.cast_member == cast_member {
                        matching_instances.push(ScriptInstanceRef::from_id(
                            instance_id as u32,
                            instance_entry.ref_count.get()
                        ));
                    }
                }

                for instance_ref in matching_instances {
                    let _ = script_set_prop(player, &instance_ref, &prop_name, &value, false);
                }
            });
        }
    }
    Ok(DatumRef::Void)
}
