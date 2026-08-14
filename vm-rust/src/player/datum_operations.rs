use std::cmp::min;
use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{datum_formatting::{datum_to_string_for_concat, format_datum}, datum_ref::DatumRef, handlers::types::TypeHandlers},
};

use super::{sprite::ColorRef, DirPlayer, ScriptError};

/// Director's integer value of a color: a paletteIndex is its index; an RGB
/// color packs to `r<<16 | g<<8 | b` (the 24-bit value `getPixel(pt, #integer)`
/// returns for 32-bit images). Used when a color is combined with a scalar.
fn color_ref_to_i32(c: &ColorRef) -> i32 {
    match c {
        ColorRef::PaletteIndex(i) => *i as i32,
        ColorRef::Rgb(r, g, b) => ((*r as i32) << 16) | ((*g as i32) << 8) | (*b as i32),
    }
}

/// Allocate a new `DateObject` whose timestamp is `days_delta` days from
/// the source date's timestamp. Used by `add_datums` / `subtract_datums`
/// to implement Director's `date + N` and `date - N` arithmetic, which
/// return a brand new date value rather than mutating the source.
fn shift_date_by_days(
    src_date_id: u32,
    days_delta: i64,
    player: &mut DirPlayer,
) -> Result<u32, ScriptError> {
    let src_ms = player
        .date_objects
        .get(&src_date_id)
        .ok_or_else(|| ScriptError::new(format!("Date object {} not found", src_date_id)))?
        .timestamp_ms;
    let new_ms = src_ms + days_delta * 24 * 60 * 60 * 1000;
    let new_id = player.allocator.get_free_script_instance_id();
    let new_obj = crate::player::handlers::datum_handlers::date::DateObject::from_timestamp(
        new_id, new_ms,
    );
    player.date_objects.insert(new_id, new_obj);
    Ok(new_id)
}

/// Perform a binary op on two inline components, preserving int/float semantics.
/// If either operand is float, result is float.
fn inline_binop_2(
    a: [f64; 2], af: u8,
    b: [f64; 2], bf: u8,
    op: fn(f64, f64) -> f64,
) -> ([f64; 2], u8) {
    let vals = [op(a[0], b[0]), op(a[1], b[1])];
    let flags = af | bf; // float if either is float
    (vals, flags)
}

fn inline_binop_4(
    a: [f64; 4], af: u8,
    b: [f64; 4], bf: u8,
    op: fn(f64, f64) -> f64,
) -> ([f64; 4], u8) {
    let vals = [op(a[0], b[0]), op(a[1], b[1]), op(a[2], b[2]), op(a[3], b[3])];
    let flags = af | bf;
    (vals, flags)
}

/// Apply a scalar op to each component of an inline point.
/// Result type: if scalar is float OR component is float, result is float.
fn inline_scalar_2(
    a: [f64; 2], af: u8,
    scalar: f64, scalar_is_float: bool,
    op: fn(f64, f64) -> f64,
) -> ([f64; 2], u8) {
    let vals = [op(a[0], scalar), op(a[1], scalar)];
    let flags = if scalar_is_float { 0b11 } else { af };
    (vals, flags)
}

fn inline_scalar_4(
    a: [f64; 4], af: u8,
    scalar: f64, scalar_is_float: bool,
    op: fn(f64, f64) -> f64,
) -> ([f64; 4], u8) {
    let vals = [op(a[0], scalar), op(a[1], scalar), op(a[2], scalar), op(a[3], scalar)];
    let flags = if scalar_is_float { 0b1111 } else { af };
    (vals, flags)
}

/// Extract point components from a list datum (for Point + List ops).
fn list_to_point_vals(player: &DirPlayer, list: &VecDeque<DatumRef>) -> Result<([f64; 2], u8), ScriptError> {
    if list.len() != 2 {
        return Err(ScriptError::new(format!("Invalid list length for point op: {}", list.len())));
    }
    let (v0, f0) = Datum::datum_to_inline_component(player.get_datum(&list[0]))?;
    let (v1, f1) = Datum::datum_to_inline_component(player.get_datum(&list[1]))?;
    let flags = (if f0 { 1u8 } else { 0 }) | (if f1 { 2u8 } else { 0 });
    Ok(([v0, v1], flags))
}

fn list_to_rect_vals(player: &DirPlayer, list: &VecDeque<DatumRef>) -> Result<([f64; 4], u8), ScriptError> {
    if list.len() != 4 {
        return Err(ScriptError::new(format!("Invalid list length for rect op: {}", list.len())));
    }
    let mut vals = [0.0; 4];
    let mut flags = 0u8;
    for i in 0..4 {
        let (v, f) = Datum::datum_to_inline_component(player.get_datum(&list[i]))?;
        vals[i] = v;
        if f { flags |= 1 << i; }
    }
    Ok((vals, flags))
}

/// A symbol takes part in arithmetic as its name, exactly as it does in a
/// comparison (see `compare.rs::datum_greater_than`, where symbols are
/// pre-converted to strings). Non-numeric text then coerces to 0, which is how
/// every arithmetic function below already treats strings.
///
/// The Director 11.5 Scripting Dictionary defines `-` (and `+`, `*`, `/`) only
/// over "numerical expressions" and says nothing about symbols, so the rule is
/// inferred from Director's general permissiveness with non-numeric operands —
/// it doesn't raise, it coerces. Merlin's Revenge 3 needs it in a generic
/// helper that diffs two values of unknown type:
///
///   on VarDiff var1, var2
///     diff = max(var1, var2) - min(var1, var2)
///
/// called with `0` and `#none`. `max()` "works with ASCII characters, similar
/// to the way < and > operators work with strings" (dictionary, `max()`), and
/// `#none > 0` is true, so max returns the symbol and min the integer, leaving
/// `#none - 0` to evaluate.
fn symbol_as_arithmetic_operand(datum: &Datum) -> Option<Datum> {
    match datum {
        Datum::Symbol(name) => Some(Datum::String(name.clone().to_string())),
        _ => None,
    }
}

pub fn add_datums(left: Datum, right: Datum, player: &mut DirPlayer) -> Result<Datum, ScriptError> {
    if let Some(left) = symbol_as_arithmetic_operand(&left) {
        return add_datums(left, right, player);
    }
    if let Some(right) = symbol_as_arithmetic_operand(&right) {
        return add_datums(left, right, player);
    }
    match (&left, &right) {
        (Datum::Void, some) => Ok(some.clone()),
        (some, Datum::Void) => Ok(some.clone()),
        (Datum::Int(a), Datum::Int(b)) => Ok(Datum::Int(a + b)),
        (Datum::Float(a), Datum::Float(b)) => Ok(Datum::Float(a + b)),
        (Datum::Float(a), Datum::Int(b)) => Ok(Datum::Float(a + (*b as f64))),
        (Datum::Int(a), Datum::Float(b)) => Ok(Datum::Float((*a as f64) + b)),
        (Datum::Rect(a, af), Datum::Rect(b, bf)) => {
            let (vals, flags) = inline_binop_4(*a, *af, *b, *bf, |x, y| x + y);
            Ok(Datum::Rect(vals, flags))
        }
        (Datum::Rect(a, af), Datum::List(_, ref_list, _)) => {
            let (bv, bf) = list_to_rect_vals(player, ref_list)?;
            let (vals, flags) = inline_binop_4(*a, *af, bv, bf, |x, y| x + y);
            Ok(Datum::Rect(vals, flags))
        }
        // Director: `rect + N` adds N to each side of the rect.
        (Datum::Rect(a, af), Datum::Int(b)) => {
            let (vals, flags) = inline_scalar_4(*a, *af, *b as f64, false, |x, y| x + y);
            Ok(Datum::Rect(vals, flags))
        }
        (Datum::Rect(a, af), Datum::Float(b)) => {
            let (vals, flags) = inline_scalar_4(*a, *af, *b, true, |x, y| x + y);
            Ok(Datum::Rect(vals, flags))
        }
        (Datum::Int(a), Datum::Rect(b, bf)) => {
            let (vals, flags) = inline_scalar_4(*b, *bf, *a as f64, false, |x, y| y + x);
            Ok(Datum::Rect(vals, flags))
        }
        (Datum::Float(a), Datum::Rect(b, bf)) => {
            let (vals, flags) = inline_scalar_4(*b, *bf, *a, true, |x, y| y + x);
            Ok(Datum::Rect(vals, flags))
        }
        // Director: `rect + point` offsets the rect by the point
        // (adds x to left+right, y to top+bottom).
        (Datum::Rect(a, af), Datum::Point(p, pf)) => {
            let bv = [p[0], p[1], p[0], p[1]];
            let bf = ((pf & 0b01) * 0b0101) | ((pf & 0b10) * 0b0101);
            let (vals, flags) = inline_binop_4(*a, *af, bv, bf, |x, y| x + y);
            Ok(Datum::Rect(vals, flags))
        }
        (Datum::Point(p, pf), Datum::Rect(b, bf)) => {
            let av = [p[0], p[1], p[0], p[1]];
            let af = ((pf & 0b01) * 0b0101) | ((pf & 0b10) * 0b0101);
            let (vals, flags) = inline_binop_4(av, af, *b, *bf, |x, y| x + y);
            Ok(Datum::Rect(vals, flags))
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
        // Director applies a scalar element-wise across a list, the same way it
        // does for a rect ("If you add a single value to a rectangle, Lingo …
        // adds it to each element in the rectangle" — 11.5 Scripting Dictionary,
        // rect() entry) and the way multiply_datums already handles
        // `list * scalar`. Scripts lean on this where a slot may hold either a
        // vector-ish list or a plain 0: Heatwave Racing interpolates a tilt
        // vector across four trackmap cells with
        //   gtv = g1[3] * g1p + g2[3] * g2p + g3[3] * g3p + g4[3] * g4p
        // where a missing cell falls back to [0,0,0,0], so that cell's `g[3]` is
        // the integer 0 while the others are 3-element lists. Erroring on the
        // mixed add aborted the handler; the script's own
        // `if gtv = 0 then gtv = [0,0,0]` shows a scalar result is expected when
        // every cell is missing.
        (Datum::List(_, list, _), Datum::Int(_) | Datum::Float(_)) => {
            let item_refs: Vec<DatumRef> = list.iter().cloned().collect();
            let scalar = right.clone();
            let mut ref_list = VecDeque::with_capacity(item_refs.len());
            for item in &item_refs {
                let item_datum = player.get_datum(item).clone();
                let sum = add_datums(item_datum, scalar.clone(), player)?;
                ref_list.push_back(player.alloc_datum(sum));
            }
            Ok(Datum::List(DatumType::List, ref_list, false))
        }
        (Datum::Int(_) | Datum::Float(_), Datum::List(_, list, _)) => {
            let item_refs: Vec<DatumRef> = list.iter().cloned().collect();
            let scalar = left.clone();
            let mut ref_list = VecDeque::with_capacity(item_refs.len());
            for item in &item_refs {
                let item_datum = player.get_datum(item).clone();
                let sum = add_datums(scalar.clone(), item_datum, player)?;
                ref_list.push_back(player.alloc_datum(sum));
            }
            Ok(Datum::List(DatumType::List, ref_list, false))
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
        // Two property lists combine value-by-value, keeping the LEFT list's
        // property names — the same positional, shared-prefix rule as the
        // linear-list arm directly above. The Scripting Dictionary documents
        // arithmetic for rect() and point() but is silent on lists, so this
        // mirrors the linear case rather than matching keys up; in practice both
        // operands come from one template and their key order agrees.
        //
        // Merlin's Revenge 2 sums two speed tables when a character morphs:
        //   p.w.movSpeeds = p.w.movSpeeds + pmt.info.speeds
        // where each is [#norm: .., #easy: .., #hard: .., #web: .., #nav: ..].
        (Datum::PropList(pairs_a, _), Datum::PropList(pairs_b, _)) => {
            let count = min(pairs_a.len(), pairs_b.len());
            let mut result = VecDeque::with_capacity(count);
            for i in 0..count {
                let key = pairs_a[i].0.clone();
                let a = player.get_datum(&pairs_a[i].1).clone();
                let b = player.get_datum(&pairs_b[i].1).clone();
                let value = add_datums(a, b, player)?;
                let value_ref = player.alloc_datum(value);
                result.push_back((key, value_ref));
            }
            Ok(Datum::PropList(result, false))
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
        (Datum::Point(a, af), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_binop_2(*a, *af, *b, *bf, |x, y| x + y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Point(a, af), Datum::List(_, ref_list, _)) => {
            let (bv, bf) = list_to_point_vals(player, ref_list)?;
            let (vals, flags) = inline_binop_2(*a, *af, bv, bf, |x, y| x + y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::List(_, ref_list, _), Datum::Point(b, bf)) => {
            let (av, af) = list_to_point_vals(player, ref_list)?;
            let (vals, flags) = inline_binop_2(av, af, *b, *bf, |x, y| x + y);
            Ok(Datum::Point(vals, flags))
        }
        // Scalar spreads across every component — see the note on the matching
        // `subtract_datums` arms. Addition already had `point + int`; the float
        // and reversed forms were missing.
        (Datum::Point(a, af), Datum::Int(b)) => {
            let (vals, flags) = inline_scalar_2(*a, *af, *b as f64, false, |x, y| x + y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Point(a, af), Datum::Float(b)) => {
            let (vals, flags) = inline_scalar_2(*a, *af, *b, true, |x, y| x + y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Int(a), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_scalar_2(*b, *bf, *a as f64, false, |x, y| x + y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Float(a), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_scalar_2(*b, *bf, *a, true, |x, y| x + y);
            Ok(Datum::Point(vals, flags))
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
        // Director date arithmetic: `date + N` returns a new date N days
        // later (and `N + date` is symmetric). Used by ClubMarian's daily
        // login window check (`if NetDate = ParamDate or NetDate = ParamDate + 1`).
        (Datum::DateRef(date_id), Datum::Int(days)) => {
            Ok(Datum::DateRef(shift_date_by_days(*date_id, *days as i64, player)?))
        }
        (Datum::Int(days), Datum::DateRef(date_id)) => {
            Ok(Datum::DateRef(shift_date_by_days(*date_id, *days as i64, player)?))
        }
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = TypeHandlers::float_impl(left).unwrap_or(0.0);
            Ok(Datum::Float(left_float + (*right as f64)))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = TypeHandlers::float_impl(left).unwrap_or(0.0);
            Ok(Datum::Float(left_float + right))
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = TypeHandlers::float_impl(right).unwrap_or(0.0);
            Ok(Datum::Float(left + right_float))
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = TypeHandlers::float_impl(right).unwrap_or(0.0);
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
    // See `symbol_as_arithmetic_operand`.
    if let Some(left) = symbol_as_arithmetic_operand(&left) {
        return subtract_datums(left, right, player);
    }
    if let Some(right) = symbol_as_arithmetic_operand(&right) {
        return subtract_datums(left, right, player);
    }
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
        (Datum::Rect(a, af), Datum::Rect(b, bf)) => {
            let (vals, flags) = inline_binop_4(*a, *af, *b, *bf, |x, y| x - y);
            Ok(Datum::Rect(vals, flags))
        }
        (Datum::Rect(a, af), Datum::List(_, ref_list, _)) => {
            let (bv, bf) = list_to_rect_vals(player, ref_list)?;
            let (vals, flags) = inline_binop_4(*a, *af, bv, bf, |x, y| x - y);
            Ok(Datum::Rect(vals, flags))
        }
        // Director: `rect - N` subtracts N from each side of the rect.
        (Datum::Rect(a, af), Datum::Int(b)) => {
            let (vals, flags) = inline_scalar_4(*a, *af, *b as f64, false, |x, y| x - y);
            Ok(Datum::Rect(vals, flags))
        }
        (Datum::Rect(a, af), Datum::Float(b)) => {
            let (vals, flags) = inline_scalar_4(*a, *af, *b, true, |x, y| x - y);
            Ok(Datum::Rect(vals, flags))
        }
        // Director: `rect - point` offsets the rect by the negative point.
        (Datum::Rect(a, af), Datum::Point(p, pf)) => {
            let bv = [p[0], p[1], p[0], p[1]];
            let bf = ((pf & 0b01) * 0b0101) | ((pf & 0b10) * 0b0101);
            let (vals, flags) = inline_binop_4(*a, *af, bv, bf, |x, y| x - y);
            Ok(Datum::Rect(vals, flags))
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
        // List ± scalar applies element-wise, the same recursion `add_datums`
        // already implements (and that this repo notes for list × scalar).
        // dkbarrel's `Terrain Builder.Barrelchkcol` does
        //     coldata = terrdata - playerY
        // with terrdata a 9-element height list and playerY a number.
        (Datum::List(_, list, _), Datum::Int(_) | Datum::Float(_)) => {
            let item_refs: Vec<DatumRef> = list.iter().cloned().collect();
            let scalar = right.clone();
            let mut ref_list = VecDeque::with_capacity(item_refs.len());
            for item in &item_refs {
                let item_datum = player.get_datum(item).clone();
                let diff = subtract_datums(item_datum, scalar.clone(), player)?;
                ref_list.push_back(player.alloc_datum(diff));
            }
            Ok(Datum::List(DatumType::List, ref_list, false))
        }
        // Scalar - list is NOT commutative: each element is subtracted FROM the
        // scalar, so build `scalar - item` rather than reusing the arm above.
        (Datum::Int(_) | Datum::Float(_), Datum::List(_, list, _)) => {
            let item_refs: Vec<DatumRef> = list.iter().cloned().collect();
            let scalar = left.clone();
            let mut ref_list = VecDeque::with_capacity(item_refs.len());
            for item in &item_refs {
                let item_datum = player.get_datum(item).clone();
                let diff = subtract_datums(scalar.clone(), item_datum, player)?;
                ref_list.push_back(player.alloc_datum(diff));
            }
            Ok(Datum::List(DatumType::List, ref_list, false))
        }
        // Two property lists combine value-by-value, keeping the LEFT list's
        // property names — the same positional, shared-prefix rule as the
        // linear-list arm directly above. The Scripting Dictionary documents
        // arithmetic for rect() and point() but is silent on lists, so this
        // mirrors the linear case rather than matching keys up; in practice both
        // operands come from one template and their key order agrees.
        //
        // Included so subtraction stays symmetric with addition.
        (Datum::PropList(pairs_a, _), Datum::PropList(pairs_b, _)) => {
            let count = min(pairs_a.len(), pairs_b.len());
            let mut result = VecDeque::with_capacity(count);
            for i in 0..count {
                let key = pairs_a[i].0.clone();
                let a = player.get_datum(&pairs_a[i].1).clone();
                let b = player.get_datum(&pairs_b[i].1).clone();
                let value = subtract_datums(a, b, player)?;
                let value_ref = player.alloc_datum(value);
                result.push_back((key, value_ref));
            }
            Ok(Datum::PropList(result, false))
        }
        (Datum::Point(a, af), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_binop_2(*a, *af, *b, *bf, |x, y| x - y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Point(a, af), Datum::List(_, ref_list, _)) => {
            let (bv, bf) = list_to_point_vals(player, ref_list)?;
            let (vals, flags) = inline_binop_2(*a, *af, bv, bf, |x, y| x - y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::List(_, ref_list, _), Datum::Point(b, bf)) => {
            let (av, af) = list_to_point_vals(player, ref_list)?;
            let (vals, flags) = inline_binop_2(av, af, *b, *bf, |x, y| x - y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Int(a), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_scalar_2(*b, *bf, *a as f64, false, |b, a| a - b);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Float(a), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_scalar_2(*b, *bf, *a, true, |b, a| a - b);
            Ok(Datum::Point(vals, flags))
        }
        // A scalar spreads across every component, the same rule the Scripting
        // Dictionary states for rectangles ("If you add a single value to a
        // rectangle, Lingo... adds it to each element in the rectangle") and the
        // same one the Rect arms above and multiply/divide already follow. Only
        // the REVERSED form (`scalar - point`) existed here, so the far more
        // common `point - scalar` raised "Invalid operands".
        //
        // Merlin's Revenge 2 converts a 1-based grid cell to a 0-based offset
        // with `maploc = point(c, r) - 1` while drawing the minimap.
        (Datum::Point(a, af), Datum::Int(b)) => {
            let (vals, flags) = inline_scalar_2(*a, *af, *b as f64, false, |x, y| x - y);
            Ok(Datum::Point(vals, flags))
        }
        (Datum::Point(a, af), Datum::Float(b)) => {
            let (vals, flags) = inline_scalar_2(*a, *af, *b, true, |x, y| x - y);
            Ok(Datum::Point(vals, flags))
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
        // Unlike `+`, `-` has no string meaning, so two strings subtract
        // numerically with the same non-numeric-is-0 coercion as the arms
        // below. Reached by `#a - #b` via `symbol_as_arithmetic_operand`.
        (Datum::String(left), Datum::String(right)) => Ok(Datum::Float(
            left.parse::<f64>().unwrap_or(0.0) - right.parse::<f64>().unwrap_or(0.0),
        )),
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = TypeHandlers::float_impl(left).unwrap_or(0.0);
            Ok(Datum::Float(left_float - (*right as f64)))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = TypeHandlers::float_impl(left).unwrap_or(0.0);
            Ok(Datum::Float(left_float - right))
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = TypeHandlers::float_impl(right).unwrap_or(0.0);
            Ok(Datum::Float(left - right_float))
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = TypeHandlers::float_impl(right).unwrap_or(0.0);
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
        // Director date arithmetic: `date - N` returns a new date N days
        // earlier. Mirrors `date + N` in `add_datums`.
        (Datum::DateRef(date_id), Datum::Int(days)) => {
            Ok(Datum::DateRef(shift_date_by_days(*date_id, -(*days as i64), player)?))
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
    // See `symbol_as_arithmetic_operand`.
    let left = symbol_as_arithmetic_operand(&left).unwrap_or(left);
    let right = symbol_as_arithmetic_operand(&right).unwrap_or(right);

    let result = match (&left, &right) {
        (Datum::Void, Datum::Void) => Datum::Int(0),
        (Datum::Void, Datum::Int(_))
        | (Datum::Int(_), Datum::Void) => Datum::Int(0),
        (Datum::Void, Datum::Float(_))
        | (Datum::Float(_), Datum::Void) => Datum::Float(0.0),
        (Datum::Vector(_), Datum::Void)
        | (Datum::Void, Datum::Vector(_)) => Datum::Vector([0.0, 0.0, 0.0]),
        (Datum::Point(..), Datum::Void)
        | (Datum::Void, Datum::Point(..)) => {
            Datum::Point([0.0, 0.0], 0)
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
        // Vector * Vector = dot product (Director Lingo convention)
        (Datum::Vector(a), Datum::Vector(b)) => Datum::Float(a[0]*b[0] + a[1]*b[1] + a[2]*b[2]),
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
        (Datum::Rect(a, af), Datum::Int(right)) => {
            let (vals, flags) = inline_scalar_4(*a, *af, *right as f64, false, |x, y| x * y);
            Datum::Rect(vals, flags)
        }
        (Datum::Rect(a, af), Datum::Float(right)) => {
            let (vals, flags) = inline_scalar_4(*a, *af, *right, true, |x, y| x * y);
            Datum::Rect(vals, flags)
        }
        (Datum::Float(left), Datum::Rect(b, bf)) => {
            let (vals, flags) = inline_scalar_4(*b, *bf, *left, true, |x, y| y * x);
            Datum::Rect(vals, flags)
        }
        (Datum::Point(a, af), Datum::Int(scalar)) => {
            let (vals, flags) = inline_scalar_2(*a, *af, *scalar as f64, false, |x, y| x * y);
            Datum::Point(vals, flags)
        }
        (Datum::Point(a, af), Datum::Float(scalar)) => {
            let (vals, flags) = inline_scalar_2(*a, *af, *scalar, true, |x, y| x * y);
            Datum::Point(vals, flags)
        }
        (Datum::Float(left), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_scalar_2(*b, *bf, *left, true, |x, y| y * x);
            Datum::Point(vals, flags)
        }
        (Datum::Int(left), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_scalar_2(*b, *bf, *left as f64, false, |x, y| y * x);
            Datum::Point(vals, flags)
        }
        (Datum::Point(a, af), Datum::Point(b, bf)) => {
            let (vals, flags) = inline_binop_2(*a, *af, *b, *bf, |x, y| x * y);
            Datum::Point(vals, flags)
        }
        // List * List — element-wise over the shared prefix, matching the
        // `(List, List)` arms `add_datums` / `subtract_datums` already have.
        // Director pairs list operands positionally (the Scripting Dictionary
        // describes the same rule for rects/points, which ARE lists: "each
        // element of the first list [operates on] the corresponding element of
        // the second list").
        //
        // AreaZero's `[M] Text Misc` GetAndSetTagTextureSize rescales a mesh's
        // UVs with `tTCoord = (tTCoord * tPerc) + tdiff`, where BOTH operands are
        // 2-element lists ([u,v] * [0.7715, 1.0]). Addition already worked, so
        // only the multiply raised "Mul operator only works with ints and floats".
        (Datum::List(_, list_a, _), Datum::List(_, list_b, _)) => {
            let a_refs: Vec<DatumRef> = list_a.iter().cloned().collect();
            let b_refs: Vec<DatumRef> = list_b.iter().cloned().collect();
            let n = min(a_refs.len(), b_refs.len());
            let mut result = VecDeque::with_capacity(n);
            for i in 0..n {
                let product = multiply_datums(a_refs[i].clone(), b_refs[i].clone(), player)?;
                result.push_back(player.alloc_datum(product));
            }
            Datum::List(DatumType::List, result, false)
        }
        (Datum::List(_, list, _), Datum::Float(right)) => {
            // Collect the element refs up front so the recursive multiply (for
            // nested lists) can borrow the player mutably without aliasing the
            // outer list's borrow.
            let item_refs: Vec<DatumRef> = list.iter().cloned().collect();
            let right_val = *right;
            let mut ref_list = VecDeque::new();
            for item in &item_refs {
                let item_datum = player.get_datum(item).clone();
                let result_datum = match &item_datum {
                    Datum::Int(n) => Datum::Float((*n as f64) * right_val),
                    Datum::Float(n) => Datum::Float(*n * right_val),
                    // Director recurses into nested lists / points / rects /
                    // vectors, scaling each sub-element by the scalar
                    // (e.g. gspeed[1] * 1.5 where gspeed[1] is a list of lists).
                    Datum::List(..) | Datum::Point(..) | Datum::Rect(..) | Datum::Vector(_) => {
                        multiply_datums(item.clone(), right_ref.clone(), player)?
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Mul operator in list only works with ints and floats. Given: {}",
                            format_datum(item, player)
                        )))
                    }
                };
                ref_list.push_back(player.alloc_datum(result_datum));
            }
            Datum::List(DatumType::List, ref_list, false)
        }
        (Datum::String(left), Datum::Int(right)) => {
            if *right == 0 {
                Datum::Int(0)
            } else if let Some(left_float) = TypeHandlers::float_impl(left) {
                Datum::Float(left_float * (*right as f64))
            } else {
                // Director returns random, arbitrarily large int for string * int if string isn't a number
                // Some movies rely on this behavior, so we replicate it here.
                Datum::Int(123456789)
            }
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = TypeHandlers::float_impl(left).unwrap_or(0.0);
            Datum::Float(left_float * right)
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_float = TypeHandlers::float_impl(right).unwrap_or(0.0);
            Datum::Float(left * right_float)
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_float = TypeHandlers::float_impl(right).unwrap_or(0.0);
            Datum::Float((*left as f64) * right_float)
        }
        (Datum::Point(a, af), Datum::List(_, list, _)) if list.len() == 2 => {
            let (bv, bf) = list_to_point_vals(player, list)?;
            let (vals, flags) = inline_binop_2(*a, *af, bv, bf, |x, y| x * y);
            Datum::Point(vals, flags)
        }
        (Datum::List(_, list, _), Datum::Point(b, bf)) if list.len() == 2 => {
            let (av, af) = list_to_point_vals(player, list)?;
            let (vals, flags) = inline_binop_2(av, af, *b, *bf, |x, y| x * y);
            Datum::Point(vals, flags)
        }
        // List * Int, ANY length. Int elements stay Int (Director only widens to
        // float when a float is involved); nested lists/points/rects/vectors
        // recurse, mirroring the List * Float arm above.
        (Datum::List(_, list, _), Datum::Int(right)) => {
            let item_refs: Vec<DatumRef> = list.iter().cloned().collect();
            let right_val = *right;
            let mut ref_list = VecDeque::new();
            for item in &item_refs {
                let item_datum = player.get_datum(item).clone();
                let result_datum = match &item_datum {
                    Datum::Int(n) => Datum::Int(n * right_val),
                    Datum::Float(n) => Datum::Float(*n * right_val as f64),
                    Datum::List(..) | Datum::Point(..) | Datum::Rect(..) | Datum::Vector(_) => {
                        multiply_datums(item.clone(), right_ref.clone(), player)?
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Mul operator in list only works with ints and floats. Given: {}",
                            format_datum(item, player)
                        )))
                    }
                };
                ref_list.push_back(player.alloc_datum(result_datum));
            }
            Datum::List(DatumType::List, ref_list, false)
        }
        // PropList * scalar — element-wise over the VALUES, keys preserved,
        // recursing into nested lists exactly like the List arms above (Director
        // applies the operation through the structure; see the rect note in the
        // dictionary, "adds it to each element in the rectangle" — the same rule
        // as the `list * scalar` behaviour this mirrors).
        //
        // Merlin's Revenge 3's modResidents scales a whole Counter struct:
        //   productionTime = pCurrentGroupSize * timeToBuildSingle
        // with pCurrentGroupSize = [#theCount: 5, #tim: [0, 5], #inc: -1,
        // #fin: 0, #looped: 0]. The nested `#tim` list is why the recursion
        // matters — a flat numeric-only pass would reject it.
        (Datum::PropList(pairs, _), Datum::Int(_) | Datum::Float(_)) => {
            let pair_refs: Vec<(DatumRef, DatumRef)> = pairs.iter().cloned().collect();
            let mut result_pairs = VecDeque::new();
            for (key_ref, value_ref) in &pair_refs {
                let scaled = multiply_datums(value_ref.clone(), right_ref.clone(), player)?;
                let scaled_ref = player.alloc_datum(scaled);
                result_pairs.push_back((key_ref.clone(), scaled_ref));
            }
            Datum::PropList(result_pairs, false)
        }
        (Datum::Int(_) | Datum::Float(_), Datum::PropList(..)) => {
            multiply_datums(right_ref.clone(), left_ref.clone(), player)?
        }

        // scalar * List — Director's element-wise multiply is commutative, so
        // reuse the List * scalar arms above rather than keeping a second, subtly
        // different copy. These were previously limited to 2-element lists, so a
        // 3-element vector (`-1.9 * [-25.2, -43.2, 0]`, as the physics code in
        // Hey Arnold! Runaway Bus writes it) fell through to a type error.
        (Datum::Int(_) | Datum::Float(_), Datum::List(..)) => {
            multiply_datums(right_ref.clone(), left_ref.clone(), player)?
        }

        // Transform3d * Vector = apply transform to point
        (Datum::Transform3d(m), Datum::Vector(v)) => {
            let x = m[0]*v[0] + m[4]*v[1] + m[8]*v[2]  + m[12];
            let y = m[1]*v[0] + m[5]*v[1] + m[9]*v[2]  + m[13];
            let z = m[2]*v[0] + m[6]*v[1] + m[10]*v[2] + m[14];
            Datum::Vector([x, y, z])
        }
        // Transform3d * Transform3d = matrix multiply.
        // Director transforms are column-major / column-vector: `transform * vector`
        // applies the transform to the vector (T·v), so `A * B` must be the standard
        // product A·B for (A*B)*v == A*(B*v) to hold. `A * B` therefore applies B's
        // effects first, then A's (e.g. `bone.worldTransform * localOffset` places the
        // local offset in the bone's world frame). Column-major indexing: M[row][col] = m[col*4+row].
        (Datum::Transform3d(a), Datum::Transform3d(b)) => {
            let mut r = [0.0f64; 16];
            for col in 0..4 {
                for row in 0..4 {
                    r[col * 4 + row] =
                        a[row] * b[col*4]
                        + a[4 + row] * b[col*4 + 1]
                        + a[8 + row] * b[col*4 + 2]
                        + a[12 + row] * b[col*4 + 3];
                }
            }
            Datum::transform3d(r)
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
    // See `symbol_as_arithmetic_operand`.
    let left = symbol_as_arithmetic_operand(&left).unwrap_or(left);
    let right = symbol_as_arithmetic_operand(&right).unwrap_or(right);

    let result = match (&left, &right) {
        (Datum::Void, _) => Datum::Int(0),
        (Datum::Int(_), Datum::Void) | (Datum::Float(_), Datum::Void) => Datum::Int(0), // div by VOID → 0
        (Datum::Int(left), Datum::Int(right)) => {
            // Lingo coerces divisor 0 to 1 (ScummVM: LC::divData)
            let d = if *right == 0 { 1 } else { *right };
            Datum::Int(left / d)
        }
        (Datum::Int(left), Datum::Float(right)) => {
            let d = if *right == 0.0 { 1.0 } else { *right };
            Datum::Float((*left as f64) / d)
        }
        (Datum::Float(left), Datum::Int(right)) => {
            let d = if *right == 0 { 1.0 } else { *right as f64 };
            Datum::Float(left / d)
        }
        (Datum::Float(left), Datum::Float(right)) => {
            let d = if *right == 0.0 { 1.0 } else { *right };
            Datum::Float(left / d)
        }
        // Vector / scalar
        (Datum::Vector(v), Datum::Int(s)) => { let s = *s as f64; if s == 0.0 { Datum::Vector([0.0, 0.0, 0.0]) } else { Datum::Vector([v[0] / s, v[1] / s, v[2] / s]) } }
        (Datum::Vector(v), Datum::Float(s)) => if *s == 0.0 { Datum::Vector([0.0, 0.0, 0.0]) } else { Datum::Vector([v[0] / s, v[1] / s, v[2] / s]) },
        (Datum::Point(a, af), Datum::Int(right)) => {
            // Int / Int = Int (truncating), preserving int type per-component
            let d = if *right == 0 { 1 } else { *right };
            let vals = [a[0] / d as f64, a[1] / d as f64];
            // For int/int division, result components that were int stay int (truncated)
            let result_vals = [
                if Datum::inline_is_float(*af, 0) { vals[0] } else { (a[0] as i32 / d) as f64 },
                if Datum::inline_is_float(*af, 1) { vals[1] } else { (a[1] as i32 / d) as f64 },
            ];
            Datum::Point(result_vals, *af)
        }
        (Datum::Point(a, af), Datum::Float(right)) => {
            let d = if *right == 0.0 { 1.0 } else { *right };
            let (vals, flags) = inline_scalar_2(*a, *af, d, true, |x, y| x / y);
            Datum::Point(vals, flags)
        }
        (Datum::Float(left), Datum::Point(b, bf)) => {
            let vals = [
                if b[0] == 0.0 { 0.0 } else { left / b[0] },
                if b[1] == 0.0 { 0.0 } else { left / b[1] },
            ];
            Datum::Point(vals, 0b11) // float / anything = float
        }
        (Datum::Point(a, af), Datum::Point(b, bf)) => {
            // Per-component int-or-float division: if either operand is float
            // at that index, do float division; else integer truncating division.
            let flags = *af | *bf;
            let vals = [
                if b[0] == 0.0 { 0.0 }
                else if Datum::inline_is_float(flags, 0) { a[0] / b[0] }
                else { (a[0] as i32 / b[0] as i32) as f64 },
                if b[1] == 0.0 { 0.0 }
                else if Datum::inline_is_float(flags, 1) { a[1] / b[1] }
                else { (a[1] as i32 / b[1] as i32) as f64 },
            ];
            Datum::Point(vals, flags)
        }
        (Datum::Point(a, af), Datum::List(_, ref_list, _)) if ref_list.len() == 2 => {
            let (bv, bf) = list_to_point_vals(player, ref_list)?;
            let flags = *af | bf;
            let vals = [
                if bv[0] == 0.0 { 0.0 }
                else if Datum::inline_is_float(flags, 0) { a[0] / bv[0] }
                else { (a[0] as i32 / bv[0] as i32) as f64 },
                if bv[1] == 0.0 { 0.0 }
                else if Datum::inline_is_float(flags, 1) { a[1] / bv[1] }
                else { (a[1] as i32 / bv[1] as i32) as f64 },
            ];
            Datum::Point(vals, flags)
        }
        (Datum::Rect(a, af), Datum::Int(right)) => {
            let d = if *right == 0 { 1 } else { *right };
            let result_vals = [
                if Datum::inline_is_float(*af, 0) { a[0] / d as f64 } else { (a[0] as i32 / d) as f64 },
                if Datum::inline_is_float(*af, 1) { a[1] / d as f64 } else { (a[1] as i32 / d) as f64 },
                if Datum::inline_is_float(*af, 2) { a[2] / d as f64 } else { (a[2] as i32 / d) as f64 },
                if Datum::inline_is_float(*af, 3) { a[3] / d as f64 } else { (a[3] as i32 / d) as f64 },
            ];
            Datum::Rect(result_vals, *af)
        }
        (Datum::Rect(a, af), Datum::Float(right)) => {
            let d = if *right == 0.0 { 1.0 } else { *right };
            let (vals, flags) = inline_scalar_4(*a, *af, d, true, |x, y| x / y);
            Datum::Rect(vals, flags)
        }
        (Datum::Int(left), Datum::String(right)) => {
            let right_val = TypeHandlers::float_impl(right).ok_or_else(|| {
                ScriptError::new(format!("Cannot divide int by string: {}", right))
            })?;
            Datum::Float((*left as f64) / right_val)
        }
        (Datum::Float(left), Datum::String(right)) => {
            let right_val = TypeHandlers::float_impl(right).ok_or_else(|| {
                ScriptError::new(format!("Cannot divide float by string: {}", right))
            })?;
            Datum::Float(left / right_val)
        }
        (Datum::String(left), Datum::Int(right)) => {
            let left_float = TypeHandlers::float_impl(left).unwrap_or(0.0);
            Datum::Float(left_float / (*right as f64))
        }
        (Datum::String(left), Datum::Float(right)) => {
            let left_float = TypeHandlers::float_impl(left).unwrap_or(0.0);
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
        // A color combined with a scalar coerces to its packed integer value
        // (Director: rgb(r,g,b) → r<<16|g<<8|b, paletteIndex → index). unicraft's
        // terrainPreview samples a heightmap via `getPixel(x,y) / 65536.0` (no
        // #integer) and relies on this to pull the red channel out of the packed RGB.
        (Datum::ColorRef(c), Datum::Int(r)) => {
            if *r == 0 { Datum::Int(0) } else { Datum::Int(color_ref_to_i32(c) / *r) }
        }
        (Datum::ColorRef(c), Datum::Float(r)) => Datum::Float(color_ref_to_i32(c) as f64 / *r),
        (Datum::Int(l), Datum::ColorRef(c)) => {
            let d = color_ref_to_i32(c);
            if d == 0 { Datum::Int(0) } else { Datum::Int(*l / d) }
        }
        (Datum::Float(l), Datum::ColorRef(c)) => Datum::Float(l / color_ref_to_i32(c) as f64),
        (Datum::List(_, list, _), Datum::Int(_) | Datum::Float(_)) => {
            let mut result = VecDeque::new();
            for item in list {
                let a_val = player.get_datum(item).clone();
                let b_val = right.clone();
                let quot = divide_datums(
                    player.alloc_datum(a_val),
                    player.alloc_datum(b_val),
                    player,
                )?;
                result.push_back(player.alloc_datum(quot));
            }
            Datum::List(DatumType::List, result, false)
        }
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
