use crate::{director::lingo::datum::{datum_bool, Datum}, player::{allocator::ScriptInstanceAllocatorTrait, cast_lib::CastMemberRef, player_call_script_handler, player_handle_scope_return, reserve_player_mut, script::{get_lctx_for_script, ScriptInstance}, script_ref::ScriptInstanceRef, DatumRef, ScriptError}};

pub struct ScriptDatumHandlers {}

impl ScriptDatumHandlers {
  pub fn has_async_handler(name: &String) -> bool {
    match name.as_str() {
      "new" => true,
      _ => false,
    }
  }

  pub async fn call_async(datum: &DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      "new" => Self::new(datum, &args).await,
      _ => Err(ScriptError::new(format!("No async handler {handler_name} for script datum")))
    }
  }

  pub fn call(datum: &DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      "handler" => Self::handler(datum, args),
      _ => Err(ScriptError::new(format!("No handler {handler_name} for script datum")))
    }
  }

  pub fn handler(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let name = player.get_datum(&args[0]).string_value()?;
      let script_ref = match player.get_datum(datum) {
        Datum::ScriptRef(script_ref) => script_ref,
        _ => return Err(ScriptError::new("Cannot create new instance of non-script".to_string())),
      };
      let script = player.movie.cast_manager.get_script_by_ref(script_ref).unwrap();
      let own_handler = script.get_own_handler(&name);
      Ok(player.alloc_datum(datum_bool(own_handler.is_some())))
    })
  }

  pub fn create_script_instance(script_ref: &CastMemberRef) -> (ScriptInstanceRef, DatumRef) {
    reserve_player_mut(|player| {
      let instance_id = player.allocator.get_free_script_instance_id();
      let script = player.movie.cast_manager.get_script_by_ref(&script_ref).unwrap();
      let lctx: &crate::director::lingo::script::ScriptContext = get_lctx_for_script(player, script).unwrap();
      let instance = ScriptInstance::new(instance_id, script_ref.to_owned(), script, lctx);
      let instance_ref = player.allocator.alloc_script_instance(instance);
      let datum_ref = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
      (instance_ref, datum_ref)
    })
  }

  pub async fn new(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let (script_ref, new_handler_ref) = reserve_player_mut(|player| {
      let script_ref = match player.get_datum(datum) {
        Datum::ScriptRef(script_ref) => script_ref,
        _ => return Err(ScriptError::new("Cannot create new instance of non-script".to_string())),
      };
      let script = player.movie.cast_manager.get_script_by_ref(script_ref).unwrap();
      let new_handler_ref = script.get_own_handler_ref(&"new".to_string());
      Ok((script_ref.clone(), new_handler_ref))
    })?;

    let (instance_ref, datum_ref) = Self::create_script_instance(&script_ref);
    if let Some(new_handler_ref) = new_handler_ref {
      let result_scope = player_call_script_handler(Some(instance_ref), new_handler_ref, args).await?;
      player_handle_scope_return(&result_scope);
      return Ok(result_scope.return_value);
    } else {
      return Ok(datum_ref);
    }
  }
}
