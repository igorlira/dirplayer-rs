use crate::{
    director::lingo::datum::Datum,
    player::{
        js_lingo_loader::{
            get_js_object, get_js_object_prop, invoke_js_object_method, set_js_object_prop,
        },
        reserve_player_ref, symbols::symbol::Symbol, DatumRef, DirPlayer, ScriptError,
        ScriptErrorCode,
    },
};

pub struct JsObjectDatumHandlers {}

impl JsObjectDatumHandlers {
    fn handle_of(player: &DirPlayer, datum: &DatumRef) -> Result<u32, ScriptError> {
        match player.get_datum(datum) {
            Datum::JsObjectRef(id) => Ok(*id),
            _ => Err(ScriptError::new("Expected JS object reference".to_string())),
        }
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let id = reserve_player_ref(|player| Self::handle_of(player, datum))?;
        match invoke_js_object_method(id, handler_name.as_str(), args) {
            Some(Ok(result)) => Ok(result),
            Some(Err(message)) => Err(ScriptError::new(format!(
                "{} (JS object handler {})",
                message, handler_name
            ))),
            None => Err(ScriptError::new_code(
                ScriptErrorCode::HandlerNotFound,
                format!("No handler {handler_name} for JS object datum"),
            )),
        }
    }

    /// Property reads fall back to VOID for an absent property, matching how
    /// Lingo reads a missing property off a script instance (and how JS reads
    /// `undefined`) rather than raising.
    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop_name: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        let id = Self::handle_of(player, datum)?;
        Ok(get_js_object_prop(id, prop_name.as_str()).unwrap_or(DatumRef::Void))
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop_name: Symbol,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        let id = Self::handle_of(player, datum)?;
        let Some((_, obj)) = get_js_object(id) else {
            return Ok(());
        };
        let value = crate::player::js_lingo_loader::datum_ref_to_js_value(player, value_ref);
        set_js_object_prop(&obj, prop_name.as_str(), value);
        Ok(())
    }
}
