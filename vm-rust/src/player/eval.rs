use log::error;
use pest::{iterators::{Pair, Pairs}, pratt_parser::{Assoc, Op, PrattParser}, Parser};

use crate::{
    director::lingo::datum::{Datum, DatumType, datum_bool}, js_api::ascii_safe, player::{DirPlayer, bytecode::get_set::GetSetUtils, datum_operations::{add_datums, subtract_datums}, handlers::datum_handlers::player_call_datum_handler, player_call_global_handler, reserve_player_mut, script::{get_obj_prop, player_set_obj_prop}}
};

use super::{sprite::ColorRef, DatumRef, ScriptError};

#[derive(Parser)]
#[grammar = "lingo.pest"]
struct LingoParser;

fn tokenize_lingo(_expr: &String) -> Vec<String> {
    [].to_vec()
}

#[derive(Debug, PartialEq)]
pub enum LingoExpr {
    SymbolLiteral(String),
    StringLiteral(String),
    ListLiteral(Vec<LingoExpr>),
    VoidLiteral,
    BoolLiteral(bool),
    IntLiteral(i32),
    FloatLiteral(f32),
    PropListLiteral(Vec<(LingoExpr, LingoExpr)>),
    HandlerCall(String, Vec<LingoExpr>),
    ObjProp(Box<LingoExpr>, String),
    ObjHandlerCall(Box<LingoExpr>, String, Vec<LingoExpr>),
    ColorLiteral(ColorRef),
    RectLiteral((i32, i32, i32, i32)),
    PointLiteral((i32, i32)),
    Identifier(String),
    Assignment(Box<LingoExpr>, Box<LingoExpr>),
    Add(Box<LingoExpr>, Box<LingoExpr>),
    Subtract(Box<LingoExpr>, Box<LingoExpr>),
}

/// Evaluate a static Lingo expression. This does not support function calls.
pub fn eval_lingo_pair_static(pair: Pair<Rule>) -> Result<DatumRef, ScriptError> {
    let inner_rule = pair.as_rule();
    match pair.as_rule() {
        Rule::expr => eval_lingo_pair_static(pair.into_inner().next().unwrap()),
        Rule::list => eval_lingo_pair_static(pair.into_inner().next().unwrap()),
        Rule::multi_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let result = eval_lingo_pair_static(inner_pair)?;
                result_vec.push(result);
            }
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::List(DatumType::List, result_vec, false)))
            })
        }
        Rule::string => {
            let str_val = pair.into_inner().next().unwrap().as_str();
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::String(str_val.to_owned())))
            })
        }
        Rule::prop_list => eval_lingo_pair_static(pair.into_inner().next().unwrap()),
        Rule::multi_prop_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let mut pair_inner = inner_pair.into_inner();
                let key = eval_lingo_pair_static(pair_inner.next().unwrap())?;
                let value = eval_lingo_pair_static(pair_inner.next().unwrap())?;

                result_vec.push((key, value));
            }
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::PropList(result_vec, false)))
            })
        }
        Rule::empty_prop_list => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::PropList(vec![], false)))
        }),
        Rule::number_int => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::Int(pair.as_str().parse::<i32>().unwrap())))
        }),
        Rule::number_float => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::Float(pair.as_str().parse::<f32>().unwrap())))
        }),
        Rule::rect => {
            let mut inner = pair.into_inner();
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::IntRect((
                    inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                    inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                    inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                    inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                ))))
            })
        }
        Rule::rgb_num_color => {
            let mut inner = pair.into_inner();
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(
                    inner.next().unwrap().as_str().parse::<u8>().unwrap(),
                    inner.next().unwrap().as_str().parse::<u8>().unwrap(),
                    inner.next().unwrap().as_str().parse::<u8>().unwrap(),
                ))))
            })
        }
        Rule::rgb_str_color => {
            let mut inner = pair.into_inner();
            let str_inner = inner.next().unwrap().into_inner().next().unwrap();
            let str_val = str_inner.as_str();
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::ColorRef(ColorRef::from_hex(str_val))))
            })
        }
        Rule::rgb_color => {
            let inner = pair.into_inner().next().unwrap();
            eval_lingo_pair_static(inner)
        }
        Rule::symbol => {
            let str_val = pair.into_inner().next().unwrap().as_str();
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Symbol(str_val.to_owned())))
            })
        }
        Rule::bool_true => reserve_player_mut(|player| {
            Ok(player.alloc_datum(datum_bool(true)))
        }),
        Rule::bool_false => reserve_player_mut(|player| {
            Ok(player.alloc_datum(datum_bool(false)))
        }),
        Rule::void => Ok(DatumRef::Void),
        Rule::string_empty => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::String("".to_owned())))
        }),
        Rule::nohash_symbol => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::Symbol(pair.as_str().to_owned())))
        }),
        Rule::point => {
            let mut inner = pair.into_inner();
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::IntPoint((
                    inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                    inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                ))))
            })
        }
        Rule::empty_list => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::List(DatumType::List, vec![], false)))
        }),
        _ => Err(ScriptError::new(format!(
            "Invalid static Lingo expression {:?}",
            inner_rule
        ))),
    }
}

fn get_eval_top_level_prop(player: &mut DirPlayer, prop_name: &str) -> Result<DatumRef, ScriptError> {
    if let Some(global_ref) = player.globals.get(prop_name) {
        Ok(global_ref.clone())
    } else {
        let result = GetSetUtils::get_top_level_prop(player, prop_name)?;
        Ok(player.alloc_datum(result))
    }
}

fn parse_lingo_expr_runtime(pairs: Pairs<'_, Rule>, pratt: &PrattParser<Rule>) -> Result<LingoExpr, ScriptError> {
    pratt
        .map_primary(|pair| {
            parse_lingo_rule_runtime(pair, pratt)
        })
        .map_infix(|lhs, op, rhs| {
            match op.as_rule() {
                Rule::add => {
                    let left = lhs?;
                    let right = rhs?;
                    Ok(LingoExpr::Add(Box::new(left), Box::new(right)))
                },
                Rule::subtract => {
                    let left = lhs?;
                    let right = rhs?;
                    Ok(LingoExpr::Subtract(Box::new(left), Box::new(right)))
                }
                Rule::obj_prop => {
                    let obj_ref = lhs?;
                    let rhs = rhs?;
                    match rhs {
                        LingoExpr::Identifier(name) => {
                            let prop_name = name;
                            Ok(LingoExpr::ObjProp(Box::new(obj_ref), prop_name))
                        }
                        LingoExpr::HandlerCall(name, args) => {
                            Ok(LingoExpr::ObjHandlerCall(Box::new(obj_ref), name, args))
                        }
                        _ => Err(ScriptError::new(format!("Invalid object prop operator rhs {:?}", rhs))),
                    }
                },
                _ => Err(ScriptError::new(format!("Invalid infix operator {:?}", op.as_rule()))),
            }
        })
        .parse(pairs)
}

/// Evaluate a dynamic Lingo expression at runtime.
pub fn parse_lingo_rule_runtime(pair: Pair<'_, Rule>, pratt: &PrattParser<Rule>) -> Result<LingoExpr, ScriptError> {
    let inner_rule = pair.as_rule();
    match pair.as_rule() {
        Rule::expr => {
            let inner_pair = pair.into_inner();
            let ast = parse_lingo_expr_runtime(inner_pair, pratt)?;
            Ok(ast)
        }
        Rule::multi_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let result = parse_lingo_expr_runtime(inner_pair.into_inner(), pratt)?;
                result_vec.push(result);
            }
            Ok(LingoExpr::ListLiteral(result_vec))
        }
        Rule::string => {
            let str_val = pair.into_inner().next().unwrap().as_str();
            Ok(LingoExpr::StringLiteral(str_val.to_owned()))
        }
        Rule::multi_prop_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let mut pair_inner = inner_pair.into_inner();
                let key = parse_lingo_rule_runtime(pair_inner.next().unwrap(), pratt)?;
                let value = parse_lingo_expr_runtime(pair_inner.next().unwrap().into_inner(), pratt)?;

                result_vec.push((key, value));
            }
            Ok(LingoExpr::PropListLiteral(result_vec))
        }
        Rule::empty_prop_list => Ok(LingoExpr::PropListLiteral(vec![])),
        Rule::number_int => Ok(LingoExpr::IntLiteral(pair.as_str().parse::<i32>().unwrap())),
        Rule::number_float => Ok(LingoExpr::FloatLiteral(pair.as_str().parse::<f32>().unwrap())),
        Rule::rect => {
            let mut inner = pair.into_inner();
            Ok(LingoExpr::RectLiteral((
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
            )))
        }
        Rule::rgb_num_color => {
            let mut inner = pair.into_inner();
            Ok(LingoExpr::ColorLiteral(ColorRef::Rgb(
                inner.next().unwrap().as_str().parse::<u8>().unwrap(),
                inner.next().unwrap().as_str().parse::<u8>().unwrap(),
                inner.next().unwrap().as_str().parse::<u8>().unwrap(),
            )))
        }
        Rule::rgb_str_color => {
            let mut inner = pair.into_inner();
            let str_inner = inner.next().unwrap().into_inner().next().unwrap();
            let str_val = str_inner.as_str();
            Ok(LingoExpr::ColorLiteral(ColorRef::from_hex(str_val)))
        }
        Rule::rgb_color => {
            let inner = pair.into_inner().next().unwrap();
            parse_lingo_rule_runtime(inner, pratt)
        }
        Rule::symbol => {
            let str_val = pair.into_inner().next().unwrap().as_str();
            Ok(LingoExpr::SymbolLiteral(str_val.to_owned()))
        }
        Rule::bool_true => Ok(LingoExpr::BoolLiteral(true)),
        Rule::bool_false => Ok(LingoExpr::BoolLiteral(false)),
        Rule::void => Ok(LingoExpr::VoidLiteral),
        Rule::string_empty => Ok(LingoExpr::StringLiteral("".to_owned())),
        Rule::nohash_symbol => Ok(LingoExpr::SymbolLiteral(pair.as_str().to_owned())),
        Rule::point => {
            let mut inner = pair.into_inner();
            Ok(LingoExpr::PointLiteral((
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
            )))
        }
        Rule::empty_list => Ok(LingoExpr::ListLiteral(vec![])),
        // Rule::handler_call | Rule::command_inline => eval_lingo_call(pair.into_inner(), &pratt).await,
        Rule::handler_call | Rule::command_inline => {
            let mut inner = pair.into_inner();
            let handler_name = inner.next().unwrap().as_str();
            let mut args = vec![];
            if let Some(args_pair) = inner.next() {
                for arg in args_pair.into_inner() {
                    let arg_val = parse_lingo_expr_runtime(arg.into_inner(), pratt)?;
                    args.push(arg_val);
                }
            }

            Ok(LingoExpr::HandlerCall(handler_name.to_owned(), args))
        }
        Rule::ident => {
            let str_val = pair.as_str();
            Ok(LingoExpr::Identifier(str_val.to_owned()))
        }
        Rule::assignment => {
            let mut inner = pair.into_inner();
            let left_pair = inner.next().unwrap();
            let right_pair = inner.next().unwrap();

            let left_expr = match left_pair.as_rule() {
                Rule::ident => {
                    let ident_name = left_pair.as_str();
                    LingoExpr::Identifier(ident_name.to_owned())
                },
                _ => parse_lingo_rule_runtime(left_pair, pratt)?,
            };

            let right_expr = parse_lingo_expr_runtime(right_pair.into_inner(), pratt)?;

            Ok(LingoExpr::Assignment(Box::new(left_expr), Box::new(right_expr)))
        }
        _ => Err(ScriptError::new(format!(
            "Invalid runtime Lingo expression {:?}",
            inner_rule
        ))),
    }
}

pub async fn eval_lingo_expr_ast_runtime(expr: &LingoExpr) -> Result<DatumRef, ScriptError> {
    match expr {
        LingoExpr::SymbolLiteral(s) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Symbol(s.to_string())))
            })
        },
        LingoExpr::StringLiteral(s) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::String(s.to_string())))
            })
        },
        LingoExpr::ListLiteral(items) => {
            let mut datum_items = vec![];
            for item in items {
                let datum = Box::pin(eval_lingo_expr_ast_runtime(item)).await?;
                datum_items.push(datum);
            }
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::List(DatumType::List, datum_items, false)))
            })
        },
        LingoExpr::VoidLiteral => Ok(DatumRef::Void),
        LingoExpr::BoolLiteral(b) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Int(if *b { 1 } else { 0 })))
            })
        },
        LingoExpr::IntLiteral(i) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Int(*i)))
            })
        },
        LingoExpr::FloatLiteral(f) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Float(*f)))
            })
        },
        LingoExpr::PropListLiteral(pairs) => {
            let mut datum_pairs = vec![];
            for (key_expr, value_expr) in pairs {
                let key_datum = Box::pin(eval_lingo_expr_ast_runtime(key_expr)).await?;
                let value_datum = Box::pin(eval_lingo_expr_ast_runtime(value_expr)).await?;
                datum_pairs.push((key_datum, value_datum));
            }
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::PropList(datum_pairs, false)))
            })
        },
        LingoExpr::HandlerCall(handler_name, args) => {
            let mut datum_args = vec![];
            for arg in args {
                let datum = Box::pin(eval_lingo_expr_ast_runtime(arg)).await?;
                datum_args.push(datum);
            }
            player_call_global_handler(&handler_name, &datum_args).await
        },
        LingoExpr::ObjProp(obj_expr, prop_name) => {
            let obj_datum = Box::pin(eval_lingo_expr_ast_runtime(obj_expr.as_ref())).await?;
            reserve_player_mut(|player| {
                get_obj_prop(player, &obj_datum, prop_name)
            })
        },
        LingoExpr::ColorLiteral(color_ref) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::ColorRef(color_ref.clone())))
            })
        },
        LingoExpr::RectLiteral((x, y, w, h)) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::IntRect((*x, *y, *w, *h))))
            })
        },
        LingoExpr::PointLiteral((x, y)) => {
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::IntPoint((*x, *y))))
            })
        },
        LingoExpr::Identifier(ident_name) => {
            reserve_player_mut(|player| {
                get_eval_top_level_prop(player, ident_name)
            })
        }
        LingoExpr::ObjHandlerCall(obj_expr, handler_name, args) => {
            let obj_datum = Box::pin(eval_lingo_expr_ast_runtime(obj_expr.as_ref())).await?;
            let mut datum_args = vec![];
            for arg in args {
                let datum = Box::pin(eval_lingo_expr_ast_runtime(arg)).await?;
                datum_args.push(datum);
            }
            player_call_datum_handler(&obj_datum, handler_name, &datum_args).await
        }
        LingoExpr::Assignment(left_expr, right_expr) => {
            let right_datum = Box::pin(eval_lingo_expr_ast_runtime(right_expr.as_ref())).await?;

            match left_expr.as_ref() {
                LingoExpr::Identifier(ident_name) => {
                    reserve_player_mut(|player| {
                        player.globals.insert(ident_name.to_owned(), right_datum.clone());
                        Ok(right_datum)
                    })
                },
                LingoExpr::ObjProp(obj_expr, prop_name) => {
                    let obj_datum = Box::pin(eval_lingo_expr_ast_runtime(obj_expr.as_ref())).await?;
                    player_set_obj_prop( &obj_datum, prop_name, &right_datum).await?;
                    Ok(DatumRef::Void)
                },
                _ => Err(ScriptError::new("Invalid assignment left-hand side".to_string())),
            }
        }
        LingoExpr::Add(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs.as_ref())).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs.as_ref())).await?;
            reserve_player_mut(|player| {
                let left = player.get_datum(&left).clone();
                let right = player.get_datum(&right).clone();
                let result = add_datums(left, right, player)?;
                Ok(player.alloc_datum(result))
            })
        }
        LingoExpr::Subtract(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs.as_ref())).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs.as_ref())).await?;
            reserve_player_mut(|player| {
                let left = player.get_datum(&left).clone();
                let right = player.get_datum(&right).clone();
                let result = subtract_datums(left, right, player)?;
                Ok(player.alloc_datum(result))
            })
        }
    }
}

pub fn eval_lingo_expr_static(expr: String) -> Result<DatumRef, ScriptError> {
    let _tokens = tokenize_lingo(&expr);
    match LingoParser::parse(Rule::eval_expr, expr.as_str()) {
        Ok(parse_result) => {
            let expr_pair = &parse_result.enumerate().next().unwrap();
            eval_lingo_pair_static(expr_pair.1.clone())
        }
        Err(e) => {
            error!("eval_lingo_expr_static parse error: {}", ascii_safe(&e.to_string()));
            Ok(DatumRef::Void)
        }
    }
}

pub fn parse_lingo_expr_ast_runtime(rule: Rule, expr: String) -> Result<LingoExpr, ScriptError> {
    let pratt = create_lingo_pratt_parser();
    let _tokens = tokenize_lingo(&expr);
    match LingoParser::parse(rule, expr.as_str()) {
        Ok(parse_result) => {
            let expr_pair = &parse_result.enumerate().next().unwrap();
            let mut ast = parse_lingo_rule_runtime(expr_pair.1.clone(), &pratt)?;

            // In command context, convert bare identifiers to handler calls
            if rule == Rule::command_eval_expr {
                if let LingoExpr::Identifier(name) = ast {
                    ast = LingoExpr::HandlerCall(name, vec![]);
                }
            }

            Ok(ast)
        }
        Err(e) => {
            Err(ScriptError::new(ascii_safe(&e.to_string())))
        }
    }
}

pub async fn eval_lingo_expr_runtime(expr: String) -> Result<DatumRef, ScriptError> {
    let ast = parse_lingo_expr_ast_runtime(Rule::eval_expr, expr)?;
    eval_lingo_expr_ast_runtime(&ast).await
}

fn create_lingo_pratt_parser() -> PrattParser<Rule> {
    PrattParser::new()
        .op(Op::infix(Rule::add, Assoc::Left) | Op::infix(Rule::subtract, Assoc::Left))
        .op(Op::infix(Rule::obj_prop, Assoc::Left))
}

pub async fn eval_lingo_command(expr: String) -> Result<DatumRef, ScriptError> {
    let ast = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, expr)?;
    eval_lingo_expr_ast_runtime(&ast).await
}
