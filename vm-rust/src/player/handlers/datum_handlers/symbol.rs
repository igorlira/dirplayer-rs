use crate::{
    director::lingo::datum::Datum,
    player::{symbols::symbol::Symbol, DatumRef, DirPlayer, ScriptError},
};

pub struct SymbolDatumHandlers {}

impl SymbolDatumHandlers {
    #[allow(dead_code, unused_variables)]
    pub fn call(
        datum: DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        Err(ScriptError::new(format!(
            "No handler {handler_name} for symbol"
        )))
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        _: &DatumRef,
        prop: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        match prop.as_str() {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol(Symbol::from_str("symbol")))),
            _ => Err(ScriptError::new(format!(
                "Cannot get symbol property {}",
                prop
            ))),
        }
    }
}
