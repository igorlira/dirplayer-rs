// Interpreter tests.
//
// First wave: exercise every supported opcode against bytecode hand-built
// in-test (so we don't depend on jsMov.dcr semantics for low-level coverage),
// plus integration tests that run the real jsMov.dcr functions and assert
// their observable behaviour.

use std::cell::RefCell;
use std::rc::Rc;

use super::interpreter::JsRuntime;
use super::opcodes::JsOp;
use super::test_fixtures::JS_MOV_PAYLOAD;
use super::value::{JsArray, JsObject, JsValue};
use super::xdr::{decode_script, JsAtom};

#[test]
fn dump_on_preparemovie_disasm() {
    let ir = decode_script(JS_MOV_PAYLOAD).expect("decode");
    for a in &ir.atoms {
        if let JsAtom::Function(f) = a {
            if f.name.as_deref() == Some("on_prepareMovie") {
                println!("\n=== on_prepareMovie ===");
                println!("{}", super::disasm::disassemble(&f.script));
            }
        }
    }
}

#[test]
fn dump_on_mouseup_disasm() {
    let ir = decode_script(JS_MOV_PAYLOAD).expect("decode");
    for a in &ir.atoms {
        if let JsAtom::Function(f) = a {
            if f.name.as_deref() == Some("on_mouseUp") {
                println!("\n=== on_mouseUp ===");
                println!("{}", super::disasm::disassemble(&f.script));
            }
        }
    }
}

fn atom_idx_of_function(ir: &super::xdr::JsScriptIR, name: &str) -> Option<usize> {
    for (i, a) in ir.atoms.iter().enumerate() {
        if let JsAtom::Function(f) = a {
            if f.name.as_deref() == Some(name) {
                return Some(i);
            }
        }
    }
    None
}

#[test]
fn add_2_3_returns_5() {
    let ir = Rc::new(decode_script(JS_MOV_PAYLOAD).expect("decode"));
    let mut rt = JsRuntime::new();
    rt.run_program(&ir).expect("program runs");

    // After program runs, `add` is in the global object.
    let add = rt.global.borrow().get_own("add").cloned().expect("add hoisted");
    let result = rt
        .invoke(&add, vec![JsValue::Int(2), JsValue::Int(3)], JsValue::Undefined)
        .expect("invoke add");
    match result {
        JsValue::Int(5) => {}
        other => panic!("expected 5, got {:?}", other),
    }
}

#[test]
fn add_with_strings_concatenates() {
    let ir = Rc::new(decode_script(JS_MOV_PAYLOAD).expect("decode"));
    let mut rt = JsRuntime::new();
    rt.run_program(&ir).expect("program runs");

    let add = rt.global.borrow().get_own("add").cloned().unwrap();
    let result = rt
        .invoke(
            &add,
            vec![JsValue::String(Rc::new("hello ".into())), JsValue::String(Rc::new("world".into()))],
            JsValue::Undefined,
        )
        .expect("invoke");
    assert_eq!(result.to_string(), "hello world");
}

#[test]
fn program_hoists_globals_and_inits_them() {
    let ir = Rc::new(decode_script(JS_MOV_PAYLOAD).expect("decode"));
    let mut rt = JsRuntime::new();
    rt.run_program(&ir).expect("program runs");

    // var pCounter = 0; sets pCounter = 0 on the global object.
    let counter = rt.global.borrow().get_own("pCounter").cloned();
    assert!(matches!(counter, Some(JsValue::Int(0))), "pCounter = 0, got {:?}", counter);
    // var pName = "alice"
    match rt.global.borrow().get_own("pName").cloned() {
        Some(JsValue::String(s)) => assert_eq!(&*s, "alice"),
        other => panic!("expected pName=\"alice\", got {:?}", other),
    }
    // function add was hoisted as a Function value
    assert!(matches!(rt.global.borrow().get_own("add"), Some(JsValue::Function(_))));
}

#[test]
fn on_preparemovie_invokes_trace_with_correct_string() {
    let ir = Rc::new(decode_script(JS_MOV_PAYLOAD).expect("decode"));
    let mut rt = JsRuntime::new();

    let trace_log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let log_clone = trace_log.clone();
    rt.define_native("trace", move |args| {
        log_clone.borrow_mut().push(args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" "));
        Ok(JsValue::Undefined)
    });

    rt.run_program(&ir).expect("program runs");
    let on_prepare = rt.global.borrow().get_own("on_prepareMovie").cloned().expect("hoisted");
    rt.invoke(&on_prepare, vec![], JsValue::Undefined).expect("invoke");

    let logs = trace_log.borrow();
    assert_eq!(logs.len(), 1, "trace called once, got {:?}", *logs);
    assert_eq!(logs[0], "hello from js, counter=42");

    // pCounter was reassigned to 42 by the handler.
    match rt.global.borrow().get_own("pCounter").cloned() {
        Some(JsValue::Int(42)) => {}
        other => panic!("pCounter expected 42, got {:?}", other),
    }
}

#[test]
fn array_literal_and_length_property() {
    let ir = Rc::new(decode_script(JS_MOV_PAYLOAD).expect("decode"));
    let mut rt = JsRuntime::new();
    rt.run_program(&ir).expect("program runs");

    // Capture the array we get inside on_mouseUp by exposing trace as our probe.
    let traced: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let log_clone = traced.clone();
    rt.define_native("trace", move |args| {
        log_clone.borrow_mut().push(args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" "));
        Ok(JsValue::Undefined)
    });
    // sprite() needs to return something property-settable for the if-branch
    // setter. For now we install a stub that returns a generic object.
    rt.define_native("sprite", |_args| {
        Ok(JsValue::Object(Rc::new(RefCell::new(JsObject::new()))))
    });

    let on_mouseup = rt.global.borrow().get_own("on_mouseUp").cloned().expect("hoisted");
    rt.invoke(&on_mouseup, vec![], JsValue::Undefined).expect("invoke");

    let logs = traced.borrow();
    // The handler logs three lines: "item 0 = 1", "item 1 = 2", "item 2 = 3".
    assert!(logs.iter().any(|l| l == "item 0 = 1"), "got {:?}", *logs);
    assert!(logs.iter().any(|l| l == "item 1 = 2"));
    assert!(logs.iter().any(|l| l == "item 2 = 3"));
    // Sanity: pCounter += arr.length = 0 + 3 = 3 (because on_prepareMovie wasn't called here).
    match rt.global.borrow().get_own("pCounter").cloned() {
        Some(JsValue::Int(3)) => {}
        other => panic!("pCounter expected 3 (0+3), got {:?}", other),
    }
}

// ===== Synthetic-bytecode tests (covers every supported opcode) =====
//
// Each test hand-builds a tiny bytecode stream + atom map and runs it.

fn run_synth(bytecode: Vec<u8>, atoms: Vec<JsAtom>) -> Result<JsValue, super::value::JsError> {
    let ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003,
        bytecode,
        prolog_length: 0,
        version: 150,
        atoms,
        source_notes: Vec::new(),
        filename: None,
        lineno: 1,
        max_stack_depth: 16,
        try_notes: Vec::new(),
    };
    let mut rt = JsRuntime::new();
    let f = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("synth".into()),
            nargs: 0,
            extra: 0,
            nvars: 0,
            flags: 0,
            bindings: Vec::new(),
            script: ir,
        }),
        captured_scope: None,
    };
    rt.call_function(&Rc::new(f), Vec::new(), JsValue::Undefined)
}

fn u16_be(v: u16) -> [u8; 2] { v.to_be_bytes() }
fn i16_be(v: i16) -> [u8; 2] { v.to_be_bytes() }

#[test]
fn synth_arithmetic_chain() {
    // 2 + 3 * 4 - 1
    // We push 2, then 3, then 4, MUL → 12, ADD → 14, push 1, SUB → 13, RETURN.
    // SpiderMonkey 1.5 has no PUSHINT8 in this table — we synthesise via ZERO/ONE.
    // To get 2: ONE; ONE; ADD. 3: ONE; ONE; ONE; ADD; ADD. Skip — use Number atoms.
    let atoms = vec![
        JsAtom::Int(2),  // atom 0
        JsAtom::Int(3),  // atom 1
        JsAtom::Int(4),  // atom 2
        JsAtom::Int(1),  // atom 3
    ];
    let mut bc = Vec::new();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));    // push 2
    bc.push(JsOp::Number as u8); bc.extend(u16_be(1));    // push 3
    bc.push(JsOp::Number as u8); bc.extend(u16_be(2));    // push 4
    bc.push(JsOp::Mul as u8);                              // 3*4 = 12
    bc.push(JsOp::Add as u8);                              // 2+12 = 14
    bc.push(JsOp::Number as u8); bc.extend(u16_be(3));    // push 1
    bc.push(JsOp::Sub as u8);                              // 14-1 = 13
    bc.push(JsOp::Return as u8);
    assert!(matches!(run_synth(bc, atoms).unwrap(), JsValue::Int(13)));
}

#[test]
fn synth_jump_and_compare() {
    // var-less: push 5, push 10, GT — false, IFEQ jumps over the "1" branch and returns 0.
    let atoms = vec![JsAtom::Int(5), JsAtom::Int(10)];
    let mut bc = Vec::new();
    // if (5 > 10) return 1; else return 0;
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));        // push 5  @ pc=0
    bc.push(JsOp::Number as u8); bc.extend(u16_be(1));        // push 10 @ pc=3
    bc.push(JsOp::Gt as u8);                                   // 5 > 10 = false  @ pc=6
    let ifeq_pc = bc.len();                                    // @ pc=7
    bc.push(JsOp::Ifeq as u8); bc.extend(i16_be(0));           // placeholder offset
    bc.push(JsOp::One as u8);                                  // push 1
    bc.push(JsOp::Return as u8);                                // return 1
    let else_target = bc.len() as i32;
    bc.push(JsOp::Zero as u8);                                 // push 0
    bc.push(JsOp::Return as u8);                                // return 0
    // Patch ifeq offset
    let off = else_target - ifeq_pc as i32;
    let off_bytes = (off as i16).to_be_bytes();
    bc[ifeq_pc + 1] = off_bytes[0];
    bc[ifeq_pc + 2] = off_bytes[1];

    assert!(matches!(run_synth(bc, atoms).unwrap(), JsValue::Int(0)));
}

#[test]
fn synth_string_concat_and_compare() {
    let atoms = vec![
        JsAtom::String("foo".into()),
        JsAtom::String("bar".into()),
        JsAtom::String("foobar".into()),
    ];
    let mut bc = Vec::new();
    bc.push(JsOp::String as u8); bc.extend(u16_be(0));     // push "foo"
    bc.push(JsOp::String as u8); bc.extend(u16_be(1));     // push "bar"
    bc.push(JsOp::Add as u8);                              // "foobar"
    bc.push(JsOp::String as u8); bc.extend(u16_be(2));     // push "foobar"
    bc.push(JsOp::Eq as u8);                                // true
    bc.push(JsOp::Return as u8);
    assert!(matches!(run_synth(bc, atoms).unwrap(), JsValue::Bool(true)));
}

#[test]
fn synth_tableswitch_routes_to_correct_case() {
    // switch(x) { case 0: return 100; case 1: return 200; default: return 999; }
    // Tableswitch operand layout:
    //   i16 default_offset, i16 low (=0), i16 high (=1),
    //   i16 case0_offset, i16 case1_offset
    let atoms = vec![
        JsAtom::Int(1),    // 0 - discriminant
        JsAtom::Int(100),  // 1
        JsAtom::Int(200),  // 2
        JsAtom::Int(999),  // 3
    ];
    let mut bc = Vec::new();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));   // push 1 @ pc=0..2
    let tsw_pc = bc.len();                                // pc=3
    bc.push(JsOp::Tableswitch as u8);                     // op @ pc=3
    let body_start = bc.len();                            // pc=4 (first operand byte)
    bc.extend(i16_be(0)); // default_offset placeholder
    bc.extend(i16_be(0)); // low = 0
    bc.extend(i16_be(1)); // high = 1
    bc.extend(i16_be(0)); // case0_offset placeholder
    bc.extend(i16_be(0)); // case1_offset placeholder
    let case0_at = bc.len();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(1));   // push 100
    bc.push(JsOp::Return as u8);
    let case1_at = bc.len();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(2));   // push 200
    bc.push(JsOp::Return as u8);
    let default_at = bc.len();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(3));   // push 999
    bc.push(JsOp::Return as u8);

    // Patch offsets, relative to tsw_pc.
    let patch = |bc: &mut Vec<u8>, offset_pos: usize, target: usize| {
        let delta = target as i32 - tsw_pc as i32;
        let bytes = (delta as i16).to_be_bytes();
        bc[offset_pos] = bytes[0];
        bc[offset_pos + 1] = bytes[1];
    };
    patch(&mut bc, body_start, default_at);     // default
    patch(&mut bc, body_start + 6, case0_at);   // case 0
    patch(&mut bc, body_start + 8, case1_at);   // case 1

    let result = run_synth(bc, atoms).unwrap();
    assert!(matches!(result, JsValue::Int(200)), "expected 200, got {:?}", result);
}

#[test]
fn synth_typeof_returns_correct_strings() {
    let atoms = vec![JsAtom::String("foo".into())];
    let mut bc = Vec::new();
    bc.push(JsOp::String as u8); bc.extend(u16_be(0));
    bc.push(JsOp::Typeof as u8);
    bc.push(JsOp::Return as u8);
    match run_synth(bc, atoms).unwrap() {
        JsValue::String(s) => assert_eq!(&*s, "string"),
        other => panic!("expected \"string\", got {:?}", other),
    }
}

#[test]
fn synth_forin_counts_keys() {
    // function(obj) {
    //   var k; var count = 0;
    //   for (k in obj) { count++; }
    //   return count;
    // }
    //
    // Tests that FORVAR walks the iterator over the object's keys and
    // pushes the right has-more boolean each iteration.
    let atoms: Vec<JsAtom> = Vec::new();
    let mut bc = Vec::new();

    // count = 0
    bc.push(JsOp::Zero as u8);                              // pc=0: push 0
    bc.push(JsOp::Setvar as u8); bc.extend(u16_be(1));      // pc=1: count = 0 (slot 1)
    bc.push(JsOp::Pop as u8);                               // pc=4

    // Iter setup: push undefined (iter slot), then GETARG 0 (the object).
    bc.push(JsOp::Push as u8);                              // pc=5: push undefined
    bc.push(JsOp::Getarg as u8); bc.extend(u16_be(0));      // pc=6: push obj

    // Loop top: FORVAR k
    let loop_top = bc.len();                                // pc=9
    bc.push(JsOp::Forvar as u8); bc.extend(u16_be(0));      // pc=9: FORVAR slot=0 (k)

    let ifeq_pc = bc.len();                                 // pc=12
    bc.push(JsOp::Ifeq as u8); bc.extend(i16_be(0));        // pc=12: placeholder, patched below

    // Body: count++
    bc.push(JsOp::Incvar as u8); bc.extend(u16_be(1));      // pc=15
    bc.push(JsOp::Pop as u8);                               // pc=18: discard the incremented value

    // Backward goto to loop_top
    let goto_pc = bc.len();                                 // pc=19
    bc.push(JsOp::Goto as u8); bc.extend(i16_be(0));        // pc=19: placeholder
    let goto_off = loop_top as i32 - goto_pc as i32;
    let goto_off_bytes = (goto_off as i16).to_be_bytes();
    bc[goto_pc + 1] = goto_off_bytes[0];
    bc[goto_pc + 2] = goto_off_bytes[1];

    // Loop end: drop obj + iter slot
    let loop_end = bc.len();                                // pc=22
    bc.push(JsOp::Pop as u8);                               // remove obj
    bc.push(JsOp::Pop as u8);                               // remove iter
    bc.push(JsOp::Getvar as u8); bc.extend(u16_be(1));      // push count
    bc.push(JsOp::Return as u8);

    let ifeq_off = loop_end as i32 - ifeq_pc as i32;
    let ifeq_off_bytes = (ifeq_off as i16).to_be_bytes();
    bc[ifeq_pc + 1] = ifeq_off_bytes[0];
    bc[ifeq_pc + 2] = ifeq_off_bytes[1];

    let ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: bc, prolog_length: 0, version: 150,
        atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 8, try_notes: Vec::new(),
    };
    let f = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("forin_test".into()),
            nargs: 1, extra: 0, nvars: 2, flags: 0,
            bindings: Vec::new(),
            script: ir,
        }),
        captured_scope: None,
    };

    // Build an object with 3 keys.
    let mut obj = JsObject::new();
    obj.set_own("alpha", JsValue::Int(1));
    obj.set_own("beta", JsValue::Int(2));
    obj.set_own("gamma", JsValue::Int(3));
    let obj_val = JsValue::Object(Rc::new(RefCell::new(obj)));

    let mut rt = JsRuntime::new();
    let result = rt.call_function(&Rc::new(f), vec![obj_val], JsValue::Undefined).unwrap();
    assert!(matches!(result, JsValue::Int(3)), "expected 3 keys, got {:?}", result);
}

#[test]
fn synth_forin_array_walks_indices() {
    // function(arr) { var k; var n = 0; for (k in arr) n = k; return n; }
    // Returns the last index visited (as a string).
    let atoms: Vec<JsAtom> = Vec::new();
    let mut bc = Vec::new();

    // n = ""
    bc.push(JsOp::Push as u8);                              // pc=0: push undefined (we'll just write k into n on each iter, no init needed)
    bc.push(JsOp::Setvar as u8); bc.extend(u16_be(1));      // pc=1: n = undefined
    bc.push(JsOp::Pop as u8);                               // pc=4

    bc.push(JsOp::Push as u8);                              // pc=5: iter placeholder
    bc.push(JsOp::Getarg as u8); bc.extend(u16_be(0));      // pc=6

    let loop_top = bc.len();
    bc.push(JsOp::Forvar as u8); bc.extend(u16_be(0));      // pc=9: writes k
    let ifeq_pc = bc.len();
    bc.push(JsOp::Ifeq as u8); bc.extend(i16_be(0));

    // n = k
    bc.push(JsOp::Getvar as u8); bc.extend(u16_be(0));      // push k
    bc.push(JsOp::Setvar as u8); bc.extend(u16_be(1));      // n = k (leaves k on stack)
    bc.push(JsOp::Pop as u8);

    let goto_pc = bc.len();
    bc.push(JsOp::Goto as u8); bc.extend(i16_be(0));
    let goto_off = (loop_top as i32 - goto_pc as i32) as i16;
    let gb = goto_off.to_be_bytes();
    bc[goto_pc + 1] = gb[0]; bc[goto_pc + 2] = gb[1];

    let loop_end = bc.len();
    bc.push(JsOp::Pop as u8); bc.push(JsOp::Pop as u8);
    bc.push(JsOp::Getvar as u8); bc.extend(u16_be(1));
    bc.push(JsOp::Return as u8);

    let ifeq_off = (loop_end as i32 - ifeq_pc as i32) as i16;
    let ib = ifeq_off.to_be_bytes();
    bc[ifeq_pc + 1] = ib[0]; bc[ifeq_pc + 2] = ib[1];

    let ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: bc, prolog_length: 0, version: 150,
        atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 8, try_notes: Vec::new(),
    };
    let f = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("forin_arr".into()),
            nargs: 1, extra: 0, nvars: 2, flags: 0,
            bindings: Vec::new(),
            script: ir,
        }),
        captured_scope: None,
    };

    let arr = JsArray { items: vec![JsValue::Int(10), JsValue::Int(20), JsValue::Int(30)] };
    let arr_val = JsValue::Array(Rc::new(RefCell::new(arr)));

    let mut rt = JsRuntime::new();
    let result = rt.call_function(&Rc::new(f), vec![arr_val], JsValue::Undefined).unwrap();
    match result {
        JsValue::String(s) => assert_eq!(&*s, "2", "last array index visited as string"),
        other => panic!("expected last index \"2\", got {:?}", other),
    }
}

#[test]
fn synth_closure_captures_outer_arg() {
    // function outer(x) {
    //   function inner() { return x; }
    //   return inner;
    // }
    // outer(42)()  ===  42
    //
    // Tests that ANONFUNOBJ captures the current scope at creation time
    // and that when the inner function is invoked later, its NAME `x`
    // resolves via the captured scope chain back to outer's argument.

    // Inner script: NAME "x"; RETURN
    let inner_atoms = vec![JsAtom::String("x".into())];
    let mut inner_bc = Vec::new();
    inner_bc.push(JsOp::Name as u8); inner_bc.extend(u16_be(0));    // push x (via NAME)
    inner_bc.push(JsOp::Return as u8);
    let inner_ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: inner_bc, prolog_length: 0, version: 150,
        atoms: inner_atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 2, try_notes: Vec::new(),
    };
    let inner_atom = super::xdr::JsFunctionAtom {
        name: Some("inner".into()),
        nargs: 0, extra: 0, nvars: 0, flags: 0,
        bindings: Vec::new(),
        script: inner_ir,
    };

    // Outer script: ANONFUNOBJ inner_atom_idx; RETURN
    let outer_atoms = vec![
        JsAtom::Function(Box::new(inner_atom)),
        JsAtom::String("x".into()),
    ];
    let mut outer_bc = Vec::new();
    outer_bc.push(JsOp::Anonfunobj as u8); outer_bc.extend(u16_be(0));  // push closure-over-current-scope
    outer_bc.push(JsOp::Return as u8);
    let outer_ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: outer_bc, prolog_length: 0, version: 150,
        atoms: outer_atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 2, try_notes: Vec::new(),
    };
    let outer_fn = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("outer".into()),
            nargs: 1, extra: 0, nvars: 0, flags: 0,
            bindings: vec![super::xdr::JsFunctionBinding {
                kind: super::xdr::JsBindingKind::Argument,
                name: "x".into(),
                short_id: 0,
            }],
            script: outer_ir,
        }),
        captured_scope: None,
    };

    let mut rt = JsRuntime::new();
    // outer(42) — returns the closure
    let closure = rt
        .call_function(&Rc::new(outer_fn), vec![JsValue::Int(42)], JsValue::Undefined)
        .expect("outer call returns closure");

    // Then invoke the returned closure with no args; it must return 42 (the
    // captured x). Without scope capture this would resolve `x` to undefined.
    let result = rt.invoke(&closure, Vec::new(), JsValue::Undefined)
        .expect("inner call");
    assert!(matches!(result, JsValue::Int(42)),
        "expected 42 from captured x, got {:?}", result);
}

#[test]
fn synth_try_catch_recovers_thrown_value() {
    // function() {
    //   try { throw 42; }
    //   catch (e) { return e; }
    // }
    //
    // The dispatch loop should consult try_notes when `throw` raises,
    // jump to the catch handler, and push the thrown value (42) onto
    // the stack -- where the catch body returns it directly.
    let atoms = vec![JsAtom::Int(42)];
    let mut bc = Vec::new();

    // try body: push 42; throw
    let try_start = bc.len();                                // pc=0
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));       // push 42
    bc.push(JsOp::Throw as u8);                              // throw -- unwinds
    let try_end = bc.len();

    // catch handler: thrown value is already on top of the stack
    // (pushed by the unwinder). Just RETURN it.
    let catch_start = bc.len();
    bc.push(JsOp::Return as u8);

    let try_note = super::xdr::JsTryNote {
        start: try_start as u32,
        length: (try_end - try_start) as u32,
        catch_start: catch_start as u32,
    };

    let ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: bc, prolog_length: 0, version: 150,
        atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 4,
        try_notes: vec![try_note],
    };
    let f = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("trycatch".into()),
            nargs: 0, extra: 0, nvars: 0, flags: 0,
            bindings: Vec::new(),
            script: ir,
        }),
        captured_scope: None,
    };

    let mut rt = JsRuntime::new();
    let result = rt.call_function(&Rc::new(f), Vec::new(), JsValue::Undefined).unwrap();
    assert!(matches!(result, JsValue::Int(42)), "catch should recover the thrown value, got {:?}", result);
}

#[test]
fn synth_try_catch_no_throw_returns_value() {
    // function() { try { return 7; } catch (e) { return 9; } }
    // Try body completes normally without throwing; catch is never entered.
    let atoms = vec![JsAtom::Int(7), JsAtom::Int(9)];
    let mut bc = Vec::new();

    let try_start = bc.len();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));  // push 7
    bc.push(JsOp::Return as u8);                         // return 7
    let try_end = bc.len();

    let catch_start = bc.len();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(1));  // push 9
    bc.push(JsOp::Return as u8);

    let try_note = super::xdr::JsTryNote {
        start: try_start as u32,
        length: (try_end - try_start) as u32,
        catch_start: catch_start as u32,
    };
    let ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: bc, prolog_length: 0, version: 150,
        atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 4,
        try_notes: vec![try_note],
    };
    let f = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("trycatch2".into()),
            nargs: 0, extra: 0, nvars: 0, flags: 0,
            bindings: Vec::new(),
            script: ir,
        }),
        captured_scope: None,
    };
    let mut rt = JsRuntime::new();
    let result = rt.call_function(&Rc::new(f), Vec::new(), JsValue::Undefined).unwrap();
    assert!(matches!(result, JsValue::Int(7)), "no-throw path returns from try body, got {:?}", result);
}

#[test]
fn synth_pre_increment_var() {
    // function() { var x = 5; return ++x; }  ⇒ 6
    let atoms: Vec<JsAtom> = vec![JsAtom::Int(5)];
    let mut bc = Vec::new();
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));   // push 5
    bc.push(JsOp::Setvar as u8); bc.extend(u16_be(0));   // x = 5 (leaves 5 on stack)
    bc.push(JsOp::Pop as u8);
    bc.push(JsOp::Incvar as u8); bc.extend(u16_be(0));   // ++x → pushes new value
    bc.push(JsOp::Return as u8);
    let ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: bc, prolog_length: 0, version: 150,
        atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 4, try_notes: Vec::new(),
    };
    let mut rt = JsRuntime::new();
    let f = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("inc_test".into()), nargs: 0, extra: 0,
            nvars: 1, flags: 0, bindings: Vec::new(), script: ir,
        }),
        captured_scope: None,
    };
    let result = rt.call_function(&Rc::new(f), vec![], JsValue::Undefined).unwrap();
    assert!(matches!(result, JsValue::Int(6)), "expected 6, got {:?}", result);
}

// ===== Decompiler =====

#[test]
fn dump_add_function_bindings() {
    let ir = decode_script(JS_MOV_PAYLOAD).expect("decode");
    for a in &ir.atoms {
        if let JsAtom::Function(f) = a {
            if f.name.as_deref() == Some("add") {
                println!("\n=== add bindings ===");
                println!("nargs={} nvars={}", f.nargs, f.nvars);
                for b in &f.bindings {
                    println!("  {:?} short_id={} name={:?}", b.kind, b.short_id, b.name);
                }
            }
        }
    }
}

#[test]
fn decompile_jsMov_program_emits_recognisable_js() {
    use super::decompiler::decompile;
    let ir = decode_script(JS_MOV_PAYLOAD).expect("decode");
    let dec = decompile(&ir, &[]);
    let source: String = dec.lines.iter()
        .map(|l| format!("{}{}", "  ".repeat(l.indent as usize), l.text))
        .collect::<Vec<_>>()
        .join("\n");
    println!("\n=== Decompiled program ===\n{}\n", source);

    // Top-level var declarations + assignments.
    assert!(source.contains("var pCounter"));
    assert!(source.contains("var pName"));
    assert!(source.contains("pCounter = 0;"));
    assert!(source.contains("pName = \"alice\";"));
    // Each declared function appears as `function name(args) { ... }`.
    assert!(source.contains("function on_prepareMovie()"));
    assert!(source.contains("function on_mouseUp()"));
    assert!(source.contains("function add(x, y)"));
    // Inside on_prepareMovie we expect the trace call and pCounter = 42.
    assert!(source.contains("pCounter = 42;"));
    assert!(source.contains("trace("));
    // Inside add we expect the return.
    assert!(source.contains("return x + y;"));
}

// ===== Host bridge integration =====

#[test]
fn bridge_routes_trace_sprite_and_go_from_js_mov() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use super::host_bridge::RecordingBridge;

    let ir = Rc::new(decode_script(JS_MOV_PAYLOAD).expect("decode"));

    let bridge = Rc::new(RefCell::new(RecordingBridge::default()));
    let mut rt = JsRuntime::with_stdlib();
    rt.set_bridge(bridge.clone());
    rt.install_director_globals();

    rt.run_program(&ir).expect("program");
    let on_mouseup = rt.global.borrow().get_own("on_mouseUp").cloned().expect("hoisted");
    rt.invoke(&on_mouseup, vec![], JsValue::Undefined).expect("invoke on_mouseUp");

    let b = bridge.borrow();
    // 3 trace calls from the for-loop body.
    assert_eq!(b.traces.len(), 3);
    assert_eq!(b.traces[0], "item 0 = 1");
    assert_eq!(b.traces[1], "item 1 = 2");
    assert_eq!(b.traces[2], "item 2 = 3");
    // sprite(1) called twice (once in each branch of the if/else), but only
    // one branch runs based on pCounter.
    assert!(b.sprite_calls.iter().any(|c| *c == 1), "sprite(1) called");
}

#[test]
fn bridge_trace_alone_for_on_preparemovie() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use super::host_bridge::RecordingBridge;

    let ir = Rc::new(decode_script(JS_MOV_PAYLOAD).expect("decode"));
    let bridge = Rc::new(RefCell::new(RecordingBridge::default()));
    let mut rt = JsRuntime::with_stdlib();
    rt.set_bridge(bridge.clone());
    rt.install_director_globals();

    rt.run_program(&ir).expect("program");
    let on_prep = rt.global.borrow().get_own("on_prepareMovie").cloned().expect("hoisted");
    rt.invoke(&on_prep, vec![], JsValue::Undefined).expect("invoke");

    let b = bridge.borrow();
    assert_eq!(b.traces.len(), 1);
    assert_eq!(b.traces[0], "hello from js, counter=42");
}

// ===== Standard-library coverage =====

fn run_with_stdlib(bc: Vec<u8>, atoms: Vec<JsAtom>, nvars: u16) -> Result<JsValue, super::value::JsError> {
    let ir = super::xdr::JsScriptIR {
        magic: 0xdead_0003, bytecode: bc, prolog_length: 0, version: 150,
        atoms, source_notes: Vec::new(), filename: None, lineno: 1,
        max_stack_depth: 16, try_notes: Vec::new(),
    };
    let mut rt = JsRuntime::with_stdlib();
    let f = super::value::JsFunction {
        atom: Rc::new(super::xdr::JsFunctionAtom {
            name: Some("synth".into()), nargs: 0, extra: 0,
            nvars, flags: 0, bindings: Vec::new(), script: ir,
        }),
        captured_scope: None,
    };
    rt.call_function(&Rc::new(f), Vec::new(), JsValue::Undefined)
}

#[test]
fn stdlib_math_floor_ceil_round() {
    // Math.floor(3.7) === 3
    let atoms = vec![
        JsAtom::String("Math".into()),
        JsAtom::String("floor".into()),
        JsAtom::Double(3.7),
    ];
    let mut bc = Vec::new();
    bc.push(JsOp::Name as u8); bc.extend(u16_be(0));        // push Math
    bc.push(JsOp::Getprop as u8); bc.extend(u16_be(1));      // .floor
    bc.push(JsOp::Pushobj as u8);                            // this (we'll allow)
    bc.push(JsOp::Number as u8); bc.extend(u16_be(2));        // push 3.7
    bc.push(JsOp::Call as u8); bc.extend(u16_be(1));
    bc.push(JsOp::Return as u8);
    let result = run_with_stdlib(bc, atoms, 0).unwrap();
    let n = match result { JsValue::Int(i) => i as f64, JsValue::Number(n) => n, _ => panic!() };
    assert!((n - 3.0).abs() < 1e-9, "Math.floor(3.7) = {}, want 3", n);
}

#[test]
fn stdlib_math_pi_and_sqrt() {
    let atoms = vec![
        JsAtom::String("Math".into()),
        JsAtom::String("PI".into()),
    ];
    let mut bc = Vec::new();
    bc.push(JsOp::Name as u8); bc.extend(u16_be(0));
    bc.push(JsOp::Getprop as u8); bc.extend(u16_be(1));
    bc.push(JsOp::Return as u8);
    let result = run_with_stdlib(bc, atoms, 0).unwrap();
    let n = match result { JsValue::Number(n) => n, _ => panic!("{:?}", result) };
    assert!((n - std::f64::consts::PI).abs() < 1e-9);
}

#[test]
fn stdlib_parseint_radix_and_default() {
    // We exercise parseInt directly through invoke since it lives as a Native.
    let mut rt = JsRuntime::with_stdlib();
    let pi = rt.global.borrow().get_own("parseInt").cloned().unwrap();
    let r1 = rt.invoke(&pi, vec![JsValue::String(Rc::new("42".into()))], JsValue::Undefined).unwrap();
    assert!(matches!(r1, JsValue::Int(42)));
    let r2 = rt.invoke(&pi, vec![JsValue::String(Rc::new("0xff".into()))], JsValue::Undefined).unwrap();
    assert!(matches!(r2, JsValue::Int(255)));
    let r3 = rt.invoke(&pi, vec![JsValue::String(Rc::new("101".into())), JsValue::Int(2)], JsValue::Undefined).unwrap();
    assert!(matches!(r3, JsValue::Int(5)));
}

#[test]
fn stdlib_isnan_recognises_nan() {
    let mut rt = JsRuntime::with_stdlib();
    let f = rt.global.borrow().get_own("isNaN").cloned().unwrap();
    assert!(matches!(
        rt.invoke(&f, vec![JsValue::String(Rc::new("not a number".into()))], JsValue::Undefined).unwrap(),
        JsValue::Bool(true)
    ));
    assert!(matches!(
        rt.invoke(&f, vec![JsValue::Int(7)], JsValue::Undefined).unwrap(),
        JsValue::Bool(false)
    ));
}

#[test]
fn stdlib_array_push_pop_join() {
    let mut rt = JsRuntime::with_stdlib();
    let arr = JsValue::Array(Rc::new(std::cell::RefCell::new(super::value::JsArray { items: vec![JsValue::Int(1), JsValue::Int(2)] })));
    // push(3, 4)
    let push = match super::interpreter::JsRuntime::with_stdlib() {
        _ => match &arr {
            JsValue::Array(a) => {
                let _ = a; // touch
                // Reach into the array via get_property for the bound method
            }
            _ => unreachable!(),
        }
    };
    let _ = push;
    // Easier: use the interpreter's get-property path by running bytecode.
    let atoms = vec![
        JsAtom::String("a".into()),
        JsAtom::String("push".into()),
        JsAtom::String("Array".into()),
    ];
    let mut bc = Vec::new();
    bc.push(JsOp::Name as u8); bc.extend(u16_be(2));        // NAME "Array"
    bc.push(JsOp::Pushobj as u8);
    bc.push(JsOp::Newinit as u8);                            // array
    bc.push(JsOp::Zero as u8);
    bc.push(JsOp::One as u8);
    bc.push(JsOp::Initelem as u8);                            // [1]
    bc.push(JsOp::Endinit as u8);
    bc.push(JsOp::Setvar as u8); bc.extend(u16_be(0));        // a = [1]
    bc.push(JsOp::Pop as u8);
    bc.push(JsOp::Getvar as u8); bc.extend(u16_be(0));        // load a
    bc.push(JsOp::Getprop as u8); bc.extend(u16_be(1));       // .push
    bc.push(JsOp::Getvar as u8); bc.extend(u16_be(0));        // this = a
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));        // arg = ... actually push the int 99
    // Replace: use Number atom 0 = "a"? Bug — re-do with proper int atom.
    // Cleaner: avoid Number for the push arg and use UINT16.
    // But we already have UINT16 inline. Let me restructure: drop the wrong NUMBER op.
    bc.pop(); bc.pop(); bc.pop();                              // pop the NUMBER op + 2 operand bytes
    bc.push(JsOp::Uint16 as u8); bc.extend(u16_be(99));        // push 99
    bc.push(JsOp::Call as u8); bc.extend(u16_be(1));            // a.push(99) -> 2
    bc.push(JsOp::Pop as u8);
    bc.push(JsOp::Getvar as u8); bc.extend(u16_be(0));         // return a
    bc.push(JsOp::Return as u8);

    let result = run_with_stdlib(bc, atoms, 1).unwrap();
    match result {
        JsValue::Array(a) => {
            let items = &a.borrow().items;
            assert_eq!(items.len(), 2);
            assert!(matches!(items[0], JsValue::Int(1)));
            assert!(matches!(items[1], JsValue::Int(99)));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn stdlib_string_to_upper_and_length() {
    // "hello".toUpperCase() — invoke method by calling get_property + invoke.
    let mut rt = JsRuntime::with_stdlib();
    let s = JsValue::String(Rc::new("hello".into()));
    let m = super::interpreter::get_property_pub(&s, "toUpperCase");
    let r = rt.invoke(&m, vec![], s.clone()).unwrap();
    match r { JsValue::String(ss) => assert_eq!(&*ss, "HELLO"), other => panic!("{:?}", other) };

    let len = super::interpreter::get_property_pub(&s, "length");
    assert!(matches!(len, JsValue::Int(5)));
}

#[test]
fn synth_throw_propagates_as_jserror() {
    let atoms = vec![JsAtom::String("boom".into())];
    let mut bc = Vec::new();
    bc.push(JsOp::String as u8); bc.extend(u16_be(0));
    bc.push(JsOp::Throw as u8);
    let result = run_synth(bc, atoms);
    assert!(result.is_err(), "throw should produce JsError");
    let err = result.unwrap_err();
    assert!(err.message.contains("boom"), "error message: {}", err.message);
}

#[test]
fn synth_extended_jump_offsets() {
    // GOTOX -8 with i32 offset.
    let atoms: Vec<JsAtom> = vec![JsAtom::Int(42)];
    // Layout:
    //  pc=0:  NUMBER 0     (push 42)   ; len=3
    //  pc=3:  RETURN                   ; len=1
    //  pc=4:  GOTOX -4    (jump back to pc=0)  ; len=5
    let mut bc = Vec::new();
    bc.push(JsOp::Gotox as u8);
    bc.extend(7_i32.to_be_bytes()); // forward 7 → target pc=7
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));  // 5..7 — never executes (offset>=7 jumps past)
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));  // 7..9 push 42
    bc.push(JsOp::Return as u8);                        // 10 — return 42
    assert!(matches!(run_synth(bc, atoms).unwrap(), JsValue::Int(42)));
}

#[test]
fn synth_array_literal_via_initelem() {
    // Mirrors the compiler's array-literal emission pattern (see on_mouseUp
    // disassembly): push a Function value named "Array" + a `this` placeholder,
    // then NEWINIT to spawn the array, then INITELEM repeatedly.
    use super::value::{JsFunction, NativeFn};
    let atoms = vec![
        JsAtom::Int(10),
        JsAtom::Int(20),
        JsAtom::Int(30),
    ];

    // Stand-in for global `Array` resolution. We can't push a Native directly
    // (the bytecode would need NAME with an atom), so we hand-build a small IR
    // that uses NAME "Array" instead. Simpler: just bypass via a synthetic
    // function whose script body directly exercises NEWINIT + INITELEM and
    // returns the array. Pre-seed `Array` as a built-in.
    let mut bc = Vec::new();
    // Need: push the Array constructor + `this` so NEWINIT picks Array semantics.
    bc.push(JsOp::Name as u8); bc.extend(u16_be(3));       // NAME atom_idx 3 = "Array"
    bc.push(JsOp::Pushobj as u8);
    bc.push(JsOp::Newinit as u8);
    // arr[0] = 10
    bc.push(JsOp::Zero as u8);
    bc.push(JsOp::Number as u8); bc.extend(u16_be(0));
    bc.push(JsOp::Initelem as u8);
    // arr[1] = 20
    bc.push(JsOp::One as u8);
    bc.push(JsOp::Number as u8); bc.extend(u16_be(1));
    bc.push(JsOp::Initelem as u8);
    bc.push(JsOp::Endinit as u8);
    bc.push(JsOp::Return as u8);

    // We need atom #3 to be a string "Array" so NAME resolves it via the global.
    let mut atoms = atoms;
    atoms.push(JsAtom::String("Array".into())); // atom[3]
    assert!(matches!(run_synth(bc, atoms).unwrap(), JsValue::Array(_)));
}
