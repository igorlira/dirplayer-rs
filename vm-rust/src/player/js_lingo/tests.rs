// Integration tests for the XDR decoder + disassembler, exercised against
// the jsMov.dcr payload (one top-level JSScript whose atom map contains
// three nested function objects).

use super::disasm::disassemble;
use super::opcodes::JsOp;
use super::test_fixtures::*;
use super::variable_length::{read_i16_operand, read_u16_operand};
use super::xdr::{decode_script, iter_ops, JsAtom, JsBindingKind, JsScriptIR};

fn collect_ops(bc: &[u8]) -> Vec<(usize, JsOp, Vec<u8>)> {
    iter_ops(bc)
        .map(|r| r.expect("decode error"))
        .map(|i| (i.offset, i.op, i.operand.to_vec()))
        .collect()
}

fn const_operand(operand: &[u8]) -> u16 {
    read_u16_operand(operand).expect("u16 operand")
}

fn payload() -> JsScriptIR {
    decode_script(JS_MOV_PAYLOAD).expect("decode top-level JSScript")
}

#[test]
fn top_level_script_header() {
    let ir = payload();
    assert_eq!(ir.magic, 0xdead_0003);
    assert_eq!(ir.version, 150, "JSVERSION_1_5");
    assert_eq!(ir.prolog_length, 15, "5 declarations × 3 bytes = 15");
    assert_eq!(ir.lineno, 1);
}

#[test]
fn top_level_atoms_include_globals_and_functions() {
    let ir = payload();

    // Strings: var names + the "alice" literal.
    let names: Vec<&str> = ir
        .atoms
        .iter()
        .filter_map(|a| match a {
            JsAtom::String(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"pCounter"));
    assert!(names.contains(&"pName"));
    assert!(names.contains(&"alice"));

    // Three function atoms — one per top-level function declaration.
    let fn_names: Vec<String> = ir
        .atoms
        .iter()
        .filter_map(|a| match a {
            JsAtom::Function(f) => f.name.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(fn_names.len(), 3, "expected 3 declared functions, got {:?}", fn_names);
    assert!(fn_names.iter().any(|n| n == "on_prepareMovie"));
    assert!(fn_names.iter().any(|n| n == "on_mouseUp"));
    assert!(fn_names.iter().any(|n| n == "add"));
}

#[test]
fn top_level_program_prologue() {
    let ir = payload();
    let ops = collect_ops(&ir.bytecode);
    // Prologue: DEFVAR(0), DEFVAR(1), DEFFUN, DEFFUN, DEFFUN.
    assert!(matches!(ops[0].1, JsOp::Defvar));
    assert_eq!(const_operand(&ops[0].2), 0);
    assert!(matches!(ops[1].1, JsOp::Defvar));
    assert_eq!(const_operand(&ops[1].2), 1);
    assert!(matches!(ops[2].1, JsOp::Deffun));
    assert!(matches!(ops[3].1, JsOp::Deffun));
    assert!(matches!(ops[4].1, JsOp::Deffun));

    // Body assigns pCounter = 0 and pName = "alice".
    let kinds: Vec<JsOp> = ops.iter().map(|(_, o, _)| *o).collect();
    assert!(kinds.contains(&JsOp::Bindname));
    assert!(kinds.contains(&JsOp::Zero));
    assert!(kinds.contains(&JsOp::Setname));
    assert!(kinds.contains(&JsOp::String));
}

fn function_atom<'a>(ir: &'a JsScriptIR, name: &str) -> &'a super::xdr::JsFunctionAtom {
    for a in &ir.atoms {
        if let JsAtom::Function(f) = a {
            if f.name.as_deref() == Some(name) {
                return f;
            }
        }
    }
    panic!("function {} not found in atom map", name);
}

#[test]
fn add_function_is_exactly_three_instructions_and_return() {
    let ir = payload();
    let f = function_atom(&ir, "add");
    assert_eq!(f.nargs, 2);
    assert_eq!(f.nvars, 0);

    // Bindings are the two arguments x, y.
    let args: Vec<&str> = f
        .bindings
        .iter()
        .filter(|b| b.kind == JsBindingKind::Argument)
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(args, vec!["x", "y"]);

    let ops = collect_ops(&f.script.bytecode);
    assert_eq!(ops.len(), 4, "expected exactly: GETARG 0; GETARG 1; ADD; RETURN");
    assert_eq!(ops[0].1, JsOp::Getarg);
    assert_eq!(const_operand(&ops[0].2), 0);
    assert_eq!(ops[1].1, JsOp::Getarg);
    assert_eq!(const_operand(&ops[1].2), 1);
    assert_eq!(ops[2].1, JsOp::Add);
    assert_eq!(ops[3].1, JsOp::Return);
}

#[test]
fn on_preparemovie_calls_trace_with_concat() {
    let ir = payload();
    let f = function_atom(&ir, "on_prepareMovie");
    let kinds: Vec<JsOp> = collect_ops(&f.script.bytecode)
        .into_iter()
        .map(|(_, o, _)| o)
        .collect();
    assert!(kinds.contains(&JsOp::Add), "string concat in trace argument");
    assert!(kinds.contains(&JsOp::Call), "trace() invocation");
}

#[test]
fn on_mouseup_has_loop_and_conditional() {
    let ir = payload();
    let f = function_atom(&ir, "on_mouseUp");

    // arr and obj are locals; i is a local for the loop.
    let locals: Vec<&str> = f
        .bindings
        .iter()
        .filter(|b| b.kind != JsBindingKind::Argument)
        .map(|b| b.name.as_str())
        .collect();
    assert!(locals.contains(&"arr"), "arr is a local");
    assert!(locals.contains(&"obj"), "obj is a local");
    assert!(locals.contains(&"i"), "i is a local");

    let kinds: Vec<JsOp> = collect_ops(&f.script.bytecode)
        .into_iter()
        .map(|(_, o, _)| o)
        .collect();
    assert!(kinds.contains(&JsOp::Ifeq) || kinds.contains(&JsOp::Ifne),
        "for-loop conditional branch");
    assert!(kinds.contains(&JsOp::Goto), "for-loop back-edge");
    assert!(kinds.contains(&JsOp::Gt), "if (pCounter > 100)");
    assert!(kinds.contains(&JsOp::Setprop), "sprite(1).locH = ...");
    assert!(kinds.contains(&JsOp::Newinit), "array/object literal init");
}

#[test]
fn iter_ops_consumes_every_byte_of_each_script() {
    let ir = payload();
    fn walk(s: &JsScriptIR) {
        let mut n = 0usize;
        for r in iter_ops(&s.bytecode) {
            n += r.expect("clean op").length;
        }
        assert_eq!(n, s.bytecode.len(), "every byte consumed");
        for a in &s.atoms {
            if let JsAtom::Function(f) = a {
                walk(&f.script);
            }
        }
    }
    walk(&ir);
}

#[test]
fn disassembly_renders_full_program_tree() {
    let ir = payload();
    let dump = disassemble(&ir);
    // Top-level identifiers + atoms.
    assert!(dump.contains("pCounter"));
    assert!(dump.contains("\"alice\""));
    // Each nested function appears.
    assert!(dump.contains("on_prepareMovie"));
    assert!(dump.contains("on_mouseUp"));
    assert!(dump.contains("function add"));
    // Nested function body must be disassembled too.
    assert!(dump.contains("getarg"));
    assert!(dump.contains("return"));
}

#[test]
fn signed_jump_operand_reads_negative() {
    let v = read_i16_operand(&[0xff, 0xfb]).unwrap();
    assert_eq!(v, -5);
}
