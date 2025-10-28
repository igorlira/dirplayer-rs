use log::error;
use pest::{iterators::Pair, Parser};

use crate::{
    console_error,
    director::lingo::datum::{datum_bool, Datum, DatumType},
    js_api::ascii_safe,
};

use super::{sprite::ColorRef, DatumRef, DirPlayer, ScriptError};

#[derive(Parser)]
#[grammar = "lingo.pest"]
struct LingoParser;

fn tokenize_lingo(_expr: &String) -> Vec<String> {
    [].to_vec()
}

pub fn eval_lingo_pair(pair: Pair<Rule>, player: &mut DirPlayer) -> Result<DatumRef, ScriptError> {
    // warn!("eval_lingo_expr: {:?}", pair);

    let inner_rule = pair.as_rule();
    match pair.as_rule() {
        Rule::expr => eval_lingo_pair(pair.into_inner().next().unwrap(), player),
        Rule::list => eval_lingo_pair(pair.into_inner().next().unwrap(), player),
        Rule::multi_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let result = eval_lingo_pair(inner_pair, player)?;
                result_vec.push(result);
            }
            Ok(player.alloc_datum(Datum::List(DatumType::List, result_vec, false)))
        }
        Rule::string => {
            let str_val = pair.into_inner().next().unwrap().as_str();
            Ok(player.alloc_datum(Datum::String(str_val.to_owned())))
        }
        Rule::prop_list => eval_lingo_pair(pair.into_inner().next().unwrap(), player),
        Rule::multi_prop_list => {
            let mut result_vec = vec![];
            for inner_pair in pair.into_inner() {
                let mut pair_inner = inner_pair.into_inner();
                let key = eval_lingo_pair(pair_inner.next().unwrap(), player)?;
                let value = eval_lingo_pair(pair_inner.next().unwrap(), player)?;

                result_vec.push((key, value));
            }
            Ok(player.alloc_datum(Datum::PropList(result_vec, false)))
        }
        Rule::empty_prop_list => Ok(player.alloc_datum(Datum::PropList(vec![], false))),
        Rule::number_int => {
            Ok(player.alloc_datum(Datum::Int(pair.as_str().parse::<i32>().unwrap())))
        }
        Rule::number_float => {
            Ok(player.alloc_datum(Datum::Float(pair.as_str().parse::<f32>().unwrap())))
        }
        Rule::rect => {
            let mut inner = pair.into_inner();
            Ok(player.alloc_datum(Datum::IntRect((
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
            ))))
        }
        Rule::rgb_num_color => {
            let mut inner = pair.into_inner();
            Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(
                inner.next().unwrap().as_str().parse::<u8>().unwrap(),
                inner.next().unwrap().as_str().parse::<u8>().unwrap(),
                inner.next().unwrap().as_str().parse::<u8>().unwrap(),
            ))))
        }
        Rule::rgb_str_color => {
            let mut inner = pair.into_inner();
            let str_inner = inner.next().unwrap().into_inner().next().unwrap();
            let str_val = str_inner.as_str();
            Ok(player.alloc_datum(Datum::ColorRef(ColorRef::from_hex(str_val))))
        }
        Rule::rgb_color => {
            let inner = pair.into_inner().next().unwrap();
            eval_lingo_pair(inner, player)
        }
        Rule::symbol => {
            let str_val = pair.into_inner().next().unwrap().as_str();
            Ok(player.alloc_datum(Datum::Symbol(str_val.to_owned())))
        }
        Rule::bool_true => Ok(player.alloc_datum(datum_bool(true))),
        Rule::bool_false => Ok(player.alloc_datum(datum_bool(false))),
        Rule::void => Ok(DatumRef::Void),
        Rule::string_empty => Ok(player.alloc_datum(Datum::String("".to_owned()))),
        Rule::nohash_symbol => Ok(player.alloc_datum(Datum::Symbol(pair.as_str().to_owned()))),
        Rule::point => {
            let mut inner = pair.into_inner();
            Ok(player.alloc_datum(Datum::IntPoint((
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
                inner.next().unwrap().as_str().parse::<i32>().unwrap(),
            ))))
        }
        Rule::empty_list => Ok(player.alloc_datum(Datum::List(DatumType::List, vec![], false))),
        _ => Err(ScriptError::new(format!(
            "Invalid Lingo expression {:?}",
            inner_rule
        ))),
    }
}

pub fn eval_lingo(expr: String, player: &mut DirPlayer) -> Result<DatumRef, ScriptError> {
    let _tokens = tokenize_lingo(&expr);
    match LingoParser::parse(Rule::eval_expr, expr.as_str()) {
        Ok(parse_result) => {
            let expr_pair = &parse_result.enumerate().next().unwrap();
            eval_lingo_pair(expr_pair.1.clone(), player)
        }
        Err(e) => {
            error!("Lingo parse error: {}", ascii_safe(&e.to_string()));
            Ok(DatumRef::Void)
        }
    }
}
