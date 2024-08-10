use crate::{director::lingo::datum::{datum_bool, Datum}, player::{reserve_player_mut, DatumRef, ScriptError}};


pub struct NetHandlers { }

impl NetHandlers {
  pub fn net_done(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let task_id = if let Some(task_id_ref) = &args.get(0) {
        let task_id_datum = player.get_datum(task_id_ref);
        Some(task_id_datum.int_value()? as u32)
      } else {
        None
      };
      let task_state = player.net_manager.get_task_state(task_id);
      let is_done = task_state.is_some_and(|state| state.is_done());
      Ok(player.alloc_datum(datum_bool(is_done)))
    })
  }
  
  pub fn preload_net_thing(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let url = player.get_datum(&args[0]).string_value()?;
      let task_id = player.net_manager.preload_net_thing(url);
      Ok(player.alloc_datum(Datum::Int(task_id as i32)))
    })
  }

  pub fn get_net_text(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let url = player.get_datum(&args[0]).string_value()?;
      let task_id = player.net_manager.preload_net_thing(url);
      // TODO should the task be tagged as a text task?
      Ok(player.alloc_datum(Datum::Int(task_id as i32)))
    })
  }

  pub fn get_stream_status(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let (state, error, url, is_ok) = {
        let task_id = player.get_datum(&args[0]).int_value()? as u32;
        let task = player.net_manager.get_task(task_id).unwrap();
        let task_state = &player.net_manager.get_task_state(Some(task_id)).unwrap();
        let (state, error) = if task_state.is_done() && task_state.result.as_ref().unwrap().is_ok() {
          ("Complete", "OK")
        } else if task_state.is_done() && task_state.result.as_ref().unwrap().is_err() {
          ("Complete", "Task failed")
        } else {
          ("InProgress", "")
        };
        let is_ok = task_state.is_done() && task_state.result.as_ref().unwrap().is_ok();
        (state.to_owned(), error.to_owned(), task.url.to_owned(), is_ok)
      };
      let result_map = Datum::PropList(vec![
        (player.alloc_datum(Datum::String("URL".to_owned())), player.alloc_datum(Datum::String(url))),
        (player.alloc_datum(Datum::String("state".to_owned())), player.alloc_datum(Datum::String(state))),
        (player.alloc_datum(Datum::String("bytesSoFar".to_owned())), player.alloc_datum(Datum::Int(if is_ok { 100 } else { 0 }))),
        (player.alloc_datum(Datum::String("bytesTotal".to_owned())), player.alloc_datum(Datum::Int(100))),
        (player.alloc_datum(Datum::String("error".to_owned())), player.alloc_datum(Datum::String(error))),
      ], false);
      Ok(player.alloc_datum(result_map))
    })
  }

  pub fn net_error(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let task_id = args.get(0).map(|datum_ref| player.get_datum(datum_ref).int_value().unwrap() as u32);
      let task_state = player.net_manager.get_task_state(task_id).unwrap();
      let is_ok = task_state.is_done() && task_state.result.as_ref().unwrap().is_ok();
      let error = if is_ok {
        Datum::String("OK".to_owned())
      } else if let Some(Err(error)) = task_state.result.as_ref() {
        Datum::Int(*error)
      } else {
        Datum::Int(0)
      };
      Ok(player.alloc_datum(error))
    })
  }

  pub fn net_text_result(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let task_id = args.get(0).map(|datum_ref| player.get_datum(datum_ref).int_value().unwrap() as u32);
      let task_state = player.net_manager.get_task_state(task_id).unwrap();
      let is_ok = task_state.is_done() && task_state.result.as_ref().unwrap().is_ok();
      let text = if is_ok {
        let text = task_state.result.as_ref().unwrap().as_ref().unwrap();
        Datum::String(String::from_utf8_lossy(text).to_string())
      } else {
        Datum::String("".to_owned())
      };
      Ok(player.alloc_datum(text))
    })
  }
}
