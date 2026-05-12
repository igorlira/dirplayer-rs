// Glue between Lscr-parsed JS literal blocks and the js_lingo decoder.
// Phase 1 only emits a disassembly dump so the format work can be validated
// against real movies before the translator goes in.

use crate::director::lingo::datum::Datum;
use crate::player::js_lingo::{decode_script, disasm::disassemble};
use crate::player::script::Script;

pub fn diagnose_js_script(script: &Script) {
    let mut block_index = 0usize;
    for (i, lit) in script.chunk.literals.iter().enumerate() {
        if let Datum::JavaScript(data) = lit {
            block_index += 1;
            log::info!(
                "[js-lingo] {}:{} literal[{}] is JSScript block #{}, {} bytes — decoding",
                script.member_ref.cast_lib,
                script.member_ref.cast_member,
                i,
                block_index,
                data.len()
            );
            match decode_script(data) {
                Ok(ir) => {
                    let dump = disassemble(&ir);
                    for line in dump.lines() {
                        log::info!("[js-lingo]   {}", line);
                    }
                }
                Err(e) => {
                    log::warn!("[js-lingo]   decode failed: {}", e);
                }
            }
        }
    }
}
