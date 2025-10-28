use crate::player::{
    player_call_script_handler, player_handle_scope_return, reserve_player_mut, reserve_player_ref,
    script_ref::ScriptInstanceRef, DatumRef, DirPlayer, ScriptError, ScriptErrorCode,
};

use super::script_instance::ScriptInstanceUtils;

pub struct SpriteDatumHandlers {}

pub struct SpriteDatumUtils {}

impl SpriteDatumUtils {
    pub fn get_script_instance_ids(
        datum: &DatumRef,
        player: &DirPlayer,
    ) -> Result<Vec<ScriptInstanceRef>, ScriptError> {
        let sprite_num = player.get_datum(datum).to_sprite_ref()?;
        let sprite = player.movie.score.get_sprite(sprite_num);
        if sprite.is_none() {
            return Ok(vec![]);
        }
        let sprite = sprite.unwrap();
        let instances = &sprite.script_instance_list;
        Ok(instances.clone())
    }
}

impl SpriteDatumHandlers {
    pub fn has_async_handler(datum: &DatumRef, handler_name: &String) -> Result<bool, ScriptError> {
        return reserve_player_ref(|player| {
            let sprite_num = player.get_datum(datum).to_sprite_ref()?;
            let sprite = player.movie.score.get_sprite(sprite_num);
            if sprite.is_none() {
                return Ok(false);
            }
            let sprite = sprite.unwrap();
            let instances = &sprite.script_instance_list;
            for instance in instances {
                let handler = ScriptInstanceUtils::get_script_instance_handler(
                    handler_name,
                    instance,
                    player,
                )?;
                if handler.is_some() {
                    return Ok(true);
                }
            }
            Ok(false)
        });
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "getProp" => {
                reserve_player_mut(|player| {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "getProp requires at least 1 argument".to_string(),
                        ));
                    }

                    let sprite_num = player.get_datum(datum).to_sprite_ref()?;

                    // Get the property name from the first arg
                    let prop_name = player.get_datum(&args[0]).string_value()?;

                    // First, try to get it as a built-in sprite property
                    match crate::player::score::sprite_get_prop(
                        player,
                        sprite_num as i16,
                        &prop_name,
                    ) {
                        Ok(prop_datum) => {
                            let result = player.alloc_datum(prop_datum);

                            // If there's a second argument, it's a sub-property access
                            if args.len() > 1 {
                                return crate::player::handlers::types::TypeUtils::get_sub_prop(
                                    &result, &args[1], player,
                                );
                            }

                            return Ok(result);
                        }
                        Err(_) => {
                            // Not a built-in sprite property, try script instances
                        }
                    }

                    let sprite = player.movie.score.get_sprite(sprite_num);
                    if sprite.is_none() {
                        return Err(ScriptError::new(format!("Sprite {} not found", sprite_num)));
                    }

                    // Clone the script instance list to avoid borrow conflicts
                    let instance_refs = sprite.unwrap().script_instance_list.clone();

                    // Try to get the property from the sprite's script instances
                    for instance_ref in instance_refs {
                        if let Ok(result) = crate::player::script::script_get_prop(
                            player,
                            &instance_ref,
                            &prop_name,
                        ) {
                            // If there's a second argument, it's a sub-property access
                            if args.len() > 1 {
                                return crate::player::handlers::types::TypeUtils::get_sub_prop(
                                    &result, &args[1], player,
                                );
                            }
                            return Ok(result);
                        }
                    }

                    // If not found anywhere, return void
                    Ok(DatumRef::Void)
                })
            }
            _ => Err(ScriptError::new_code(
                ScriptErrorCode::HandlerNotFound,
                format!("No sync handler {handler_name} for sprite"),
            )),
        }
    }

    pub async fn call_async(
        datum: DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let instance_refs =
            reserve_player_ref(|player| SpriteDatumUtils::get_script_instance_ids(&datum, player))?;
        for instance_ref in instance_refs {
            let handler_ref = reserve_player_ref(|player| {
                ScriptInstanceUtils::get_script_instance_handler(
                    handler_name,
                    &instance_ref,
                    player,
                )
            })?;
            if let Some(handler_ref) = handler_ref {
                let result_scope =
                    player_call_script_handler(Some(instance_ref), handler_ref, args).await?;
                player_handle_scope_return(&result_scope);
                return Ok(result_scope.return_value);
            }
        }
        Err(ScriptError::new(format!(
            "No async handler {handler_name} for sprite"
        )))
    }
}
