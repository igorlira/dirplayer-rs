use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        cast_lib::INVALID_CAST_MEMBER_REF,
        datum_formatting::format_datum,
        events::{player_invoke_event_to_instances, player_invoke_static_event},
        reserve_player_mut,
        score::get_sprite_at,
        DatumRef, ScriptError,
    },
    utils::log_i,
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

    pub fn go(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let datum: &Datum = player.get_datum(&args[0]);
            let datum_type = datum.type_enum();
            let destination_frame = match datum_type {
                DatumType::Int => Some(datum.int_value()? as u32),
                DatumType::String => {
                    let label = datum.string_value()?;
                    let frame_label = player
                        .movie
                        .score
                        .frame_labels
                        .iter()
                        .find(|fl| fl.label == label);
                    frame_label.map(|frame_label| frame_label.frame_num as u32)
                }
                _ => None,
            };
            match destination_frame {
                Some(frame) => {
                    player.next_frame = Some(frame);
                    Ok(DatumRef::Void)
                }
                None => Err(ScriptError::new(
                    "Unsupported or invalid frame label passed to go()".to_string(),
                )),
            }
        })
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
            let sprite_num = player.get_datum(&args[0]).int_value().unwrap();
            let message: String = player.get_datum(&args[1]).symbol_value().unwrap();
            let remaining_args = &args[2..].to_vec();
            let sprite = player.movie.score.get_sprite(sprite_num as i16).unwrap();
            // TODO what is behavior if sprite is null/out of bounds
            let receivers = sprite.script_instance_list.clone();
            (message.clone(), remaining_args.clone(), receivers)
        });
        let mut handled_by_sprite = false;
        for receiver in receivers {
            let receivers = vec![receiver];
            handled_by_sprite =
                player_invoke_event_to_instances(&message, &remaining_args, &receivers).await?
                    || handled_by_sprite;
        }
        if !handled_by_sprite {
            player_invoke_static_event(&message, &remaining_args).await?;
        }
        reserve_player_mut(|player: &mut crate::player::DirPlayer| {
            Ok(player.alloc_datum(Datum::Int(handled_by_sprite as i32)))
        })
    }

    pub async fn send_all_sprites(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let (message, remaining_args, receivers) = reserve_player_mut(|player| {
            let message = player.get_datum(&args[0]).symbol_value().unwrap();
            let remaining_args = &args[1..].to_vec();
            let receivers = player.movie.score.get_active_script_instance_list();
            (message.clone(), remaining_args.clone(), receivers)
        });
        let mut handled_by_sprite = false;
        for receiver in receivers {
            let receivers = vec![receiver];
            handled_by_sprite =
                player_invoke_event_to_instances(&message, &remaining_args, &receivers).await?
                    || handled_by_sprite;
        }
        if !handled_by_sprite {
            player_invoke_static_event(&message, &remaining_args).await?;
        }
        reserve_player_mut(|player: &mut crate::player::DirPlayer| {
            Ok(player.alloc_datum(Datum::Int(handled_by_sprite as i32)))
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

    pub fn get_pref(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Ok(DatumRef::Void)
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

    pub fn update_stage(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // TODO: re-render
        // The updateStage() method redraws sprites, performs transitions, plays sounds, sends a prepareFrame message
        // (affecting movie and behavior scripts), and sends a stepFrame message (which affects actorList)
        Ok(DatumRef::Void)
    }

    pub fn rollover(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
            Ok(player.alloc_datum(Datum::Int(sprite.unwrap_or(0) as i32)))
        })
    }
}
