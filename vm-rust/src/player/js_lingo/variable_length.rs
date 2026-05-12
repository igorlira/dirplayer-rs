// Variable-length opcode handling: tableswitch / lookupswitch.
// Reference: jsdmx/src/jsopcode.c lines 200-253 (the disassembler).
//
// tableswitch layout (after the 1-byte op):
//   i16 default_offset
//   i16 low_case
//   i16 high_case
//   (high - low + 1) * i16 case_offsets
//
// lookupswitch layout (after the 1-byte op):
//   i16 default_offset
//   u16 npairs
//   npairs * (u16 atom_index + i16 case_offset)
//
// All multi-byte values are big-endian within the bytecode stream
// (matches jsopcode.h GET_JUMP_OFFSET / GET_ATOM_INDEX which assemble
// from high-byte first). The XDR layer doesn't byte-swap the bytecode
// payload (it goes through JS_XDRBytes raw), so the in-buffer bytes are
// whatever the encoder wrote, which on x86 means the macros emit BE
// per their `(pc[1] << 8) | pc[2]` form.

use super::opcodes::JsOp;

const JUMP_OFFSET_LEN: usize = 2;
const ATOM_INDEX_LEN: usize = 2;

pub fn variable_op_length(op: JsOp, slice: &[u8]) -> Result<usize, String> {
    match op {
        JsOp::Tableswitch => {
            if slice.len() < 1 + 3 * JUMP_OFFSET_LEN {
                return Err(format!("tableswitch truncated at header"));
            }
            let low = read_i16_be(&slice[1 + JUMP_OFFSET_LEN..])?;
            let high = read_i16_be(&slice[1 + 2 * JUMP_OFFSET_LEN..])?;
            let cases = (high as i32 - low as i32 + 1).max(0) as usize;
            Ok(1 + 3 * JUMP_OFFSET_LEN + cases * JUMP_OFFSET_LEN)
        }
        JsOp::Lookupswitch => {
            if slice.len() < 1 + JUMP_OFFSET_LEN + ATOM_INDEX_LEN {
                return Err(format!("lookupswitch truncated at header"));
            }
            let npairs = read_u16_be(&slice[1 + JUMP_OFFSET_LEN..])? as usize;
            Ok(1 + JUMP_OFFSET_LEN + ATOM_INDEX_LEN + npairs * (ATOM_INDEX_LEN + JUMP_OFFSET_LEN))
        }
        other => Err(format!("not a variable-length op: {:?}", other)),
    }
}

fn read_u16_be(s: &[u8]) -> Result<u16, String> {
    if s.len() < 2 {
        return Err("short read for u16 BE".into());
    }
    Ok(u16::from_be_bytes([s[0], s[1]]))
}

fn read_i16_be(s: &[u8]) -> Result<i16, String> {
    Ok(read_u16_be(s)? as i16)
}

pub fn read_u16_operand(operand: &[u8]) -> Result<u16, String> {
    read_u16_be(operand)
}

pub fn read_i16_operand(operand: &[u8]) -> Result<i16, String> {
    read_i16_be(operand)
}
