use vm_rust::player::eval::{parse_lingo_expr_ast_runtime, LingoExpr, Rule};

#[test]
fn test_list_index_literal() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[1, 2, 3][1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListAccess(
            Box::new(LingoExpr::ListLiteral(vec![
                LingoExpr::IntLiteral(1),
                LingoExpr::IntLiteral(2),
                LingoExpr::IntLiteral(3),
            ])),
            Box::new(LingoExpr::IntLiteral(1))
        )
    );
}

#[test]
fn test_list_index_identifier() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "myList[2]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListAccess(
            Box::new(LingoExpr::Identifier("myList".to_string())),
            Box::new(LingoExpr::IntLiteral(2))
        )
    );
}

#[test]
fn test_list_index_with_property_access_before() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "obj.list[1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListAccess(
            Box::new(LingoExpr::ObjProp(
                Box::new(LingoExpr::Identifier("obj".to_string())),
                "list".to_string()
            )),
            Box::new(LingoExpr::IntLiteral(1))
        )
    );
}

#[test]
fn test_list_index_with_property_access_after() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "myList[1].prop".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjProp(
            Box::new(LingoExpr::ListAccess(
                Box::new(LingoExpr::Identifier("myList".to_string())),
                Box::new(LingoExpr::IntLiteral(1))
            )),
            "prop".to_string()
        )
    );
}

#[test]
fn test_sprite_scriptinstancelist_index_property() {
    // The original problem case: sprite(39).scriptInstanceList[2].plabel
    let result = parse_lingo_expr_ast_runtime(
        Rule::eval_expr,
        "sprite(39).scriptInstanceList[2].plabel".to_string(),
    );
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ObjProp(
            Box::new(LingoExpr::ListAccess(
                Box::new(LingoExpr::ObjProp(
                    Box::new(LingoExpr::HandlerCall("sprite".to_string(), vec![
                        LingoExpr::IntLiteral(39)
                    ])),
                    "scriptInstanceList".to_string()
                )),
                Box::new(LingoExpr::IntLiteral(2))
            )),
            "plabel".to_string()
        )
    );
}

#[test]
fn test_list_index_with_expression() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "myList[1 + 1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListAccess(
            Box::new(LingoExpr::Identifier("myList".to_string())),
            Box::new(LingoExpr::Add(
                Box::new(LingoExpr::IntLiteral(1)),
                Box::new(LingoExpr::IntLiteral(1))
            ))
        )
    );
}

#[test]
fn test_nested_list_index() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "matrix[1][2]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListAccess(
            Box::new(LingoExpr::ListAccess(
                Box::new(LingoExpr::Identifier("matrix".to_string())),
                Box::new(LingoExpr::IntLiteral(1))
            )),
            Box::new(LingoExpr::IntLiteral(2))
        )
    );
}

#[test]
fn test_list_index_with_handler_call() {
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "getList()[1]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListAccess(
            Box::new(LingoExpr::HandlerCall("getList".to_string(), vec![])),
            Box::new(LingoExpr::IntLiteral(1))
        )
    );
}

#[test]
fn test_list_literal() {
    // Make sure list literals still parse correctly
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[1, 2, 3]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::ListLiteral(vec![
            LingoExpr::IntLiteral(1),
            LingoExpr::IntLiteral(2),
            LingoExpr::IntLiteral(3),
        ])
    );
}

#[test]
fn test_empty_list() {
    // Make sure empty lists still parse correctly
    let result = parse_lingo_expr_ast_runtime(Rule::eval_expr, "[]".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::ListLiteral(vec![]));
}
