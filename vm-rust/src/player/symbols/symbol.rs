use std::fmt::Display;

use itertools::Format;
use lasso::Spur;

use crate::player::{ScriptError, symbols::{builtin::BuiltInSymbol, symbol_table::{SYMBOL_TABLE, get_symbol_spur}}};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Default)]
pub struct Symbol {
    pub spur: Spur,
}

// impl Eq for Symbol { }

// impl PartialEq for Symbol {
//     fn eq(&self, other: &Self) -> bool {
//         self.spur == other.spur
//     }
// }

impl Into<&str> for Symbol {
    fn into(self) -> &'static str {
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        symbol_table.get_original_string(&self.spur)
    }
}

impl Into<Symbol> for BuiltInSymbol {
    fn into(self) -> Symbol {
        Symbol::builtin(self)
    }
}

impl Into<Symbol> for &str {
    fn into(self) -> Symbol {
        Symbol::from_str(self)
    }
}

impl PartialEq<BuiltInSymbol> for Symbol { 
    fn eq(&self, other: &BuiltInSymbol) -> bool {
        self.into_builtin() == Some(*other)
    }
}

impl PartialEq<Symbol> for BuiltInSymbol {
    fn eq(&self, other: &Symbol) -> bool {
        other.into_builtin() == Some(*self)
    }
}

impl Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        let original_string = symbol_table.get_original_string(&self.spur);
        write!(f, "{}", original_string)
    }
}

impl Symbol {
    pub fn into_builtin(&self) -> Option<BuiltInSymbol> {
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        symbol_table.spur_to_builtin.get(&self.spur).copied()
    }

    pub fn into_builtin_or_error(&self) -> Result<BuiltInSymbol, ScriptError> {
        self.into_builtin().ok_or_else(|| ScriptError::new(format!("Symbol '{}' is not a built-in symbol", self)))
    }

    pub fn builtin(builtin: BuiltInSymbol) -> Self {
        // Ensure the global symbol table exists. Unlike `from_str` (which goes
        // through `get_symbol_spur` → `init_symbol_table`), constructing a
        // builtin needs no interning, so it could be the very first symbol op
        // in a fresh context and would otherwise unwrap a `None` table. The
        // `Once` makes this idempotent and it can't re-enter init
        // (`init_builtin_symbols` interns directly, never via `builtin`).
        crate::player::symbols::symbol_table::init_symbol_table();
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        let spur = symbol_table.builtin_to_spur.get(&builtin).copied().unwrap();
        Self { spur }
    }

    pub fn into_str(&self) -> &'static str {
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        symbol_table.get_original_string(&self.spur)
    }

    pub fn from_str(s: &str) -> Self {
        let spur = get_symbol_spur(s);
        Self { spur }
    }

    pub fn eq_builtin(&self, builtin: BuiltInSymbol) -> bool {
        self.into_builtin() == Some(builtin)
    }

    pub fn as_str(&self) -> &'static str {
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        symbol_table.get_original_string(&self.spur)
    }

    pub fn empty() -> Self {
        Self::builtin(BuiltInSymbol::EmptyString)
    }

    pub fn is_empty(&self) -> bool {
        self.eq_builtin(BuiltInSymbol::EmptyString)
    }
}

#[macro_export]
macro_rules! symbol_match {
    ($sym:expr, { $( $pat:pat => $body:expr ),+, _ => $default:expr $(,)? }) => {
        match ($sym).into_builtin() {
            $( Some($pat) => $body, )*
            _ => $default,
        }
    };
}
