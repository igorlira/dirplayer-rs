// XDR reader for SpiderMonkey 1.5 (jsdmx) serialized scripts.
//
// Wire format reference: jsdmx/src/jsscript.c::js_XDRScript and
// jsdmx/src/jsxdrapi.c::JS_XDRValue / JS_XDRString / JS_XDRDouble /
// JS_XDRCString / JS_XDRBytes. JSXDR_ALIGN = 4 (every variable-length field
// is zero-padded so the next field is 4-byte aligned).
//
// All multi-byte integers are little-endian on x86 authoring tools (the only
// thing Director MX 2004 was built on, modulo PowerPC Macs we don't target
// in dirplayer-rs).

use super::opcodes::JsOp;

/// JSXDR_MAGIC_SCRIPT_* values from jsdmx/src/jsxdrapi.h:180-183.
pub const JSXDR_MAGIC_SCRIPT_1: u32 = 0xdead_0001;
pub const JSXDR_MAGIC_SCRIPT_2: u32 = 0xdead_0002;
pub const JSXDR_MAGIC_SCRIPT_3: u32 = 0xdead_0003;

const JSXDR_ALIGN: usize = 4;

/// JSVAL XDR tags. Inline INT uses any odd value (low bit = JSVAL_INT tag).
const JSVAL_OBJECT: u32 = 0;
const JSVAL_INT: u32 = 1;
const JSVAL_DOUBLE: u32 = 2;
const JSVAL_STRING: u32 = 4;
const JSVAL_BOOLEAN: u32 = 6;
const JSVAL_XDRNULL: u32 = 8;
const JSVAL_XDRVOID: u32 = 10;

/// One entry in the script's atom map. Order in the IR mirrors the on-disk
/// index field so an opcode's u16 atom operand can directly index `atoms[]`.
#[derive(Debug, Clone)]
pub enum JsAtom {
    Null,
    Void,
    Bool(bool),
    Int(i32),
    Double(f64),
    String(String),
    /// Nested JSFunction (declared by `function name() {...}` or expressed
    /// as `function() {...}`). The inner script is fully decoded.
    Function(Box<JsFunctionAtom>),
    /// Other object atoms (regex objects, object literals); not yet handled.
    Unsupported(u32),
}

/// A function-typed atom (the XDR'd form of a JSFunction). Decoded directly
/// from `fun_xdrObject` in jsdmx/src/jsfun.c.
#[derive(Debug, Clone)]
pub struct JsFunctionAtom {
    pub name: Option<String>,
    pub nargs: u16,
    pub extra: u16,
    pub nvars: u16,
    pub flags: u8,
    /// `nargs + nvars` slot descriptors. Each entry binds a name to a slot
    /// (argument or local).
    pub bindings: Vec<JsFunctionBinding>,
    pub script: JsScriptIR,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsBindingKind {
    Argument, // JSXDR_FUNARG
    Variable, // JSXDR_FUNVAR
    Constant, // JSXDR_FUNCONST
}

#[derive(Debug, Clone)]
pub struct JsFunctionBinding {
    pub kind: JsBindingKind,
    /// Short id (slot number within args / vars).
    pub short_id: i32,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct JsTryNote {
    pub start: u32,
    pub length: u32,
    pub catch_start: u32,
}

#[derive(Debug, Clone)]
pub struct JsScriptIR {
    pub magic: u32,
    pub bytecode: Vec<u8>,
    /// Offset in `bytecode` where the main body begins. `bytecode[0..prolog_length]`
    /// contains DEFVAR/DEFFUN/DEFCONST decls hoisted to the top of the script.
    pub prolog_length: u32,
    pub version: u32,
    pub atoms: Vec<JsAtom>,
    pub source_notes: Vec<u8>,
    pub filename: Option<String>,
    pub lineno: u32,
    pub max_stack_depth: u32,
    pub try_notes: Vec<JsTryNote>,
}

#[derive(Debug)]
pub enum JsXdrError {
    UnexpectedEof { offset: usize, want: usize },
    BadMagic(u32),
    BadAtomTag(u32),
    BadUtf16(usize),
}

impl std::fmt::Display for JsXdrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedEof { offset, want } => {
                write!(f, "unexpected EOF at offset {} (wanted {} bytes)", offset, want)
            }
            Self::BadMagic(m) => write!(f, "bad XDR script magic: 0x{:08x}", m),
            Self::BadAtomTag(t) => write!(f, "bad XDR atom tag: 0x{:x}", t),
            Self::BadUtf16(o) => write!(f, "invalid UTF-16 at offset {}", o),
        }
    }
}

impl std::error::Error for JsXdrError {}

struct XdrReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> XdrReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn need(&self, want: usize) -> Result<(), JsXdrError> {
        if self.pos + want > self.buf.len() {
            return Err(JsXdrError::UnexpectedEof { offset: self.pos, want });
        }
        Ok(())
    }

    fn u32(&mut self) -> Result<u32, JsXdrError> {
        self.need(4)?;
        let v = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    fn bytes(&mut self, len: usize) -> Result<Vec<u8>, JsXdrError> {
        self.need(len)?;
        let v = self.buf[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(v)
    }

    /// Skip alignment padding so the next read starts on a 4-byte boundary.
    /// JSXDR_ALIGN behavior: padding is inserted by JS_XDRBytes when the
    /// stream position after a variable-length field is unaligned.
    fn align_to_4(&mut self) -> Result<(), JsXdrError> {
        let r = self.pos % JSXDR_ALIGN;
        if r != 0 {
            let pad = JSXDR_ALIGN - r;
            self.need(pad)?;
            self.pos += pad;
        }
        Ok(())
    }

    /// XDR string of u16 chars: u32 length-in-chars, then chars (UTF-16),
    /// then padding to JSXDR_ALIGN.
    fn js_string(&mut self) -> Result<String, JsXdrError> {
        let len_chars = self.u32()? as usize;
        let nbytes = len_chars * 2;
        self.need(nbytes)?;
        let raw = &self.buf[self.pos..self.pos + nbytes];
        self.pos += nbytes;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let s = String::from_utf16(&units)
            .map_err(|_| JsXdrError::BadUtf16(self.pos - nbytes))?;
        // Mozilla pads the underlying byte count, not the char count.
        if nbytes % JSXDR_ALIGN != 0 {
            let pad = JSXDR_ALIGN - (nbytes % JSXDR_ALIGN);
            self.need(pad)?;
            self.pos += pad;
        }
        Ok(s)
    }

    /// XDR C-string: u32 length-in-bytes, then bytes, then padding to 4.
    fn c_string(&mut self) -> Result<String, JsXdrError> {
        let len = self.u32()? as usize;
        let bytes = self.bytes(len)?;
        let pad = if len % JSXDR_ALIGN != 0 { JSXDR_ALIGN - (len % JSXDR_ALIGN) } else { 0 };
        if pad != 0 {
            self.need(pad)?;
            self.pos += pad;
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn js_double(&mut self) -> Result<f64, JsXdrError> {
        // jsxdrapi.c::JS_XDRDouble — encodes/decodes as two u32s. On
        // little-endian platforms the low word is written first.
        let lo = self.u32()?;
        let hi = self.u32()?;
        let bits = ((hi as u64) << 32) | (lo as u64);
        Ok(f64::from_bits(bits))
    }

    /// JS_XDRStringOrNull — u32 null_flag, then (if !null) a u16-char string.
    fn js_string_or_null(&mut self) -> Result<Option<String>, JsXdrError> {
        let null_flag = self.u32()?;
        if null_flag != 0 {
            Ok(None)
        } else {
            Ok(Some(self.js_string()?))
        }
    }
}

/// Decode one XDR'd JSScript (one 0xDEAD000x block) into an IR. The top-level
/// entry point — internally calls [`decode_script_at`] starting at offset 0.
pub fn decode_script(buf: &[u8]) -> Result<JsScriptIR, JsXdrError> {
    let mut r = XdrReader::new(buf);
    decode_script_into(&mut r)
}

fn decode_script_into(r: &mut XdrReader<'_>) -> Result<JsScriptIR, JsXdrError> {

    let magic = r.u32()?;
    if magic != JSXDR_MAGIC_SCRIPT_1
        && magic != JSXDR_MAGIC_SCRIPT_2
        && magic != JSXDR_MAGIC_SCRIPT_3
    {
        return Err(JsXdrError::BadMagic(magic));
    }

    let length = r.u32()? as usize;
    let (prolog_length, version) = if magic >= JSXDR_MAGIC_SCRIPT_2 {
        (r.u32()?, r.u32()?)
    } else {
        (0, 0)
    };

    // Bytecode is followed by alignment padding to 4 bytes.
    let bytecode = r.bytes(length)?;
    r.align_to_4()?;

    // Atom map.
    let atom_count = r.u32()? as usize;
    let mut atoms: Vec<JsAtom> = vec![JsAtom::Void; atom_count];
    for _ in 0..atom_count {
        let index = r.u32()? as usize;
        let tag = r.u32()?;
        let atom = match tag {
            JSVAL_XDRNULL => JsAtom::Null,
            JSVAL_XDRVOID => JsAtom::Void,
            JSVAL_STRING => JsAtom::String(r.js_string()?),
            JSVAL_DOUBLE => JsAtom::Double(r.js_double()?),
            JSVAL_BOOLEAN => JsAtom::Bool(r.u32()? != 0),
            JSVAL_OBJECT => decode_object_atom(r)?,
            t if t & JSVAL_INT != 0 => JsAtom::Int(r.u32()? as i32),
            other => return Err(JsXdrError::BadAtomTag(other)),
        };
        if index < atoms.len() {
            atoms[index] = atom;
        }
    }

    let notelen = r.u32()? as usize;
    let source_notes = r.bytes(notelen)?;
    r.align_to_4()?;

    // Filename: JS_XDRCStringOrNull writes a u32 null-flag first.
    let null_flag = r.u32()?;
    let filename = if null_flag != 0 {
        None
    } else {
        Some(r.c_string()?)
    };

    let lineno = r.u32()?;
    let max_stack_depth = r.u32()?;
    let numtrys = r.u32()? as usize;

    if magic >= JSXDR_MAGIC_SCRIPT_3 {
        let encodeable = r.u32()?;
        if encodeable != 0 {
            // Principals transcoder data — Director never emits this; if it
            // appeared we'd need to handle it. Fail loudly.
            return Err(JsXdrError::BadAtomTag(0xDEAD));
        }
    }

    let mut try_notes = Vec::with_capacity(numtrys);
    for _ in 0..numtrys {
        let start = r.u32()?;
        let length = r.u32()?;
        let catch_start = r.u32()?;
        try_notes.push(JsTryNote { start, length, catch_start });
    }

    Ok(JsScriptIR {
        magic,
        bytecode,
        prolog_length,
        version,
        atoms,
        source_notes,
        filename,
        lineno,
        max_stack_depth,
        try_notes,
    })
}

/// JSXDR_FUN* tags from jsdmx/src/jsfun.c — kind of property a JSFunction
/// binding represents.
const JSXDR_FUNARG: u32 = 1;
const JSXDR_FUNVAR: u32 = 2;
const JSXDR_FUNCONST: u32 = 3;

/// Decode the JSVAL_OBJECT atom payload. Cross-reference: jsobj.c::js_XDRObject.
fn decode_object_atom(r: &mut XdrReader<'_>) -> Result<JsAtom, JsXdrError> {
    // js_XDRObject:
    //   u32 classDef           (1 = first reference, 0 = already registered)
    //   if classDef: JS_XDRCString(className)
    //   u32 classId
    //   class-specific xdrObject
    let class_def = r.u32()?;
    let class_name = if class_def != 0 {
        Some(r.c_string()?)
    } else {
        None
    };
    let _class_id = r.u32()?;

    // The only class we currently know how to decode is "Function". Anything
    // else we mark as unsupported and fail loud so we don't desync atom maps.
    let is_function = match &class_name {
        Some(n) => n == "Function",
        None => true, // class was already registered earlier; assume Function
    };
    if !is_function {
        return Err(JsXdrError::BadAtomTag(0xFEED_DEAD));
    }
    decode_function_object(r)
}

/// Decode JSFunction body. Cross-reference: jsfun.c::fun_xdrObject.
fn decode_function_object(r: &mut XdrReader<'_>) -> Result<JsAtom, JsXdrError> {
    // atom name (nullable): u32 null-flag, then UTF-16 string if non-null.
    let name = r.js_string_or_null()?;
    // All u8/u16 fields go through JS_XDRUint32 — they're each 4 bytes wide.
    let nargs = (r.u32()? & 0xFFFF) as u16;
    let extra = (r.u32()? & 0xFFFF) as u16;
    let nvars = (r.u32()? & 0xFFFF) as u16;
    let flags = (r.u32()? & 0xFF) as u8;

    let total_bindings = nargs as usize + nvars as usize;
    let mut bindings = Vec::with_capacity(total_bindings);
    for _ in 0..total_bindings {
        let kind_tag = r.u32()?;
        let short_id = r.u32()? as i32;
        let prop_name = r.c_string()?;
        let kind = match kind_tag {
            JSXDR_FUNARG => JsBindingKind::Argument,
            JSXDR_FUNVAR => JsBindingKind::Variable,
            JSXDR_FUNCONST => JsBindingKind::Constant,
            other => return Err(JsXdrError::BadAtomTag(other)),
        };
        bindings.push(JsFunctionBinding { kind, short_id, name: prop_name });
    }

    // Inner script — same XDR format, recursively decoded.
    let script = decode_script_into(r)?;

    Ok(JsAtom::Function(Box::new(JsFunctionAtom {
        name,
        nargs,
        extra,
        nvars,
        flags,
        bindings,
        script,
    })))
}

/// Iterate decoded ops over a bytecode slice. Yields (offset, op, operand_bytes).
pub fn iter_ops(bc: &[u8]) -> JsOpIter<'_> {
    JsOpIter { bc, pos: 0 }
}

pub struct JsOpIter<'a> {
    bc: &'a [u8],
    pos: usize,
}

#[derive(Debug, Clone)]
pub struct JsInstruction<'a> {
    pub offset: usize,
    pub op: JsOp,
    pub operand: &'a [u8],
    /// Full instruction length in bytes (advances `pos`).
    pub length: usize,
}

impl<'a> Iterator for JsOpIter<'a> {
    type Item = Result<JsInstruction<'a>, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.bc.len() {
            return None;
        }
        let byte = self.bc[self.pos];
        let op = match JsOp::from_byte(byte) {
            Some(op) => op,
            None => return Some(Err(format!("unknown opcode 0x{:02x} at {}", byte, self.pos))),
        };
        let info = op.info();
        let len = if info.length < 0 {
            // Variable length: tableswitch / lookupswitch. Parsed here so the
            // iterator can skip cleanly. Refs jsopcode.c::js_GetVariableBytecodeLength.
            match super::variable_length::variable_op_length(op, &self.bc[self.pos..]) {
                Ok(l) => l,
                Err(e) => return Some(Err(e)),
            }
        } else {
            info.length as usize
        };
        if self.pos + len > self.bc.len() {
            return Some(Err(format!(
                "truncated instruction {} (need {} bytes, have {})",
                info.mnemonic,
                len,
                self.bc.len() - self.pos
            )));
        }
        let instr = JsInstruction {
            offset: self.pos,
            op,
            operand: &self.bc[self.pos + 1..self.pos + len],
            length: len,
        };
        self.pos += len;
        Some(Ok(instr))
    }
}
