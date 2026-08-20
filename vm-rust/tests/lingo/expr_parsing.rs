use vm_rust::player::eval::{parse_lingo_expr_ast_runtime, LingoExpr, Rule};
use vm_rust::player::lingo_compiler::compile_lingo;
use vm_rust::player::export::decompile_script;
use vm_rust::director::file::{get_variable_multiplier, read_director_file_bytes};
use vm_rust::player::{bitmap::manager::BitmapManager, cast_member::{CastMember, CastMemberType}, cast_lib::{CastLib, CastLibState}};
use fxhash::FxHashMap;
use std::path::Path;

#[test]
fn debug_delete_chunk_pattern() {
    // Test that pStr.char[1..end].delete() compiles to pushchunkvarref + getPropRef + delete
    let source = "on test me, pStr, pPattern\n  delete pStr.char[1..length(pPattern)]\nend test";
    let result = compile_lingo(source, 1).expect("compile failed");
    for handler in &result.chunk.handlers {
        println!("Handler bytecodes:");
        for (i, bc) in handler.bytecode_array.iter().enumerate() {
            println!("  {i:03}: {:?} {}", bc.opcode, bc.obj);
        }
    }
    // Verify the bytecode contains pushchunkvarref
    let any_chunk_varref = result.chunk.handlers.iter().any(|h| {
        h.bytecode_array.iter().any(|bc| format!("{:?}", bc.opcode).contains("PushChunkVarRef"))
    });
    assert!(any_chunk_varref, "expected PushChunkVarRef in bytecode");
}

#[test]
#[ignore]
fn debug_replace_chunks_source() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().expect("workspace root");
    let cast_path = workspace_root.join("public/dcr_woodpecker/fuse_client.cct");
    let bytes = std::fs::read(&cast_path).unwrap();
    let base_path = format!("file://{}", cast_path.parent().unwrap().to_string_lossy());
    let dir = read_director_file_bytes(&bytes, "fuse_client.cct", &base_path).unwrap();
    let cast_def = dir.casts.first().unwrap();
    let mut cast = CastLib {
        name: "fuse_client".to_string(),
        file_name: cast_path.to_string_lossy().to_string(),
        number: 1, is_external: false, state: CastLibState::Loaded,
        lctx: cast_def.lctx.clone(), members: FxHashMap::default(), scripts: FxHashMap::default(),
        preload_mode: 0, capital_x: cast_def.capital_x, dir_version: cast_def.dir_version,
        palette_id_offset: cast_def.palette_id_offset,
    };
    let mut bitmap_manager = BitmapManager::new();
    for (member_number, member_def) in &cast_def.members {
        let member = CastMember::from(cast.number, *member_number, member_def, &cast_def.lctx, &mut bitmap_manager, cast.dir_version, cast.palette_id_offset, &dir.font_table);
        if matches!(member.member_type, CastMemberType::Script(_)) {
            cast.insert_member(*member_number, member);
        }
    }

    for (_, script) in &cast.scripts {
        if script.name.contains("RC4") {
            let source = decompile_script(script, &cast);
            // Find delete statements
            for line in source.lines() {
                if line.trim().starts_with("delete ") {
                    println!("DELETE: {}", line.trim());
                }
            }
        }
    }
}

#[test]
fn test_symbol() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "#symbol".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::SymbolLiteral("symbol".to_string()));
}

#[test]
fn test_string() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "\"string\"".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::StringLiteral("string".to_string()));
}

#[test]
fn test_int() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "42".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::IntLiteral(42));
}

#[test]
fn test_neg_int() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "-42".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::Negate(Box::new(LingoExpr::IntLiteral(42))));
}

#[test]
fn test_float() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "42.5".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::FloatLiteral(42.5));
}

#[test]
fn test_float_ending_with_dot() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "42.".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::FloatLiteral(42.0));
}

#[test]
fn test_neg_float() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "-42.5".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::Negate(Box::new(LingoExpr::FloatLiteral(42.5))));
}

#[test]
fn test_list_empty() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::ListLiteral(vec![]));
}

#[test]
fn test_list_single() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::ListLiteral(vec![LingoExpr::IntLiteral(1)]));
}

#[test]
fn test_list_multi() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[1, 2, 3]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListLiteral(vec![
            LingoExpr::IntLiteral(1),
            LingoExpr::IntLiteral(2),
            LingoExpr::IntLiteral(3)
        ])
    );
}

#[test]
fn test_proplist_empty() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[:]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::PropListLiteral(vec![]));
}

#[test]
fn test_proplist_single() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[#key1: 1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PropListLiteral(vec![(
            LingoExpr::SymbolLiteral("key1".to_string()),
            LingoExpr::IntLiteral(1)
        )])
    );
}

#[test]
fn test_proplist_multi() {
    let result = parse_lingo_expr_ast_runtime(
        Rule::eval_expr,
        "[#key1: 1, #key2: 2, #key3: 3]".to_string(),
    );
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PropListLiteral(vec![
            (
                LingoExpr::SymbolLiteral("key1".to_string()),
                LingoExpr::IntLiteral(1)
            ),
            (
                LingoExpr::SymbolLiteral("key2".to_string()),
                LingoExpr::IntLiteral(2)
            ),
            (
                LingoExpr::SymbolLiteral("key3".to_string()),
                LingoExpr::IntLiteral(3)
            )
        ])
    );
}

#[test]
fn test_void() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "void".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::VoidLiteral);
}

#[test]
fn test_bool() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "true".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::BoolLiteral(true));

    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "false".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::BoolLiteral(false));
}

#[test]
fn test_handler_call_no_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "handler_call()".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall("handler_call".to_string(), vec![])
    );
}

#[test]
fn test_handler_call_single_arg() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "handler_call(1)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall("handler_call".to_string(), vec![LingoExpr::IntLiteral(1),])
    );
}

#[test]
fn test_handler_call_multi_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "handler_call(1, 2, 3)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "handler_call".to_string(),
            vec![
                LingoExpr::IntLiteral(1),
                LingoExpr::IntLiteral(2),
                LingoExpr::IntLiteral(3)
            ]
        )
    );
}

#[test]
fn test_obj_prop() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "obj.prop".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjProp(
            Box::new(LingoExpr::Identifier("obj".to_string())),
            "prop".to_string()
        )
    );
}

#[test]
fn test_deep_obj_prop() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "obj.prop.subprop".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjProp(
            Box::new(LingoExpr::ObjProp(
                Box::new(LingoExpr::Identifier("obj".to_string())),
                "prop".to_string()
            )),
            "subprop".to_string()
        )
    );
}

#[test]
fn test_obj_handler_call_no_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "obj.handler()".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjHandlerCall(
            Box::new(LingoExpr::Identifier("obj".to_string())),
            "handler".to_string(),
            vec![]
        )
    );
}

#[test]
fn test_obj_handler_call_single_arg() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "obj.handler(1)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjHandlerCall(
            Box::new(LingoExpr::Identifier("obj".to_string())),
            "handler".to_string(),
            vec![LingoExpr::IntLiteral(1)]
        )
    );
}

#[test]
fn test_obj_handler_call_multi_arg() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "obj.handler(1, 2, 3)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjHandlerCall(
            Box::new(LingoExpr::Identifier("obj".to_string())),
            "handler".to_string(),
            vec![
                LingoExpr::IntLiteral(1),
                LingoExpr::IntLiteral(2),
                LingoExpr::IntLiteral(3)
            ]
        )
    );
}

#[test]
fn test_deep_obj_handler_call_no_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "obj.prop.handler()".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjHandlerCall(
            Box::new(LingoExpr::ObjProp(
                Box::new(LingoExpr::Identifier("obj".to_string())),
                "prop".to_string()
            )),
            "handler".to_string(),
            vec![]
        )
    );
}

#[test]
fn test_obj_reserved_method_call_in_command_context() {
    let result = parse_lingo_expr_ast_runtime(
        Rule::command_eval_expr,
        "tSession.set(\"client_startdate\", the date)".to_string(),
    );
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjHandlerCall(
            Box::new(LingoExpr::Identifier("tSession".to_string())),
            "set".to_string(),
            vec![
                LingoExpr::StringLiteral("client_startdate".to_string()),
                LingoExpr::Identifier("the date".to_string()),
            ]
        )
    );
}

#[test]
fn test_rgb_accepts_expression_argument() {
    let result = parse_lingo_expr_ast_runtime(
        Rule::eval_expr,
        "rgb(tFontDesc[#color])".to_string(),
    );
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "rgb".to_string(),
            vec![LingoExpr::ListAccess(
                Box::new(LingoExpr::Identifier("tFontDesc".to_string())),
                Box::new(LingoExpr::SymbolLiteral("color".to_string())),
            )]
        )
    );
}

#[test]
fn test_escaped_quote_string_argument_parses() {
    let result = parse_lingo_expr_ast_runtime(
        Rule::command_eval_expr,
        "replaceChunks(tCastsStr, \"\\\"\", \"\")".to_string(),
    );
    assert!(result.is_ok());
}

#[test]
fn test_comparison_with_dotted_handler_arg_parses() {
    let result = parse_lingo_expr_ast_runtime(
        Rule::eval_expr,
        "value(_player.productVersion) >= 11".to_string(),
    );
    assert!(result.is_ok());
}
