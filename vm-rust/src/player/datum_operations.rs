use std::cmp::min;
use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{datum_formatting::{format_datum, datum_to_string_for_concat}, datum_ref::DatumRef},
};

use super::{sprite::ColorRef, DirPlayer, ScriptError};

pub fn add_datums(left: Datum, right: Datum, player: &mut DirPlayer) -> Result<Datum, ScriptError> {
    match (&left, &right) {
        (Datum::Void, some) => Ok(some.clone()),
        (some, Datum::Void) => Ok(some.clone()),
        (Datum::Int(a), Datum::Int(b)) => Ok(Datum::Int(a + b)),
        (Datum::Float(a), Datum::Float(b)) => Ok(Datum::Float(a + b)),
        (Datum::Float(a), Datum::Int(b)) => Ok(Datum::Float(a + (*b as f64))),
        (Datum::Int(a), Datum::Float(b)) => Ok(Datum::Float((*a as f64) + b)),
        (Datum::Rect(a), Datum::Rect(b)) => {
            let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..4 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let sum = add_datums(a_val, b_val, player)?;
                result[i] = player.alloc_datum(sum);
            }
            Ok(Datum::Rect(result))
        }
        (Datum::Rect(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 4 {
                let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
                for i in 0..4 {
                    let a_val = player.get_datum(&a[i]).clone();
                    let b_val = player.get_datum(&ref_list[i]).clone();
                    let sum = add_datums(a_val, b_val, player)?;
                    result[i] = player.alloc_datum(sum);
                }
                Ok(Datum::Rect(result))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for add_datums: {}",
                    ref_list.len()
                )))
            }
        }
        // Vector combinations
        (Datum::Vector(a), Datum::Vector(b)) => {
            Ok(Datum::Vector([a[0] + b[0], a[1] + b[1], a[2] + b[2]]))
        }
        (Datum::Vector(a), Datum::Int(b)) => Ok(Datum::Vector([
            a[0] + *b as f64,
            a[1] + *b as f64,
            a[2] + *b as f64,
        ])),
        (Datum::Vector(a), Datum::Float(b)) => Ok(Datum::Vector([a[0] + *b, a[1] + *b, a[2] + *b])),
        (Datum::Int(a), Datum::Vector(b)) => Ok(Datum::Vector([
            *a as f64 + b[0],
            *a as f64 + b[1],
            *a as f64 + b[2],
        ])),
        (Datum::Float(a), Datum::Vector(b)) => Ok(Datum::Vector([*a + b[0], *a + b[1], *a + b[2]])),

        // Vector + List element-wise (3 elements)
        (Datum::Vector(a), Datum::List(_, list, _)) if list.len() == 3 => {
            let mut result = [0.0; 3];
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => *n as f64,
                    Datum::Float(f) => *f,
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot add Vector to non-numeric list element".to_string(),
                        ))
                    }
                };
                result[i] = a[i] + val;
            }
            Ok(Datum::Vector(result))
        }
        (Datum::List(_, list, _), Datum::Vector(b)) if list.len() == 3 => {
            let mut result = VecDeque::with_capacity(3);
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => Datum::Float(*n as f64 + b[i]),
                    Datum::Float(f) => Datum::Float(*f + b[i]),
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot add list element to Vector".to_string(),
                        ))
                    }
                };
                result.push_back(player.alloc_datum(val));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::List(_, list_a, _), Datum::List(_, list_b, _)) => {
            let intersection_count = min(list_a.len(), list_b.len());
            let mut result = VecDeque::with_capacity(intersection_count);
            for i in 0..intersection_count {
                let a = player.get_datum(&list_a[i]).clone();
                let b = player.get_datum(&list_b[i]).clone();
                let result_datum = add_datums(a, b, player)?;
                result.push_back(player.alloc_datum(result_datum));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::List(_, list, _), Datum::Int(i)) => {
            let mut result_refs = VecDeque::new();
            for r in list {
                let datum = player.get_datum(r);
                let result_datum = match datum {
                    Datum::Int(n) => Datum::Int(n + i),
                    Datum::Float(n) => Datum::Float(n + *i as f64),
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Invalid list element for add_datums: {}",
                            r
                        )))
                    }
                };
                result_refs.push_back(player.alloc_datum(result_datum));
            }
            Ok(Datum::List(DatumType::List, result_refs, false))
        }
        (Datum::String(s), Datum::List(_, list, _)) => {
            let formatted = list
                .iter()
                .map(|r| datum_to_string_for_concat(player.get_datum(r), player))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(Datum::String(format!("{}{}", s, formatted)))
        }
        (Datum::List(_, list, _), Datum::String(s)) => {
            let formatted = list
                .iter()
                .map(|r| datum_to_string_for_concat(player.get_datum(r), player))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(Datum::String(format!("{}{}", formatted, s)))
        }
        (Datum::Point(a), Datum::Point(b)) => {
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let sum = add_datums(a_val, b_val, player)?;
                result[i] = player.alloc_datum(sum);
            }
            Ok(Datum::Point(result))
        }
        (Datum::Point(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 2 {
                let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
                for i in 0..2 {
                    let a_val = player.get_datum(&a[i]).clone();
                    let b_val = player.get_datum(&ref_list[i]).clone();
                    let sum = add_datums(a_val, b_val, player)?;
                    result[i] = player.alloc_datum(sum);
                }
                Ok(Datum::Point(result))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for add_datums: {}",
                    ref_list.len()
                )))
            }
        }
        (Datum::List(_, ref_list, _), Datum::Point(b)) => {
            if ref_list.len() == 2 {
                let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
                for i in 0..2 {
                    let a_val = player.get_datum(&ref_list[i]).clone();
                    let b_val = player.get_datum(&b[i]).clone();
                    let sum = add_datums(a_val, b_val, player)?;
                    result[i] = player.alloc_datum(sum);
                }
                Ok(Datum::Point(result))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for add_datums: {}",
                    ref_list.len()
                )))
            }
        }
        (Datum::Point(a), Datum::Int(b)) => {
            let b_ref = player.alloc_datum(Datum::Int(*b));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&b_ref).clone();
                let sum = add_datums(a_val, b_val, player)?;
                result[i] = player.alloc_datum(sum);
            }
            Ok(Datum::Point(result))
        }
        (Datum::ColorRef(a), Datum::ColorRef(b)) => match (a, b) {
            (ColorRef::PaletteIndex(a), ColorRef::PaletteIndex(b)) => {
                Ok(Datum::ColorRef(ColorRef::PaletteIndex(a + b)))
            }
            (ColorRef::Rgb(a_r, a_g, a_b), ColorRef::Rgb(b_r, b_g, b_b)) => Ok(Datum::ColorRef(
                ColorRef::Rgb(a_r + b_r, a_g + b_g, a_b + b_b),
            )),
            _ => Err(ScriptError::new(format!(
                "Invalid operands for add_datums: {:?}, {:?}",
                a, b
            ))),
        },
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float(left_float + (*right as f64)))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float(left_float + right))
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = right.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float(left + right_float))
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = right.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float((*left as f64) + right_float))
        }
        // String + anything: concatenate as strings
        (Datum::String(left), _) => {
            let right_str = datum_to_string_for_concat(&right, player);
            Ok(Datum::String(format!("{}{}", left, right_str)))
        }
        _ => Err(ScriptError::new(format!(
            "Invalid operands for add_datums: {}, {}",
            left.type_str(),
            right.type_str()
        ))),
    }
}

pub fn subtract_datums(
    left: Datum,
    right: Datum,
    player: &mut DirPlayer,
) -> Result<Datum, ScriptError> {
    match (&left, &right) {
        (Datum::Void, Datum::Void) => Ok(Datum::Int(0)),
        (Datum::Void, Datum::Int(r)) => Ok(Datum::Int(-r)),
        (Datum::Int(l), Datum::Void) => Ok(Datum::Int(*l)),
        (Datum::Void, Datum::Float(r)) => Ok(Datum::Float(-r)),
        (Datum::Float(l), Datum::Void) => Ok(Datum::Float(*l)),
        (Datum::Int(left), Datum::Int(right)) => Ok(Datum::Int(left.wrapping_sub(*right))),
        (Datum::Float(left), Datum::Float(right)) => Ok(Datum::Float(left - right)),
        (Datum::Float(left), Datum::Int(right)) => Ok(Datum::Float(left - (*right as f64))),
        (Datum::Int(left), Datum::Float(right)) => Ok(Datum::Float((*left as f64) - right)),
        (Datum::Rect(a), Datum::Rect(b)) => {
            let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..4 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let diff = subtract_datums(a_val, b_val, player)?;
                result[i] = player.alloc_datum(diff);
            }
            Ok(Datum::Rect(result))
        }
        (Datum::Rect(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 4 {
                let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
                for i in 0..4 {
                    let a_val = player.get_datum(&a[i]).clone();
                    let b_val = player.get_datum(&ref_list[i]).clone();
                    let diff = subtract_datums(a_val, b_val, player)?;
                    result[i] = player.alloc_datum(diff);
                }
                Ok(Datum::Rect(result))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for subtract_datums: {}",
                    ref_list.len()
                )))
            }
        }
        // Vector
        (Datum::Vector(a), Datum::Vector(b)) => {
            Ok(Datum::Vector([a[0] - b[0], a[1] - b[1], a[2] - b[2]]))
        }
        (Datum::Vector(a), Datum::Int(b)) => Ok(Datum::Vector([
            a[0] - *b as f64,
            a[1] - *b as f64,
            a[2] - *b as f64,
        ])),
        (Datum::Vector(a), Datum::Float(b)) => Ok(Datum::Vector([a[0] - *b, a[1] - *b, a[2] - *b])),
        (Datum::Int(a), Datum::Vector(b)) => Ok(Datum::Vector([
            *a as f64 - b[0],
            *a as f64 - b[1],
            *a as f64 - b[2],
        ])),
        (Datum::Float(a), Datum::Vector(b)) => Ok(Datum::Vector([*a - b[0], *a - b[1], *a - b[2]])),

        // Vector <-> List
        (Datum::Vector(a), Datum::List(_, list, _)) if list.len() == 3 => {
            let mut result = [0.0; 3];
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => *n as f64,
                    Datum::Float(f) => *f,
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot subtract non-numeric list element from Vector".to_string(),
                        ))
                    }
                };
                result[i] = a[i] - val;
            }
            Ok(Datum::Vector(result))
        }
        (Datum::List(_, list, _), Datum::Vector(b)) if list.len() == 3 => {
            let mut result = VecDeque::with_capacity(3);
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => Datum::Float(*n as f64 - b[i]),
                    Datum::Float(f) => Datum::Float(*f - b[i]),
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot subtract Vector from list element".to_string(),
                        ))
                    }
                };
                result.push_back(player.alloc_datum(val));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::List(_, list_a, _), Datum::List(_, list_b, _)) => {
            let intersection_count = min(list_a.len(), list_b.len());
            let mut result = VecDeque::with_capacity(intersection_count);
            for i in 0..intersection_count {
                let a = player.get_datum(&list_a[i]).clone();
                let b = player.get_datum(&list_b[i]).clone();
                let result_datum = subtract_datums(a, b, player)?;
                result.push_back(player.alloc_datum(result_datum));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::Point(a), Datum::Point(b)) => {
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let diff = subtract_datums(a_val, b_val, player)?;
                result[i] = player.alloc_datum(diff);
            }
            Ok(Datum::Point(result))
        }
        (Datum::Point(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 2 {
                let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
                for i in 0..2 {
                    let a_val = player.get_datum(&a[i]).clone();
                    let b_val = player.get_datum(&ref_list[i]).clone();
                    let diff = subtract_datums(a_val, b_val, player)?;
                    result[i] = player.alloc_datum(diff);
                }
                Ok(Datum::Point(result))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for subtract_datums: {}",
                    ref_list.len()
                )))
            }
        }
        (Datum::List(_, ref_list, _), Datum::Point(b)) => {
            if ref_list.len() == 2 {
                let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
                for i in 0..2 {
                    let a_val = player.get_datum(&ref_list[i]).clone();
                    let b_val = player.get_datum(&b[i]).clone();
                    let diff = subtract_datums(a_val, b_val, player)?;
                    result[i] = player.alloc_datum(diff);
                }
                Ok(Datum::Point(result))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for subtract_datums: {}",
                    ref_list.len()
                )))
            }
        }
        (Datum::Int(a), Datum::Point(b)) => {
            let a_ref = player.alloc_datum(Datum::Int(*a));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a_ref).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let diff = subtract_datums(a_val, b_val, player)?;
                result[i] = player.alloc_datum(diff);
            }
            Ok(Datum::Point(result))
        }
        (Datum::ColorRef(a), Datum::ColorRef(b)) => match (a, b) {
            (ColorRef::PaletteIndex(a), ColorRef::PaletteIndex(b)) => {
                Ok(Datum::ColorRef(ColorRef::PaletteIndex(a.wrapping_sub(*b))))
            }
            (ColorRef::Rgb(a_r, a_g, a_b), ColorRef::Rgb(b_r, b_g, b_b)) => {
                Ok(Datum::ColorRef(ColorRef::Rgb(
                    a_r.saturating_sub(*b_r),
                    a_g.saturating_sub(*b_g),
                    a_b.saturating_sub(*b_b),
                )))
            }
            _ => Err(ScriptError::new(format!(
                "Invalid operands for subtract_datums: {:?}, {:?}",
                a, b
            ))),
        },
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float(left_float - (*right as f64)))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float(left_float - right))
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = right.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float(left - right_float))
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = right.parse::<f64>().unwrap_or(0.0);
            Ok(Datum::Float((*left as f64) - right_float))
        }
        (Datum::DateRef(a_id), Datum::DateRef(b_id)) => {
            let a_ms = player.date_objects.get(a_id)
                .ok_or_else(|| ScriptError::new(format!("Date object {} not found", a_id)))?.timestamp_ms;
            let b_ms = player.date_objects.get(b_id)
                .ok_or_else(|| ScriptError::new(format!("Date object {} not found", b_id)))?.timestamp_ms;
            let diff_days = (a_ms - b_ms) / (1000 * 60 * 60 * 24);
            Ok(Datum::Int(diff_days as i32))
        }
        (Datum::Void, Datum::Int(r)) => Ok(Datum::Float(0.0 - (*r as f64))),
        (Datum::Void, Datum::Float(r)) => Ok(Datum::Float(0.0 - r)),
        (Datum::Void, Datum::Void) => Ok(Datum::Int(0)),
        (Datum::Int(l), Datum::Void) => Ok(Datum::Float((*l as f64) - 0.0)),
        (Datum::Float(l), Datum::Void) => Ok(Datum::Float(*l - 0.0)),
        (Datum::Void, some) => Ok(some.clone()),
        (some, Datum::Void) => Ok(some.clone()),
        _ => Err(ScriptError::new(format!(
            "Invalid operands for subtract_datums: {}, {}",
            left.type_str(),
            right.type_str()
        ))),
    }
}

pub fn multiply_datums(
    left_ref: DatumRef,
    right_ref: DatumRef,
    player: &mut DirPlayer,
) -> Result<Datum, ScriptError> {
    let left = player.get_datum(&left_ref).clone();
    let right = player.get_datum(&right_ref).clone();

    let result = match (&left, &right) {
        (Datum::Void, Datum::Void) => Datum::Int(0),
        (Datum::Void, Datum::Int(_))
        | (Datum::Int(_), Datum::Void) => Datum::Int(0),
        (Datum::Void, Datum::Float(_))
        | (Datum::Float(_), Datum::Void) => Datum::Float(0.0),
        (Datum::Vector(_), Datum::Void)
        | (Datum::Void, Datum::Vector(_)) => Datum::Vector([0.0, 0.0, 0.0]),
        (Datum::Point(_), Datum::Void)
        | (Datum::Void, Datum::Point(_)) => {
            let zero_x = player.alloc_datum(Datum::Int(0));
            let zero_y = player.alloc_datum(Datum::Int(0));
            Datum::Point([zero_x, zero_y])
        }
        (Datum::Int(left), Datum::Int(right)) => Datum::Int(left * right),
        (Datum::Int(left), Datum::Float(right)) => Datum::Float((*left as f64) * right),
        (Datum::Float(left), Datum::Int(right)) => Datum::Float(*left * (*right as f64)),
        (Datum::Float(left), Datum::Float(right)) => Datum::Float(left * right),
        // Vector * scalar
        (Datum::Vector(v), Datum::Int(s)) => Datum::Vector([v[0] * *s as f64, v[1] * *s as f64, v[2] * *s as f64]),
        (Datum::Vector(v), Datum::Float(s)) => Datum::Vector([v[0] * s, v[1] * s, v[2] * s]),
        (Datum::Int(s), Datum::Vector(v)) => Datum::Vector([*s as f64 * v[0], *s as f64 * v[1], *s as f64 * v[2]]),
        (Datum::Float(s), Datum::Vector(v)) => Datum::Vector([s * v[0], s * v[1], s * v[2]]),
        // Color * scalar
        (Datum::ColorRef(c), Datum::Float(s)) => {
            match c {
                ColorRef::Rgb(r, g, b) => Datum::ColorRef(ColorRef::Rgb(
                    (*r as f64 * s).clamp(0.0, 255.0) as u8,
                    (*g as f64 * s).clamp(0.0, 255.0) as u8,
                    (*b as f64 * s).clamp(0.0, 255.0) as u8,
                )),
                _ => Datum::ColorRef(c.clone()),
            }
        }
        (Datum::ColorRef(c), Datum::Int(s)) => {
            let sf = *s as f64;
            match c {
                ColorRef::Rgb(r, g, b) => Datum::ColorRef(ColorRef::Rgb(
                    (*r as f64 * sf).clamp(0.0, 255.0) as u8,
                    (*g as f64 * sf).clamp(0.0, 255.0) as u8,
                    (*b as f64 * sf).clamp(0.0, 255.0) as u8,
                )),
                _ => Datum::ColorRef(c.clone()),
            }
        }
        (Datum::Float(s), Datum::ColorRef(c)) => {
            match c {
                ColorRef::Rgb(r, g, b) => Datum::ColorRef(ColorRef::Rgb(
                    (s * *r as f64).clamp(0.0, 255.0) as u8,
                    (s * *g as f64).clamp(0.0, 255.0) as u8,
                    (s * *b as f64).clamp(0.0, 255.0) as u8,
                )),
                _ => Datum::ColorRef(c.clone()),
            }
        }
        (Datum::Int(s), Datum::ColorRef(c)) => {
            let sf = *s as f64;
            match c {
                ColorRef::Rgb(r, g, b) => Datum::ColorRef(ColorRef::Rgb(
                    (sf * *r as f64).clamp(0.0, 255.0) as u8,
                    (sf * *g as f64).clamp(0.0, 255.0) as u8,
                    (sf * *b as f64).clamp(0.0, 255.0) as u8,
                )),
                _ => Datum::ColorRef(c.clone()),
            }
        }
        (Datum::Rect(a), Datum::Int(right)) => {
            let right_ref = player.alloc_datum(Datum::Int(*right));
            let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..4 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Rect(result)
        }
        (Datum::Rect(a), Datum::Float(right)) => {
            let right_ref = player.alloc_datum(Datum::Float(*right));
            let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..4 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Rect(result)
        }
        (Datum::Float(left), Datum::Rect(b)) => {
            let left_ref = player.alloc_datum(Datum::Float(*left));
            let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..4 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Rect(result)
        }
        (Datum::Point(arr), Datum::Int(scalar)) => {
            let scalar_ref = player.alloc_datum(Datum::Int(*scalar));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let val = player.get_datum(&arr[i]).clone();
                let scalar_val = player.get_datum(&scalar_ref).clone();
                let product = multiply_datums(
                    player.alloc_datum(val),
                    player.alloc_datum(scalar_val),
                    player
                )?;
                result[i] = player.alloc_datum(product);
            }
            Datum::Point(result)
        }
        (Datum::Point(arr), Datum::Float(scalar)) => {
            let scalar_ref = player.alloc_datum(Datum::Float(*scalar));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let val = player.get_datum(&arr[i]).clone();
                let scalar_val = player.get_datum(&scalar_ref).clone();
                let product = multiply_datums(
                    player.alloc_datum(val),
                    player.alloc_datum(scalar_val),
                    player
                )?;
                result[i] = player.alloc_datum(product);
            }
            Datum::Point(result)
        }
        (Datum::Float(left), Datum::Point(b)) => {
            let left_ref = player.alloc_datum(Datum::Float(*left));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        (Datum::Int(left), Datum::Point(b)) => {
            let left_ref = player.alloc_datum(Datum::Int(*left));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        (Datum::Point(left_arr), Datum::Point(right_arr)) => {
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let left_val = player.get_datum(&left_arr[i]).clone();
                let right_val = player.get_datum(&right_arr[i]).clone();
                let product = multiply_datums(
                    player.alloc_datum(left_val),
                    player.alloc_datum(right_val),
                    player
                )?;
                result[i] = player.alloc_datum(product);
            }
            Datum::Point(result)
        }
        (Datum::List(_, list, _), Datum::Float(right)) => {
            let mut new_list = vec![];
            for item in list {
                let item_datum = player.get_datum(item).clone();
                let result_datum = match &item_datum {
                    Datum::Int(n) => Datum::Float((*n as f64) * right),
                    Datum::Float(n) => Datum::Float(n * right),
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Mul operator in list only works with ints and floats. Given: {}",
                            format_datum(item, player)
                        )))
                    }
                };
                new_list.push(result_datum);
            }
            let mut ref_list = VecDeque::new();
            for item in new_list {
                ref_list.push_back(player.alloc_datum(item));
            }
            Datum::List(DatumType::List, ref_list, false)
        }
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Datum::Float(left_float * (*right as f64))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Datum::Float(left_float * right)
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = right.parse::<f64>().unwrap_or(0.0);
            Datum::Float(left * right_float)
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = right.parse::<f64>().unwrap_or(0.0);
            Datum::Float((*left as f64) * right_float)
        }
        // Point multiplication
        (Datum::Point(a), Datum::Int(right)) => {
            let right_ref = player.alloc_datum(Datum::Int(*right));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        (Datum::Point(a), Datum::Float(right)) => {
            let right_ref = player.alloc_datum(Datum::Float(*right));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        (Datum::Int(left), Datum::Point(b)) => {
            let left_ref = player.alloc_datum(Datum::Int(*left));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        (Datum::Float(left), Datum::Point(b)) => {
            let left_ref = player.alloc_datum(Datum::Float(*left));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        (Datum::Point(p), Datum::List(_, list, _)) if list.len() == 2 => {
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&p[i]).clone();
                let b_val = player.get_datum(&list[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        (Datum::List(_, list, _), Datum::Point(p)) if list.len() == 2 => {
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&list[i]).clone();
                let b_val = player.get_datum(&p[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::Point(result)
        }
        // List (2 elements) as Point multiplication
        (Datum::List(_, list, _), Datum::Int(right)) if list.len() == 2 => {
            let right_ref = player.alloc_datum(Datum::Int(*right));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&list[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::List(DatumType::List, VecDeque::from(result.to_vec()), false)
        }
        (Datum::List(_, list, _), Datum::Float(right)) if list.len() == 2 => {
            let right_ref = player.alloc_datum(Datum::Float(*right));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&list[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::List(DatumType::List, VecDeque::from(result.to_vec()), false)
        }
        (Datum::Int(left), Datum::List(_, list, _)) if list.len() == 2 => {
            let left_ref = player.alloc_datum(Datum::Int(*left));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&list[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::List(DatumType::List, VecDeque::from(result.to_vec()), false)
        }
        (Datum::Float(left), Datum::List(_, list, _)) if list.len() == 2 => {
            let left_ref = player.alloc_datum(Datum::Float(*left));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&list[i]).clone();
                let prod = multiply_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(prod);
            }
            Datum::List(DatumType::List, VecDeque::from(result.to_vec()), false)
        }

        _ => {
            return Err(ScriptError::new(format!(
                "Mul operator only works with ints and floats. Given: {}, {}",
                format_datum(&left_ref, player),
                format_datum(&right_ref, player)
            )))
        }
    };
    Ok(result)
}

pub fn divide_datums(
    left: DatumRef,
    right: DatumRef,
    player: &mut DirPlayer,
) -> Result<Datum, ScriptError> {
    let left = player.get_datum(&left).clone();
    let right = player.get_datum(&right).clone();

    let result = match (&left, &right) {
        (Datum::Void, _) => Datum::Int(0),
        (Datum::Int(_), Datum::Void) | (Datum::Float(_), Datum::Void) => Datum::Int(0), // div by VOID → 0
        (Datum::Int(left), Datum::Int(right)) => Datum::Int(left / right),
        (Datum::Int(left), Datum::Float(right)) => Datum::Float((*left as f64) / right),
        (Datum::Float(left), Datum::Int(right)) => Datum::Float(left / (*right as f64)),
        (Datum::Float(left), Datum::Float(right)) => Datum::Float(left / right),
        // Vector / scalar
        (Datum::Vector(v), Datum::Int(s)) => { let s = *s as f64; Datum::Vector([v[0] / s, v[1] / s, v[2] / s]) }
        (Datum::Vector(v), Datum::Float(s)) => Datum::Vector([v[0] / s, v[1] / s, v[2] / s]),
        (Datum::Point(a), Datum::Int(right)) => {
            let right_ref = player.alloc_datum(Datum::Int(*right));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(quot);
            }
            Datum::Point(result)
        }
        (Datum::Point(a), Datum::Float(right)) => {
            let right_ref = player.alloc_datum(Datum::Float(*right));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(quot);
            }
            Datum::Point(result)
        }
        (Datum::Float(left), Datum::Point(b)) => {
            let left_ref = player.alloc_datum(Datum::Float(*left));
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&left_ref).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(quot);
            }
            Datum::Point(result)
        }
        (Datum::Point(a), Datum::Point(b)) => {
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&b[i]).clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(quot);
            }
            Datum::Point(result)
        }
        // Point / List: element-wise division (Director treats 2-element lists as points)
        (Datum::Point(a), Datum::List(_, ref_list, _)) if ref_list.len() == 2 => {
            let mut result: [DatumRef; 2] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..2 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&ref_list[i]).clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(quot);
            }
            Datum::Point(result)
        }
        (Datum::Rect(a), Datum::Int(right)) => {
            let right_ref = player.alloc_datum(Datum::Int(*right));
            let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..4 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(quot);
            }
            Datum::Rect(result)
        }
        (Datum::Rect(a), Datum::Float(right)) => {
            let right_ref = player.alloc_datum(Datum::Float(*right));
            let mut result: [DatumRef; 4] = std::array::from_fn(|_| DatumRef::Void);
            for i in 0..4 {
                let a_val = player.get_datum(&a[i]).clone();
                let b_val = player.get_datum(&right_ref).clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player
                )?;
                result[i] = player.alloc_datum(quot);
            }
            Datum::Rect(result)
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_val = right
                .parse::<f64>()
                .map_err(|_| ScriptError::new(format!("Cannot divide int by string: {}", right)))?;
            Datum::Float((*left as f64) / right_val)
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_val = right.parse::<f64>().map_err(|_| {
                ScriptError::new(format!("Cannot divide float by string: {}", right))
            })?;
            Datum::Float(left / right_val)
        }
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Datum::Float(left_float / (*right as f64))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = left.parse::<f64>().unwrap_or(0.0);
            Datum::Float(left_float / right)
        }
        // List / scalar: element-wise division
        (Datum::List(list_type, items, sorted), Datum::Int(_)) | (Datum::List(list_type, items, sorted), Datum::Float(_)) => {
            let scalar_ref = player.alloc_datum(right.clone());
            let mut result_items = VecDeque::with_capacity(items.len());
            for item_ref in items {
                let item_val = player.get_datum(item_ref).clone();
                let quot = divide_datums(
                    player.alloc_datum(item_val),
                    scalar_ref.clone(),
                    player,
                )?;
                result_items.push_back(player.alloc_datum(quot));
            }
            Datum::List(list_type.clone(), result_items, *sorted)
        }
        (Datum::Void, _) => Datum::Int(0),
        _ => {
            return Err(ScriptError::new(format!(
                "Div operator only works with ints and floats (Provided: {} and {})",
                left.type_str(),
                right.type_str()
            )))
        }
    };
    Ok(result)
}

pub fn concat_datums(
    left: Datum,
    right: Datum,
    player: &mut DirPlayer,
) -> Result<Datum, ScriptError> {   
    let left_str = datum_to_string_for_concat(&left, player);
    let right_str = datum_to_string_for_concat(&right, player);
    
    Ok(Datum::String(format!("{}{}", left_str, right_str)))
}
