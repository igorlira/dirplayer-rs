use crate::director::lingo::datum::{Datum, DatumType};
use crate::player::datum_ref::DatumRef;

use crate::player::allocator::{DatumAllocator, DatumAllocatorTrait};
use crate::player::reserve_player_ref;

#[derive(Clone, Debug, PartialEq)]
pub enum StaticDatum {
    Int(i32),
    Float(f32),
    String(String),
    Symbol(String),
    List(Vec<StaticDatum>),
    PropList(Vec<(StaticDatum, StaticDatum)>),
    IntPoint(i32, i32),
    IntRect(i32, i32, i32, i32),
    Void,
}

impl From<DatumRef> for StaticDatum {
    fn from(dref: DatumRef) -> Self {
        // Resolve DatumRef into a Datum
        let datum_opt: Option<Datum> = match &dref {
            DatumRef::Void => None,
            DatumRef::Ref(_, _) => {
                reserve_player_ref(|player| Some(player.allocator.get_datum(&dref).clone()))
            }
        };

        match datum_opt {
            None => StaticDatum::Void,
            Some(datum) => match datum {
                Datum::Int(i) => StaticDatum::Int(i),
                Datum::Float(f) => StaticDatum::Float(f),
                Datum::String(s) => StaticDatum::String(s),
                Datum::Symbol(s) => StaticDatum::Symbol(s),
                Datum::List(_, vec, _) => {
                    StaticDatum::List(vec.into_iter().map(StaticDatum::from).collect())
                }
                Datum::PropList(pairs, _) => StaticDatum::PropList(
                    pairs
                        .into_iter()
                        .map(|(k, v)| (StaticDatum::from(k), StaticDatum::from(v)))
                        .collect(),
                ),
                Datum::IntPoint((x, y)) => StaticDatum::IntPoint(x, y),
                Datum::IntRect((l, t, r, b)) => StaticDatum::IntRect(l, t, r, b),
                _ => StaticDatum::Void,
            },
        }
    }
}

impl From<Datum> for StaticDatum {
    fn from(d: Datum) -> Self {
        match d {
            Datum::Int(i) => StaticDatum::Int(i),
            Datum::Float(f) => StaticDatum::Float(f),
            Datum::String(s) => StaticDatum::String(s),
            Datum::Symbol(s) => StaticDatum::Symbol(s),
            Datum::List(_, vec, _) => {
                StaticDatum::List(vec.into_iter().map(StaticDatum::from).collect())
            }
            Datum::PropList(pairs, _) => StaticDatum::PropList(
                pairs
                    .into_iter()
                    .map(|(k, v)| (StaticDatum::from(k), StaticDatum::from(v)))
                    .collect(),
            ),
            Datum::IntPoint((x, y)) => StaticDatum::IntPoint(x, y),
            Datum::IntRect((l, t, r, b)) => StaticDatum::IntRect(l, t, r, b),
            _ => StaticDatum::Void,
        }
    }
}

// Helper function to convert StaticDatum to common types
impl StaticDatum {
    pub fn as_string(&self) -> Option<String> {
        match self {
            StaticDatum::String(s) => Some(s.clone()),
            StaticDatum::Symbol(s) => Some(s.clone()),
            StaticDatum::Int(i) => Some(i.to_string()),
            StaticDatum::Float(f) => Some(f.to_string()),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i32> {
        match self {
            StaticDatum::Int(i) => Some(*i),
            StaticDatum::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f32> {
        match self {
            StaticDatum::Float(f) => Some(*f),
            StaticDatum::Int(i) => Some(*i as f32),
            StaticDatum::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            StaticDatum::Int(i) => Some(*i != 0),
            _ => None,
        }
    }
}

fn static_datum_to_runtime(param: &StaticDatum, allocator: &mut DatumAllocator) -> DatumRef {
    match param {
        StaticDatum::String(s) => allocator.alloc_datum(Datum::String(s.clone())).unwrap(),
        StaticDatum::Int(i) => allocator.alloc_datum(Datum::Int(*i)).unwrap(),
        StaticDatum::Float(f) => allocator.alloc_datum(Datum::Float(*f)).unwrap(),
        StaticDatum::Symbol(s) => allocator.alloc_datum(Datum::Symbol(s.clone())).unwrap(),
        StaticDatum::List(items) => {
            let datum_refs: Vec<DatumRef> = items
                .iter()
                .map(|item| static_datum_to_runtime(item, allocator))
                .collect();
            allocator
                .alloc_datum(Datum::List(DatumType::List, datum_refs, false))
                .unwrap()
        }
        StaticDatum::PropList(items) => {
            let datum_refs: Vec<(DatumRef, DatumRef)> = items
                .iter()
                .map(|(key, val)| {
                    let key_ref = static_datum_to_runtime(key, allocator);
                    let val_ref = static_datum_to_runtime(val, allocator);
                    (key_ref, val_ref)
                })
                .collect();
            allocator
                .alloc_datum(Datum::PropList(datum_refs, false))
                .unwrap()
        }
        StaticDatum::IntPoint(x, y) => allocator.alloc_datum(Datum::IntPoint((*x, *y))).unwrap(),
        StaticDatum::IntRect(left, top, right, bottom) => allocator
            .alloc_datum(Datum::IntRect((*left, *top, *right, *bottom)))
            .unwrap(),
        StaticDatum::Void => DatumRef::Void,
        _ => {
            web_sys::console::log_1(&format!("⚠️ Unhandled StaticDatum type, using Void").into());
            DatumRef::Void
        }
    }
}
