use log::error;
use pest::{
    iterators::{Pair, Pairs},
    pratt_parser::{Assoc, Op, PrattParser},
    Parser,
};

use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType, StringChunkType, StringChunkExpr},
    js_api::ascii_safe,
    player::{
        bytecode::{get_set::GetSetUtils, string::StringBytecodeHandler},
        datum_operations::{add_datums, divide_datums, multiply_datums, subtract_datums},
        handlers::datum_handlers::{player_call_datum_handler, prop_list::PropListUtils, string_chunk::StringChunkUtils},
        player_call_global_handler, reserve_player_mut,
        script::{get_lctx_for_script, get_obj_prop, player_set_obj_prop, script_get_prop_opt},
        DirPlayer,
    },
};

use super::{cast_lib::INVALID_CAST_MEMBER_REF, datum_formatting::format_datum, sprite::ColorRef, DatumRef, ScriptError};

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
    FloatLiteral(f64),
    PropListLiteral(Vec<(LingoExpr, LingoExpr)>),
    HandlerCall(String, Vec<LingoExpr>),
    ObjProp(Box<LingoExpr>, String),
    ObjHandlerCall(Box<LingoExpr>, String, Vec<LingoExpr>),
    ListAccess(Box<LingoExpr>, Box<LingoExpr>), // list_expr, index_expr
    ColorLiteral(ColorRef),
    RectLiteral(Vec<(LingoExpr, LingoExpr, LingoExpr, LingoExpr)>),
    PointLiteral(Vec<(LingoExpr, LingoExpr)>),
    MemberRef(Box<LingoExpr>, Option<Box<LingoExpr>>), // member_num, optional cast_lib
    Identifier(String),
    Assignment(Box<LingoExpr>, Box<LingoExpr>),
    Add(Box<LingoExpr>, Box<LingoExpr>),
    Subtract(Box<LingoExpr>, Box<LingoExpr>),
    Multiply(Box<LingoExpr>, Box<LingoExpr>),
    Divide(Box<LingoExpr>, Box<LingoExpr>),
    Join(Box<LingoExpr>, Box<LingoExpr>),
    JoinPad(Box<LingoExpr>, Box<LingoExpr>),
    And(Box<LingoExpr>, Box<LingoExpr>),
    Or(Box<LingoExpr>, Box<LingoExpr>),
    Eq(Box<LingoExpr>, Box<LingoExpr>),
    Ne(Box<LingoExpr>, Box<LingoExpr>),
    Lt(Box<LingoExpr>, Box<LingoExpr>),
    Gt(Box<LingoExpr>, Box<LingoExpr>),
    Le(Box<LingoExpr>, Box<LingoExpr>),
    Ge(Box<LingoExpr>, Box<LingoExpr>),
    Not(Box<LingoExpr>),
    PutBefore(Box<LingoExpr>, Box<LingoExpr>),
    PutAfter(Box<LingoExpr>, Box<LingoExpr>),
    PutInto(Box<LingoExpr>, Box<LingoExpr>),
    PutDisplay(Box<LingoExpr>),
    ThePropOf(Box<LingoExpr>, String), // "the X of Y" constructs
    ChunkExpr(String, Box<LingoExpr>, Box<LingoExpr>),
    DeleteChunk(Box<LingoExpr>), // delete <chunk_expr>
}

/// Evaluate a static Lingo expression. This does not support function calls.
pub fn eval_lingo_pair_static(pair: Pair<Rule>) -> Result<DatumRef, ScriptError> {
    let inner_rule = pair.as_rule();
    match pair.as_rule() {
        Rule::expr => {
            let inner = pair.into_inner().next()
                .ok_or_else(|| ScriptError::new("Expected expression content".to_string()))?;
            eval_lingo_pair_static(inner)
        },
        Rule::term_arg => {
            let inner = pair.into_inner().next()
                .ok_or_else(|| ScriptError::new("Expected term_arg content".to_string()))?;
            eval_lingo_pair_static(inner)
        },
        Rule::list => {
            let inner = pair.into_inner().next()
                .ok_or_else(|| ScriptError::new("Expected list content".to_string()))?;
            eval_lingo_pair_static(inner)
        },
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
            let str_val = pair.into_inner().next()
                .ok_or_else(|| ScriptError::new("Expected string content".to_string()))?
                .as_str();
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(str_val.to_owned()))))
        }
        Rule::prop_list => {
            let inner = pair.into_inner().next()
                .ok_or_else(|| ScriptError::new("Expected prop list content".to_string()))?;
            eval_lingo_pair_static(inner)
        },
        Rule::multi_prop_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let mut pair_inner = inner_pair.into_inner();
                let key = eval_lingo_pair_static(pair_inner.next()
                    .ok_or_else(|| ScriptError::new("Expected prop list key".to_string()))?)?;
                let value = eval_lingo_pair_static(pair_inner.next()
                    .ok_or_else(|| ScriptError::new("Expected prop list value".to_string()))?)?;

                result_vec.push((key, value));
            }
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::PropList(result_vec, false))))
        }
        Rule::empty_prop_list => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::PropList(vec![], false))))
        }
        Rule::number_int => reserve_player_mut(|player| {
            let val = pair.as_str().parse::<i32>()
                .map_err(|e| ScriptError::new(format!("Invalid integer: {}", e)))?;
            Ok(player.alloc_datum(Datum::Int(val)))
        }),
        Rule::number_float => reserve_player_mut(|player| {
            let val = pair.as_str().parse::<f64>()
                .map_err(|e| ScriptError::new(format!("Invalid float: {}", e)))?;
            Ok(player.alloc_datum(Datum::Float(val)))
        }),
        Rule::rect => {
            let mut inner = pair.into_inner();
            reserve_player_mut(|player| {
                let x = parse_number_value(inner.next().ok_or_else(|| ScriptError::new("Expected rect x".to_string()))?)?;
                let y = parse_number_value(inner.next().ok_or_else(|| ScriptError::new("Expected rect y".to_string()))?)?;
                let w = parse_number_value(inner.next().ok_or_else(|| ScriptError::new("Expected rect width".to_string()))?)?;
                let h = parse_number_value(inner.next().ok_or_else(|| ScriptError::new("Expected rect height".to_string()))?)?;
                
                let x_ref = player.alloc_datum(if x.fract() == 0.0 { Datum::Int(x as i32) } else { Datum::Float(x) });
                let y_ref = player.alloc_datum(if y.fract() == 0.0 { Datum::Int(y as i32) } else { Datum::Float(y) });
                let w_ref = player.alloc_datum(if w.fract() == 0.0 { Datum::Int(w as i32) } else { Datum::Float(w) });
                let h_ref = player.alloc_datum(if h.fract() == 0.0 { Datum::Int(h as i32) } else { Datum::Float(h) });
                
                Ok(player.alloc_datum(Datum::Rect([x_ref, y_ref, w_ref, h_ref])))
            })
        }
        Rule::rgb_num_color => {
            let mut inner = pair.into_inner();
            reserve_player_mut(|player| {
                let r = inner.next().ok_or_else(|| ScriptError::new("Expected red component".to_string()))?.as_str().parse::<u8>()
                    .map_err(|e| ScriptError::new(format!("Invalid red: {}", e)))?;
                let g = inner.next().ok_or_else(|| ScriptError::new("Expected green component".to_string()))?.as_str().parse::<u8>()
                    .map_err(|e| ScriptError::new(format!("Invalid green: {}", e)))?;
                let b = inner.next().ok_or_else(|| ScriptError::new("Expected blue component".to_string()))?.as_str().parse::<u8>()
                    .map_err(|e| ScriptError::new(format!("Invalid blue: {}", e)))?;
                Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(r, g, b))))
            })
        }
        Rule::rgb_str_color => {
            let mut inner = pair.into_inner();
            let str_inner = inner.next()
                .ok_or_else(|| ScriptError::new("Expected rgb string".to_string()))?
                .into_inner().next()
                .ok_or_else(|| ScriptError::new("Expected rgb string content".to_string()))?;
            let str_val = str_inner.as_str();
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::ColorRef(ColorRef::from_hex(str_val))))
            })
        }
        Rule::rgb_color => {
            let mut inner = pair.into_inner();
            if let Some(inner_pair) = inner.next() {
                // recursively call the static evaluator
                eval_lingo_pair_static(inner_pair)
            } else {
                // fallback to default static behavior
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Void)))
            }
        }
        Rule::symbol => {
            let str_val = pair.into_inner().next()
                .ok_or_else(|| ScriptError::new("Expected symbol name".to_string()))?
                .as_str();
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Symbol(str_val.to_owned()))))
        }
        Rule::bool_true => reserve_player_mut(|player| Ok(player.alloc_datum(datum_bool(true)))),
        Rule::bool_false => reserve_player_mut(|player| Ok(player.alloc_datum(datum_bool(false)))),
        Rule::void => Ok(DatumRef::Void),
        Rule::string_empty => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String("".to_owned()))))
        }
        Rule::return_const => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String("\r\n".to_owned()))))
        }
        Rule::nohash_symbol => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::Symbol(pair.as_str().to_owned())))
        }),
        Rule::point => {
            let mut inner = pair.into_inner();
            reserve_player_mut(|player| {
                let x = parse_number_value(inner.next().ok_or_else(|| ScriptError::new("Expected point x".to_string()))?)?;
                let y = parse_number_value(inner.next().ok_or_else(|| ScriptError::new("Expected point y".to_string()))?)?;
                
                let x_ref = player.alloc_datum(if x.fract() == 0.0 { Datum::Int(x as i32) } else { Datum::Float(x) });
                let y_ref = player.alloc_datum(if y.fract() == 0.0 { Datum::Int(y as i32) } else { Datum::Float(y) });
                
                Ok(player.alloc_datum(Datum::Point([x_ref, y_ref])))
            })
        }
        Rule::empty_list => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::List(DatumType::List, vec![], false)))
        }),
        Rule::the_prop => {
            // For multi-word properties like "the long time", we need to get the full text
            // and extract the property name
            let full_text = pair.as_str();
            let prop_name = if full_text.starts_with("the ") || full_text.starts_with("THE ") || full_text.starts_with("The ") {
                &full_text[4..]  // Skip "the "
            } else {
                // Shouldn't happen with correct grammar, but handle it
                full_text
            };
            reserve_player_mut(|player| {
                let prop_value = player.get_movie_prop(prop_name)?;
                Ok(prop_value)
            })
        }
        Rule::member_ref => {
            let mut inner = pair.into_inner();

            // First expression is the member name or number
            let member_expr = inner.next()
                .ok_or_else(|| ScriptError::new("Expected member identifier".to_string()))?;
            let member_id_ref = eval_lingo_pair_static(member_expr)?;

            // Optional: "of castLib X"
            let cast_lib_ref = if let Some(castlib_expr) = inner.next() {
                Some(eval_lingo_pair_static(castlib_expr)?)
            } else {
                None
            };

            reserve_player_mut(|player| {
                let member_id_datum = player.get_datum(&member_id_ref).clone();

                // Get cast lib datum if specified
                let cast_lib_datum = cast_lib_ref.as_ref().map(|r| player.get_datum(r).clone());

                // Use find_member_ref_by_identifiers for proper member lookup
                // This handles both string names and numeric member IDs
                let member_result = player.movie.cast_manager.find_member_ref_by_identifiers(
                    &member_id_datum,
                    cast_lib_datum.as_ref(),
                    &player.allocator,
                )?;

                let member_ref = match member_result {
                    Some(r) => r,
                    None => {
                        // If cast_lib was specified, create a ref with the specified values
                        // Otherwise return invalid ref
                        if let Some(cast_datum) = cast_lib_datum {
                            let cast_lib_num = match cast_datum {
                                Datum::Int(num) => num,
                                Datum::CastLib(num) => num as i32,
                                _ => return Err(ScriptError::new(format!(
                                    "Expected int or castLib, got {:?}",
                                    cast_datum.type_enum()
                                ))),
                            };
                            let member_num = member_id_datum.int_value().unwrap_or(0);
                            super::cast_lib::CastMemberRef {
                                cast_lib: cast_lib_num,
                                cast_member: member_num,
                            }
                        } else {
                            INVALID_CAST_MEMBER_REF
                        }
                    }
                };

                Ok(player.alloc_datum(Datum::CastMember(member_ref)))
            })
        }
        Rule::castlib_ref => {
            let mut inner = pair.into_inner();
            
            let castlib_expr = inner.next()
                .ok_or_else(|| ScriptError::new("Expected castLib identifier".to_string()))?;
            let castlib_ref = eval_lingo_pair_static(castlib_expr)?;
            
            reserve_player_mut(|player| {
                let castlib_num = player.get_datum(&castlib_ref).int_value()
                    .or_else(|_| {
                        // If it's not an int, try to get it as a string (named castLib)
                        let name = player.get_datum(&castlib_ref).string_value()?;
                        // Convert castLib name to number
                        let cast = player
                            .movie
                            .cast_manager
                            .get_cast_by_name(&name)
                            .ok_or_else(|| ScriptError::new(format!("CastLib not found: {}", name)))?;
                        Ok(cast.number as i32)
                    })?;
                
                // Return a CastLib reference datum
                Ok(player.alloc_datum(Datum::CastLib(castlib_num as u32)))
            })
        }
        Rule::config_key | Rule::config_ident_part => {
            // Config keys treated as strings in static context
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::String(pair.as_str().to_owned())))
            })
        }
        _ => Err(ScriptError::new(format!(
            "Invalid static Lingo expression {:?}",
            inner_rule
        ))),
    }
}

fn parse_number_value(pair: Pair<Rule>) -> Result<f64, ScriptError> {
    match pair.as_rule() {
        Rule::number_int => {
            pair.as_str().parse::<f64>()
                .map_err(|e| ScriptError::new(format!("Invalid number: {}", e)))
        }
        Rule::number_float => {
            pair.as_str().parse::<f64>()
                .map_err(|e| ScriptError::new(format!("Invalid number: {}", e)))
        }
        _ => Err(ScriptError::new(format!("Expected number, got {:?}", pair.as_rule())))
    }
}

fn get_eval_top_level_prop(
    player: &mut DirPlayer,
    prop_name: &str,
) -> Result<DatumRef, ScriptError> {
    if prop_name.starts_with("the ") {
        let actual_prop = &prop_name[4..];
        let result = player.get_movie_prop(actual_prop)?;
        return Ok(result);
    }

    // When a breakpoint is active, resolve against the selected (or topmost) scope first
    if player.current_breakpoint.is_some() && player.scope_count > 0 {
        let scope_idx = player.eval_scope_index
            .unwrap_or(player.scope_count - 1) as usize;
        let scope = &player.scopes[scope_idx];

        // Check locals by reverse-looking up the name_id from the name table
        {
            let script_ref_for_locals = scope.script_ref.clone();
            if let Some(script_rc) = player.movie.cast_manager.get_script_by_ref(&script_ref_for_locals) {
                if let Some(lctx) = get_lctx_for_script(player, &script_rc) {
                    if let Some(name_id) = lctx.names.iter().position(|n| n.eq_ignore_ascii_case(prop_name)) {
                        if let Some(local_ref) = player.scopes[scope_idx].locals.get(&(name_id as u16)) {
                            return Ok(local_ref.clone());
                        }
                    }
                }
            }
        }

        // Check "me" (the receiver)
        if prop_name == "me" {
            if let Some(receiver) = scope.receiver.clone() {
                return Ok(player.alloc_datum(Datum::ScriptInstanceRef(receiver)));
            }
        }

        // Resolve handler name from the scope's handler_name_id
        let script_ref = scope.script_ref.clone();
        let handler_name_id = scope.handler_name_id;
        if let Some(script_rc) = player.movie.cast_manager.get_script_by_ref(&script_ref) {
            let script = script_rc.clone();
            // Find the handler whose name_id matches this scope's handler_name_id
            let handler_name = script.handlers.iter()
                .find(|(_, h)| h.name_id == handler_name_id)
                .map(|(name, _)| name.as_str().to_owned());
            if let Some(handler_name) = handler_name {
                if let Some(handler_def) = script.get_own_handler(&handler_name) {
                    let handler_def = handler_def.clone();
                    // Check handler arguments by name
                    if let Some(lctx) = get_lctx_for_script(player, &script) {
                        for (i, &name_id) in handler_def.argument_name_ids.iter().enumerate() {
                            if let Some(name) = lctx.names.get(name_id as usize) {
                                if name.eq_ignore_ascii_case(prop_name) {
                                    if let Some(arg_ref) = player.scopes[scope_idx].args.get(i) {
                                        return Ok(arg_ref.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check properties on the receiver (me) object
        let receiver = player.scopes[scope_idx].receiver.clone();
        if let Some(receiver_ref) = receiver {
            let prop_name_str = prop_name.to_string();
            if let Some(result) = script_get_prop_opt(player, &receiver_ref, &prop_name_str) {
                return Ok(result);
            }
        }
    }

    if let Some(global_ref) = player.globals.get(prop_name) {
        Ok(global_ref.clone())
    } else {
        // Try to get as a top-level prop, but if it fails, return an error about undefined variable
        match GetSetUtils::get_top_level_prop(player, prop_name) {
            Ok(result) => Ok(player.alloc_datum(result)),
            Err(_) => Err(ScriptError::new(format!("Undefined variable: {}", prop_name)))
        }
    }
}

fn parse_lingo_expr_runtime(
    pairs: Pairs<'_, Rule>,
    pratt: &PrattParser<Rule>,
) -> Result<LingoExpr, ScriptError> {
    pratt
        .map_primary(|pair| parse_lingo_rule_runtime(pair, pratt))
        .map_prefix(|op, rhs| match op.as_rule() {
            Rule::not_op => {
                let right = rhs?;
                Ok(LingoExpr::Not(Box::new(right)))
            }
            _ => Err(ScriptError::new(format!(
                "Invalid prefix operator {:?}",
                op.as_rule()
            ))),
        })
        .map_postfix(|lhs, op| match op.as_rule() {
            Rule::list_index => {
                let list_expr = lhs?;
                // Extract the expression inside the brackets
                let index_pairs = op.into_inner();
                let index_expr = parse_lingo_expr_runtime(index_pairs, pratt)?;
                Ok(LingoExpr::ListAccess(Box::new(list_expr), Box::new(index_expr)))
            }
            _ => Err(ScriptError::new(format!(
                "Invalid postfix operator {:?}",
                op.as_rule()
            ))),
        })
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::add => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Add(Box::new(left), Box::new(right)))
            }
            Rule::subtract => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Subtract(Box::new(left), Box::new(right)))
            }
            Rule::multiply => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Multiply(Box::new(left), Box::new(right)))
            }
            Rule::divide => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Divide(Box::new(left), Box::new(right)))
            }
            Rule::join => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Join(Box::new(left), Box::new(right)))
            }
            Rule::join_pad => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::JoinPad(Box::new(left), Box::new(right)))
            }
            Rule::and_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::And(Box::new(left), Box::new(right)))
            }
            Rule::or_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Or(Box::new(left), Box::new(right)))
            }
            Rule::eq_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Eq(Box::new(left), Box::new(right)))
            }
            Rule::ne_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Ne(Box::new(left), Box::new(right)))
            }
            Rule::lt_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Lt(Box::new(left), Box::new(right)))
            }
            Rule::gt_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Gt(Box::new(left), Box::new(right)))
            }
            Rule::le_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Le(Box::new(left), Box::new(right)))
            }
            Rule::ge_op => {
                let left = lhs?;
                let right = rhs?;
                Ok(LingoExpr::Ge(Box::new(left), Box::new(right)))
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
                    _ => Err(ScriptError::new(format!(
                        "Invalid object prop operator rhs {:?}",
                        rhs
                    ))),
                }
            }
            _ => Err(ScriptError::new(format!(
                "Invalid infix operator {:?}",
                op.as_rule()
            ))),
        })
        .parse(pairs)
}

/// Evaluate a dynamic Lingo expression at runtime.
pub fn parse_lingo_rule_runtime(
    pair: Pair<'_, Rule>,
    pratt: &PrattParser<Rule>,
) -> Result<LingoExpr, ScriptError> {
    let inner_rule = pair.as_rule();
    match pair.as_rule() {
        Rule::expr => {
            let inner_pair = pair.into_inner();
            let ast = parse_lingo_expr_runtime(inner_pair, pratt)?;
            Ok(ast)
        }
        Rule::term => {
            let inner_pair = pair.into_inner();
            let ast = parse_lingo_expr_runtime(inner_pair, pratt)?;
            Ok(ast)
        }
        Rule::term_arg => {
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
                let value =
                    parse_lingo_expr_runtime(pair_inner.next().unwrap().into_inner(), pratt)?;

                result_vec.push((key, value));
            }
            Ok(LingoExpr::PropListLiteral(result_vec))
        }
        Rule::empty_prop_list => Ok(LingoExpr::PropListLiteral(vec![])),
        Rule::number_int => Ok(LingoExpr::IntLiteral(pair.as_str().parse::<i32>().unwrap())),
        Rule::number_float => Ok(LingoExpr::FloatLiteral(
            pair.as_str().parse::<f64>().unwrap(),
        )),
        Rule::rect => {
            let mut inner = pair.into_inner();
            let x_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            let y_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            let w_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            let h_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            
            Ok(LingoExpr::RectLiteral(vec![(x_expr, y_expr, w_expr, h_expr)]))
        }
        Rule::point => {
            let mut inner = pair.into_inner();
            let x_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            let y_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            
            Ok(LingoExpr::PointLiteral(vec![(x_expr, y_expr)]))
        }
        Rule::member_ref => {
            let mut inner = pair.into_inner();
            
            // First expression is the member number
            let member_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            
            // Optional: "of castLib X"
            let cast_lib_expr = if let Some(castlib_pair) = inner.next() {
                Some(Box::new(parse_lingo_rule_runtime(castlib_pair, pratt)?))
            } else {
                None
            };
            
            Ok(LingoExpr::MemberRef(Box::new(member_expr), cast_lib_expr))
        }
        Rule::sprite_ref => {
            let mut inner = pair.into_inner();
            let sprite_num_pair = inner.next().ok_or_else(|| ScriptError::new("Expected sprite number".to_string()))?;
            let sprite_num_expr = parse_lingo_expr_runtime(sprite_num_pair.into_inner(), pratt)?;
            Ok(LingoExpr::HandlerCall("sprite".to_string(), vec![sprite_num_expr]))
        }
        Rule::sprite_of_expr => {
            let mut inner = pair.into_inner();
            let prop_name_pair = inner.next().ok_or_else(|| ScriptError::new("Expected property name".to_string()))?;
            let prop_name = prop_name_pair.as_str().to_string();
            let sprite_pair = inner.next().ok_or_else(|| ScriptError::new("Expected sprite expression".to_string()))?;
            let sprite_expr = parse_lingo_rule_runtime(sprite_pair, pratt)?;
            Ok(LingoExpr::ObjProp(Box::new(sprite_expr), prop_name))
        }
        Rule::castlib_ref => {
            let mut inner = pair.into_inner();
            let castlib_expr = parse_lingo_rule_runtime(inner.next().unwrap(), pratt)?;
            Ok(LingoExpr::HandlerCall("castLib".to_string(), vec![castlib_expr]))
        }
        Rule::castlib_of_expr => {
            let mut inner = pair.into_inner();
            let prop_name_pair = inner.next().ok_or_else(|| ScriptError::new("Expected property name".to_string()))?;
            let prop_name = prop_name_pair.as_str().to_string();
            let castlib_pair = inner.next().ok_or_else(|| ScriptError::new("Expected castLib expression".to_string()))?;
            let castlib_expr = parse_lingo_rule_runtime(castlib_pair, pratt)?;
            Ok(LingoExpr::ObjProp(Box::new(castlib_expr), prop_name))
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
            let mut inner = pair.clone().into_inner();
            if let Some(inner_pair) = inner.next() {
                parse_lingo_rule_runtime(inner_pair, pratt)
            } else {
                let s = pair.as_str();
                Ok(LingoExpr::ColorLiteral(ColorRef::from_hex(s)))
            }
        }
        Rule::symbol => {
            let str_val = pair.into_inner().next().unwrap().as_str();
            Ok(LingoExpr::SymbolLiteral(str_val.to_owned()))
        }
        Rule::bool_true => Ok(LingoExpr::BoolLiteral(true)),
        Rule::bool_false => Ok(LingoExpr::BoolLiteral(false)),
        Rule::void => Ok(LingoExpr::VoidLiteral),
        Rule::string_empty => Ok(LingoExpr::StringLiteral("".to_owned())),
        Rule::return_const => Ok(LingoExpr::StringLiteral("\r\n".to_owned())),
        Rule::nohash_symbol => Ok(LingoExpr::SymbolLiteral(pair.as_str().to_owned())),
        Rule::empty_list => Ok(LingoExpr::ListLiteral(vec![])),
        Rule::put_handler_call => {
            // For put_handler_call, "put" is not captured as a child, only handler_call_args is
            let mut inner = pair.into_inner();
            let mut args = vec![];
            
            if let Some(args_container) = inner.next() {
                // This should be handler_call_args
                for arg_pair in args_container.into_inner() {
                    let arg_pairs = arg_pair.into_inner();
                    let arg_val = parse_lingo_expr_runtime(arg_pairs, pratt)?;
                    args.push(arg_val);
                }
            }

            Ok(LingoExpr::HandlerCall("put".to_string(), args))
        }
        Rule::handler_call | Rule::command_inline => {
            let mut inner = pair.into_inner();
            let handler_name_pair = inner.next().ok_or_else(|| ScriptError::new("Expected handler name".to_string()))?;
            let handler_name = handler_name_pair.as_str();
            let mut args = vec![];
            
            if let Some(args_container) = inner.next() {
                match args_container.as_rule() {
                    Rule::handler_call_args => {
                        // Process expr children (comma-separated in parentheses)
                        for arg_pair in args_container.into_inner() {
                            let arg_pairs = arg_pair.into_inner();
                            let arg_val = parse_lingo_expr_runtime(arg_pairs, pratt)?;
                            args.push(arg_val);
                        }
                    }
                    Rule::command_inline_args_comma => {
                        // Process expr children (comma-separated)
                        for arg_pair in args_container.into_inner() {
                            let arg_pairs = arg_pair.into_inner();
                            let arg_val = parse_lingo_expr_runtime(arg_pairs, pratt)?;
                            args.push(arg_val);
                        }
                    }
                    Rule::command_inline_args_space => {
                        // Process term_arg children (space-separated)
                        for arg_pair in args_container.into_inner() {
                            // arg_pair is a term_arg, recursively process it
                            let arg_val = parse_lingo_rule_runtime(arg_pair, pratt)?;
                            args.push(arg_val);
                        }
                    }
                    Rule::command_inline_args_single => {
                        // Process single expr
                        let expr_pair = args_container.into_inner().next()
                            .ok_or_else(|| ScriptError::new("Expected expr in single arg".to_string()))?;
                        let arg_pairs = expr_pair.into_inner();
                        let arg_val = parse_lingo_expr_runtime(arg_pairs, pratt)?;
                        args.push(arg_val);
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Unexpected args rule: {:?}",
                            args_container.as_rule()
                        )));
                    }
                }
            }

            Ok(LingoExpr::HandlerCall(handler_name.to_owned(), args))
        }
        Rule::lang_ident | Rule::ident => {
            Ok(LingoExpr::Identifier(pair.as_str().to_owned()))
        }
        Rule::prop_name => {
            // Property names (including reserved keywords when used after dot)
            Ok(LingoExpr::Identifier(pair.as_str().to_owned()))
        }
        Rule::config_key | Rule::config_ident_part => {
            // Configuration keys (allows asterisks in identifiers)
            Ok(LingoExpr::Identifier(pair.as_str().to_owned()))
        }
        Rule::dotted_ident => {
            // Parse dotted identifiers like "obj.prop.subprop" into nested ObjProp expressions
            let full_str = pair.as_str();
            let parts: Vec<&str> = full_str.split('.').collect();
            
            if parts.is_empty() {
                return Err(ScriptError::new("Empty dotted identifier".to_string()));
            }
            
            // Start with the first identifier
            let mut result = LingoExpr::Identifier(parts[0].to_owned());
            
            // Chain the rest as ObjProp accesses
            for part in &parts[1..] {
                result = LingoExpr::ObjProp(Box::new(result), part.to_string());
            }
            
            Ok(result)
        }
        Rule::assignment_expr => {
            let mut inner = pair.into_inner();

            let first_term = inner.next().ok_or_else(|| ScriptError::new("Expected first term in assignment_expr".to_string()))?;
            let mut result = parse_lingo_rule_runtime(first_term, pratt)?;

            while let Some(next_pair) = inner.next() {
                if next_pair.as_rule() == Rule::obj_prop {
                    if let Some(term_pair) = inner.next() {
                        let prop_name = term_pair.as_str();
                        result = LingoExpr::ObjProp(Box::new(result), prop_name.to_string());
                    }
                } else if next_pair.as_rule() == Rule::list_index {
                    // Bracket indexing: [expr]
                    let index_pairs = next_pair.into_inner();
                    let index_expr = parse_lingo_expr_runtime(index_pairs, pratt)?;
                    result = LingoExpr::ListAccess(Box::new(result), Box::new(index_expr));
                } else {
                    let prop_name = next_pair.as_str();
                    result = LingoExpr::ObjProp(Box::new(result), prop_name.to_string());
                }
            }

            Ok(result)
        }
        Rule::assignment => {
            let mut inner = pair.into_inner();
            let left_pair = inner.next().ok_or_else(|| ScriptError::new("Expected left side of assignment".to_string()))?;
            let right_pair = inner.next().ok_or_else(|| ScriptError::new("Expected right side of assignment".to_string()))?;

            let left_expr = if left_pair.as_rule() == Rule::assignment_expr {
                parse_lingo_rule_runtime(left_pair, pratt)?
            } else {
                match left_pair.as_rule() {
                    Rule::ident | Rule::lang_ident => {
                        let ident_name = left_pair.as_str();
                        LingoExpr::Identifier(ident_name.to_owned())
                    }
                    Rule::dotted_ident => {
                        parse_lingo_rule_runtime(left_pair, pratt)?
                    }
                    _ => parse_lingo_rule_runtime(left_pair, pratt)?,
                }
            };

            let right_expr = parse_lingo_expr_runtime(right_pair.into_inner(), pratt)?;

            Ok(LingoExpr::Assignment(
                Box::new(left_expr),
                Box::new(right_expr),
            ))
        }
        Rule::put_display => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected expression in put display".to_string()))?;
            let value_expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
            Ok(LingoExpr::PutDisplay(Box::new(value_expr)))
        }
        Rule::put_display_multi => {
            let mut inner = pair.into_inner();
            let mut exprs = vec![];
            for expr_pair in inner {
                let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
                exprs.push(expr);
            }
            // Multiple comma-separated args means this is a handler call
            Ok(LingoExpr::HandlerCall("put".to_string(), exprs))
        }
        Rule::put_into => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected expression".to_string()))?;
            let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
            let target_pair = inner.next().ok_or_else(|| ScriptError::new("Expected target identifier".to_string()))?;
            let target_name = target_pair.as_str().to_string();
            Ok(LingoExpr::PutInto(
                Box::new(expr), 
                Box::new(LingoExpr::Identifier(target_name))
            ))
        }
        Rule::put_before => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected expression".to_string()))?;
            let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
            let target_pair = inner.next().ok_or_else(|| ScriptError::new("Expected target identifier".to_string()))?;
            let target_name = target_pair.as_str().to_string();
            Ok(LingoExpr::PutBefore(
                Box::new(expr), 
                Box::new(LingoExpr::Identifier(target_name))
            ))
        },
        Rule::put_after => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected expression".to_string()))?;
            let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
            let target_pair = inner.next().ok_or_else(|| ScriptError::new("Expected target identifier".to_string()))?;
            let target_name = target_pair.as_str().to_string();
            Ok(LingoExpr::PutAfter(
                Box::new(expr), 
                Box::new(LingoExpr::Identifier(target_name))
            ))
        },
        Rule::put_into_chunk => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected expression".to_string()))?;
            let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
            let chunk_pair = inner.next().ok_or_else(|| ScriptError::new("Expected chunk expression".to_string()))?;
            let chunk = parse_lingo_rule_runtime(chunk_pair, pratt)?;
            Ok(LingoExpr::PutInto(Box::new(expr), Box::new(chunk)))
        },
        Rule::put_before_chunk => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected expression".to_string()))?;
            let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
            let chunk_pair = inner.next().ok_or_else(|| ScriptError::new("Expected chunk expression".to_string()))?;
            let chunk = parse_lingo_rule_runtime(chunk_pair, pratt)?;
            Ok(LingoExpr::PutBefore(Box::new(expr), Box::new(chunk)))
        },
        Rule::put_after_chunk => {
            let mut inner = pair.into_inner();
            let expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected expression".to_string()))?;
            let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
            let chunk_pair = inner.next().ok_or_else(|| ScriptError::new("Expected chunk expression".to_string()))?;
            let chunk = parse_lingo_rule_runtime(chunk_pair, pratt)?;
            Ok(LingoExpr::PutAfter(Box::new(expr), Box::new(chunk)))
        },
        Rule::put_statement => {
            let mut inner = pair.clone().into_inner();
            if let Some(inner_pair) = inner.next() {
                parse_lingo_rule_runtime(inner_pair, pratt)
            } else {
                parse_lingo_rule_runtime(pair, pratt)
            }
        }
        Rule::set_statement => {
            let mut inner = pair.into_inner();
            let left_pair = inner.next().ok_or_else(|| ScriptError::new("Expected left side of set statement".to_string()))?;
            let right_pair = inner.next().ok_or_else(|| ScriptError::new("Expected right side of set statement".to_string()))?;
            let left_expr = parse_lingo_expr_runtime(left_pair.into_inner(), pratt)?;
            let right_expr = parse_lingo_expr_runtime(right_pair.into_inner(), pratt)?;
            Ok(LingoExpr::Assignment(Box::new(left_expr), Box::new(right_expr)))
        }
        Rule::delete_statement => {
            let mut inner = pair.into_inner();
            let chunk_pair = inner.next().ok_or_else(|| ScriptError::new("Expected chunk expression after delete".to_string()))?;
            let chunk_expr = parse_lingo_rule_runtime(chunk_pair, pratt)?;
            Ok(LingoExpr::DeleteChunk(Box::new(chunk_expr)))
        }
        Rule::chunk_expr => {
            let mut inner = pair.into_inner();
            let chunk_type_pair = inner.next().ok_or_else(|| ScriptError::new("Expected chunk type".to_string()))?;
            let chunk_type = chunk_type_pair.as_str().to_lowercase();
            let index_pair = inner.next().ok_or_else(|| ScriptError::new("Expected index expression".to_string()))?;
            let index_expr = parse_lingo_expr_runtime(index_pair.into_inner(), pratt)?;
            let source_pair = inner.next().ok_or_else(|| ScriptError::new("Expected source expression".to_string()))?;
            let source_expr = match source_pair.as_rule() {
                Rule::ident | Rule::lang_ident => {
                    // Regular identifier - just use it as-is
                    LingoExpr::Identifier(source_pair.as_str().to_string())
                },
                Rule::the_prop => {
                    // "the X" property - parse it to get the full "the X" form
                    parse_lingo_rule_runtime(source_pair, pratt)?
                },
                Rule::the_prop_of => {
                    // "the X of Y" - parse recursively
                    parse_lingo_rule_runtime(source_pair, pratt)?
                },
                Rule::chunk_expr => {
                    // Nested chunk expression
                    parse_lingo_rule_runtime(source_pair, pratt)?
                },
                _ => parse_lingo_rule_runtime(source_pair, pratt)?,
            };
            Ok(LingoExpr::ChunkExpr(chunk_type, Box::new(index_expr), Box::new(source_expr)))
        }
        Rule::the_prop => {
            // For multi-word properties like "the long time", we need to get the full text
            // The full text is already "the property_name" format
            let full_text = pair.as_str();
            // Return an identifier that will be resolved at runtime
            Ok(LingoExpr::Identifier(full_text.to_string()))
        }
        Rule::the_prop_of => {
            let mut inner = pair.into_inner();
            let prop_name_pair = inner.next().ok_or_else(|| ScriptError::new("Expected property name after 'the'".to_string()))?;
            let prop_name = prop_name_pair.as_str().to_string();
            let target_pair = inner.next().ok_or_else(|| ScriptError::new("Expected target after 'of'".to_string()))?;
            
            // Check what kind of target we have
            let target_expr = match target_pair.as_rule() {
                Rule::castlib_of_expr | Rule::sprite_of_expr | Rule::prop_of_expr => {
                    // These are already structured as "X of Y", parse them directly
                    parse_lingo_rule_runtime(target_pair, pratt)?
                }
                _ => {
                    // Regular expression
                    parse_lingo_expr_runtime(target_pair.into_inner(), pratt)?
                }
            };
            
            Ok(LingoExpr::ThePropOf(Box::new(target_expr), prop_name))
        }
        Rule::prop_of_expr => {
            let mut inner = pair.into_inner();
            let prop_name_pair = inner.next().ok_or_else(|| ScriptError::new("Expected property name".to_string()))?;
            let prop_name = prop_name_pair.as_str().to_string();
            let obj_expr_pair = inner.next().ok_or_else(|| ScriptError::new("Expected object expression".to_string()))?;
            let obj_expr = parse_lingo_expr_runtime(obj_expr_pair.into_inner(), pratt)?;
            Ok(LingoExpr::ObjProp(Box::new(obj_expr), prop_name))
        }
        Rule::parens_list => {
            let mut inner = pair.into_inner();
            let mut exprs = vec![];
            for expr_pair in inner {
                let expr = parse_lingo_expr_runtime(expr_pair.into_inner(), pratt)?;
                exprs.push(expr);
            }
            Ok(LingoExpr::ListLiteral(exprs))
        }
        Rule::parens_empty => {
            Ok(LingoExpr::ListLiteral(vec![]))
        }
        _ => Err(ScriptError::new(format!(
            "Invalid runtime Lingo expression {:?}",
            inner_rule
        ))),
    }
}

async fn handle_put_into_chunk(
    value_ref: DatumRef,
    target: &LingoExpr,
) -> Result<DatumRef, ScriptError> {
    // Extract chunk expression components (type, index, source variable)
    let (chunk_type_str, index_expr, source_expr) = match target {
        LingoExpr::ChunkExpr(chunk_type, index, source) => (chunk_type, index, source),
        _ => return Err(ScriptError::new("Expected chunk expression".to_string())),
    };

    // Evaluate the index expression to get the numeric index
    let index_ref = Box::pin(eval_lingo_expr_ast_runtime(index_expr)).await?;
    let index = reserve_player_mut(|player| {
        player.get_datum(&index_ref).int_value()
    })?;

    let chunk_type = StringChunkType::from(chunk_type_str);

    // Get the source variable name (e.g., "myStr")
    let source_name = match source_expr.as_ref() {
        LingoExpr::Identifier(name) => name.clone(),
        _ => return Err(ScriptError::new("Expected identifier as chunk source".to_string())),
    };

    // Convert the value to insert into a string
    let value_str = reserve_player_mut(|player| {
        use crate::player::datum_formatting::datum_to_string_for_concat;
        let value_datum = player.get_datum(&value_ref);
        Ok(datum_to_string_for_concat(value_datum, player))
    })?;

    // Perform the chunk replacement operation
    reserve_player_mut(|player| {
        // Get current value of the variable (or empty string if undefined)
        let current_ref = player.globals.get(&source_name)
            .cloned()
            .unwrap_or_else(|| player.alloc_datum(Datum::String(String::new())));
        
        let current_str = player.get_datum(&current_ref).string_value()?;

        // Build chunk expression with start/end at same index for single chunk
        let chunk_expr = StringChunkExpr {
            chunk_type,
            start: index,
            end: index,
            item_delimiter: player.movie.item_delimiter,
        };

        // Use the existing string utility to perform the replacement
        let new_string = StringChunkUtils::string_by_putting_into_chunk(
            &current_str,
            &chunk_expr,
            &value_str
        )?;

        // Update the variable with the modified string
        let new_string_ref = player.alloc_datum(Datum::String(new_string));
        player.globals.insert(source_name, new_string_ref);

        Ok(DatumRef::Void)
    })
}

async fn handle_put_before_chunk(
    value_ref: DatumRef,
    target: &LingoExpr,
) -> Result<DatumRef, ScriptError> {
    // Extract chunk expression components
    let (chunk_type_str, index_expr, source_expr) = match target {
        LingoExpr::ChunkExpr(chunk_type, index, source) => (chunk_type, index, source),
        _ => return Err(ScriptError::new("Expected chunk expression".to_string())),
    };

    // Evaluate the index
    let index_ref = Box::pin(eval_lingo_expr_ast_runtime(index_expr)).await?;
    let index = reserve_player_mut(|player| {
        player.get_datum(&index_ref).int_value()
    })?;

    let chunk_type = StringChunkType::from(chunk_type_str);

    // Get source variable name
    let source_name = match source_expr.as_ref() {
        LingoExpr::Identifier(name) => name.clone(),
        _ => return Err(ScriptError::new("Expected identifier as chunk source".to_string())),
    };

    // Convert value to string
    let value_str = reserve_player_mut(|player| {
        use crate::player::datum_formatting::datum_to_string_for_concat;
        let value_datum = player.get_datum(&value_ref);
        Ok(datum_to_string_for_concat(value_datum, player))
    })?;

    // Perform the insertion operation
    reserve_player_mut(|player| {
        let current_ref = player.globals.get(&source_name)
            .cloned()
            .unwrap_or_else(|| player.alloc_datum(Datum::String(String::new())));
        
        let current_str = player.get_datum(&current_ref).string_value()?;

        let chunk_expr = StringChunkExpr {
            chunk_type,
            start: index,
            end: index,
            item_delimiter: player.movie.item_delimiter,
        };

        // Use the before insertion utility
        let new_string = StringChunkUtils::string_by_putting_before_chunk(
            &current_str,
            &chunk_expr,
            &value_str
        )?;

        let new_string_ref = player.alloc_datum(Datum::String(new_string));
        player.globals.insert(source_name, new_string_ref);

        Ok(DatumRef::Void)
    })
}

async fn handle_put_after_chunk(
    value_ref: DatumRef,
    target: &LingoExpr,
) -> Result<DatumRef, ScriptError> {
    // Extract chunk expression components
    let (chunk_type_str, index_expr, source_expr) = match target {
        LingoExpr::ChunkExpr(chunk_type, index, source) => (chunk_type, index, source),
        _ => return Err(ScriptError::new("Expected chunk expression".to_string())),
    };

    // Evaluate the index
    let index_ref = Box::pin(eval_lingo_expr_ast_runtime(index_expr)).await?;
    let index = reserve_player_mut(|player| {
        player.get_datum(&index_ref).int_value()
    })?;

    let chunk_type = StringChunkType::from(chunk_type_str);

    // Get source variable name
    let source_name = match source_expr.as_ref() {
        LingoExpr::Identifier(name) => name.clone(),
        _ => return Err(ScriptError::new("Expected identifier as chunk source".to_string())),
    };

    // Convert value to string
    let value_str = reserve_player_mut(|player| {
        use crate::player::datum_formatting::datum_to_string_for_concat;
        let value_datum = player.get_datum(&value_ref);
        Ok(datum_to_string_for_concat(value_datum, player))
    })?;

    // Perform the insertion operation
    reserve_player_mut(|player| {
        let current_ref = player.globals.get(&source_name)
            .cloned()
            .unwrap_or_else(|| player.alloc_datum(Datum::String(String::new())));
        
        let current_str = player.get_datum(&current_ref).string_value()?;

        let chunk_expr = StringChunkExpr {
            chunk_type,
            start: index,
            end: index,
            item_delimiter: player.movie.item_delimiter,
        };

        // Use the after insertion utility
        let new_string = StringChunkUtils::string_by_putting_after_chunk(
            &current_str,
            &chunk_expr,
            &value_str
        )?;

        let new_string_ref = player.alloc_datum(Datum::String(new_string));
        player.globals.insert(source_name, new_string_ref);

        Ok(DatumRef::Void)
    })
}

pub async fn eval_lingo_expr_ast_runtime(expr: &LingoExpr) -> Result<DatumRef, ScriptError> {
    match expr {
        LingoExpr::SymbolLiteral(s) => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Symbol(s.to_string()))))
        }
        LingoExpr::StringLiteral(s) => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(s.to_string()))))
        }
        LingoExpr::ListLiteral(items) => {
            let mut datum_items = vec![];
            for item in items {
                let datum = Box::pin(eval_lingo_expr_ast_runtime(item)).await?;
                datum_items.push(datum);
            }
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::List(DatumType::List, datum_items, false)))
            })
        }
        LingoExpr::VoidLiteral => Ok(DatumRef::Void),
        LingoExpr::BoolLiteral(b) => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(if *b { 1 } else { 0 }))))
        }
        LingoExpr::IntLiteral(i) => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(*i))))
        }
        LingoExpr::FloatLiteral(f) => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Float(*f))))
        }
        LingoExpr::PropListLiteral(pairs) => {
            let mut datum_pairs = vec![];
            for (key_expr, value_expr) in pairs {
                let key_datum = Box::pin(eval_lingo_expr_ast_runtime(key_expr)).await?;
                let value_datum = Box::pin(eval_lingo_expr_ast_runtime(value_expr)).await?;
                datum_pairs.push((key_datum, value_datum));
            }
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::PropList(datum_pairs, false))))
        }
        LingoExpr::HandlerCall(handler_name, args) => {
            let mut datum_args = vec![];
            for arg in args {
                let datum = Box::pin(eval_lingo_expr_ast_runtime(arg)).await?;
                datum_args.push(datum);
            }
            // When a breakpoint is active and there's a receiver, try calling the handler on the receiver first
            let receiver_call = reserve_player_mut(|player| {
                if player.current_breakpoint.is_some() && player.scope_count > 0 {
                    let scope_idx = player.eval_scope_index
                        .unwrap_or(player.scope_count - 1) as usize;
                    let scope = &player.scopes[scope_idx];
                    if let Some(receiver_ref) = scope.receiver.clone() {
                        let script_ref = scope.script_ref.clone();
                        if let Some(script) = player.movie.cast_manager.get_script_by_ref(&script_ref) {
                            if script.get_own_handler(handler_name).is_some() {
                                let me_ref = player.alloc_datum(Datum::ScriptInstanceRef(receiver_ref));
                                return Some(me_ref);
                            }
                        }
                    }
                }
                None
            });
            if let Some(me_ref) = receiver_call {
                player_call_datum_handler(&me_ref, handler_name, &datum_args).await
            } else {
                player_call_global_handler(&handler_name, &datum_args).await
            }
        }
        LingoExpr::ObjProp(obj_expr, prop_name) => {
            let obj_datum = Box::pin(eval_lingo_expr_ast_runtime(obj_expr.as_ref())).await?;
            reserve_player_mut(|player| get_obj_prop(player, &obj_datum, prop_name))
        }
        LingoExpr::ListAccess(list_expr, index_expr) => {
            let list_ref = Box::pin(eval_lingo_expr_ast_runtime(list_expr.as_ref())).await?;
            let index_ref = Box::pin(eval_lingo_expr_ast_runtime(index_expr.as_ref())).await?;
            
            reserve_player_mut(|player| {
                let list_datum = player.get_datum(&list_ref);
                let index_datum = player.get_datum(&index_ref);

                // Access list/proplist/point/rect element
                match list_datum {
                    Datum::List(_, items, _) => {
                        let index_num = index_datum.int_value()?;
                        if index_num < 1 || index_num as usize > items.len() {
                            Err(ScriptError::new(format!(
                                "List index {} out of bounds (list has {} items)",
                                index_num,
                                items.len()
                            )))
                        } else {
                            // Lingo uses 1-based indexing, so subtract 1
                            Ok(items[(index_num - 1) as usize].clone())
                        }
                    }
                    // PropList: int index = Nth value, symbol/string = key lookup
                    Datum::PropList(pairs, _) => {
                        PropListUtils::get_at(pairs, &index_ref, &player.allocator)
                    }
                    // Point indexed by number: 1=x, 2=y
                    Datum::Point(point_arr) => {
                        let index_num = index_datum.int_value()?;
                        if index_num < 1 || index_num > 2 {
                            Err(ScriptError::new(format!(
                                "Point index {} out of bounds (must be 1 or 2)", index_num
                            )))
                        } else {
                            Ok(point_arr[(index_num - 1) as usize].clone())
                        }
                    }
                    // Rect indexed by number: 1-4
                    Datum::Rect(rect_arr) => {
                        let index_num = index_datum.int_value()?;
                        if index_num < 1 || index_num > 4 {
                            Err(ScriptError::new(format!(
                                "Rect index {} out of bounds (must be 1-4)", index_num
                            )))
                        } else {
                            Ok(rect_arr[(index_num - 1) as usize].clone())
                        }
                    }
                    _ => Err(ScriptError::new(format!(
                        "Cannot index non-list type: {:?}",
                        list_datum.type_enum()
                    ))),
                }
            })
        }
        LingoExpr::ThePropOf(obj_expr, prop_name) => {
            // Special case: "the number of castMembers of X" should request
            // "number of castMembers" as a compound property from X
            if prop_name == "number" || prop_name == "count" {
                if let LingoExpr::ObjProp(inner_obj, inner_prop) = obj_expr.as_ref() {
                    // We have "the number of <property> of <object>"
                    // Convert to compound property: "number of <property>"
                    let compound_prop = format!("{} of {}", prop_name, inner_prop);
                    let inner_datum = Box::pin(eval_lingo_expr_ast_runtime(inner_obj.as_ref())).await?;
                    return reserve_player_mut(|player| get_obj_prop(player, &inner_datum, &compound_prop));
                }
            }
            
            // For other cases, evaluate obj_expr and get the property
            let obj_datum = Box::pin(eval_lingo_expr_ast_runtime(obj_expr.as_ref())).await?;
            reserve_player_mut(|player| get_obj_prop(player, &obj_datum, prop_name))
        }
        LingoExpr::ColorLiteral(color_ref) => {
            reserve_player_mut(|player| Ok(player.alloc_datum(Datum::ColorRef(color_ref.clone()))))
        }
        LingoExpr::RectLiteral(values) => {
            if values.len() != 1 {
                return Err(ScriptError::new("RectLiteral must have 1 tuple of 4 elements".to_string()));
            }
            let (x_expr, y_expr, w_expr, h_expr) = &values[0];
            
            // Evaluate each component expression
            let x_datum = Box::pin(eval_lingo_expr_ast_runtime(x_expr)).await?;
            let y_datum = Box::pin(eval_lingo_expr_ast_runtime(y_expr)).await?;
            let w_datum = Box::pin(eval_lingo_expr_ast_runtime(w_expr)).await?;
            let h_datum = Box::pin(eval_lingo_expr_ast_runtime(h_expr)).await?;
            
            // Create the Rect datum with DatumRef components
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Rect([x_datum, y_datum, w_datum, h_datum])))
            })
        }
        LingoExpr::PointLiteral(values) => {
            if values.len() != 1 {
                return Err(ScriptError::new("PointLiteral must have 1 tuple of 2 elements".to_string()));
            }
            let (x_expr, y_expr) = &values[0];
            
            // Evaluate each component expression
            let x_datum = Box::pin(eval_lingo_expr_ast_runtime(x_expr)).await?;
            let y_datum = Box::pin(eval_lingo_expr_ast_runtime(y_expr)).await?;
            
            // Create the Point datum with DatumRef components
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::Point([x_datum, y_datum])))
            })
        }
        LingoExpr::MemberRef(member_expr, cast_lib_expr) => {
            // Evaluate member name or number
            let member_id_ref = Box::pin(eval_lingo_expr_ast_runtime(member_expr.as_ref())).await?;

            // Evaluate cast lib (or None if not specified)
            let cast_lib_ref = if let Some(expr) = cast_lib_expr {
                Some(Box::pin(eval_lingo_expr_ast_runtime(expr.as_ref())).await?)
            } else {
                None
            };

            reserve_player_mut(|player| {
                let member_id_datum = player.get_datum(&member_id_ref).clone();

                // Get cast lib datum if specified
                let cast_lib_datum = cast_lib_ref.as_ref().map(|r| player.get_datum(r).clone());

                // Use find_member_ref_by_identifiers for proper member lookup
                // This handles both string names and numeric member IDs
                let member_result = player.movie.cast_manager.find_member_ref_by_identifiers(
                    &member_id_datum,
                    cast_lib_datum.as_ref(),
                    &player.allocator,
                )?;

                let member_ref = match member_result {
                    Some(r) => r,
                    None => {
                        // If cast_lib was specified, create a ref with member 0
                        // Otherwise return invalid ref
                        if let Some(cast_datum) = cast_lib_datum {
                            let cast_lib_num = match cast_datum {
                                Datum::Int(num) => num,
                                Datum::CastLib(num) => num as i32,
                                _ => return Err(ScriptError::new(format!(
                                    "Expected int or castLib, got {:?}",
                                    cast_datum.type_enum()
                                ))),
                            };
                            // Try to get member number for explicit reference
                            let member_num = member_id_datum.int_value().unwrap_or(0);
                            super::cast_lib::CastMemberRef {
                                cast_lib: cast_lib_num,
                                cast_member: member_num,
                            }
                        } else {
                            super::cast_lib::INVALID_CAST_MEMBER_REF
                        }
                    }
                };

                Ok(player.alloc_datum(Datum::CastMember(member_ref)))
            })
        }
        LingoExpr::Identifier(ident_name) => {
            reserve_player_mut(|player| get_eval_top_level_prop(player, ident_name))
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
                LingoExpr::Identifier(ident_name) => reserve_player_mut(|player| {
                    if ident_name.starts_with("the ") {
                        let prop_name = &ident_name[4..];
                        let right_datum_value = player.get_datum(&right_datum).clone();
                        player.set_movie_prop(prop_name, right_datum_value)?;
                        Ok(right_datum)
                    } else {
                        player.globals.insert(ident_name.to_owned(), right_datum.clone());
                        Ok(right_datum)
                    }
                }),
                LingoExpr::ObjProp(obj_expr, prop_name) => {
                    let obj_datum =
                        Box::pin(eval_lingo_expr_ast_runtime(obj_expr.as_ref())).await?;
                    player_set_obj_prop(&obj_datum, prop_name, &right_datum).await?;
                    Ok(DatumRef::Void)
                }
                LingoExpr::ThePropOf(obj_expr, prop_name) => {
                    let obj_datum =
                        Box::pin(eval_lingo_expr_ast_runtime(obj_expr.as_ref())).await?;
                    player_set_obj_prop(&obj_datum, prop_name, &right_datum).await?;
                    Ok(DatumRef::Void)
                }
                // Handle bracket-indexed assignment: list[index] = value
                LingoExpr::ListAccess(list_expr, index_expr) => {
                    let list_ref = Box::pin(eval_lingo_expr_ast_runtime(list_expr.as_ref())).await?;
                    let index_ref = Box::pin(eval_lingo_expr_ast_runtime(index_expr.as_ref())).await?;
                    reserve_player_mut(|player| {
                        let list_datum = player.get_datum(&list_ref);
                        let index_datum = player.get_datum(&index_ref);
                        match list_datum {
                            Datum::List(_, items, _) => {
                                let index_num = index_datum.int_value()?;
                                let adjusted = if index_num >= 1 { (index_num - 1) as usize } else { 0 };
                                if adjusted >= items.len() {
                                    return Err(ScriptError::new(format!(
                                        "List index {} out of bounds (list has {} items)",
                                        index_num, items.len()
                                    )));
                                }
                                let (_, list_vec, _) = player.get_datum_mut(&list_ref).to_list_mut()?;
                                list_vec[adjusted] = right_datum.clone();
                                Ok(right_datum)
                            }
                            Datum::PropList(..) => {
                                let formatted_key = format_datum(&index_ref, &player);
                                PropListUtils::set_at(player, &list_ref, &index_ref, &right_datum, formatted_key)?;
                                Ok(right_datum)
                            }
                            Datum::Point(_) => {
                                let index_num = index_datum.int_value()?;
                                let adjusted = if index_num >= 1 { (index_num - 1) as usize } else { 0 };
                                if adjusted >= 2 {
                                    return Err(ScriptError::new(format!(
                                        "Point index {} out of bounds", index_num
                                    )));
                                }
                                let point = player.get_datum_mut(&list_ref).to_point_mut()?;
                                point[adjusted] = right_datum.clone();
                                Ok(right_datum)
                            }
                            Datum::Rect(_) => {
                                let index_num = index_datum.int_value()?;
                                let adjusted = if index_num >= 1 { (index_num - 1) as usize } else { 0 };
                                if adjusted >= 4 {
                                    return Err(ScriptError::new(format!(
                                        "Rect index {} out of bounds", index_num
                                    )));
                                }
                                let rect = player.get_datum_mut(&list_ref).to_rect_mut()?;
                                rect[adjusted] = right_datum.clone();
                                Ok(right_datum)
                            }
                            _ => Err(ScriptError::new(format!(
                                "Cannot assign to index of type: {}",
                                list_datum.type_str()
                            ))),
                        }
                    })
                }
                _ => Err(ScriptError::new(
                    "Invalid assignment left-hand side".to_string(),
                )),
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
        LingoExpr::Multiply(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs.as_ref())).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs.as_ref())).await?;
            reserve_player_mut(|player| {
                let result = multiply_datums(left, right, player)?;
                Ok(player.alloc_datum(result))
            })
        }
        LingoExpr::Divide(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs.as_ref())).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs.as_ref())).await?;
            reserve_player_mut(|player| {
                let result = divide_datums(left, right, player)?;
                Ok(player.alloc_datum(result))
            })
        }
        LingoExpr::Join(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs.as_ref())).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs.as_ref())).await?;
            reserve_player_mut(|player| {
                let result = StringBytecodeHandler::concat_datums(left, right, player, false)?;
                Ok(result)
            })
        }
        LingoExpr::JoinPad(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs.as_ref())).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs.as_ref())).await?;
            reserve_player_mut(|player| {
                let result = StringBytecodeHandler::concat_datums(left, right, player, true)?;
                Ok(result)
            })
        }
        LingoExpr::PutDisplay(value_expr) => {
            let value = Box::pin(eval_lingo_expr_ast_runtime(value_expr)).await?;
            reserve_player_mut(|player| {
                use crate::player::handlers::manager::BuiltInHandlerManager;
                BuiltInHandlerManager::put(&vec![value])?;
                Ok(DatumRef::Void)
            })
        }
        LingoExpr::PutInto(expr, target) => {
            // Evaluate the expression
            let value = Box::pin(eval_lingo_expr_ast_runtime(expr)).await?;
            
            // Get target variable name
            let target_name = match target.as_ref() {
                LingoExpr::Identifier(name) => name.clone(),
                LingoExpr::ChunkExpr(..) => {
                    // Handle chunk assignment - use existing PutChunk logic
                    return handle_put_into_chunk(value, target).await;
                },
                _ => return Err(ScriptError::new("Invalid put into target".to_string())),
            };
            
            // Set the global variable
            reserve_player_mut(|player| {
                player.globals.insert(target_name, value.clone());
                Ok(DatumRef::Void)
            })
        },
        LingoExpr::PutBefore(expr, target) => {
            // Evaluate the expression
            let value = Box::pin(eval_lingo_expr_ast_runtime(expr)).await?;
            
            let target_name = match target.as_ref() {
                LingoExpr::Identifier(name) => name.clone(),
                LingoExpr::ChunkExpr(..) => {
                    // Handle chunk before - use existing logic
                    return handle_put_before_chunk(value, target).await;
                },
                _ => return Err(ScriptError::new("Invalid put before target".to_string())),
            };
            
            // Get current value, concatenate value before it, set back
            reserve_player_mut(|player| {
                let current = player.globals.get(&target_name)
                    .cloned()
                    .unwrap_or_else(|| player.alloc_datum(Datum::String(String::new())));
                
                let value_datum = player.get_datum(&value);
                let current_datum = player.get_datum(&current);
                
                // Use datum_to_string_for_concat for proper conversion
                use crate::player::datum_formatting::datum_to_string_for_concat;
                let value_str = datum_to_string_for_concat(value_datum, player);
                let current_str = datum_to_string_for_concat(current_datum, player);
                
                let result = Datum::String(format!("{}{}", value_str, current_str));
                let result_ref = player.alloc_datum(result);
                
                player.globals.insert(target_name, result_ref);
                Ok(DatumRef::Void)
            })
        },
        LingoExpr::PutAfter(expr, target) => {
            // Evaluate the expression
            let value = Box::pin(eval_lingo_expr_ast_runtime(expr)).await?;
            
            let target_name = match target.as_ref() {
                LingoExpr::Identifier(name) => name.clone(),
                LingoExpr::ChunkExpr(..) => {
                    // Handle chunk after - use existing logic
                    return handle_put_after_chunk(value, target).await;
                },
                _ => return Err(ScriptError::new("Invalid put after target".to_string())),
            };
            
            // Get current value, concatenate value after it, set back
            reserve_player_mut(|player| {
                let current = player.globals.get(&target_name)
                    .cloned()
                    .unwrap_or_else(|| player.alloc_datum(Datum::String(String::new())));
                
                let value_datum = player.get_datum(&value);
                let current_datum = player.get_datum(&current);
                
                use crate::player::datum_formatting::datum_to_string_for_concat;
                let current_str = datum_to_string_for_concat(current_datum, player);
                let value_str = datum_to_string_for_concat(value_datum, player);
                
                let result = Datum::String(format!("{}{}", current_str, value_str));
                let result_ref = player.alloc_datum(result);
                
                player.globals.insert(target_name, result_ref);
                Ok(DatumRef::Void)
            })
        },
        LingoExpr::DeleteChunk(chunk_target) => {
            // "delete char 1 of s" - delete a chunk from a variable
            let (chunk_type_str, index_expr, source_expr) = match chunk_target.as_ref() {
                LingoExpr::ChunkExpr(chunk_type, index, source) => (chunk_type, index, source),
                _ => return Err(ScriptError::new("Expected chunk expression after delete".to_string())),
            };

            let index_ref = Box::pin(eval_lingo_expr_ast_runtime(index_expr)).await?;
            let index = reserve_player_mut(|player| {
                player.get_datum(&index_ref).int_value()
            })?;

            let chunk_type = StringChunkType::from(chunk_type_str);

            let source_name = match source_expr.as_ref() {
                LingoExpr::Identifier(name) => name.clone(),
                _ => return Err(ScriptError::new("Expected identifier as chunk source for delete".to_string())),
            };

            reserve_player_mut(|player| {
                let current_ref = player.globals.get(&source_name)
                    .cloned()
                    .unwrap_or_else(|| player.alloc_datum(Datum::String(String::new())));

                let current_str = player.get_datum(&current_ref).string_value()?;

                let chunk_expr = StringChunkExpr {
                    chunk_type,
                    start: index,
                    end: 0,
                    item_delimiter: player.movie.item_delimiter,
                };

                let new_string = StringChunkUtils::string_by_deleting_chunk(
                    &current_str,
                    &chunk_expr,
                )?;

                let new_string_ref = player.alloc_datum(Datum::String(new_string));
                player.globals.insert(source_name, new_string_ref);

                Ok(DatumRef::Void)
            })
        },
        LingoExpr::And(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs)).await?;
            reserve_player_mut(|player| {
                let left_val = player.get_datum(&left).int_value()?;
                let right_val = player.get_datum(&right).int_value()?;
                let result = if left_val != 0 && right_val != 0 { 1 } else { 0 };
                Ok(player.alloc_datum(Datum::Int(result)))
            })
        }
        LingoExpr::Or(lhs, rhs) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(lhs)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(rhs)).await?;
            reserve_player_mut(|player| {
                let left_val = player.get_datum(&left).int_value()?;
                let right_val = player.get_datum(&right).int_value()?;
                let result = if left_val != 0 || right_val != 0 { 1 } else { 0 };
                Ok(player.alloc_datum(Datum::Int(result)))
            })
        }
        LingoExpr::Not(operand) => {
            let value = Box::pin(eval_lingo_expr_ast_runtime(operand)).await?;
            reserve_player_mut(|player| {
                let int_val = player.get_datum(&value).int_value()?;
                let result = if int_val == 0 { 1 } else { 0 };
                Ok(player.alloc_datum(Datum::Int(result)))
            })
        }
        LingoExpr::ChunkExpr(chunk_type, index_expr, source_expr) => {
            // Evaluate the source expression to get a string
            let source_ref = Box::pin(eval_lingo_expr_ast_runtime(source_expr)).await?;
            let source_string = reserve_player_mut(|player| {
                player.get_datum(&source_ref).string_value()
            })?;
            
            // Evaluate the index expression
            let index_ref = Box::pin(eval_lingo_expr_ast_runtime(index_expr)).await?;
            let index = reserve_player_mut(|player| {
                player.get_datum(&index_ref).int_value()
            })?;
            
            // Convert chunk type string to StringChunkType
            let chunk_type_enum = StringChunkType::from(chunk_type);
            
            // Create chunk expression
            let chunk_expr = reserve_player_mut(|player| {
                Ok(StringChunkExpr {
                    chunk_type: chunk_type_enum,
                    start: index,
                    end: index,
                    item_delimiter: player.movie.item_delimiter,
                })
            })?;
            
            // Extract the chunk
            let result_string = StringChunkUtils::resolve_chunk_expr_string(&source_string, &chunk_expr)?;
            
            reserve_player_mut(|player| {
                Ok(player.alloc_datum(Datum::String(result_string)))
            })
        }
        LingoExpr::Eq(left, right) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(left)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(right)).await?;
            reserve_player_mut(|player| {
                let left_datum = player.get_datum(&left);
                let right_datum = player.get_datum(&right);
                let result = crate::player::compare::datum_equals(
                    left_datum, 
                    right_datum, 
                    &player.allocator
                )?;
                Ok(player.alloc_datum(Datum::Int(if result { 1 } else { 0 })))
            })
        },
        LingoExpr::Ne(left, right) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(left)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(right)).await?;
            reserve_player_mut(|player| {
                let left_datum = player.get_datum(&left);
                let right_datum = player.get_datum(&right);
                let result = !crate::player::compare::datum_equals(
                    left_datum, 
                    right_datum, 
                    &player.allocator
                )?;
                Ok(player.alloc_datum(Datum::Int(if result { 1 } else { 0 })))
            })
        },
        LingoExpr::Lt(left, right) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(left)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(right)).await?;
            reserve_player_mut(|player| {
                let left_datum = player.get_datum(&left);
                let right_datum = player.get_datum(&right);
                let result = crate::player::compare::datum_less_than(
                    left_datum, 
                    right_datum,
                    &player.allocator
                )?;
                Ok(player.alloc_datum(Datum::Int(if result { 1 } else { 0 })))
            })
        },
        LingoExpr::Gt(left, right) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(left)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(right)).await?;
            reserve_player_mut(|player| {
                let left_datum = player.get_datum(&left);
                let right_datum = player.get_datum(&right);
                let result = crate::player::compare::datum_greater_than(
                    left_datum, 
                    right_datum,
                    &player.allocator
                )?;
                Ok(player.alloc_datum(Datum::Int(if result { 1 } else { 0 })))
            })
        },
        LingoExpr::Le(left, right) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(left)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(right)).await?;
            reserve_player_mut(|player| {
                let left_datum = player.get_datum(&left);
                let right_datum = player.get_datum(&right);
                let is_eq = crate::player::compare::datum_equals(
                    left_datum, 
                    right_datum, 
                    &player.allocator
                )?;
                let is_lt = crate::player::compare::datum_less_than(
                    left_datum, 
                    right_datum,
                    &player.allocator
                )?;
                Ok(player.alloc_datum(Datum::Int(if is_eq || is_lt { 1 } else { 0 })))
            })
        },
        LingoExpr::Ge(left, right) => {
            let left = Box::pin(eval_lingo_expr_ast_runtime(left)).await?;
            let right = Box::pin(eval_lingo_expr_ast_runtime(right)).await?;
            reserve_player_mut(|player| {
                let left_datum = player.get_datum(&left);
                let right_datum = player.get_datum(&right);
                let is_eq = crate::player::compare::datum_equals(
                    left_datum, 
                    right_datum, 
                    &player.allocator
                )?;
                let is_gt = crate::player::compare::datum_greater_than(
                    left_datum, 
                    right_datum,
                    &player.allocator
                )?;
                Ok(player.alloc_datum(Datum::Int(if is_eq || is_gt { 1 } else { 0 })))
            })
        },
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
            let error_msg = format!("eval_lingo_expr_static parse error: {}", ascii_safe(&e.to_string()));
            error!("{}", error_msg);
            web_sys::console::error_1(&error_msg.clone().into());
            Err(ScriptError::new(error_msg))
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
        Err(e) => Err(ScriptError::new(ascii_safe(&e.to_string()))),
    }
}

pub async fn eval_lingo_expr_runtime(expr: String) -> Result<DatumRef, ScriptError> {
    let ast = parse_lingo_expr_ast_runtime(Rule::eval_expr, expr)?;
    eval_lingo_expr_ast_runtime(&ast).await
}

fn create_lingo_pratt_parser() -> PrattParser<Rule> {
    PrattParser::new()
        .op(Op::infix(Rule::or_op, Assoc::Left))              // Lowest: or
        .op(Op::infix(Rule::and_op, Assoc::Left))             // and
        .op(Op::prefix(Rule::not_op))                         // not (prefix)
        .op(Op::infix(Rule::eq_op, Assoc::Left)               // = comparison
            | Op::infix(Rule::ne_op, Assoc::Left)             // <>
            | Op::infix(Rule::lt_op, Assoc::Left)             // 
            | Op::infix(Rule::gt_op, Assoc::Left)             // >
            | Op::infix(Rule::le_op, Assoc::Left)             // <=
            | Op::infix(Rule::ge_op, Assoc::Left))            // >=
        .op(Op::infix(Rule::join, Assoc::Left)                // & concatenation
            | Op::infix(Rule::join_pad, Assoc::Left))         // && padded concat
        .op(Op::infix(Rule::add, Assoc::Left)                 // +, -
            | Op::infix(Rule::subtract, Assoc::Left))
        .op(Op::infix(Rule::multiply, Assoc::Left)            // *, /
            | Op::infix(Rule::divide, Assoc::Left))
        .op(Op::infix(Rule::obj_prop, Assoc::Left)            // Highest: .
            | Op::postfix(Rule::list_index))                  // and [index]
}

pub async fn eval_lingo_command(expr: String) -> Result<DatumRef, ScriptError> {
    let ast = parse_lingo_expr_ast_runtime(Rule::command_eval_expr, expr)?;
    eval_lingo_expr_ast_runtime(&ast).await
}

// Helper functions for testing config parsing without requiring player instance
/// Parse a config value to check if it's valid Lingo syntax
/// Returns Ok if the value can be parsed, Err with parse error message if not
pub fn test_parse_lingo_value(value_str: &str) -> Result<(), String> {
    LingoParser::parse(Rule::eval_expr, value_str)
        .map(|_| ())
        .map_err(|e| format!("{}", e))
}

/// Parse a config key to check if it's valid
/// Returns Ok if the key can be parsed, Err with parse error message if not
pub fn test_parse_config_key(key_str: &str) -> Result<(), String> {
    LingoParser::parse(Rule::config_key, key_str)
        .map(|_| ())
        .map_err(|e| format!("{}", e))
}
