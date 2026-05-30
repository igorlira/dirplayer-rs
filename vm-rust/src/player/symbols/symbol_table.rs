use fxhash::FxHashMap;
use lasso::{Rodeo, Spur};

use crate::player::symbols::builtin::BuiltInSymbol;

pub struct SymbolTable {
    interner: Rodeo,
    original_strings: FxHashMap<Spur, String>,
    pub spur_to_builtin: FxHashMap<Spur, BuiltInSymbol>,
    pub builtin_to_spur: FxHashMap<BuiltInSymbol, Spur>,
}

pub static mut SYMBOL_TABLE: Option<SymbolTable> = None;

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            interner: Rodeo::default(),
            original_strings: FxHashMap::default(),
            spur_to_builtin: FxHashMap::default(),
            builtin_to_spur: FxHashMap::default(),
        }
    }

    pub fn intern(&mut self, string: &str) -> Spur {
        let lower_string = string.to_lowercase();
        let spur = self.interner.get_or_intern(&lower_string);
        if !self.original_strings.contains_key(&spur) {
            self.original_strings.insert(spur, string.to_owned());
        }
        spur
    }

    pub fn get_original_string(&self, spur: &Spur) -> &str {
        self.original_strings.get(spur).expect("Original string not found").as_str()
    }
}

pub fn init_symbol_table() {
    unsafe {
        SYMBOL_TABLE = Some(SymbolTable::new());
    }
    crate::player::symbols::builtin::init_builtin_symbols();
}

pub fn get_symbol_spur(string: &str) -> Spur {
    unsafe {
        SYMBOL_TABLE
            .as_mut()
            .expect("Symbol table not initialized")
            .intern(string)
    }
}

pub fn get_spur_string_owned(spur: Spur) -> String {
    unsafe {
        SYMBOL_TABLE
            .as_ref()
            .expect("Symbol table not initialized")
            .get_original_string(&spur)
            .to_owned()
    }
}

pub fn get_spur_string(spur: Spur) -> &'static str {
    unsafe {
        SYMBOL_TABLE
            .as_ref()
            .expect("Symbol table not initialized")
            .get_original_string(&spur)
    }
}

pub fn spur(string: &str) -> Spur {
    get_symbol_spur(string)
}

pub struct BuiltinKeywords {
    
}
