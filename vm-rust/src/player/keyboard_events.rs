use super::{
    cast_member::CastMemberType,
    events::{player_invoke_event_to_instances, player_dispatch_movie_callback, player_invoke_frame_and_movie_scripts},
    player_is_playing, reserve_player_mut, DatumRef, DirPlayer, ScriptError,
};

fn get_next_focus_sprite_id(player: &DirPlayer, after: i16) -> i16 {
    for sprite_id in after + 1..=player.movie.score.get_channel_count() as i16 {
        let sprite = player.movie.score.get_sprite(sprite_id);
        let member_ref = sprite.and_then(|x| x.member.clone());
        let member = member_ref.and_then(|x| player.movie.cast_manager.find_member_by_ref(&x));
        let field = member.and_then(|x| match &x.member_type {
            CastMemberType::Field(field) => Some(field),
            _ => None,
        });

        if field.is_none() {
            continue;
        }
        let field = field.unwrap();
        if field.editable {
            return sprite_id;
        }
    }
    return -1;
}

pub async fn player_key_down(key: String, code: u16) -> Result<DatumRef, ScriptError> {
    if !player_is_playing().await {
        return Ok(DatumRef::Void);
    }

    // Note: keyboard_manager.key_down() is NOT called here because it's already
    // handled immediately in the WASM entry point (lib.rs key_down()). Calling it
    // here would re-add keys from stale queued commands after the user released them.
    let (instance_ids, is_editable_field, sprite_id) = reserve_player_mut(|player| {
        if player.keyboard_focus_sprite != -1 {
            let sprite_id = player.keyboard_focus_sprite as i16;
            player.sync_script_instance_list(sprite_id);
            let sprite = player.movie.score.get_sprite(sprite_id);
            if let Some(sprite) = sprite {
                let instance_list = sprite.script_instance_list.clone();
                let member_ref = sprite.member.clone();
                let member =
                    member_ref.and_then(|x| player.movie.cast_manager.find_member_by_ref(&x));
                let is_editable = member.map_or(false, |m| {
                    if let CastMemberType::Field(f) = &m.member_type {
                        f.editable
                    } else {
                        false
                    }
                });
                (Some(instance_list), is_editable, sprite_id)
            } else {
                (None, false, -1)
            }
        } else {
            (None, false, -1)
        }
    });

    // Director event propagation order:
    // 1. Behavior scripts on focused sprite (synchronous)
    // 2. If not handled → frame script → movie scripts
    // 3. Movie callback
    // 4. If no script handled it → default text insertion
    let mut handled = false;
    if let Some(ref instances) = instance_ids {
        if !instances.is_empty() {
            handled = player_invoke_event_to_instances(
                &"keyDown".to_string(), &vec![], instances,
            ).await?;
        }
    }
    if !handled {
        player_invoke_frame_and_movie_scripts(&"keyDown".to_string(), &vec![]).await?;
    }
    player_dispatch_movie_callback("keyDown").await?;

    // Default text insertion: only if no script handled the event.
    if is_editable_field && !handled {
        reserve_player_mut(|player| {
            if player.keyboard_focus_sprite != sprite_id {
                return;
            }
            let sprite = player.movie.score.get_sprite(sprite_id);
            if let Some(sprite) = sprite {
                let member_ref = sprite.member.clone();
                let member = member_ref.and_then(|x| player.movie.cast_manager.find_mut_member_by_ref(&x));
                if let Some(member) = member {
                    if let CastMemberType::Field(field_member) = &mut member.member_type {
                        if field_member.editable {
                            if key == "Backspace" {
                                field_member.text.pop();
                            } else if key == "Tab" {
                                let next_focus_sprite_id =
                                    get_next_focus_sprite_id(player, sprite_id);
                                player.keyboard_focus_sprite = next_focus_sprite_id;
                            } else if key == "Enter" {
                                // Don't insert RETURN - let the keyDown handler deal with it
                            } else if key.len() == 1 {
                                field_member.text.push_str(&key);
                            }
                        }
                    }
                }
            }
        });
    }

    Ok(DatumRef::Void)
}

pub async fn player_key_up(key: String, code: u16) -> Result<DatumRef, ScriptError> {
    if !player_is_playing().await {
        return Ok(DatumRef::Void);
    }
    // Note: keyboard_manager.key_up() is NOT called here because it's already
    // handled immediately in the WASM entry point (lib.rs key_up()).
    let instance_ids = reserve_player_mut(|player| {
        if player.keyboard_focus_sprite != -1 {
            let sprite = player.keyboard_focus_sprite as usize;
            let sprite = player.movie.score.get_sprite(sprite as i16);
            sprite.map(|x| x.script_instance_list.clone())
        } else {
            None
        }
    });
    let mut handled = false;
    if let Some(ref instances) = instance_ids {
        if !instances.is_empty() {
            handled = player_invoke_event_to_instances(
                &"keyUp".to_string(), &vec![], instances,
            ).await?;
        }
    }
    if !handled {
        player_invoke_frame_and_movie_scripts(&"keyUp".to_string(), &vec![]).await?;
    }
    player_dispatch_movie_callback("keyUp").await?;
    Ok(DatumRef::Void)
}
