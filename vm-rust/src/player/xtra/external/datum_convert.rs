use std::collections::VecDeque;

use serde_json::{json, Value};

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{reserve_player_mut, reserve_player_ref, DatumRef},
};

/// Serialize a list of DatumRef args to a JSON array string for plugin calls.
pub fn args_to_json(args: &[DatumRef]) -> String {
    let arr: Vec<Value> = args
        .iter()
        .map(|r| reserve_player_ref(|p| datum_to_json(p.get_datum(r))))
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
}

/// Convert a single Datum to a serde_json Value using the WIT datum variant encoding.
pub fn datum_to_json(datum: &Datum) -> Value {
    match datum {
        Datum::Void | Datum::Null => json!({"type": "void"}),
        Datum::Int(n) => json!({"type": "int", "value": n}),
        Datum::Float(f) => json!({"type": "float", "value": f}),
        Datum::String(s) => json!({"type": "string", "value": s}),
        Datum::Symbol(s) => json!({"type": "symbol", "value": s}),
        Datum::List(_, items, _) => {
            let elems: Vec<Value> = items
                .iter()
                .map(|r| reserve_player_ref(|p| datum_to_json(p.get_datum(r))))
                .collect();
            json!({"type": "list", "value": elems})
        }
        Datum::PropList(pairs, _) => {
            let encoded: Vec<Value> = pairs
                .iter()
                .map(|(k, v)| {
                    let kj = reserve_player_ref(|p| datum_to_json(p.get_datum(k)));
                    let vj = reserve_player_ref(|p| datum_to_json(p.get_datum(v)));
                    json!([kj, vj])
                })
                .collect();
            json!({"type": "prop-list", "value": encoded})
        }
        Datum::Point([x, y], _) => json!({"type": "point", "value": [x, y]}),
        Datum::Rect([l, t, r, b], _) => json!({"type": "rect", "value": [l, t, r, b]}),
        Datum::XtraInstance(name, id) => {
            json!({"type": "xtra-ref", "value": {"name": name, "id": id}})
        }
        // All other types are not representable across the plugin boundary; pass as void.
        _ => json!({"type": "void"}),
    }
}

/// Parse the JSON result from a plugin call and allocate a new DatumRef.
pub fn json_to_datum_ref(json_str: &str) -> Result<DatumRef, String> {
    let v: Value = serde_json::from_str(json_str)
        .map_err(|e| format!("xtra result parse error: {e}"))?;
    let datum = json_value_to_datum(&v)?;
    Ok(reserve_player_mut(|p| p.alloc_datum(datum)))
}

fn json_value_to_datum(v: &Value) -> Result<Datum, String> {
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("void");
    match typ {
        "void" => Ok(Datum::Void),
        "int" => {
            let n = v["value"]
                .as_i64()
                .ok_or_else(|| "xtra result: int value missing".to_string())?;
            Ok(Datum::Int(n as i32))
        }
        "float" => {
            let f = v["value"]
                .as_f64()
                .ok_or_else(|| "xtra result: float value missing".to_string())?;
            Ok(Datum::Float(f))
        }
        "string" => {
            let s = v["value"]
                .as_str()
                .ok_or_else(|| "xtra result: string value missing".to_string())?
                .to_string();
            Ok(Datum::String(s))
        }
        "symbol" => {
            let s = v["value"]
                .as_str()
                .ok_or_else(|| "xtra result: symbol value missing".to_string())?
                .to_string();
            Ok(Datum::Symbol(s))
        }
        "bool" => {
            let b = v["value"]
                .as_bool()
                .ok_or_else(|| "xtra result: bool value missing".to_string())?;
            Ok(Datum::Int(if b { 1 } else { 0 }))
        }
        "list" => {
            let arr = v["value"]
                .as_array()
                .ok_or_else(|| "xtra result: list value missing".to_string())?;
            let mut items: VecDeque<DatumRef> = VecDeque::new();
            for elem in arr {
                let datum = json_value_to_datum(elem)?;
                let r = reserve_player_mut(|p| p.alloc_datum(datum));
                items.push_back(r);
            }
            Ok(Datum::List(DatumType::List, items, false))
        }
        "prop-list" => {
            let arr = v["value"]
                .as_array()
                .ok_or_else(|| "xtra result: prop-list value missing".to_string())?;
            let mut pairs: VecDeque<(DatumRef, DatumRef)> = VecDeque::new();
            for pair in arr {
                let elems = pair
                    .as_array()
                    .ok_or_else(|| "xtra result: prop-list pair must be array".to_string())?;
                if elems.len() != 2 {
                    return Err("xtra result: prop-list pair must have 2 elements".to_string());
                }
                let k = json_value_to_datum(&elems[0])?;
                let v = json_value_to_datum(&elems[1])?;
                let kr = reserve_player_mut(|p| p.alloc_datum(k));
                let vr = reserve_player_mut(|p| p.alloc_datum(v));
                pairs.push_back((kr, vr));
            }
            Ok(Datum::PropList(pairs, false))
        }
        "point" => {
            let arr = v["value"]
                .as_array()
                .ok_or_else(|| "xtra result: point value missing".to_string())?;
            if arr.len() != 2 {
                return Err("xtra result: point must have 2 elements".to_string());
            }
            let x = arr[0].as_f64().unwrap_or(0.0);
            let y = arr[1].as_f64().unwrap_or(0.0);
            Ok(Datum::Point([x, y], 0))
        }
        "rect" => {
            let arr = v["value"]
                .as_array()
                .ok_or_else(|| "xtra result: rect value missing".to_string())?;
            if arr.len() != 4 {
                return Err("xtra result: rect must have 4 elements".to_string());
            }
            let vals: Vec<f64> = arr.iter().map(|e| e.as_f64().unwrap_or(0.0)).collect();
            Ok(Datum::Rect([vals[0], vals[1], vals[2], vals[3]], 0))
        }
        "xtra-ref" => {
            let name = v["value"]["name"]
                .as_str()
                .ok_or_else(|| "xtra result: xtra-ref name missing".to_string())?
                .to_string();
            let id = v["value"]["id"]
                .as_u64()
                .ok_or_else(|| "xtra result: xtra-ref id missing".to_string())? as u32;
            Ok(Datum::XtraInstance(name, id))
        }
        other => Err(format!("xtra result: unknown datum type '{other}'")),
    }
}
