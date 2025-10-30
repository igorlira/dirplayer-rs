use vm_rust::player::{eval::{parse_lingo_expr_ast_runtime, LingoExpr, Rule}};

#[test]
fn test_symbol() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "#symbol".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::SymbolLiteral("symbol".to_string())
    );
}

#[test]
fn test_string() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "\"string\"".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::StringLiteral("string".to_string())
    );
}

#[test]
fn test_int() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "42".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::IntLiteral(42)
    );
}

#[test]
fn test_neg_int() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "-42".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::IntLiteral(-42)
    );
}

#[test]
fn test_float() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "42.5".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::FloatLiteral(42.5)
    );
}

#[test]
fn test_float_ending_with_dot() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "42.".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::FloatLiteral(42.0)
    );
}

#[test]
fn test_neg_float() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "-42.5".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::FloatLiteral(-42.5)
    );
}

#[test]
fn test_list_empty() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListLiteral(vec![])
    );
}

#[test]
fn test_list_single() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListLiteral(vec![LingoExpr::IntLiteral(1)])
    );
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
    assert_eq!(
        ast,
        LingoExpr::PropListLiteral(vec![])
    );
}

#[test]
fn test_proplist_single() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[#key1: 1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PropListLiteral(vec![
            (LingoExpr::SymbolLiteral("key1".to_string()), LingoExpr::IntLiteral(1))
        ])
    );
}

#[test]
fn test_proplist_multi() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[#key1: 1, #key2: 2, #key3: 3]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PropListLiteral(vec![
            (LingoExpr::SymbolLiteral("key1".to_string()), LingoExpr::IntLiteral(1)),
            (LingoExpr::SymbolLiteral("key2".to_string()), LingoExpr::IntLiteral(2)),
            (LingoExpr::SymbolLiteral("key3".to_string()), LingoExpr::IntLiteral(3))
        ])
    );
}

#[test]
fn test_void() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "void".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::VoidLiteral
    );
}

#[test]
fn test_bool() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "true".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::BoolLiteral(true)
    );

    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "false".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::BoolLiteral(false)
    );
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
        LingoExpr::HandlerCall("handler_call".to_string(), vec![
            LingoExpr::IntLiteral(1),
        ])
    );
}

#[test]
fn test_handler_call_multi_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "handler_call(1, 2, 3)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall("handler_call".to_string(), vec![
            LingoExpr::IntLiteral(1),
            LingoExpr::IntLiteral(2),
            LingoExpr::IntLiteral(3)
        ])
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
