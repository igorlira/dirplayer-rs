use crate::player::{
    player_call_script_handler, player_handle_scope_return, reserve_player_ref, DatumRef, DirPlayer, ScriptError, ScriptErrorCode
};

use super::script_instance::ScriptInstanceUtils;

pub struct SpriteDatumHandlers {}

pub struct SpriteDatumUtils {}

impl SpriteDatumUtils {
    pub fn get_script_instance_ids(
        datum: &DatumRef,
        player: &DirPlayer,
    ) -> Result<Vec<u32>, ScriptError> {
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
                    *instance,
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
        _: &DatumRef,
        handler_name: &String,
        _: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            _ => Err(ScriptError::new_code(ScriptErrorCode::HandlerNotFound, format!(
                "No sync handler {handler_name} for sprite"
            ))),
        }
    }

    pub async fn call_async(
        datum: DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let instance_ids =
            reserve_player_ref(|player| SpriteDatumUtils::get_script_instance_ids(&datum, player))?;
        for instance_id in instance_ids {
            let handler_ref = reserve_player_ref(|player| {
                ScriptInstanceUtils::get_script_instance_handler(handler_name, instance_id, player)
            })?;
            if let Some(handler_ref) = handler_ref {
                let result_scope = player_call_script_handler(Some(instance_id), handler_ref, args).await?;
                player_handle_scope_return(&result_scope);
                return Ok(result_scope.return_value);
            }
        }
        Err(ScriptError::new(format!(
            "No async handler {handler_name} for sprite"
        )))
    }
}
