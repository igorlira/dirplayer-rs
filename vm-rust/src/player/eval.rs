use log::error;
use pest::{iterators::{Pair, Pairs}, Parser};

use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType}, js_api::ascii_safe, player::{bytecode::get_set::GetSetUtils, player_call_global_handler, reserve_player_mut}
};

use super::{sprite::ColorRef, DatumRef, ScriptError};

#[derive(Parser)]
#[grammar = "lingo.pest"]
struct LingoParser;

fn tokenize_lingo(_expr: &String) -> Vec<String> {
    [].to_vec()
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

/// Evaluate a dynamic Lingo expression at runtime.
pub async fn eval_lingo_pair_runtime(pair: Pair<'_, Rule>) -> Result<DatumRef, ScriptError> {
    // warn!("eval_lingo_expr: {:?}", pair);

    let inner_rule = pair.as_rule();
    match pair.as_rule() {
        Rule::expr => Box::pin(eval_lingo_pair_runtime(pair.into_inner().next().unwrap())).await,
        Rule::list => Box::pin(eval_lingo_pair_runtime(pair.into_inner().next().unwrap())).await,
        Rule::multi_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let result = Box::pin(eval_lingo_pair_runtime(inner_pair)).await?;
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
        Rule::prop_list => Box::pin(eval_lingo_pair_runtime(pair.into_inner().next().unwrap())).await,
        Rule::multi_prop_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let mut pair_inner = inner_pair.into_inner();
                let key = Box::pin(eval_lingo_pair_runtime(pair_inner.next().unwrap())).await?;
                let value = Box::pin(eval_lingo_pair_runtime(pair_inner.next().unwrap())).await?;

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
            Box::pin(eval_lingo_pair_runtime(inner)).await
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
        Rule::handler_call | Rule::command_inline => eval_lingo_call(pair.into_inner()).await,
        Rule::ident => {
            let str_val = pair.as_str();
            reserve_player_mut(|player| {
                if let Some(global_ref) = player.globals.get(str_val) {
                    Ok(global_ref.clone())
                } else {
                    let result = GetSetUtils::get_top_level_prop(player, str_val)?;
                    Ok(player.alloc_datum(result))
                }
            })
        }
        _ => Err(ScriptError::new(format!(
            "Invalid runtime Lingo expression {:?}",
            inner_rule
        ))),
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

pub async fn eval_lingo_expr_runtime(expr: String) -> Result<DatumRef, ScriptError> {
    let _tokens = tokenize_lingo(&expr);
    match LingoParser::parse(Rule::eval_expr, expr.as_str()) {
        Ok(parse_result) => {
            let expr_pair = &parse_result.enumerate().next().unwrap();
            eval_lingo_pair_runtime(expr_pair.1.clone()).await
        }
        Err(e) => {
            Err(ScriptError::new(ascii_safe(&e.to_string())))
        }
    }
}

pub async fn eval_lingo_command(expr: String) -> Result<DatumRef, ScriptError> {
    let _tokens = tokenize_lingo(&expr);
    match LingoParser::parse(Rule::command_eval_expr, expr.as_str()) {
        Ok(parse_result) => {
            let expr_pair = &parse_result.enumerate().next().unwrap();
            eval_lingo_pair_runtime(expr_pair.1.clone()).await
        }
        Err(e) => {
            Err(ScriptError::new(format!("eval_lingo_command parse error: {}", ascii_safe(&e.to_string()))))
        }
    }
}

async fn eval_lingo_call(pair: Pairs<'_, Rule>) -> Result<DatumRef, ScriptError> {
    let mut command_container = pair.into_iter();

    let handler_name = command_container.next().unwrap().as_str();
    let mut args = vec![];

    if let Some(args_pair) = command_container.next() {
        for arg_pair in args_pair.into_inner() {
            let arg = Box::pin(eval_lingo_pair_runtime(arg_pair)).await?;
            args.push(arg);
        }
    }

    player_call_global_handler(&handler_name.to_string(), &args).await
}
