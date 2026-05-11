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
    if let Some(tx) = unsafe { PLAYER_EVENT_TX.clone() } {
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
    if let Some(tx) = unsafe { PLAYER_EVENT_TX.clone() } {
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
    if let Some(tx) = unsafe { PLAYER_EVENT_TX.clone() } {
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
    let tx = unsafe { PLAYER_EVENT_TX.clone() }.unwrap();
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

     for instance_id in instance_ids {
        player_invoke_targeted_event(
            handler_name,
            args,
            Some(&vec![instance_id].as_ref()),
        ).await;
    }
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
                    if !w3d.runtime_state.animation_playing { continue; }
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
    });
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
            warn!(
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
