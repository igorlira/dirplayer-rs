use crate::{console_warn, director::lingo::datum::Datum, player::{reserve_player_mut, timeout::Timeout, DatumRef, DirPlayer, ScriptError, VOID_DATUM_REF}};

pub struct TimeoutDatumHandlers {}

impl TimeoutDatumHandlers {
  #[allow(dead_code, unused_variables)]
  pub fn call(datum: &DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      "new" => Self::new(datum, args),
      "forget" => Self::forget(datum, args),
      _ => Err(ScriptError::new(format!("No handler {handler_name} for timeout")))
    }
  }

  pub fn new(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let timeout_period = player.get_datum(&args[0]).int_value()?;
      let timeout_handler = player.get_datum(&args[1]).string_value()?;
      let target_ref = args[2].clone();
      let timeout_datum = player.get_datum(&datum);
      let timeout_name = match timeout_datum {
        Datum::TimeoutRef(timeout_name) => timeout_name,
        _ => return Err(ScriptError::new("Cannot create timeout from non-timeout".to_string())),
      };

      let mut timeout = Timeout {
        handler: timeout_handler,
        name: timeout_name.to_owned(),
        period: timeout_period as u32,
        target_ref,
        is_scheduled: false,
      };
      timeout.schedule();
      player.timeout_manager.add_timeout(timeout);
      Ok(datum.clone())
    })
  }

  fn forget(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let timeout_name = {
        let timeout_ref = player.get_datum(datum);
        match timeout_ref {
          Datum::TimeoutRef(timeout_name) => Ok(timeout_name.to_owned()),
          _ => Err(ScriptError::new("Cannot forget non-timeout".to_string())),
        }?
      };
      player.timeout_manager.forget_timeout(&timeout_name);
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn get_prop(player: &mut DirPlayer, datum: &DatumRef, prop: &String) -> Result<DatumRef, ScriptError> {
    let timeout_ref = player.get_datum(datum);
    let _timeout_name = match timeout_ref {
      Datum::TimeoutRef(timeout_name) => Ok(timeout_name),
      _ => Err(ScriptError::new("Cannot get prop of non-timeout".to_string())),
    }?;
    let timeout = player.timeout_manager.get_timeout(_timeout_name);
    match prop.as_str() {
      "name" => {
        Ok(player.alloc_datum(Datum::String(_timeout_name.to_owned())))
      },
      "target" => {
        Ok(timeout.map_or(VOID_DATUM_REF.clone(), |x| x.target_ref.clone()))
      }
      _ => {
        Err(ScriptError::new(format!("Cannot get timeout property {}", prop)))
      },
    }
  }

  pub fn set_prop(player: &mut DirPlayer, datum: &DatumRef, prop: &String, value: &DatumRef) -> Result<(), ScriptError> {
    let timeout_ref = player.get_datum(datum);
    let _timeout_name = {
      match timeout_ref {
        Datum::TimeoutRef(timeout_name) => Ok(timeout_name.clone()),
        _ => Err(ScriptError::new("Cannot set prop of non-timeout".to_string())),
      }?
    };
    let timeout = player.timeout_manager.get_timeout_mut(&_timeout_name);
    match prop.as_str() {
      "target" => {
        let new_target = value;
        if let Some(timeout) = timeout {
          timeout.target_ref = new_target.clone();
        } else {
          return Err(ScriptError::new("Cannot set target of unscheduled timeout".to_string()));
        }
        Ok(())
      }
      _ => {
        Err(ScriptError::new(format!("Cannot set timeout property {}", prop)))
      },
    }
  }
}
