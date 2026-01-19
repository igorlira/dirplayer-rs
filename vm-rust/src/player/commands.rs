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
    player::{eval::eval_lingo_command, PLAYER_OPT},
    utils::{log_i, ToHexString},
};

use super::{
    allocator::ScriptInstanceAllocatorTrait,
    cast_lib::CastMemberRef,
    cast_member::CastMemberType,
    datum_ref::{DatumId, DatumRef},
    events::{
        player_dispatch_callback_event, player_dispatch_event_to_sprite,
        player_dispatch_targeted_event, player_wait_available,
    },
    font::player_load_system_font,
    keyboard_events::{player_key_down, player_key_up},
    player_alloc_datum, player_call_script_handler, player_dispatch_global_event,
    player_is_playing, reserve_player_mut, reserve_player_ref,
    score::{concrete_sprite_hit_test, get_sprite_at},
    script::ScriptInstanceId,
    script_ref::ScriptInstanceRef,
    PlayerVMExecutionItem, ScriptError, ScriptReceiver, PLAYER_TX,
};

#[allow(dead_code)]
pub enum PlayerVMCommand {
    DispatchEvent(String, Vec<DatumRef>),
    Play,
    Stop,
    Reset,
    LoadMovieFromFile(String),
    SetExternalParams(HashMap<String, String>),
    SetBasePath(String),
    SetSystemFontPath(String),
    AddBreakpoint(String, String, usize),
    RemoveBreakpoint(String, String, usize),
    ToggleBreakpoint(String, String, usize),
    ResumeBreakpoint,
    SetStageSize(u32, u32),
    TimeoutTriggered(TimeoutRef),
    PrintMemberBitmapHex(CastMemberRef),
    MouseDown((i32, i32)),
    MouseUp((i32, i32)),
    MouseMove((i32, i32)),
    KeyDown(String, u16),
    KeyUp(String, u16),
    RequestDatum(DatumId),
    RequestScriptInstanceSnapshot(ScriptInstanceId),
    SubscribeToMember(CastMemberRef),
    UnsubscribeFromMember(CastMemberRef),
    TriggerAlertHook,
    EvalLingoCommand(String),
}

pub fn _format_player_cmd(command: &PlayerVMCommand) -> String {
    match command {
        PlayerVMCommand::DispatchEvent(name, _) => format!("DispatchEvent({})", name),
        PlayerVMCommand::Play => "Play".to_string(),
        PlayerVMCommand::Stop => "Stop".to_string(),
        PlayerVMCommand::Reset => "Reset".to_string(),
        PlayerVMCommand::LoadMovieFromFile(path) => format!("LoadMovieFromFile({})", path),
        PlayerVMCommand::SetExternalParams(params) => {
            format!("SetExternalParams({:?})", params.keys().collect::<Vec<_>>())
        }
        PlayerVMCommand::SetBasePath(path) => format!("SetBasePath({})", path),
        PlayerVMCommand::SetSystemFontPath(path) => format!("SetSystemFontPath({})", path),
        PlayerVMCommand::AddBreakpoint(script_name, handler_name, bytecode_index) => format!(
            "AddBreakpoint({}, {}, {})",
            script_name, handler_name, bytecode_index
        ),
        PlayerVMCommand::RemoveBreakpoint(script_name, handler_name, bytecode_index) => format!(
            "RemoveBreakpoint({}, {}, {})",
            script_name, handler_name, bytecode_index
        ),
        PlayerVMCommand::ToggleBreakpoint(script_name, handler_name, bytecode_index) => format!(
            "ToggleBreakpoint({}, {}, {})",
            script_name, handler_name, bytecode_index
        ),
        PlayerVMCommand::ResumeBreakpoint => "ResumeBreakpoint".to_string(),
        PlayerVMCommand::SetStageSize(width, height) => {
            format!("SetStageSize({}, {})", width, height)
        }
        PlayerVMCommand::TimeoutTriggered(timeout_ref) => {
            format!("TimeoutTriggered({})", timeout_ref)
        }
        PlayerVMCommand::PrintMemberBitmapHex(..) => format!("PrintMemberBitmapHex(..)"),
        PlayerVMCommand::MouseDown((x, y)) => format!("MouseDown({}, {})", x, y),
        PlayerVMCommand::MouseUp((x, y)) => format!("MouseUp({}, {})", x, y),
        PlayerVMCommand::MouseMove((x, y)) => format!("MouseMove({}, {})", x, y),
        PlayerVMCommand::KeyDown(key, ..) => format!("KeyDown({})", key),
        PlayerVMCommand::KeyUp(key, ..) => format!("KeyUp({})", key),
        PlayerVMCommand::RequestDatum(datum_ref) => format!("RequestDatum({})", datum_ref),
        PlayerVMCommand::RequestScriptInstanceSnapshot(script_instance_id) => {
            format!("RequestScriptInstanceSnapshot({})", script_instance_id)
        }
        PlayerVMCommand::SubscribeToMember(member_ref) => {
            format!("SubscribeToMember({:?})", member_ref)
        }
        PlayerVMCommand::UnsubscribeFromMember(member_ref) => {
            format!("UnsubscribeFromMember({:?})", member_ref)
        }
        PlayerVMCommand::TriggerAlertHook => "TriggerAlertHook".to_string(),
        PlayerVMCommand::EvalLingoCommand(cmd) => format!("EvalLingoCommand({})", cmd),
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
                // TODO ignore error if it's a CancelledException
                // TODO print stack trace
                reserve_player_mut(|player| player.on_script_error(&err));
                if let Some(completer) = item.completer {
                    completer.complete(Err(err)).await;
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
        PlayerVMCommand::SetSystemFontPath(path) => {
            console_warn!("Loading system font: {}", path);
            player_load_system_font(&path).await;
        }
        PlayerVMCommand::Play => {
            reserve_player_mut(|player| {
                player.play();
            });
        }
        PlayerVMCommand::Stop => {
            reserve_player_mut(|player| {
                player.stop();
            });
        }
        PlayerVMCommand::Reset => {
            reserve_player_mut(|player| {
                player.reset();
            });
        }
        PlayerVMCommand::LoadMovieFromFile(file_path) => {
            let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
            player.load_movie_from_file(&file_path).await;
        }
        PlayerVMCommand::DispatchEvent(handler_name, args) => {
            player_dispatch_global_event(&handler_name, &args);
        }
        PlayerVMCommand::AddBreakpoint(script_name, handler_name, bytecode_index) => {
            reserve_player_mut(|player| {
                player
                    .breakpoint_manager
                    .add_breakpoint(script_name, handler_name, bytecode_index);
            });
        }
        PlayerVMCommand::RemoveBreakpoint(script_name, handler_name, bytecode_index) => {
            reserve_player_mut(|player| {
                player.breakpoint_manager.remove_breakpoint(
                    script_name,
                    handler_name,
                    bytecode_index,
                );
            });
        }
        PlayerVMCommand::ToggleBreakpoint(script_name, handler_name, bytecode_index) => {
            reserve_player_mut(|player| {
                player.breakpoint_manager.toggle_breakpoint(
                    script_name,
                    handler_name,
                    bytecode_index,
                );
            });
        }
        PlayerVMCommand::ResumeBreakpoint => {
            reserve_player_mut(|player| {
                player.resume_breakpoint();
            });
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
            let instance_ids = reserve_player_mut(|player| {
                let now = Local::now().timestamp_millis().abs();
                let is_double_click = (now - player.last_mouse_down_time) < 500;
                player.mouse_loc = (x, y);
                player.movie.mouse_down = true;  // Track mouse button state
                player.movie.click_loc = (x, y); // Store click location
                player.is_double_click = is_double_click;
                player.last_mouse_down_time = now;
                let sprite = get_sprite_at(player, x, y, true);
                if let Some(sprite_number) = sprite {
                    debug!("🖱️  MouseDown on sprite #{}", sprite_number);
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
                        player.mouse_down_sprite = sprite_number as i16;
                        player.click_on_sprite = sprite_number as i16;
                    } else {
                        player.mouse_down_sprite = -1;
                        player.click_on_sprite = 0;
                    }

                    let instances = sprite.map(|x| x.script_instance_list.clone());
                    if let Some(ref inst) = instances {
                        debug!("🖱️  Sprite has {} behaviors", inst.len());
                    }

                    instances
                } else {
                    None
                }
            });

            // Get the sprite number we just stored
            let sprite_num = reserve_player_ref(|player| {
                if player.mouse_down_sprite > 0 {
                    Some(player.mouse_down_sprite as u16)
                } else {
                    None
                }
            });

            if let Some(sprite_num) = sprite_num {
                // Use non-blocking dispatch to avoid deadlock when breakpoints are hit
                player_dispatch_event_to_sprite(
                    &"mouseDown".to_string(),
                    &vec![],
                    sprite_num,
                );
            } else {
                // Use non-blocking dispatch to avoid deadlock when breakpoints are hit
                player_dispatch_global_event(
                    &"mouseDown".to_string(),
                    &vec![]
                );
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
                       debug!("✓ Behavior script {} found for mouseDown", script_id);
                        s
                    }
                    None => {
                        debug!("✗ Behavior script {} NOT FOUND in lctx.scripts for mouseDown", script_id);
                        return None;
                    }
                };
                
                let handler = script.get_own_handler_ref(&"mouseDown".to_string())?;
                
                Some((None, handler, vec![]))
            });

            if let Some((receiver, handler, args)) = cast_member_script_call {
                // Spawn the handler call to avoid blocking the command loop on breakpoints
                spawn_local(async move {
                    let _ = player_call_script_handler(receiver, handler, &args).await;
                });
            }

            return Ok(DatumRef::Void);
        }
        PlayerVMCommand::MouseUp((x, y)) => {
            if !player_is_playing().await {
                return Ok(DatumRef::Void);
            }
            let result = reserve_player_mut(|player| {
                player.mouse_loc = (x, y);
                player.movie.mouse_down = false;  // Track mouse button state
                let sprite_num_to_notify = player.mouse_down_sprite;
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
            let instance_ids = result.as_ref().map(|x| &x.0);
            let event_name = if is_inside {
                "mouseUp"
            } else {
                "mouseUpOutSide"
            };
            
            // Get the sprite number from result
            // Use non-blocking dispatch to avoid deadlock when breakpoints are hit
            if let Some((_, _, sprite_num)) = result.as_ref() {
                if *sprite_num > 0 {
                    player_dispatch_event_to_sprite(
                        &event_name.to_string(),
                        &vec![],
                        *sprite_num as u16,
                    );
                }
            }

            let cast_member_script_call = reserve_player_mut(|player| {
                let sprite_num = get_sprite_at(player, x, y, false)?;
                debug!("Getting sprite at ({}, {}): sprite #{}", x, y, sprite_num);
                
                let sprite = player.movie.score.get_sprite(sprite_num as i16)?;
                let member_ref = sprite.member.as_ref()?;

                debug!("Sprite {} uses member: cast_lib={}, cast_member={}", 
                    sprite_num, member_ref.cast_lib, member_ref.cast_member);

                let member = player.movie.cast_manager.find_member_by_ref(member_ref)?;

                debug!("Member '{}' (type: {:?})", member.name, 
                    match &member.member_type {
                        CastMemberType::Bitmap(_) => "Bitmap",
                        CastMemberType::Script(_) => "Script",
                        _ => "Other"
                    }
                );

                let handler_name = if is_inside { "mouseUp" } else { "mouseUpOutSide" };

                // Get script_id from bitmap member and look it up in lctx.scripts
                debug!("Getting script_id from member...");
                let script_id = match member.get_script_id() {
                    Some(id) => {
                        debug!("Member has script_id: {}", id);
                        id
                    }
                    None => {
                        debug!("⚠️  Member has NO script_id");
                        return None;
                    }
                };
                
                // Get the behavior script directly from lctx.scripts
                debug!("Looking up behavior script {} from lctx.scripts...", script_id);
                
                let script = {
                    let cast_lib = player.movie.cast_manager.get_cast_mut(member_ref.cast_lib as u32);
                    cast_lib.get_behavior_script_from_lctx(script_id)
                };
                
                let script = match script {
                    Some(s) => {
                        debug!("✓ Behavior script {} found! Type: {:?}", script_id, s.script_type);
                        debug!("Available handlers: {:?}", 
                            s.handlers.keys().collect::<Vec<_>>()
                        );
                        s
                    }
                    None => {
                        debug!("✗ Behavior script {} NOT FOUND in lctx.scripts!", script_id);
                        return None;
                    }
                };

                debug!(
                    "Looking for '{}' handler in script {}",
                    handler_name, script_id
                );

                // Try to get the handler - convert to lowercase for lookup
                let handler = script.get_own_handler_ref(&handler_name.to_lowercase());
                
                // ADD THIS CHECK:
                if handler.is_none() {
                    debug!("⚠️  Handler '{}' NOT FOUND in script {}", handler_name, script_id);
                    return None;
                }
                
                debug!("✓ Handler '{}' found!", handler_name);
                
                Some((None, handler.unwrap(), vec![]))
            });

            if let Some((receiver, handler, args)) = cast_member_script_call {
                debug!("Calling player_call_script_handler...");

                // Spawn the handler call to avoid blocking the command loop on breakpoints
                spawn_local(async move {
                    let _ = player_call_script_handler(receiver, handler, &args).await;
                    debug!("✓ Handler executed successfully");
                });
            }

            reserve_player_mut(|player| {
                player.is_double_click = false;
            });
            return Ok(DatumRef::Void);
        }
        PlayerVMCommand::MouseMove((x, y)) => {
            if !player_is_playing().await {
                return Ok(DatumRef::Void);
            }
            let (sprite_num, hovered_sprite) = reserve_player_mut(|player| {
                player.mouse_loc = (x, y);

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
            return player_key_down(key, code).await;
        }
        PlayerVMCommand::KeyUp(key, code) => {
            return player_key_up(key, code).await;
        }
        PlayerVMCommand::RequestDatum(datum_id) => {
            reserve_player_ref(|player| {
                if let Some(datum_ref) = player.allocator.get_datum_ref(datum_id) {
                    JsApi::dispatch_datum_snapshot(&datum_ref, player);
                }
            });
        }
        PlayerVMCommand::RequestScriptInstanceSnapshot(script_instance_id) => {
            reserve_player_ref(|player| {
                JsApi::dispatch_script_instance_snapshot(
                    if script_instance_id > 0 {
                        Some(
                            player
                                .allocator
                                .get_script_instance_ref(script_instance_id)
                                .unwrap(),
                        )
                    } else {
                        None
                    },
                    player,
                );
            });
        }
        PlayerVMCommand::SubscribeToMember(member_ref) => {
            reserve_player_mut(|player| {
                if !player.subscribed_member_refs.contains(&member_ref) {
                    player.subscribed_member_refs.push(member_ref.clone());
                }
            });
            JsApi::dispatch_cast_member_changed(member_ref);
        }
        PlayerVMCommand::UnsubscribeFromMember(member_ref) => {
            reserve_player_mut(|player| {
                player.subscribed_member_refs.retain(|x| x != &member_ref);
            });
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
        PlayerVMCommand::EvalLingoCommand(command) => {
            JsApi::dispatch_debug_message(&command);
            let result = eval_lingo_command(command).await?;
            return Ok(result);
        }
    }
    Ok(DatumRef::Void)
}
