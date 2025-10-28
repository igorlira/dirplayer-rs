use async_std::channel::Receiver;
use log::warn;

use crate::{
    console_warn,
    director::lingo::datum::{Datum, VarRef},
    player::{
        handlers::datum_handlers::player_call_datum_handler, player_is_playing, reserve_player_mut,
    },
};

use super::{
    cast_lib::CastMemberRef, handlers::datum_handlers::script_instance::ScriptInstanceUtils,
    player_call_script_handler, player_semaphone, reserve_player_ref, script::ScriptInstanceId,
    script_ref::ScriptInstanceRef, DatumRef, ScriptError, ScriptErrorCode, PLAYER_EVENT_TX,
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

pub async fn player_invoke_event_to_instances(
    handler_name: &String,
    args: &Vec<DatumRef>,
    instance_refs: &Vec<ScriptInstanceRef>,
) -> Result<bool, ScriptError> {
    let recv_instance_handlers = reserve_player_ref(|player| {
        // let receiver_refs = get_active_scripts(&player.movie, &player.get_hydrated_globals());
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
        let scope =
            player_call_script_handler(Some(script_instance_ref), handler_ref, args).await?;
        if !scope.passed {
            handled = true;
            break;
        }
    }

    Ok(handled)
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
        let result =
            player_call_script_handler(None, (script_member_ref, handler_name.to_owned()), args)
                .await?;
        if !result.passed {
            handled = true;
            break;
        }
    }
    Ok(handled)
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

pub async fn player_wait_available() {
    player_semaphone().lock().await;
}
