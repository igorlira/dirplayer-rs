// JavaScript (jsLingo) script support.
//
// Director MX 2004+ ships a modified SpiderMonkey 1.5 interpreter (jsdmx).
// Scripts authored as JavaScript-syntax get compiled to SpiderMonkey
// bytecode and XDR-serialized into the standard Lscr chunk's literal-data
// area as one or more `0xDEAD0003` blocks (see jsxdrapi.h::JSXDR_MAGIC_SCRIPT_*).
//
// On parse we leave the blocks raw (Datum::JavaScript) — see
// director/chunks/script.rs::parse_javascript_literals. This module turns
// those blocks into a runnable form by:
//
//   1. Decoding the XDR payload into a JsScriptIR (atoms + bytecode + try notes).
//   2. (Future) Translating each script's bytecode to Lingo bytecode the
//      existing VM dispatch loop can run, or feeding it to a JS-native interp.
//
// For Phase 1 we go as far as IR + disassembly. Phase 2 adds translation.

pub mod builtins;
pub mod disasm;
pub mod host_bridge;
pub mod interpreter;
pub mod opcodes;
pub mod value;
pub mod variable_length;
pub mod xdr;

#[cfg(test)]
mod test_fixtures;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod interp_tests;

pub use xdr::{decode_script, JsAtom, JsScriptIR, JsTryNote, JsXdrError};
