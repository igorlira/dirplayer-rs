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

    /// Like `from_str`, but this name CLAIMS the global display spelling even if
    /// one was already recorded. Only for a cast's own name table (LNAM) — see
    /// `SymbolTable::intern_authoritative` for the measured Director behaviour
    /// this restores.
    pub fn from_str_authoritative(s: &str) -> Self {
        let spur = crate::player::symbols::symbol_table::get_symbol_spur_authoritative(s);
        Self { spur }
    }

    pub fn eq_builtin(&self, builtin: BuiltInSymbol) -> bool {
        self.into_builtin() == Some(builtin)
    }

    pub fn as_str(&self) -> &'static str {
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        symbol_table.get_original_string(&self.spur)
    }

    /// Case-normalised spelling — see `SymbolTable::get_lower_string`. Use
    /// this (with lowercase literals) whenever matching a symbol against
    /// string literals; `as_str` is for display only.
    pub fn as_lower_str(&self) -> &'static str {
        let symbol_table = unsafe { SYMBOL_TABLE.as_ref().unwrap() };
        symbol_table.get_lower_string(&self.spur)
    }

    /// Case-insensitive compare against a plain string. `intern` lowercases,
    /// so symbol identity is already case-insensitive; this gives the same
    /// semantics at the many call sites that still hold a `&str`, and keeps
    /// Director's case-insensitive name matching intact.
    pub fn eq_ignore_ascii_case(&self, other: &str) -> bool {
        self.as_lower_str().eq_ignore_ascii_case(other)
    }

    /// The case-normalised spelling as an owned String, for the call sites
    /// that used to do `name.to_lowercase()` on a `String` name.
    pub fn to_lowercase(&self) -> String {
        self.as_lower_str().to_string()
    }

    pub fn to_ascii_lowercase(&self) -> String {
        self.as_lower_str().to_string()
    }

    pub fn splitn(&self, n: usize, pat: char) -> std::str::SplitN<'static, char> {
        self.as_str().splitn(n, pat)
    }

    pub fn starts_with(&self, pat: &str) -> bool {
        self.as_lower_str().starts_with(&pat.to_ascii_lowercase())
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

// Comparing a Symbol to a string literal is case-insensitive, matching both
// `intern`'s normalisation and Director's own name semantics.
impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool { self.as_lower_str().eq_ignore_ascii_case(other) }
}
impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool { self.as_lower_str().eq_ignore_ascii_case(other) }
}
impl PartialEq<Symbol> for &str {
    fn eq(&self, other: &Symbol) -> bool { other.as_lower_str().eq_ignore_ascii_case(self) }
}
impl PartialEq<String> for Symbol {
    fn eq(&self, other: &String) -> bool { self.as_lower_str().eq_ignore_ascii_case(other) }
}

// Symbol is Copy, so `sym == &other` shows up wherever an iterator hands back a
// reference. Mirror the owned comparison rather than making every call site
// dereference.
impl PartialEq<&Symbol> for Symbol {
    fn eq(&self, other: &&Symbol) -> bool { self.spur == other.spur }
}
// The reverse direction of the &str/String comparisons defined above, so a
// `String == Symbol` reads the same as `Symbol == String`.
impl PartialEq<Symbol> for String {
    fn eq(&self, other: &Symbol) -> bool { other.as_lower_str().eq_ignore_ascii_case(self) }
}
impl PartialEq<Symbol> for &String {
    fn eq(&self, other: &Symbol) -> bool { other.as_lower_str().eq_ignore_ascii_case(self) }
}
