use std::cmp::min;

use crate::director::lingo::datum::{Datum, DatumType};

use super::{sprite::ColorRef, DirPlayer, ScriptError};

use crate::player::datum_formatting::datum_to_string_for_concat;

pub fn add_datums(left: Datum, right: Datum, player: &mut DirPlayer) -> Result<Datum, ScriptError> {
    match (&left, &right) {
        (Datum::Void, some) => Ok(some.clone()),
        (some, Datum::Void) => Ok(some.clone()),
        (Datum::Int(a), Datum::Int(b)) => Ok(Datum::Int(a + b)),
        (Datum::Float(a), Datum::Float(b)) => Ok(Datum::Float(a + b)),
        (Datum::Float(a), Datum::Int(b)) => Ok(Datum::Float(a + (*b as f32))),
        (Datum::Int(a), Datum::Float(b)) => Ok(Datum::Float((*a as f32) + b)),
        (Datum::IntRect(a), Datum::IntRect(b)) => {
            Ok(Datum::IntRect((a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3)))
        }
        (Datum::IntRect(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 4 {
                let b = ref_list
                    .iter()
                    .map(|r| player.get_datum(r).int_value().map(|x| x as i32))
                    .collect::<Result<Vec<i32>, ScriptError>>()?;
                Ok(Datum::IntRect((
                    a.0 + b[0],
                    a.1 + b[1],
                    a.2 + b[2],
                    a.3 + b[3],
                )))
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
            a[0] + *b as f32,
            a[1] + *b as f32,
            a[2] + *b as f32,
        ])),
        (Datum::Vector(a), Datum::Float(b)) => Ok(Datum::Vector([a[0] + *b, a[1] + *b, a[2] + *b])),
        (Datum::Int(a), Datum::Vector(b)) => Ok(Datum::Vector([
            *a as f32 + b[0],
            *a as f32 + b[1],
            *a as f32 + b[2],
        ])),
        (Datum::Float(a), Datum::Vector(b)) => Ok(Datum::Vector([*a + b[0], *a + b[1], *a + b[2]])),

        // Vector + List element-wise (3 elements)
        (Datum::Vector(a), Datum::List(_, list, _)) if list.len() == 3 => {
            let mut result = [0.0; 3];
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => *n as f32,
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
            let mut result = Vec::with_capacity(3);
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => Datum::Float(*n as f32 + b[i]),
                    Datum::Float(f) => Datum::Float(*f + b[i]),
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot add list element to Vector".to_string(),
                        ))
                    }
                };
                result.push(player.alloc_datum(val));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::List(_, list_a, _), Datum::List(_, list_b, _)) => {
            let intersection_count = min(list_a.len(), list_b.len());
            let mut result = Vec::with_capacity(intersection_count);
            for i in 0..intersection_count {
                let a = player.get_datum(&list_a[i]).clone();
                let b = player.get_datum(&list_b[i]).clone();
                let result_datum = add_datums(a, b, player)?;
                result.push(player.alloc_datum(result_datum));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::List(_, list, _), Datum::Int(i)) => {
            let mut result_refs = vec![];
            for r in list {
                let datum = player.get_datum(r);
                let result_datum = match datum {
                    Datum::Int(n) => Datum::Int(n + i),
                    Datum::Float(n) => Datum::Float(n + *i as f32),
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Invalid list element for add_datums: {}",
                            r
                        )))
                    }
                };
                result_refs.push(player.alloc_datum(result_datum));
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
        (Datum::IntPoint(a), Datum::IntPoint(b)) => Ok(Datum::IntPoint((a.0 + b.0, a.1 + b.1))),
        (Datum::IntPoint(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 2 {
                let b = ref_list
                    .iter()
                    .map(|r| player.get_datum(r).int_value().map(|x| x as i32))
                    .collect::<Result<Vec<i32>, ScriptError>>()?;
                Ok(Datum::IntPoint((a.0 + b[0], a.1 + b[1])))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for add_datums: {}",
                    ref_list.len()
                )))
            }
        }
        (Datum::IntPoint(a), Datum::Int(b)) => {
            Ok(Datum::IntPoint((a.0 + *b as i32, a.1 + *b as i32)))
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
            let left_float = left.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float(left_float + (*right as f32)))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = left.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float(left_float + right))
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = right.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float(left + right_float))
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = right.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float((*left as f32) + right_float))
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
        (Datum::Int(left), Datum::Int(right)) => Ok(Datum::Int(left.wrapping_sub(*right))),
        (Datum::Float(left), Datum::Float(right)) => Ok(Datum::Float(left - right)),
        (Datum::Float(left), Datum::Int(right)) => Ok(Datum::Float(left - (*right as f32))),
        (Datum::Int(left), Datum::Float(right)) => Ok(Datum::Float((*left as f32) - right)),
        (Datum::IntRect(a), Datum::IntRect(b)) => Ok(Datum::IntRect((
            a.0.wrapping_sub(b.0),
            a.1.wrapping_sub(b.1),
            a.2.wrapping_sub(b.2),
            a.3.wrapping_sub(b.3),
        ))),
        (Datum::IntRect(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 4 {
                let b = ref_list
                    .iter()
                    .map(|r| player.get_datum(r).int_value().map(|x| x as i32))
                    .collect::<Result<Vec<i32>, ScriptError>>()?;
                Ok(Datum::IntRect((
                    a.0.wrapping_sub(b[0]),
                    a.1.wrapping_sub(b[1]),
                    a.2.wrapping_sub(b[2]),
                    a.3.wrapping_sub(b[3]),
                )))
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
            a[0] - *b as f32,
            a[1] - *b as f32,
            a[2] - *b as f32,
        ])),
        (Datum::Vector(a), Datum::Float(b)) => Ok(Datum::Vector([a[0] - *b, a[1] - *b, a[2] - *b])),
        (Datum::Int(a), Datum::Vector(b)) => Ok(Datum::Vector([
            *a as f32 - b[0],
            *a as f32 - b[1],
            *a as f32 - b[2],
        ])),
        (Datum::Float(a), Datum::Vector(b)) => Ok(Datum::Vector([*a - b[0], *a - b[1], *a - b[2]])),

        // Vector <-> List
        (Datum::Vector(a), Datum::List(_, list, _)) if list.len() == 3 => {
            let mut result = [0.0; 3];
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => *n as f32,
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
            let mut result = Vec::with_capacity(3);
            for i in 0..3 {
                let val = match player.get_datum(&list[i]) {
                    Datum::Int(n) => Datum::Float(*n as f32 - b[i]),
                    Datum::Float(f) => Datum::Float(*f - b[i]),
                    _ => {
                        return Err(ScriptError::new(
                            "Cannot subtract Vector from list element".to_string(),
                        ))
                    }
                };
                result.push(player.alloc_datum(val));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::List(_, list_a, _), Datum::List(_, list_b, _)) => {
            let intersection_count = min(list_a.len(), list_b.len());
            let mut result = Vec::with_capacity(intersection_count);
            for i in 0..intersection_count {
                let a = player.get_datum(&list_a[i]).clone();
                let b = player.get_datum(&list_b[i]).clone();
                let result_datum = subtract_datums(a, b, player)?;
                result.push(player.alloc_datum(result_datum));
            }
            Ok(Datum::List(DatumType::List, result, false))
        }
        (Datum::IntPoint(a), Datum::IntPoint(b)) => Ok(Datum::IntPoint((
            a.0.wrapping_sub(b.0),
            a.1.wrapping_sub(b.1),
        ))),
        (Datum::IntPoint(a), Datum::List(_, ref_list, _)) => {
            if ref_list.len() == 2 {
                let b = ref_list
                    .iter()
                    .map(|r| player.get_datum(r).int_value().map(|x| x as i32))
                    .collect::<Result<Vec<i32>, ScriptError>>()?;
                Ok(Datum::IntPoint((
                    a.0.wrapping_sub(b[0]),
                    a.1.wrapping_sub(b[1]),
                )))
            } else {
                Err(ScriptError::new(format!(
                    "Invalid list length for subtract_datums: {}",
                    ref_list.len()
                )))
            }
        }
        (Datum::Int(a), Datum::IntPoint(b)) => Ok(Datum::IntPoint((
            (*a as i32).wrapping_sub(b.0),
            (*a as i32).wrapping_sub(b.1),
        ))),
        (Datum::ColorRef(a), Datum::ColorRef(b)) => match (a, b) {
            (ColorRef::PaletteIndex(a), ColorRef::PaletteIndex(b)) => {
                Ok(Datum::ColorRef(ColorRef::PaletteIndex(a.wrapping_sub(*b))))
            }
            (ColorRef::Rgb(a_r, a_g, a_b), ColorRef::Rgb(b_r, b_g, b_b)) => {
                Ok(Datum::ColorRef(ColorRef::Rgb(
                    a_r.wrapping_sub(*b_r),
                    a_g.wrapping_sub(*b_g),
                    a_b.wrapping_sub(*b_b),
                )))
            }
            _ => Err(ScriptError::new(format!(
                "Invalid operands for subtract_datums: {:?}, {:?}",
                a, b
            ))),
        },
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = left.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float(left_float - (*right as f32)))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = left.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float(left_float - right))
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = right.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float(left - right_float))
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = right.parse::<f32>().unwrap_or(0.0);
            Ok(Datum::Float((*left as f32) - right_float))
        }
        (Datum::Void, Datum::Int(r)) => Ok(Datum::Float(0.0 - (*r as f32))),
        (Datum::Void, Datum::Float(r)) => Ok(Datum::Float(0.0 - r)),
        (Datum::Void, Datum::Void) => Ok(Datum::Int(0)),
        (Datum::Int(l), Datum::Void) => Ok(Datum::Float((*l as f32) - 0.0)),
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
