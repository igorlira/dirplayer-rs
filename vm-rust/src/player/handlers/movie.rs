use crate::{director::lingo::datum::Datum, player::{cast_lib::INVALID_CAST_MEMBER_REF, datum_formatting::format_datum, events::player_invoke_global_event, reserve_player_mut, score::get_sprite_at, DatumRef, ScriptError}};

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
        Datum::String(script_name) => {
          Ok(player.movie.cast_manager.find_member_ref_by_name(&script_name))
        },
        Datum::Int(script_num) => {
          Ok(player.movie.cast_manager.find_member_ref_by_number(*script_num as u32))
        },
        _ => Err(ScriptError::new(format!("Invalid identifier for script: {}", formatted_id))), // TODO
      }?;
      let script = member_ref.to_owned().and_then(|r| player.movie.cast_manager.get_script_by_ref(&r));

      match script {
        Some(_) => {
          Ok(player.alloc_datum(Datum::ScriptRef(member_ref.unwrap())))
        },
        None => Err(ScriptError::new(format!("Script not found {}", formatted_id))),
      }
    })
  }

  pub fn member(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() > 2 {
        return Err(ScriptError::new("Too many arguments for member".to_string()));
      }
      let member_name_or_num_ref = args.get(0).unwrap();
      let member_name_or_num = player.get_datum(member_name_or_num_ref);
      if let Datum::CastMember(_) = &member_name_or_num {
        return Ok(member_name_or_num_ref.clone());
      }
      let cast_name_or_num = args.get(1).map(|x| player.get_datum(x));
      let member = player.movie.cast_manager.find_member_ref_by_identifiers(member_name_or_num, cast_name_or_num, &player.allocator)?;
      if let Some(member) = member {
        Ok(player.alloc_datum(Datum::CastMember(member.to_owned())))
      } else {
        Ok(player.alloc_datum(Datum::CastMember(INVALID_CAST_MEMBER_REF)))
      }
    })
  }

  pub fn go(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      player.next_frame = Some(player.get_datum(&args[0]).int_value()? as u32);
      Ok(DatumRef::Void)
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

  pub async fn send_all_sprites(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let (message, remaining_args) = reserve_player_mut(|player| {
      let message = player.get_datum(&args[0]).symbol_value().unwrap();
      let remaining_args = &args[1..].to_vec();
      (message.clone(), remaining_args.clone())
    });
    player_invoke_global_event(&message, &remaining_args).await
  }

  pub fn external_param_value(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let key = player.get_datum(&args[0]).string_value()?;
      let value: String = player.external_params.get(&key)
        .cloned()
        .unwrap_or_default();
      Ok(player.alloc_datum(Datum::String(value)))
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
