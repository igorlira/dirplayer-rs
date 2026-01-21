use vm_rust::player::eval::{parse_lingo_expr_ast_runtime, LingoExpr, Rule};

#[test]
fn test_global_handler_no_args() {
    // put() without args still displays (empty)
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put()".to_string());
    assert!(result.is_err());

    // let ast = result.unwrap();
    // // put() is parsed as put with empty list expression
    // assert_eq!(ast, LingoExpr::PutDisplay(Box::new(LingoExpr::ListLiteral(vec![]))));
}

#[test]
fn test_global_handler_one_arg() {
    // put(1) displays the value 1, it's not a function call
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put(1)".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::IntLiteral(1)))
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
            vec![
                LingoExpr::IntLiteral(1),
                LingoExpr::IntLiteral(2),
                LingoExpr::IntLiteral(3)
            ]
        )
    );
}

#[test]
fn test_command_no_args() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(ast, LingoExpr::HandlerCall("put".to_string(), vec![]));
}

#[test]
fn test_command_one_arg() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 1".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    // put 1 without parens is PutDisplay
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::IntLiteral(1)))
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
            vec![
                LingoExpr::IntLiteral(1),
                LingoExpr::IntLiteral(2),
                LingoExpr::IntLiteral(3)
            ]
        )
    );
}

#[test]
fn test_command_multi_args_inline() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 1 2 3".to_string());
    assert!(result.is_err());

    // let ast = result.unwrap();
    // assert_eq!(
    //     ast,
    //     LingoExpr::HandlerCall(
    //         "put".to_string(),
    //         vec![
    //             LingoExpr::IntLiteral(1),
    //             LingoExpr::IntLiteral(2),
    //             LingoExpr::IntLiteral(3)
    //         ]
    //     )
    // );
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
            Box::new(LingoExpr::ObjProp(
                Box::new(LingoExpr::Identifier("obj".to_string())),
                "prop".to_string()
            )),
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

// ==============================================================================
// PUT COMMAND TESTS - Display variations
// ==============================================================================

#[test]
fn test_put_display_string() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"hello world\"".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::StringLiteral("hello world".to_string())))
    );
}

#[test]
fn test_put_display_float() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 3.14".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::FloatLiteral(3.14)))
    );
}

#[test]
fn test_put_display_symbol() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put #mySymbol".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::SymbolLiteral("mySymbol".to_string())))
    );
}

#[test]
fn test_put_display_void() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put void".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::VoidLiteral))
    );
}

#[test]
fn test_put_display_list() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put [1, 2, 3]".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::ListLiteral(vec![
            LingoExpr::IntLiteral(1),
            LingoExpr::IntLiteral(2),
            LingoExpr::IntLiteral(3)
        ])))
    );
}

#[test]
fn test_put_display_proplist() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put [#a: 1, #b: 2]".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::PropListLiteral(vec![
            (LingoExpr::SymbolLiteral("a".to_string()), LingoExpr::IntLiteral(1)),
            (LingoExpr::SymbolLiteral("b".to_string()), LingoExpr::IntLiteral(2))
        ])))
    );
}

#[test]
fn test_put_display_identifier() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put x".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Identifier("x".to_string())))
    );
}

// ==============================================================================
// PUT COMMAND TESTS - Expressions in put display
// ==============================================================================

#[test]
fn test_put_display_addition() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 5 + 3".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Add(
            Box::new(LingoExpr::IntLiteral(5)),
            Box::new(LingoExpr::IntLiteral(3))
        )))
    );
}

#[test]
fn test_put_display_concatenation() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"value: \" & x".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Join(
            Box::new(LingoExpr::StringLiteral("value: ".to_string())),
            Box::new(LingoExpr::Identifier("x".to_string()))
        )))
    );
}

#[test]
fn test_put_display_comparison() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put x = 5".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Eq(
            Box::new(LingoExpr::Identifier("x".to_string())),
            Box::new(LingoExpr::IntLiteral(5))
        )))
    );
}

#[test]
fn test_put_display_and_operation() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put x and y".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::And(
            Box::new(LingoExpr::Identifier("x".to_string())),
            Box::new(LingoExpr::Identifier("y".to_string()))
        )))
    );
}

#[test]
fn test_put_display_or_operation() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put x or y".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Or(
            Box::new(LingoExpr::Identifier("x".to_string())),
            Box::new(LingoExpr::Identifier("y".to_string()))
        )))
    );
}

#[test]
fn test_put_display_not_operation() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put not x".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Not(
            Box::new(LingoExpr::Identifier("x".to_string()))
        )))
    );
}

#[test]
fn test_put_display_handler_call() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put ilk(x)".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::HandlerCall(
            "ilk".to_string(),
            vec![LingoExpr::Identifier("x".to_string())]
        )))
    );
}

// ==============================================================================
// PUT COMMAND TESTS - "the" properties
// ==============================================================================

#[test]
fn test_put_display_the_property() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put the itemDelimiter".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Identifier("the itemDelimiter".to_string())))
    );
}

#[test]
fn test_put_display_the_mouseLoc() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put the mouseLoc".to_string());
    assert!(result.is_ok());

    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutDisplay(Box::new(LingoExpr::Identifier("the mouseLoc".to_string())))
    );
}

#[test]
fn test_put_display_sprite_with_the_property() {
    // Test: put the rect of sprite the currentSpriteNum
    // This tests that "sprite the currentSpriteNum" can be parsed where
    // "the currentSpriteNum" is a property expression used as the sprite number
    let result = parse_lingo_expr_ast_runtime(
        Rule::command_eval_expr,
        "put the rect of sprite the currentSpriteNum".to_string()
    );
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
}

// ==============================================================================
// PUT COMMAND TESTS - put into (assignment)
// ==============================================================================

#[test]
fn test_put_into_basic() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 42 into x".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::IntLiteral(42)),
            Box::new(LingoExpr::Identifier("x".to_string()))
        )
    );
}

#[test]
fn test_put_into_string() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"hello\" into myString".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::StringLiteral("hello".to_string())),
            Box::new(LingoExpr::Identifier("myString".to_string()))
        )
    );
}

#[test]
fn test_put_into_list() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put [1, 2, 3] into myList".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::ListLiteral(vec![
                LingoExpr::IntLiteral(1),
                LingoExpr::IntLiteral(2),
                LingoExpr::IntLiteral(3)
            ])),
            Box::new(LingoExpr::Identifier("myList".to_string()))
        )
    );
}

#[test]
fn test_put_into_expression() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put 5 + 3 into result".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::Add(
                Box::new(LingoExpr::IntLiteral(5)),
                Box::new(LingoExpr::IntLiteral(3))
            )),
            Box::new(LingoExpr::Identifier("result".to_string()))
        )
    );
}

// ==============================================================================
// PUT COMMAND TESTS - put before
// ==============================================================================

#[test]
fn test_put_before_basic() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"hello \" before myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutBefore(
            Box::new(LingoExpr::StringLiteral("hello ".to_string())),
            Box::new(LingoExpr::Identifier("myStr".to_string()))
        )
    );
}

#[test]
fn test_put_before_expression() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put x & \" \" before myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutBefore(
            Box::new(LingoExpr::Join(
                Box::new(LingoExpr::Identifier("x".to_string())),
                Box::new(LingoExpr::StringLiteral(" ".to_string()))
            )),
            Box::new(LingoExpr::Identifier("myStr".to_string()))
        )
    );
}

// ==============================================================================
// PUT COMMAND TESTS - put after
// ==============================================================================

#[test]
fn test_put_after_basic() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \" world\" after myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutAfter(
            Box::new(LingoExpr::StringLiteral(" world".to_string())),
            Box::new(LingoExpr::Identifier("myStr".to_string()))
        )
    );
}

#[test]
fn test_put_after_expression() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \" \" & x after myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutAfter(
            Box::new(LingoExpr::Join(
                Box::new(LingoExpr::StringLiteral(" ".to_string())),
                Box::new(LingoExpr::Identifier("x".to_string()))
            )),
            Box::new(LingoExpr::Identifier("myStr".to_string()))
        )
    );
}

// ==============================================================================
// PUT COMMAND TESTS - put into/before/after chunks
// ==============================================================================

#[test]
fn test_put_into_char() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"X\" into char 1 of myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::StringLiteral("X".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "char".to_string(),
                Box::new(LingoExpr::IntLiteral(1)),
                Box::new(LingoExpr::Identifier("myStr".to_string()))
            ))
        )
    );
}

#[test]
fn test_put_into_word() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"goodbye\" into word 1 of myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::StringLiteral("goodbye".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "word".to_string(),
                Box::new(LingoExpr::IntLiteral(1)),
                Box::new(LingoExpr::Identifier("myStr".to_string()))
            ))
        )
    );
}

#[test]
fn test_put_into_line() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"newline\" into line 1 of myText".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::StringLiteral("newline".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "line".to_string(),
                Box::new(LingoExpr::IntLiteral(1)),
                Box::new(LingoExpr::Identifier("myText".to_string()))
            ))
        )
    );
}

#[test]
fn test_put_into_item() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"X\" into item 2 of myList".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutInto(
            Box::new(LingoExpr::StringLiteral("X".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "item".to_string(),
                Box::new(LingoExpr::IntLiteral(2)),
                Box::new(LingoExpr::Identifier("myList".to_string()))
            ))
        )
    );
}

#[test]
fn test_put_before_char() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"X\" before char 1 of myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutBefore(
            Box::new(LingoExpr::StringLiteral("X".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "char".to_string(),
                Box::new(LingoExpr::IntLiteral(1)),
                Box::new(LingoExpr::Identifier("myStr".to_string()))
            ))
        )
    );
}

#[test]
fn test_put_after_char() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"X\" after char 5 of myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutAfter(
            Box::new(LingoExpr::StringLiteral("X".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "char".to_string(),
                Box::new(LingoExpr::IntLiteral(5)),
                Box::new(LingoExpr::Identifier("myStr".to_string()))
            ))
        )
    );
}

#[test]
fn test_put_before_word() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"beautiful \" before word 2 of myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutBefore(
            Box::new(LingoExpr::StringLiteral("beautiful ".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "word".to_string(),
                Box::new(LingoExpr::IntLiteral(2)),
                Box::new(LingoExpr::Identifier("myStr".to_string()))
            ))
        )
    );
}

#[test]
fn test_put_after_word() {
    let result = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, "put \"!\" after word 2 of myStr".to_string());
    assert!(result.is_ok());
    
    let ast = result.unwrap();
    assert_eq!(
        ast,
        LingoExpr::PutAfter(
            Box::new(LingoExpr::StringLiteral("!".to_string())),
            Box::new(LingoExpr::ChunkExpr(
                "word".to_string(),
                Box::new(LingoExpr::IntLiteral(2)),
                Box::new(LingoExpr::Identifier("myStr".to_string()))
            ))
        )
    );
}