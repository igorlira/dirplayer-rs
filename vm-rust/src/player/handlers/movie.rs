use crate::{director::lingo::datum::Datum, player::{cast_lib::INVALID_CAST_MEMBER_REF, datum_formatting::format_datum, get_datum, reserve_player_mut, score::get_sprite_at, DatumRef, ScriptError, VOID_DATUM_REF}};

pub struct MovieHandlers {}

impl MovieHandlers {
  pub fn puppet_tempo(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      player.movie.puppet_tempo = get_datum(&args[0], &player.datums).int_value()? as u32;
      Ok(VOID_DATUM_REF.clone())
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
      let member = player.movie.cast_manager.find_member_ref_by_identifiers(member_name_or_num, cast_name_or_num, &player.datums)?;
      if let Some(member) = member {
        Ok(player.alloc_datum(Datum::CastMember(member.to_owned())))
      } else {
        Ok(player.alloc_datum(Datum::CastMember(INVALID_CAST_MEMBER_REF)))
      }
    })
  }

  pub fn go(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      player.next_frame = Some(get_datum(&args[0], &player.datums).int_value()? as u32);
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn puppet_sprite(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let sprite_number = player.get_datum(&args[0]).int_value()?;
      let is_puppet = player.get_datum(&args[1]).int_value()? == 1;
      let sprite = player.movie.score.get_sprite_mut(sprite_number as i16);
      sprite.puppet = is_puppet;
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn sprite(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let sprite_number = player.get_datum(&args[0]).int_value()?;
      Ok(player.alloc_datum(Datum::SpriteRef(sprite_number as i16)))
    })
  }

  pub fn external_param_value(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      // TODO
      Ok(player.alloc_datum(Datum::String("".to_string())))
    })
  }

  pub fn stop_event(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    // TODO stop event
    Ok(VOID_DATUM_REF.clone())
  }

  pub fn get_pref(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    Ok(VOID_DATUM_REF.clone())
  }

  pub fn set_pref(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    Ok(VOID_DATUM_REF.clone())
  }

  pub fn go_to_net_page(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    Ok(VOID_DATUM_REF.clone())
  }

  pub fn pass(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let scope = player.scopes.last_mut().unwrap();
      scope.passed = true;
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn update_stage(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    // TODO: re-render
    // The updateStage() method redraws sprites, performs transitions, plays sounds, sends a prepareFrame message
    // (affecting movie and behavior scripts), and sends a stepFrame message (which affects actorList)
    Ok(VOID_DATUM_REF.clone())
  }

  pub fn rollover(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
      Ok(player.alloc_datum(Datum::Int(sprite.unwrap_or(0) as i32)))
    })
  }
}
