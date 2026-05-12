// Human-readable disassembly of one JsScriptIR.
//
// Used by the cast-loading diagnostic dump and by tests. Stays read-only;
// the actual translator lives in translator.rs.

use std::fmt::Write;

use super::opcodes::{JsOp, JsOpFormat};
use super::variable_length::{read_i16_operand, read_u16_operand};
use super::xdr::{iter_ops, JsAtom, JsScriptIR};

pub fn disassemble(ir: &JsScriptIR) -> String {
    let mut out = String::new();
    disassemble_into(&mut out, ir, "");
    out
}

fn disassemble_into(out: &mut String, ir: &JsScriptIR, indent: &str) {
    let _ = writeln!(
        out,
        "{}script v{} lineno={} depth={} atoms={} bytecode={} prolog={} filename={:?}",
        indent,
        ir.version,
        ir.lineno,
        ir.max_stack_depth,
        ir.atoms.len(),
        ir.bytecode.len(),
        ir.prolog_length,
        ir.filename
    );
    if !ir.atoms.is_empty() {
        let _ = writeln!(out, "{}atoms:", indent);
        for (i, a) in ir.atoms.iter().enumerate() {
            let _ = writeln!(out, "{}  [{}] = {}", indent, i, format_atom(a));
        }
    }
    if !ir.try_notes.is_empty() {
        let _ = writeln!(out, "{}try_notes:", indent);
        for tn in &ir.try_notes {
            let _ = writeln!(
                out,
                "{}  start={} length={} catchStart={}",
                indent, tn.start, tn.length, tn.catch_start
            );
        }
    }
    let _ = writeln!(out, "{}bytecode:", indent);
    let prolog = ir.prolog_length as usize;
    for ins in iter_ops(&ir.bytecode) {
        let ins = match ins {
            Ok(i) => i,
            Err(e) => {
                let _ = writeln!(out, "{}  <decode error: {}>", indent, e);
                break;
            }
        };
        let marker = if ins.offset == prolog { ">" } else { " " };
        let info = ins.op.info();
        let operand_str = format_operand(ir, ins.op, ins.operand, ins.offset);
        let _ = writeln!(
            out,
            "{}{} {:04}: {:>4} {:<16}{}",
            indent, marker, ins.offset, ins.length, info.mnemonic, operand_str
        );
    }
    // Recurse into nested function atoms so the whole script tree shows up.
    let nested_indent = format!("{}    ", indent);
    for (i, a) in ir.atoms.iter().enumerate() {
        if let JsAtom::Function(f) = a {
            let _ = writeln!(
                out,
                "{}-- atom[{}] function {} (nargs={}, nvars={}):",
                indent,
                i,
                f.name.as_deref().unwrap_or("<anonymous>"),
                f.nargs,
                f.nvars
            );
            disassemble_into(out, &f.script, &nested_indent);
        }
    }
}

fn format_atom(a: &JsAtom) -> String {
    match a {
        JsAtom::Null => "null".into(),
        JsAtom::Void => "void".into(),
        JsAtom::Bool(b) => b.to_string(),
        JsAtom::Int(i) => i.to_string(),
        JsAtom::Double(d) => format!("{:.6}", d),
        JsAtom::String(s) => format!("{:?}", s),
        JsAtom::Function(f) => {
            let args: Vec<&str> = f
                .bindings
                .iter()
                .filter(|b| b.kind == super::xdr::JsBindingKind::Argument)
                .map(|b| b.name.as_str())
                .collect();
            format!(
                "<function {}({}) nvars={} bytecode={} >",
                f.name.as_deref().unwrap_or("<anonymous>"),
                args.join(", "),
                f.nvars,
                f.script.bytecode.len()
            )
        }
        JsAtom::Unsupported(tag) => format!("<unsupported tag={}>", tag),
    }
}

fn format_operand(ir: &JsScriptIR, op: JsOp, operand: &[u8], offset: usize) -> String {
    let info = op.info();
    match info.format {
        JsOpFormat::Byte => String::new(),
        JsOpFormat::Uint16 => match read_u16_operand(operand) {
            Ok(v) => format!(" {}", v),
            Err(e) => format!(" <{}>", e),
        },
        JsOpFormat::Local | JsOpFormat::Qarg | JsOpFormat::Qvar => match read_u16_operand(operand) {
            Ok(v) => format!(" {}", v),
            Err(e) => format!(" <{}>", e),
        },
        JsOpFormat::Const => match read_u16_operand(operand) {
            Ok(idx) => {
                let label = ir
                    .atoms
                    .get(idx as usize)
                    .map(format_atom)
                    .unwrap_or_else(|| "<oob>".into());
                format!(" #{}  ; {}", idx, label)
            }
            Err(e) => format!(" <{}>", e),
        },
        JsOpFormat::Jump => match read_i16_operand(operand) {
            Ok(delta) => {
                let target = offset as i32 + delta as i32;
                format!(" {:+}  ; -> {}", delta, target)
            }
            Err(e) => format!(" <{}>", e),
        },
        JsOpFormat::Object => match read_u16_operand(operand) {
            Ok(v) => format!(" obj#{}", v),
            Err(e) => format!(" <{}>", e),
        },
        JsOpFormat::Tableswitch | JsOpFormat::Lookupswitch => {
            // Operand payload is variable; just summarise.
            format!(" <{} bytes>", operand.len())
        }
    }
}
