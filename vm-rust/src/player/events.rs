use async_std::channel::Receiver;
use itertools::Itertools;
use log::{warn, debug};

use crate::{
    director::lingo::datum::{Datum, VarRef},
    player::{
        handlers::datum_handlers::player_call_datum_handler, player_is_playing, reserve_player_mut,
    },
};

use super::{
    allocator::ScriptInstanceAllocatorTrait,
    cast_lib::CastMemberRef, handlers::datum_handlers::script_instance::ScriptInstanceUtils,
    player_call_script_handler, player_semaphone, reserve_player_ref,
    script_ref::ScriptInstanceRef, DatumRef, ScriptError, ScriptErrorCode, ScriptReceiver,
    PLAYER_EVENT_TX, score::ScoreRef,
};

pub enum PlayerVMEvent {
    Global(String, Vec<DatumRef>),
    Targeted(String, Vec<DatumRef>, Option<Vec<ScriptInstanceRef>>),
    Callback(DatumRef, String, Vec<DatumRef>),
}

pub fn player_dispatch_global_event(handler_name: &str, args: &Vec<DatumRef>) {
    if let Some(tx) = crate::player::active_event_tx() {
        let _ = tx.try_send(PlayerVMEvent::Global(
            handler_name.to_owned(),
            args.to_owned(),
        ));
    }
}

pub fn player_dispatch_callback_event(
    receiver: DatumRef,
    handler_name: &str,
    args: &Vec<DatumRef>,
) {
    if let Some(tx) = crate::player::active_event_tx() {
        let _ = tx.try_send(PlayerVMEvent::Callback(
            receiver,
            handler_name.to_owned(),
            args.to_owned(),
        ));
    }
}

pub fn player_dispatch_targeted_event(
    handler_name: &str,
    args: &Vec<DatumRef>,
    instance_ids: Option<&Vec<ScriptInstanceRef>>,
) {
    if let Some(tx) = crate::player::active_event_tx() {
        let _ = tx.try_send(PlayerVMEvent::Targeted(
            handler_name.to_owned(),
            args.to_owned(),
            instance_ids.map(|x| x.to_owned()),
        ));
    }
}

pub fn player_dispatch_event_to_sprite(
    handler_name: &str,
    args: &Vec<DatumRef>,
    sprite_num: u16,
) {
    let instance_ids = reserve_player_mut(|player| {
        // Check the cache first — it may contain extra instances added
        // via scriptInstanceList.add() (e.g. goal parent scripts).
        let fallback = player
            .movie
            .score
            .get_sprite(sprite_num as i16)
            .map(|sprite| sprite.script_instance_list.clone());
        fallback.map(|fallback| {
            player.get_sprite_script_instance_ids(sprite_num as i16, fallback.as_slice())
        })
    });
    if instance_ids.is_none() {
        return;
    }
    let instance_ids = instance_ids.unwrap();
    let tx = crate::player::active_event_tx().unwrap();
    tx.try_send(PlayerVMEvent::Targeted(
        handler_name.to_owned(),
        args.to_owned(),
        Some(instance_ids),
    ))
    .unwrap();
}

pub async fn player_dispatch_event_to_sprite_targeted(
    handler_name: &str,
    args: &Vec<DatumRef>,
    sprite_num: u16,
) {
    let instance_ids = reserve_player_mut(|player| {
        // Check the cache first — it may contain extra instances added
        // via scriptInstanceList.add() (e.g. goal parent scripts).
        
        let fallback = player
            .movie
            .score
            .get_sprite(sprite_num as i16)
            .map(|sprite| sprite.script_instance_list.clone());
        fallback.map(|fallback| {
            player.get_sprite_script_instance_ids(sprite_num as i16, fallback.as_slice())
        })
    });
    let Some(instance_ids) = instance_ids else {
        return;
    };

    player_wait_available().await;

    // Dispatch to ALL of the sprite's behaviors in a single pass, then — if
    // none of them handled it (or the sprite has no behaviors at all) — fall
    // through to the frame + movie scripts. Director's message hierarchy for a
    // sprite-directed event is behaviors → frame → movie, stopping at the first
    // non-passing handler.
    //
    // The previous per-instance loop broke this two ways: with an EMPTY
    // instance list (a Flash sprite carries no behaviors) the loop body never
    // ran, so the static-script fall-through never fired — that's why a
    // `getURL("event: FlashLoaderLoaded")` whose handler lives in a movie
    // script (Neopets DGS `on FlashLoaderLoaded`) never reached it and the DGS
    // loader stalled. And with multiple behaviors it fired the static scripts
    // once per non-handling behavior. `player_invoke_targeted_event` with the
    // full instance list does the right thing in both cases.
    let _ = player_invoke_targeted_event(
        handler_name,
        args,
        Some(&instance_ids),
    ).await;
}

pub async fn player_invoke_event_to_instances(
    handler_name: &str,
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
                    // Don't break — in Director, all behaviors on a sprite
                    // receive the event. `pass` only controls propagation
                    // beyond behaviors to cast member/frame/movie scripts.
                }
            }
            Err(err) => {
                if err.code != ScriptErrorCode::Abort {
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
                }
                // Return the error to caller (abort propagates to stop handler chain)
                return Err(err);
            }
        }
    }
    
    Ok(handled)
}

pub async fn player_invoke_targeted_event(
    handler_name: &str,
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

pub async fn player_invoke_frame_and_movie_scripts(
    handler_name: &str,
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
        if let Some(frame_script) = &frame_script {
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
            (script_member_ref.clone(), handler_name.to_owned()),
            args
        ).await;
        match &result {
            Ok(_) => {}
            Err(err) => {
                if err.code != ScriptErrorCode::Abort {
                    web_sys::console::warn_1(&format!(
                        "  {} handler on {:?} ERROR: {}",
                        handler_name, script_member_ref, err.message
                    ).into());
                }
            }
        }
        let result = result?;

        if !result.passed {
            break;
        }
    }
    Ok(DatumRef::Void)
}

pub async fn player_invoke_static_event(
    handler_name: &str,
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
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    // First stage behavior script
    // Then frame behavior script
    // Then movie script
    // If frame is changed during exitFrame, event is no longer propagated
    // TODO find stage behaviors first

    let active_instance_scripts = reserve_player_mut(|player| {
        let mut active_instance_scripts: Vec<ScriptInstanceRef> = vec![];
        active_instance_scripts.extend(player.active_stage_script_instance_ids());
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

pub async fn player_dispatch_movie_callback(
    handler_name: &str,
) -> Result<(), ScriptError> {
    enum CallbackAction {
        CallHandler(Option<ScriptInstanceRef>, (CastMemberRef, String)),
        EvalText(String),
    }

    let action = reserve_player_ref(|player| {
        let callback = match handler_name {
            "mouseDown" => &player.movie.mouse_down_script,
            "mouseUp" => &player.movie.mouse_up_script,
            "keyDown" => &player.movie.key_down_script,
            "keyUp" => &player.movie.key_up_script,
            _ => return None,
        };
        let callback = callback.as_ref()?;
        match callback {
            ScriptReceiver::ScriptInstance(instance_ref) => {
                let script_instance = player.allocator.get_script_instance(instance_ref);
                let script = player.movie.cast_manager.get_script_by_ref(&script_instance.script)?;
                let handler = script.get_own_handler_ref(&handler_name.to_string())?;
                Some(CallbackAction::CallHandler(Some(instance_ref.clone()), handler))
            }
            ScriptReceiver::Script(script_ref) => {
                let script = player.movie.cast_manager.get_script_by_ref(script_ref)?;
                let handler = script.get_own_handler_ref(&handler_name.to_string())?;
                Some(CallbackAction::CallHandler(None, handler))
            }
            ScriptReceiver::ScriptText(text) => {
                if text.trim().starts_with("--") || text.trim().is_empty() {
                    None
                } else {
                    Some(CallbackAction::EvalText(text.to_owned()))
                }
            }
        }
    });
    match action {
        Some(CallbackAction::CallHandler(receiver, handler)) => {
            player_call_script_handler(receiver, handler, &vec![]).await?;
        }
        Some(CallbackAction::EvalText(text)) => {
            super::eval::eval_lingo_command(text).await?;
        }
        None => {}
    }
    Ok(())
}

/// Tick the per-frame W3D `#timeMS` event registrations. Walks every
/// Shockwave3D member's `runtime_state.registered_events`, fires any whose
/// next deadline has passed, advances the bookkeeping, and removes
/// completed (non-infinite) entries.
///
/// Per the Director docs, `#timeMS` handlers receive five positional args:
/// type (always 0), delta (ms since the last fire), time (ms since the
/// first fire), duration (total ms over all repetitions, or 0 for
/// infinite), and systemTime (absolute ms). Other event types
/// (#collideAny / #collideWith / #animationStarted / #animationEnded)
/// are stored by registerForEvent but not fired here — their producers
/// aren't wired up yet.
pub async fn dispatch_w3d_timer_events() {
    use crate::player::cast_member::CastMemberType;

    let now_ms = crate::player::testing_shared::now_ms();

    // Gather pending fires across all members in one borrow, then dispatch
    // them outside it. Each fire is (member_ref, instance_opt, handler_name, args).
    struct Fire {
        instance: Option<crate::player::script_ref::ScriptInstanceRef>,
        handler_name: String,
        args: [f64; 5],
    }

    let fires: Vec<Fire> = reserve_player_mut(|player| {
        let mut fires: Vec<Fire> = Vec::new();
        let cast_count = player.movie.cast_manager.casts.len();
        for cast_idx in 0..cast_count {
            // We need to walk every member with a 3D runtime. Iterate by
            // (cast_lib, member_number) and fetch via the manager so we can
            // mutate the registered_events list in place.
            let member_numbers: Vec<u32> = player
                .movie
                .cast_manager
                .casts
                .get(cast_idx)
                .map(|c| c.members.keys().copied().collect())
                .unwrap_or_default();
            for member_num in member_numbers {
                let member_ref = crate::player::cast_lib::CastMemberRef {
                    cast_lib: cast_idx as i32,
                    cast_member: member_num as i32,
                };
                let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                else { continue };
                let CastMemberType::Shockwave3d(w3d) = &mut member.member_type else { continue };
                let events = &mut w3d.runtime_state.registered_events;
                if events.is_empty() { continue; }
                let mut i = 0;
                while i < events.len() {
                    let ev = &mut events[i];
                    if !ev.event_name.eq_ignore_ascii_case("timeMS") {
                        i += 1;
                        continue;
                    }
                    // Director: first fire happens at registered_at + begin.
                    // Subsequent fires happen every period_ms after the previous fire.
                    let next_due = if ev.fires_so_far == 0 {
                        ev.registered_at_ms + ev.begin_ms as f64
                    } else {
                        ev.last_fire_ms + ev.period_ms as f64
                    };
                    if now_ms < next_due {
                        i += 1;
                        continue;
                    }
                    let delta = now_ms - ev.last_fire_ms;
                    let time = if ev.fires_so_far == 0 {
                        0.0
                    } else {
                        now_ms - (ev.registered_at_ms + ev.begin_ms as f64)
                    };
                    let duration = if ev.repetitions == 0 {
                        0.0
                    } else if ev.repetitions == 1 {
                        0.0
                    } else {
                        (ev.repetitions as f64 - 1.0) * ev.period_ms as f64
                    };
                    let system_time = now_ms;
                    fires.push(Fire {
                        instance: ev.script_instance.clone(),
                        handler_name: ev.handler_name.clone(),
                        args: [0.0, delta, time, duration, system_time],
                    });
                    ev.fires_so_far += 1;
                    ev.last_fire_ms = now_ms;
                    if ev.repetitions > 0 && ev.fires_so_far >= ev.repetitions {
                        events.remove(i);
                    } else {
                        i += 1;
                    }
                }
            }
        }
        fires
    });

    if fires.is_empty() { return; }
    player_wait_available().await;
    for fire in fires {
        let arg_refs = reserve_player_mut(|player| {
            fire.args.iter()
                .map(|v| player.alloc_datum(Datum::Float(*v)))
                .collect::<Vec<_>>()
        });
        if let Some(inst_ref) = fire.instance {
            let _ = player_invoke_event_to_instances(
                &fire.handler_name,
                &arg_refs,
                &vec![inst_ref],
            ).await;
        } else {
            let _ = player_invoke_static_event(&fire.handler_name, &arg_refs).await;
        }
    }
}

/// Per-frame W3D animation clock advance.
///
/// The renderer was advancing its own `self.animation_time` and never wrote
/// back to `runtime_state.animation_time`. Anything else reading the
/// runtime-state field (e.g. `bone[n].worldTransform` getters used by
/// ClubMarian to pin the head to the body's bone[6]) saw a stale zero, so
/// the body animated but the head stayed frozen at the motion's first
/// frame. Move the dt advance into a per-frame tick on the shared runtime
/// state so every reader sees the same time.
pub async fn tick_w3d_animations() {
    use crate::player::cast_member::CastMemberType;
    use std::sync::atomic::{AtomicU64, Ordering};
    // Wall-clock dt — fixed 1/30 ran the motion at framerate-dependent speed
    // (ClubMarian hits ~24 fps, so a 1/30 advance ran motions at ~80%).
    // Using actual elapsed time keeps motion duration tied to real seconds
    // regardless of the player's frame rate.
    static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);
    let now_ms = crate::player::testing_shared::now_ms() as u64;
    let last = LAST_TICK_MS.swap(now_ms, Ordering::Relaxed);
    let dt_seconds = if last == 0 {
        1.0_f32 / 30.0
    } else {
        // Cap at 100ms to avoid catastrophic jumps after a tab-pause.
        ((now_ms.saturating_sub(last)) as f32 / 1000.0).min(0.1)
    };
    reserve_player_mut(|player| {
        for cast in player.movie.cast_manager.casts.iter_mut() {
            for (_, member) in cast.members.iter_mut() {
                if let CastMemberType::Shockwave3d(w3d) = &mut member.member_type {
                    // ── Per-MODEL bonesPlayer clocks ──
                    // Each model animates independently (Rasterwerks clones N bots into
                    // one member; sharing a single clock froze/cross-posed them). Snapshot
                    // motion durations first (immutable scene borrow) for end-detection.
                    let motion_durations: Vec<(String, f32)> = w3d.parsed_scene.as_ref()
                        .map(|s| s.motions.iter()
                            .map(|m| (m.name.to_ascii_lowercase(), m.duration()))
                            .collect())
                        .unwrap_or_default();
                    let dur_of = |name: &str| -> f32 {
                        let nl = name.to_ascii_lowercase();
                        motion_durations.iter().find(|(n, _)| *n == nl).map(|(_, d)| *d).unwrap_or(0.0)
                    };
                    for bp in w3d.runtime_state.bones_players.values_mut() {
                        if !bp.animation_playing || bp.motion_ended { continue; }
                        bp.animation_time += dt_seconds * bp.play_rate * bp.animation_scale;
                        if bp.blend_weight < 1.0 && bp.blend_duration > 0.0 {
                            bp.blend_elapsed += dt_seconds;
                            bp.blend_weight = (bp.blend_elapsed / bp.blend_duration).min(1.0);
                        }
                        // Non-looping end: advance the per-model queue, else hold last frame.
                        if !bp.animation_loop {
                            if let Some(cur) = bp.current_motion.clone() {
                                let duration = dur_of(&cur);
                                let eff_end = if bp.animation_end_time >= 0.0 {
                                    bp.animation_end_time.min(duration)
                                } else { duration };
                                if eff_end > 0.0 && bp.animation_time >= eff_end {
                                    if !bp.motion_queue.is_empty() {
                                        let q = bp.motion_queue.remove(0);
                                        bp.current_motion = Some(q.name);
                                        bp.animation_loop = q.looped;
                                        bp.animation_start_time = q.start_time;
                                        bp.animation_end_time = q.end_time;
                                        bp.animation_scale = q.scale;
                                        bp.animation_time = if q.offset >= 0.0 { q.offset } else { q.start_time };
                                        bp.motion_ended = false;
                                        bp.previous_motion = None;
                                        bp.blend_weight = 1.0;
                                    } else {
                                        bp.animation_time = eff_end; // hold final frame
                                        bp.motion_ended = true;
                                    }
                                }
                            }
                        }
                    }

                    // ── Legacy member clock (keyframe / motion_transforms path) ──
                    if w3d.runtime_state.animation_playing {
                        let rate = w3d.runtime_state.play_rate;
                        let scale = w3d.runtime_state.animation_scale;
                        w3d.runtime_state.animation_time += dt_seconds * rate * scale;
                        if w3d.runtime_state.blend_weight < 1.0 && w3d.runtime_state.blend_duration > 0.0 {
                            w3d.runtime_state.blend_elapsed += dt_seconds;
                            w3d.runtime_state.blend_weight =
                                (w3d.runtime_state.blend_elapsed / w3d.runtime_state.blend_duration).min(1.0);
                        }
                    }
                }
            }
        }
    });
}

/// Advance every W3D member's #particle systems once per frame. Separate from
/// tick_w3d_animations because particles run regardless of animation_playing
/// (that tick early-returns for members with no playing motion). For each
/// particle model resource we sync the emitter parameters (set via
/// `resource.emitter.*` into runtime_state.emitters) and the world emit position
/// (from the model node that references the resource) into the sim, then step it.
pub async fn tick_w3d_particles() {
    use crate::player::cast_member::CastMemberType;
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);
    let now_ms = crate::player::testing_shared::now_ms() as u64;
    let last = LAST_TICK_MS.swap(now_ms, Ordering::Relaxed);
    let dt = if last == 0 {
        1.0_f32 / 30.0
    } else {
        ((now_ms.saturating_sub(last)) as f32 / 1000.0).min(0.1)
    };
    reserve_player_mut(|player| {
        for cast in player.movie.cast_manager.casts.iter_mut() {
            for (_, member) in cast.members.iter_mut() {
                if let CastMemberType::Shockwave3d(w3d) = &mut member.member_type {
                    if w3d.runtime_state.particles.is_empty() { continue; }
                    let scene = w3d.parsed_scene.clone();
                    let names: Vec<String> = w3d.runtime_state.particles.keys().cloned().collect();
                    for name in names {
                        // Emitter params (cloned) and the emit position from the model
                        // node that references this resource — gathered before the
                        // mutable particle borrow to avoid overlapping borrows.
                        let em = w3d.runtime_state.emitters.get(&name).cloned();
                        let world_pos = scene.as_ref().and_then(|sc| {
                            sc.nodes.iter()
                                .find(|n| n.model_resource_name.eq_ignore_ascii_case(&name)
                                    || n.resource_name.eq_ignore_ascii_case(&name))
                                .map(|n| {
                                    let t = w3d.runtime_state.node_transforms.get(&n.name)
                                        .copied()
                                        .unwrap_or(n.transform);
                                    [t[12], t[13], t[14]]
                                })
                        }).unwrap_or([0.0, 0.0, 0.0]);

                        if let Some(ps) = w3d.runtime_state.particles.get_mut(&name) {
                            // Emit from emitter.region when a script set it (e.g. the car demos
                            // track the exhaust pipe via `emitter.region = [exhaust.worldPosition]`),
                            // otherwise from the model node's world position (the faucet translates
                            // its ColdWater model). Without this the car smoke emitted at the origin
                            // and whited out the camera, blanking the whole 3D scene.
                            ps.emitter_position = match &em {
                                Some(e) if e.has_region =>
                                    [e.region[0] as f32, e.region[1] as f32, e.region[2] as f32],
                                _ => world_pos,
                            };
                            if let Some(em) = &em {
                                let d = em.direction;
                                let len = (d[0]*d[0] + d[1]*d[1] + d[2]*d[2]).sqrt();
                                if len > 1e-6 {
                                    ps.direction = [(d[0]/len) as f32, (d[1]/len) as f32, (d[2]/len) as f32];
                                }
                                ps.initial_speed = em.min_speed as f32;
                                ps.max_speed = em.max_speed.max(em.min_speed) as f32;
                                ps.speed_range = (em.max_speed - em.min_speed).max(0.0) as f32;
                                // emitter.angle is the cone half-angle in degrees.
                                ps.angle_range = (em.angle as f32).to_radians();
                                ps.stream = em.mode.eq_ignore_ascii_case("stream");
                                let n = (em.num_particles.max(0) as usize).min(10000);
                                if n > 0 && ps.max_particles != n {
                                    ps.initialize(n);
                                }
                            }
                            if ps.positions.is_empty() && ps.max_particles > 0 {
                                ps.initialize(ps.max_particles);
                            }
                            ps.update(dt);
                        }
                    }
                }
            }
        }
    });
}

// ─── Native Shockwave3D #collision modifier helpers ───
const W3D_COL_IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Column-major 4x4 multiply (m[col*4+row]).
fn w3d_col_mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] = a[row] * b[col * 4]
                + a[4 + row] * b[col * 4 + 1]
                + a[8 + row] * b[col * 4 + 2]
                + a[12 + row] * b[col * 4 + 3];
        }
    }
    r
}

/// Transform a point by a column-major 4x4 (w = 1).
fn w3d_col_xform_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
    ]
}

/// Per-frame native W3D collision detection (Director #collision modifier).
///
/// `addModifier(#collision)` registers a `W3dCollisionModifier` per model; this
/// sweeps every enabled collision model in each 3D member, builds a world-space
/// AABB for each (from the model resource's primitive dimensions and the node's
/// accumulated world transform), tests all pairs for overlap, and fires each
/// colliding model's `setCollisionCallback` handler with a `collisionData`
/// property list (`#modelA / #modelB / #pointOfContact / #collisionNormal`).
/// Only detection + dispatch are implemented; resolution is intentionally a
/// no-op (every consumer so far sets `collision.resolve = 0`).
pub async fn tick_w3d_collisions() {
    use crate::director::lingo::datum::Shockwave3dObjectRef;
    use crate::player::cast_member::CastMemberType;
    use std::collections::VecDeque;

    struct ColModel {
        name: String,
        min: [f32; 3],
        max: [f32; 3],
        immovable: bool,
        handler: Option<String>,
        instance: Option<ScriptInstanceRef>,
        target: Option<crate::director::lingo::datum::Datum>,
    }
    // A member-level #collideAny registration (registerForEvent(#collideAny, …)).
    struct CollideAnyReg {
        handler: String,
        instance: Option<ScriptInstanceRef>,
    }
    struct Fire {
        instance: Option<ScriptInstanceRef>,
        handler: String,
        data: DatumRef,
        target: Option<DatumRef>,
    }

    let fires: Vec<Fire> = reserve_player_mut(|player| {
        // Flush any live transform datums (model.transform.position = ...) into
        // the node_transforms cache so collision reads current positions rather
        // than the previous render's snapshot.
        crate::player::handlers::datum_handlers::shockwave3d_object::sync_persistent_transforms(player);

        let mut fires: Vec<Fire> = Vec::new();
        let cast_count = player.movie.cast_manager.casts.len();
        for cast_idx in 0..cast_count {
            let member_numbers: Vec<u32> = player
                .movie
                .cast_manager
                .casts
                .get(cast_idx)
                .map(|c| c.members.keys().copied().collect())
                .unwrap_or_default();
            for member_num in member_numbers {
                let member_ref = CastMemberRef {
                    cast_lib: cast_idx as i32,
                    cast_member: member_num as i32,
                };

                // Gather world AABBs + callbacks for every enabled collision
                // model in this member (owned, so the borrow ends before alloc).
                let (models, collide_any_regs): (Vec<ColModel>, Vec<CollideAnyReg>) = {
                    let Some(member) = player.movie.cast_manager.find_member_by_ref(&member_ref)
                    else { continue };
                    let CastMemberType::Shockwave3d(w3d) = &member.member_type else { continue };
                    if w3d.runtime_state.collision_modifiers.is_empty() { continue; }
                    let Some(scene) = w3d.parsed_scene.as_ref() else { continue };
                    let rs = &w3d.runtime_state;

                    // Local transform of a node: runtime override (case-insensitive)
                    // falling back to the parsed scene node.
                    let local_tf = |nm: &str| -> [f32; 16] {
                        rs.node_transforms
                            .get(nm)
                            .copied()
                            .or_else(|| {
                                rs.node_transforms
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case(nm))
                                    .map(|(_, v)| *v)
                            })
                            .or_else(|| {
                                scene
                                    .nodes
                                    .iter()
                                    .find(|n| n.name.eq_ignore_ascii_case(nm))
                                    .map(|n| n.transform)
                            })
                            .unwrap_or(W3D_COL_IDENTITY)
                    };

                    let mut out: Vec<ColModel> = Vec::new();
                    for (mname, cmod) in rs.collision_modifiers.iter() {
                        if !cmod.enabled { continue; }
                        if rs.detached_nodes.iter().any(|d| d.eq_ignore_ascii_case(mname)) { continue; }
                        let Some(node) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(mname))
                        else { continue };

                        // Local half-extents from the model resource's primitive dims.
                        let res_key = if !node.model_resource_name.is_empty() {
                            &node.model_resource_name
                        } else {
                            &node.resource_name
                        };
                        let prim_he = scene
                            .model_resources
                            .get(res_key.as_str())
                            .and_then(|r| match r.primitive_type.as_deref().unwrap_or("") {
                                // box: width=X, height=Y, length=Z (see primitive build).
                                "box" => Some([
                                    r.primitive_width * 0.5,
                                    r.primitive_height * 0.5,
                                    r.primitive_length * 0.5,
                                ]),
                                "sphere" => {
                                    let s = r.primitive_radius.max(0.01);
                                    Some([s, s, s])
                                }
                                // cylinder axis is Y; radius spans X/Z.
                                "cylinder" => {
                                    let rad = r.primitive_radius.max(r.primitive_top_radius).max(0.01);
                                    Some([rad, r.primitive_height * 0.5, rad])
                                }
                                "plane" => Some([r.primitive_width * 0.5, 0.05, r.primitive_length * 0.5]),
                                // Mesh model (no primitive dims): fall through to mesh bounds.
                                _ => None,
                            });
                        // For mesh models, size the collision box from the actual
                        // geometry so #collision matches the visible model. (The old
                        // 0.5-unit fallback never reached On the Run's bonuses, which
                        // float ~1.5 above the road.) Half-extents are taken about the
                        // model origin (max |min|,|max| per axis), matching the
                        // origin-centred ±he box used below.
                        let he = prim_he.unwrap_or_else(|| {
                            let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
                            let mut any = false;
                            if let Some(meshes) = scene.clod_meshes.get(res_key.as_str()) {
                                for m in meshes {
                                    for p in &m.positions {
                                        any = true;
                                        for k in 0..3 { if p[k] < mn[k] { mn[k] = p[k]; } if p[k] > mx[k] { mx[k] = p[k]; } }
                                    }
                                }
                            }
                            for rm in scene.raw_meshes.iter().filter(|m| m.name.eq_ignore_ascii_case(res_key.as_str())) {
                                for p in &rm.positions {
                                    any = true;
                                    for k in 0..3 { if p[k] < mn[k] { mn[k] = p[k]; } if p[k] > mx[k] { mx[k] = p[k]; } }
                                }
                            }
                            if any {
                                [
                                    mn[0].abs().max(mx[0].abs()).max(0.05),
                                    mn[1].abs().max(mx[1].abs()).max(0.05),
                                    mn[2].abs().max(mx[2].abs()).max(0.05),
                                ]
                            } else {
                                [0.5, 0.5, 0.5]
                            }
                        });

                        // Accumulate the world transform up the parent chain.
                        let mut world = local_tf(&node.name);
                        let mut cur_parent = node.parent_name.clone();
                        for _ in 0..32 {
                            if cur_parent.is_empty() || cur_parent.eq_ignore_ascii_case("World") {
                                break;
                            }
                            let pm = local_tf(&cur_parent);
                            world = w3d_col_mat_mul(&pm, &world);
                            cur_parent = scene
                                .nodes
                                .iter()
                                .find(|n| n.name.eq_ignore_ascii_case(&cur_parent))
                                .map(|n| n.parent_name.clone())
                                .unwrap_or_default();
                        }

                        // World AABB from the 8 transformed local-box corners.
                        let mut min = [f32::MAX; 3];
                        let mut max = [f32::MIN; 3];
                        for &sx in &[-he[0], he[0]] {
                            for &sy in &[-he[1], he[1]] {
                                for &sz in &[-he[2], he[2]] {
                                    let wp = w3d_col_xform_point(&world, [sx, sy, sz]);
                                    for k in 0..3 {
                                        if wp[k] < min[k] { min[k] = wp[k]; }
                                        if wp[k] > max[k] { max[k] = wp[k]; }
                                    }
                                }
                            }
                        }

                        out.push(ColModel {
                            name: node.name.clone(),
                            min,
                            max,
                            immovable: cmod.immovable,
                            handler: cmod.callback_handler.clone(),
                            instance: cmod.callback_instance.clone(),
                            target: cmod.callback_target.clone(),
                        });
                    }
                    // Member-level #collideAny handlers (registerForEvent(#collideAny,
                    // handler, scriptInstance)) — fired once per colliding pair below.
                    let collide_any: Vec<CollideAnyReg> = rs.registered_events.iter()
                        .filter(|e| e.event_name.eq_ignore_ascii_case("collideAny")
                            && !e.handler_name.is_empty())
                        .map(|e| CollideAnyReg {
                            handler: e.handler_name.clone(),
                            instance: e.script_instance.clone(),
                        })
                        .collect();
                    (out, collide_any)
                };

                // Test all pairs; for each overlapping pair, fire the callback of
                // every model in the pair that has one, with that model as modelA.
                for i in 0..models.len() {
                    for j in (i + 1)..models.len() {
                        let a = &models[i];
                        let b = &models[j];
                        let overlap = a.min[0] <= b.max[0] && a.max[0] >= b.min[0]
                            && a.min[1] <= b.max[1] && a.max[1] >= b.min[1]
                            && a.min[2] <= b.max[2] && a.max[2] >= b.min[2];
                        if !overlap { continue; }

                        let mid = [
                            ((a.min[0] + a.max[0] + b.min[0] + b.max[0]) * 0.25) as f64,
                            ((a.min[1] + a.max[1] + b.min[1] + b.max[1]) * 0.25) as f64,
                            ((a.min[2] + a.max[2] + b.min[2] + b.max[2]) * 0.25) as f64,
                        ];
                        for (selfm, otherm) in [(a, b), (b, a)] {
                            let Some(handler) = selfm.handler.clone() else { continue };
                            // collisionData property list: modelA is the model whose
                            // callback is firing; the handlers check both modelA and
                            // modelB, so this is consistent with Director.
                            let model_a = player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                cast_lib: member_ref.cast_lib,
                                cast_member: member_ref.cast_member,
                                object_type: "model".to_string(),
                                name: selfm.name.clone(),
                            }));
                            let model_b = player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                cast_lib: member_ref.cast_lib,
                                cast_member: member_ref.cast_member,
                                object_type: "model".to_string(),
                                name: otherm.name.clone(),
                            }));
                            let poc = player.alloc_datum(Datum::Vector(mid));
                            let normal = player.alloc_datum(Datum::Vector([1.0, 0.0, 0.0]));
                            let k_a = player.alloc_datum(Datum::Symbol("modelA".to_string()));
                            let k_b = player.alloc_datum(Datum::Symbol("modelB".to_string()));
                            let k_poc = player.alloc_datum(Datum::Symbol("pointOfContact".to_string()));
                            let k_n = player.alloc_datum(Datum::Symbol("collisionNormal".to_string()));
                            let pairs: VecDeque<(DatumRef, DatumRef)> = VecDeque::from(vec![
                                (k_a, model_a),
                                (k_b, model_b),
                                (k_poc, poc),
                                (k_n, normal),
                            ]);
                            let data = player.alloc_datum(Datum::PropList(pairs, false));
                            let target = selfm.target.clone().map(|t| player.alloc_datum(t));
                            fires.push(Fire {
                                instance: selfm.instance.clone(),
                                handler,
                                data,
                                target,
                            });
                        }

                        // Member-level #collideAny handlers (registerForEvent) fire
                        // once per colliding pair. Director 11.5 `collisionData`:
                        // modelA / modelB / pointOfContact / collisionNormal — modelA
                        // and modelB are "one"/"the other" of the pair. Order the
                        // immovable model as modelB, the convention On the Run's bonus
                        // pickup relies on (collisionData.modelB = the bonus hit).
                        if !collide_any_regs.is_empty() {
                            let (ma, mb) = if a.immovable && !b.immovable { (b, a) } else { (a, b) };
                            for reg in &collide_any_regs {
                                let model_a = player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: member_ref.cast_lib,
                                    cast_member: member_ref.cast_member,
                                    object_type: "model".to_string(),
                                    name: ma.name.clone(),
                                }));
                                let model_b = player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                    cast_lib: member_ref.cast_lib,
                                    cast_member: member_ref.cast_member,
                                    object_type: "model".to_string(),
                                    name: mb.name.clone(),
                                }));
                                let poc = player.alloc_datum(Datum::Vector(mid));
                                let normal = player.alloc_datum(Datum::Vector([1.0, 0.0, 0.0]));
                                let k_a = player.alloc_datum(Datum::Symbol("modelA".to_string()));
                                let k_b = player.alloc_datum(Datum::Symbol("modelB".to_string()));
                                let k_poc = player.alloc_datum(Datum::Symbol("pointOfContact".to_string()));
                                let k_n = player.alloc_datum(Datum::Symbol("collisionNormal".to_string()));
                                let pairs: VecDeque<(DatumRef, DatumRef)> = VecDeque::from(vec![
                                    (k_a, model_a),
                                    (k_b, model_b),
                                    (k_poc, poc),
                                    (k_n, normal),
                                ]);
                                let data = player.alloc_datum(Datum::PropList(pairs, false));
                                fires.push(Fire {
                                    instance: reg.instance.clone(),
                                    handler: reg.handler.clone(),
                                    data,
                                    target: None,
                                });
                            }
                        }
                    }
                }
            }
        }
        fires
    });

    if fires.is_empty() { return; }
    player_wait_available().await;
    for fire in fires {
        if let Some(inst) = fire.instance {
            // Script-instance target: `me` is the instance, collisionData is the
            // single explicit arg (`on handler me, collisionData`).
            let _ = player_invoke_event_to_instances(&fire.handler, &vec![fire.data], &vec![inst]).await;
        } else {
            // Non-instance target (e.g. `setCollisionCallback(#collision, member("scene"))`):
            // Director passes the registered object as the handler's FIRST arg and
            // collisionData as the SECOND (`on collision target, collisionData`).
            // Without the target, collisionData lands in the first param and the
            // collisionData param is VOID (Splat's `on collision s, collisionData`).
            let args = match fire.target {
                Some(target) => vec![target, fire.data],
                None => vec![fire.data],
            };
            let _ = player_invoke_static_event(&fire.handler, &args).await;
        }
    }
}

/// Drain queued PhysX collision reports across all PhysX members and
/// dispatch them to the script's registered `collisionCallback` handler.
///
/// PhysX simulate() runs synchronously inside a Lingo call and only queues
/// collisions onto `physx.state.pending_collisions`. AGEIA's xtra fires the
/// registered callback automatically after each simulate; we approximate
/// that by sweeping queued reports here, after prepareFrame behaviors (the
/// usual home of simulate()), and invoking the handler globally.
///
/// The arg shape mirrors `notify_collisions`: one positional `collisions`
/// list, each entry a PropList of `#objectA / #objectB / #contactPoints /
/// #contactNormals`. ClubMarian's `on collisionCallback collisions` reads
/// `collisions[i].objectA.name` to test for terrain — relies on this.
pub async fn dispatch_physx_collision_callbacks() {
    use crate::director::lingo::datum::{DatumType, PhysXObjectRef};
    use crate::player::cast_member::CastMemberType;
    use std::collections::VecDeque;

    struct Fire {
        handler_name: String,
        cast_lib: i32,
        cast_member: i32,
        // (a_id, b_id, a_name, b_name, points, normals)
        collisions: Vec<(u32, u32, Option<String>, Option<String>, Vec<[f64; 3]>, Vec<[f64; 3]>)>,
    }

    let fires: Vec<Fire> = reserve_player_mut(|player| {
        let mut fires: Vec<Fire> = Vec::new();
        let cast_count = player.movie.cast_manager.casts.len();
        for cast_idx in 0..cast_count {
            let member_numbers: Vec<u32> = player
                .movie
                .cast_manager
                .casts
                .get(cast_idx)
                .map(|c| c.members.keys().copied().collect())
                .unwrap_or_default();
            for member_num in member_numbers {
                let member_ref = CastMemberRef {
                    cast_lib: cast_idx as i32,
                    cast_member: member_num as i32,
                };
                let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                else { continue };
                let CastMemberType::PhysXPhysics(physx) = &mut member.member_type else { continue };
                if physx.state.pending_collisions.is_empty() { continue; }
                let Some(handler) = physx.state.collision_callback_handler.clone() else {
                    // No handler registered — drop the queue so it doesn't grow
                    // unbounded. Director's xtra discards reports the script
                    // never asked to hear about.
                    physx.state.pending_collisions.clear();
                    continue;
                };
                // Resolve body names while we still hold the borrow.
                let body_names: std::collections::HashMap<u32, String> =
                    physx.state.bodies.iter().map(|b| (b.id, b.name.clone())).collect();
                let mut drained: Vec<(u32, u32, Vec<[f64; 3]>, Vec<[f64; 3]>)> = Vec::new();
                std::mem::swap(&mut drained, &mut physx.state.pending_collisions);
                let collisions = drained.into_iter().map(|(a, b, pts, nms)| {
                    let na = body_names.get(&a).cloned();
                    let nb = body_names.get(&b).cloned();
                    (a, b, na, nb, pts, nms)
                }).collect();
                fires.push(Fire {
                    handler_name: handler,
                    cast_lib: cast_idx as i32,
                    cast_member: member_num as i32,
                    collisions,
                });
            }
        }
        fires
    });

    if fires.is_empty() { return; }
    player_wait_available().await;

    for fire in fires {
        let arg_refs = reserve_player_mut(|player| {
            let cast_lib = fire.cast_lib;
            let cast_member = fire.cast_member;
            let mut reports = VecDeque::new();
            for (a_id, b_id, name_a, name_b, points, normals) in &fire.collisions {
                let a_ref = match name_a {
                    Some(n) => player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                        cast_lib, cast_member,
                        object_type: "rigidBody".to_string(),
                        id: *a_id, name: n.clone(),
                    })),
                    None => DatumRef::Void,
                };
                let b_ref = match name_b {
                    Some(n) => player.alloc_datum(Datum::PhysXObjectRef(PhysXObjectRef {
                        cast_lib, cast_member,
                        object_type: "rigidBody".to_string(),
                        id: *b_id, name: n.clone(),
                    })),
                    None => DatumRef::Void,
                };
                let mut pts = VecDeque::new();
                for p in points { pts.push_back(player.alloc_datum(Datum::Vector(*p))); }
                let pts_list = player.alloc_datum(Datum::List(DatumType::List, pts, false));
                let mut nms = VecDeque::new();
                for n in normals { nms.push_back(player.alloc_datum(Datum::Vector(*n))); }
                let nms_list = player.alloc_datum(Datum::List(DatumType::List, nms, false));
                let key_a = player.alloc_datum(Datum::Symbol("objectA".to_string()));
                let key_b = player.alloc_datum(Datum::Symbol("objectB".to_string()));
                let key_pts = player.alloc_datum(Datum::Symbol("contactPoints".to_string()));
                let key_nms = player.alloc_datum(Datum::Symbol("contactNormals".to_string()));
                let mut props = VecDeque::new();
                props.push_back((key_a, a_ref));
                props.push_back((key_b, b_ref));
                props.push_back((key_pts, pts_list));
                props.push_back((key_nms, nms_list));
                reports.push_back(player.alloc_datum(Datum::PropList(props, false)));
            }
            let collisions_list = player.alloc_datum(Datum::List(DatumType::List, reports, false));
            vec![collisions_list]
        });
        let _ = player_invoke_global_event(&fire.handler_name, &arg_refs).await;
    }
}

pub async fn run_event_loop(rx: Receiver<PlayerVMEvent>) {
    warn!("Starting event loop");
    // Snapshot the player generation this loop belongs to. If the generation
    // changes (another test reset the player), this loop is stale and must
    // exit so it can't dispatch events on the new player's state.
    let generation = unsafe { crate::player::PLAYER_GENERATION };

    while !rx.is_closed() {
        if unsafe { crate::player::PLAYER_GENERATION } != generation {
            warn!("Event loop stopped (generation changed)");
            return;
        }
        let item = match rx.recv().await {
            Ok(item) => item,
            Err(_) => break, // Channel closed (sender dropped)
        };
        if unsafe { crate::player::PLAYER_GENERATION } != generation {
            warn!("Event loop stopped after recv (generation changed)");
            return;
        }
        player_wait_available().await;
        // After the semaphore yield, re-check generation before touching any
        // player state — a reset during the yield would make this dispatch
        // stale and any handler call would run on the new player.
        if unsafe { crate::player::PLAYER_GENERATION } != generation {
            warn!("Event loop stopped after semaphore (generation changed)");
            return;
        }
        if !player_is_playing().await {
            continue;
        }
        // Skip event processing when a command handler (mouseDown, keyDown, etc.)
        // is actively executing. The event loop and command loop share the scope
        // stack, so processing events (which push/pop scopes) during a command
        // handler's async yield (e.g. nothing()) would corrupt the scope data.
        // Ephemeral events like mouseWithin will be re-dispatched on the next tick.
        let skip = reserve_player_ref(|player| player.in_mouse_command || player.command_handler_yielding || player.is_in_transition);
        if skip {
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
                if err.code != super::ScriptErrorCode::Abort {
                    // TODO ignore error if it's a CancelledException
                    // TODO print stack trace
                    reserve_player_mut(|player| player.on_script_error(&err));
                }
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
            if err.code != ScriptErrorCode::Abort {
                reserve_player_mut(|player| player.on_script_error(&err));
            }
            DatumRef::Void
        }
    }
}

pub async fn player_dispatch_event_beginsprite(
    handler_name: &str,
    args: &Vec<DatumRef>
) -> Result<Vec<(ScoreRef, u32)>, ScriptError> {
    let (sprite_instances, frame_instances, all_channels) =
        reserve_player_mut(|player| {
            let mut sprite_instances: Vec<(ScoreRef, usize, ScriptInstanceRef)> = Vec::new();
            let mut frame_instances: Vec<(usize, ScriptInstanceRef)> = Vec::new();
            let mut all_channels = Vec::new();

            // Collect stage sprites - include all entered sprites with behaviors,
            // not just those in sprite_spans. Sprites initialized from channel_initialization_data
            // (D6+ path) also need beginSprite dispatched.
            for channel_number in player.active_stage_behavior_channels() {
                let Some((number, entered, fallback)) = player
                    .movie
                    .score
                    .channels
                    .get(channel_number)
                    .map(|channel| {
                        (
                            channel.number,
                            channel.sprite.entered,
                            channel.sprite.script_instance_list.clone(),
                        )
                    })
                else {
                    continue;
                };

                if !entered {
                    continue;
                }

                let instances = player.get_sprite_script_instance_ids(number as i16, fallback.as_slice());
                if instances.is_empty() || instances.iter().any(|script_ref| {
                    player
                        .allocator
                        .get_script_instance_entry(script_ref.id())
                        .map_or(true, |entry| entry.script_instance.begin_sprite_called)
                }) {
                    continue;
                }

                if number == 0 {
                    // Frame behavior (channel 0)
                    frame_instances.extend(
                        instances.into_iter().map(|inst| (number, inst))
                    );
                } else {
                    // Sprite behaviors (channel > 0) - include ScoreRef::Stage
                    sprite_instances.extend(
                        instances
                            .into_iter()
                            .map(|inst| (ScoreRef::Stage, number, inst))
                    );
                }

                all_channels.push((ScoreRef::Stage, number as u32));
            }

            // Collect filmloop sprites
            let active_filmloops = player.get_active_filmloop_scores();
            for (member_ref, filmloop_current_frame) in active_filmloops {
                let Some(filmloop_score) = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .and_then(|member| match &member.member_type {
                        super::cast_member::CastMemberType::FilmLoop(film_loop) => {
                            Some(&film_loop.score)
                        }
                        _ => None,
                    })
                else {
                    continue;
                };
                for channel_number in filmloop_score.active_channel_numbers_for_frame(filmloop_current_frame) {
                    let Some(channel) = filmloop_score.channels.get(channel_number) else {
                        continue;
                    };
                    if channel.sprite.script_instance_list.is_empty() || !channel.sprite.entered {
                        continue;
                    }
                    if channel.sprite.script_instance_list.iter().any(|script_ref| {
                        player
                            .allocator
                            .get_script_instance_entry(script_ref.id())
                            .map_or(true, |entry| entry.script_instance.begin_sprite_called)
                    }) {
                        continue;
                    }

                    let instances = channel.sprite.script_instance_list.clone();

                    // Filmloop sprites go into sprite_instances (they don't have frame behaviors)
                    // Include ScoreRef::FilmLoop so we can set the correct context when dispatching
                    if channel.number > 0 {
                        let score_ref = ScoreRef::FilmLoop(member_ref.clone());
                        sprite_instances.extend(
                            instances.into_iter().map(|inst| (score_ref.clone(), channel.number, inst))
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
    // Set the score context before invoking each event so sprite property access works correctly
    for (score_ref, sprite_number, behavior) in sprite_instances {
        // Set the score context for this sprite's behavior
        reserve_player_mut(|player| {
            player.current_score_context = score_ref.clone();
        });

        let receivers = vec![behavior.clone()];
        if let Err(err) = player_invoke_targeted_event(handler_name, args, Some(receivers).as_ref()).await {
            if err.code == ScriptErrorCode::Abort {
                reserve_player_mut(|player| {
                    player.current_score_context = ScoreRef::Stage;
                });
                return Ok(vec![]);
            }
            web_sys::console::error_1(
                &format!("Error in {} for sprite {}: {}", handler_name, sprite_number, err.message).into()
            );
            reserve_player_mut(|player| {
                player.on_script_error(&err);
            });
        }
    }

    // Reset the score context to Stage after each invocation
    reserve_player_mut(|player| {
        player.current_score_context = ScoreRef::Stage;
    });

    Ok(all_channels)
}

pub async fn dispatch_event_endsprite(sprite_nums: Vec<u32>) {
    // Legacy function - calls the new implementation with stage score
    dispatch_event_endsprite_for_score(ScoreRef::Stage, sprite_nums).await;
}

pub async fn dispatch_event_endsprite_for_score(score_ref: ScoreRef, sprite_nums: Vec<u32>) {
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

            for channel_number in sprite_nums.iter().copied().unique() {
                let Some(channel) = score.channels.get(channel_number as usize) else {
                    continue;
                };

                if channel.sprite.script_instance_list.is_empty() {
                    continue;
                }

                let entry = (
                    channel.sprite.number as u16,
                    channel.sprite.script_instance_list.clone(),
                );

                if channel.number > 0 {
                    sprite_tuple.push(entry);
                } else {
                    frame_tuple.push(entry);
                }
            }

            (sprite_tuple, frame_tuple)
        });

    // Dispatch to frame behaviors first (number == 0)
    if frame_tuple.len() > 0 {
        let _ = player_invoke_frame_and_movie_scripts(&"endSprite".to_string(), &vec![]).await;
    }

    // Set the score context for this dispatch
    reserve_player_mut(|player| {
        player.current_score_context = score_ref.clone();
    });

    // Dispatch to sprite behaviors (number > 0)
    for (sprite_num, behaviors) in sprite_tuple {
        for behavior in behaviors {
            let receivers = vec![behavior.clone()];

            if let Err(err) = player_invoke_event_to_instances(
                    &"endSprite".to_string(), &vec![], &receivers
                ).await {
                if err.code == ScriptErrorCode::Abort {
                    break;
                }
                web_sys::console::error_1(
                    &format!("Error in endSprite for sprite {}: {}", sprite_num, err.message).into()
                );
                reserve_player_mut(|player| {
                    player.on_script_error(&err);
                });
            }
        }
    }

    // Reset the score context to Stage
    reserve_player_mut(|player| {
        player.current_score_context = ScoreRef::Stage;
    });
}

pub async fn dispatch_event_to_all_behaviors(
    handler_name: &str,
    args: &Vec<DatumRef>,
) {
    use crate::player::allocator::ScriptInstanceAllocatorTrait;
    use crate::js_api::ascii_safe;
    // Skip event dispatch if we're initializing behavior properties
    let skip = reserve_player_mut(|player| {
        if player.is_initializing_behavior_props {
            warn!(
                "Blocking event '{}' during property initialization",
                handler_name
            );
            return true;
        }
        // Prevent re-entrant event dispatch (this can cause infinite loops)
        if player.is_dispatching_events {
            debug!(
                "Blocking re-entrant event dispatch for '{}'",
                handler_name
            );
            return true;
        }
        player.is_dispatching_events = true;
        false
    });

    if skip {
        return;
    }
    // Include ScoreRef to track which score context each sprite belongs to
    let (sprite_behaviors, _frame_behaviors) = reserve_player_mut(|player| {
        let mut sprites: Vec<(ScoreRef, usize, Vec<ScriptInstanceRef>)> = Vec::new();
        let mut frames = Vec::new();

        for channel_number in player.active_stage_behavior_channels() {
            let Some((number, entered, puppet, fallback)) = player
                .movie
                .score
                .channels
                .get(channel_number)
                .map(|channel| {
                    (
                        channel.number,
                        channel.sprite.entered,
                        channel.sprite.puppet,
                        channel.sprite.script_instance_list.clone(),
                    )
                })
            else {
                continue;
            };

            if !entered && !puppet {
                continue;
            }

            let behaviors =
                player.get_sprite_script_instance_ids(number as i16, fallback.as_slice());
            if behaviors.is_empty() {
                continue;
            }

            if number > 0 {
                sprites.push((ScoreRef::Stage, number, behaviors));
            } else if number == 0 {
                frames.push((number, behaviors));  // Store tuple with channel number
            }
        }

        // Collect filmloop sprites
        let active_filmloops = player.get_active_filmloop_scores();
        for (member_ref, filmloop_current_frame) in active_filmloops {
            let Some(filmloop_score) = player
                .movie
                .cast_manager
                .find_member_by_ref(&member_ref)
                .and_then(|member| match &member.member_type {
                    super::cast_member::CastMemberType::FilmLoop(film_loop) => {
                        Some(&film_loop.score)
                    }
                    _ => None,
                })
            else {
                continue;
            };
            for channel_number in filmloop_score.active_channel_numbers_for_frame(filmloop_current_frame) {
                let Some(channel) = filmloop_score.channels.get(channel_number) else {
                    continue;
                };
                if channel.sprite.script_instance_list.is_empty() || !channel.sprite.entered {
                    continue;
                }
                let behaviors = channel.sprite.script_instance_list.clone();
                if channel.number > 0 {
                    sprites.push((ScoreRef::FilmLoop(member_ref.clone()), channel.number, behaviors));
                }
            }
        }

        (sprites, frames)
    });
    // Dispatch to sprite behaviors first (channel order)
    // Set the score context before invoking each event so sprite property access works correctly
    for (score_ref, sprite_number, behaviors) in sprite_behaviors {
        // Set the score context for this sprite's behaviors
        reserve_player_mut(|player| {
            player.current_score_context = score_ref.clone();
        });

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
                if err.code == ScriptErrorCode::Abort {
                    // abort is flow control: stop all remaining handlers
                    reserve_player_mut(|player| {
                        player.is_dispatching_events = false;
                        player.current_score_context = ScoreRef::Stage;
                    });
                    return;
                }
                web_sys::console::error_1(
                    &format!("Error in {} for sprite {}: {}", handler_name, sprite_number, err.message).into()
                );
                reserve_player_mut(|player| {
                    player.on_script_error(&err);
                });
            }
        }

        // Reset the score context to Stage after processing this sprite's behaviors
        reserve_player_mut(|player| {
            player.current_score_context = ScoreRef::Stage;
        });
    }
    // Dispatch event to frame/movie scripts
    if let Err(err) = player_invoke_frame_and_movie_scripts(handler_name, args).await {
        if err.code != ScriptErrorCode::Abort {
            reserve_player_mut(|player| player.on_script_error(&err));
        }
    }

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
    handler_name: &str,
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
            if err.code == ScriptErrorCode::Abort {
                return; // abort stops the entire handler chain
            }
            // HandlerNotFound is expected when a script doesn't have the event handler
            // (e.g., timeout target script doesn't have prepareFrame or exitFrame).
            // This is normal Director behavior - just silently skip.
            if err.code != ScriptErrorCode::HandlerNotFound {
                // Log actual errors but continue with other timeouts
                log::warn!("Timeout system event {} error: {}", handler_name, err.message);
            }
        }
    }
}
