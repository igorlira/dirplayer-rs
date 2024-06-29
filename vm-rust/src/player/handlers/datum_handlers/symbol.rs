use crate::{
    director::lingo::datum::Datum,
    player::{DatumRef, DirPlayer, ScriptError},
};

pub struct SymbolDatumHandlers {}

impl SymbolDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for symbol"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        _: &DatumRef,
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        match prop.as_str() {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("symbol".to_string()))),
            _ => Err(ScriptError::new(format!(
                "Cannot get symbol property {}",
                prop
            ))),
        }
    }
}
