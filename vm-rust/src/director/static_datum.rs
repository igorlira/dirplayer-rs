use std::borrow::Borrow;
use std::collections::VecDeque;
use std::io::Read;

use binary_reader::BinaryReader;
use binary_rw::{BinaryWriter, MemoryStream};
use itertools::Itertools;
use log::warn;

use crate::director::lingo::datum::{Datum, DatumType};
use crate::director::media::reader::MediaReader;
use crate::director::media::writer::MediaWriter;
use crate::player::cast_member::Media;
use crate::player::datum_ref::DatumRef;

use crate::player::allocator::{DatumAllocator, DatumAllocatorTrait};
use crate::player::reserve_player_ref;

#[derive(Clone, Debug, PartialEq)]
pub enum StaticDatum {
    Int(i32),
    Float(f64),
    String(String),
    Symbol(String),
    List(Vec<StaticDatum>),
    PropList(Vec<(StaticDatum, StaticDatum)>),
    IntPoint(i32, i32),
    IntRect(i32, i32, i32, i32),
    Media(Vec<u8>),
    Void,
}

impl From<&DatumRef> for StaticDatum {
    fn from(dref: &DatumRef) -> Self {
        // Resolve DatumRef into a Datum
        let datum_opt: Option<Datum> = match dref {
            DatumRef::Void => None,
            DatumRef::Ref(_, _) => {
                reserve_player_ref(|player| Some(player.allocator.get_datum(dref).clone()))
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
                    StaticDatum::List(vec.iter().map(StaticDatum::from).collect())
                }
                Datum::PropList(pairs, _) => StaticDatum::PropList(
                    pairs
                        .iter()
                        .map(|(k, v)| (StaticDatum::from(k), StaticDatum::from(v)))
                        .collect(),
                ),
                Datum::Point(vals, _flags) => {
                    StaticDatum::IntPoint(vals[0] as i32, vals[1] as i32)
                }
                Datum::Rect(vals, _flags) => {
                    StaticDatum::IntRect(vals[0] as i32, vals[1] as i32, vals[2] as i32, vals[3] as i32)
                }
                _ => reserve_player_ref(|player| StaticDatum::from(player.allocator.get_datum(dref))),
            },
        }
    }
}

impl From<&Datum> for StaticDatum {
    fn from(d: &Datum) -> Self {
        match d {
            Datum::Int(i) => StaticDatum::Int(*i),
            Datum::Float(f) => StaticDatum::Float(*f),
            Datum::String(s) => StaticDatum::String(s.clone()),
            Datum::Symbol(s) => StaticDatum::Symbol(s.clone()),
            Datum::List(_, vec, _) => {
                StaticDatum::List(vec.iter().map(StaticDatum::from).collect())
            }
            Datum::PropList(pairs, _) => StaticDatum::PropList(
                pairs
                    .iter()
                    .map(|(k, v)| (StaticDatum::from(k), StaticDatum::from(v)))
                    .collect(),
            ),
            Datum::Point(vals, _flags) => {
                StaticDatum::IntPoint(vals[0] as i32, vals[1] as i32)
            }
            Datum::Rect(vals, _flags) => {
                StaticDatum::IntRect(vals[0] as i32, vals[1] as i32, vals[2] as i32, vals[3] as i32)
            }
            Datum::Media(media) => {
                let mut stream = MemoryStream::new();
                {
                    let mut writer = BinaryWriter::new(&mut stream, binary_rw::Endian::Big);
                    writer.write_media(media, binary_rw::Endian::Little).unwrap();
                }
                let bytes: Vec<u8> = stream.into();
                StaticDatum::Media(bytes)
            }
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

    pub fn as_float(&self) -> Option<f64> {
        match self {
            StaticDatum::Float(f) => Some(*f),
            StaticDatum::Int(i) => Some(*i as f64),
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

pub fn static_datum_to_runtime(param: &StaticDatum, allocator: &mut DatumAllocator) -> DatumRef {
    match param {
        StaticDatum::String(s) => allocator.alloc_datum(Datum::String(s.clone())).unwrap(),
        StaticDatum::Int(i) => allocator.alloc_datum(Datum::Int(*i)).unwrap(),
        StaticDatum::Float(f) => allocator.alloc_datum(Datum::Float(*f)).unwrap(),
        StaticDatum::Symbol(s) => allocator.alloc_datum(Datum::Symbol(s.clone())).unwrap(),
        StaticDatum::List(items) => {
            let datum_refs: VecDeque<DatumRef> = items
                .iter()
                .map(|item| static_datum_to_runtime(item, allocator))
                .collect();
            allocator
                .alloc_datum(Datum::List(DatumType::List, datum_refs, false))
                .unwrap()
        }
        StaticDatum::PropList(items) => {
            let datum_refs: VecDeque<(DatumRef, DatumRef)> = items
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
        StaticDatum::IntPoint(x, y) => {
            allocator.alloc_datum(Datum::Point([*x as f64, *y as f64], 0)).unwrap()
        }
        StaticDatum::IntRect(left, top, right, bottom) => {
            allocator.alloc_datum(Datum::Rect([*left as f64, *top as f64, *right as f64, *bottom as f64], 0)).unwrap()
        }
        StaticDatum::Media(bytes) => {
            let mut reader = BinaryReader::from_u8(bytes);
            reader.set_endian(binary_reader::Endian::Big);
            match reader.read_media() {
                Ok(media) => allocator.alloc_datum(Datum::Media(media)).unwrap(),
                Err(e) => {
                    web_sys::console::warn_1(&format!("Failed to parse media from StaticDatum: {}", e).into());
                    DatumRef::Void
                }
            }
        }
        StaticDatum::Void => DatumRef::Void,
        _ => {
            warn!("⚠️ Unhandled StaticDatum type, using Void");
            DatumRef::Void
        }
    }
}
