use super::{
    cast_member::CastMemberType, events::player_dispatch_targeted_event, player_is_playing,
    reserve_player_mut, DatumRef, DirPlayer, ScriptError,
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
    let instance_ids = reserve_player_mut(|player| {
        player.keyboard_manager.key_down(key.clone(), code);
        if player.keyboard_focus_sprite != -1 {
            let sprite_id = player.keyboard_focus_sprite as usize;
            let sprite = player.movie.score.get_sprite(sprite_id as i16);
            if let Some(sprite) = sprite {
                let instance_list = sprite.script_instance_list.clone();
                let member_ref = sprite.member.clone();
                let member =
                    member_ref.and_then(|x| player.movie.cast_manager.find_mut_member_by_ref(&x));
                if let Some(member) = member {
                    match &mut member.member_type {
                        CastMemberType::Field(field_member) => {
                            if field_member.editable {
                                if key == "Backspace" {
                                    field_member.text.pop();
                                } else if key == "Tab" {
                                    let next_focus_sprite_id =
                                        get_next_focus_sprite_id(player, sprite_id as i16);
                                    player.keyboard_focus_sprite = next_focus_sprite_id;
                                } else if key.len() == 1 {
                                    field_member.text = format!("{}{}", field_member.text, key);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some(instance_list)
            } else {
                None
            }
        } else {
            None
        }
    });
    player_dispatch_targeted_event(&"keyDown".to_string(), &vec![], instance_ids.as_ref());
    Ok(DatumRef::Void)
}

pub async fn player_key_up(key: String, code: u16) -> Result<DatumRef, ScriptError> {
    if !player_is_playing().await {
        return Ok(DatumRef::Void);
    }
    let instance_ids = reserve_player_mut(|player| {
        player.keyboard_manager.key_up(&key, code);
        if player.keyboard_focus_sprite != -1 {
            let sprite = player.keyboard_focus_sprite as usize;
            let sprite = player.movie.score.get_sprite(sprite as i16);
            sprite.map(|x| x.script_instance_list.clone())
        } else {
            None
        }
    });
    player_dispatch_targeted_event(&"keyUp".to_string(), &vec![], instance_ids.as_ref());
    Ok(DatumRef::Void)
}
