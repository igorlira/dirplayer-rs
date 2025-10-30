use vm_rust::player::{eval::{parse_lingo_expr_ast_runtime, LingoExpr, Rule}};

#[test]
fn test_global_handler_no_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put()".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "put".to_string(),
            vec![]
        )
    );
}

#[test]
fn test_global_handler_one_arg() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put(1)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "put".to_string(),
            vec![LingoExpr::IntLiteral(1)]
        )
    );
}

#[test]
fn test_global_handler_multi_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put(1, 2, 3)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "put".to_string(),
            vec![LingoExpr::IntLiteral(1), LingoExpr::IntLiteral(2), LingoExpr::IntLiteral(3)]
        )
    );
}

#[test]
fn test_command_no_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "put".to_string(),
            vec![]
        )
    );
}

#[test]
fn test_command_one_arg() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 1".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "put".to_string(),
            vec![LingoExpr::IntLiteral(1)]
        )
    );
}

#[test]
fn test_command_multi_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 1, 2, 3".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "put".to_string(),
            vec![LingoExpr::IntLiteral(1), LingoExpr::IntLiteral(2), LingoExpr::IntLiteral(3)]
        )
    );
}

#[test]
fn test_command_multi_args_inline() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 1 2 3".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::HandlerCall(
            "put".to_string(),
            vec![LingoExpr::IntLiteral(1), LingoExpr::IntLiteral(2), LingoExpr::IntLiteral(3)]
        )
    );
}

#[test]
fn test_command_multi_args_mixed() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 1 2, 3".to_string());
    assert!(result.is_err());
}


#[test]
fn test_top_level_assignment() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "obj = 1".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::Assignment(
            Box::new(LingoExpr::Identifier("obj".to_string())),
            Box::new(LingoExpr::IntLiteral(1))
        )
    );
}

#[test]
fn test_deep_assignment() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "obj.prop = 1".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::Assignment(
            Box::new(
                LingoExpr::ObjProp(
                    Box::new(LingoExpr::Identifier("obj".to_string())),
                    "prop".to_string()
                )
            ),
            Box::new(LingoExpr::IntLiteral(1))
        )
    );
}

#[test]
fn test_obj_handler_call_no_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "obj.handler()".to_string());
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
