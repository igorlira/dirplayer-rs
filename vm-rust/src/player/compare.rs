use log::warn;

use crate::{console_warn, director::lingo::datum::Datum};

use super::{
    allocator::{DatumAllocator, DatumAllocatorTrait},
    bitmap::bitmap::PaletteRef,
    handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
    DatumRef, ScriptError,
};

pub fn datum_equals(
    left: &Datum,
    right: &Datum,
    allocator: &DatumAllocator,
) -> Result<bool, ScriptError> {
    match (left, right) {
        (Datum::Int(left), Datum::Int(right)) => Ok(*left == *right),
        (Datum::Int(left), Datum::Float(right)) => Ok((*left as f32) == *right), // TODO: is this correct? Flutter compares ints instead
        (Datum::Int(left), Datum::Void) => Ok(*left == 0),
        // Handle string-to-int comparison (e.g., "2" should match key 2)
        (Datum::String(s), Datum::Int(i)) | (Datum::Int(i), Datum::String(s)) => {
            Ok(s.parse::<i32>().ok() == Some(*i))
        }
        (Datum::Float(left), Datum::Int(right)) => Ok(*left == (*right as f32)),
        (Datum::Float(left), Datum::Float(right)) => Ok(*left == *right),
        // String equality: case-insensitive (like Director `=` operator)
        (Datum::String(l), Datum::String(r)) => Ok(l.eq_ignore_ascii_case(r)),
        // StringChunk comparison for equality: case-insensitive too
        (Datum::StringChunk(..), Datum::String(right)) => {
            let left_val = left.string_value()?;
            Ok(left_val.eq_ignore_ascii_case(right))
        }
        (Datum::String(left), Datum::StringChunk(..)) => {
            let right_val = right.string_value()?;
            Ok(left.eq_ignore_ascii_case(&right_val))
        }
        (Datum::StringChunk(..), Datum::StringChunk(..)) => {
            let left_val = left.string_value()?;
            let right_val = right.string_value()?;
            Ok(left_val.eq_ignore_ascii_case(&right_val))
        }
        (Datum::ScriptInstanceRef(left), Datum::ScriptInstanceRef(right)) => Ok(**left == **right),
        (Datum::Symbol(left), Datum::Symbol(right)) => Ok(left.eq_ignore_ascii_case(right)),
        (Datum::Void, Datum::Void) => Ok(true),
        (Datum::ColorRef(left), Datum::ColorRef(right)) => Ok(*left == *right),
        (Datum::Int(_), Datum::Symbol(_)) => Ok(false),
        (Datum::Void, Datum::Int(right)) => Ok(*right == 0),
        (Datum::String(_), Datum::ScriptInstanceRef(_)) => Ok(false),
        (Datum::CastMember(member_ref), Datum::Void) => Ok(!member_ref.is_valid()), // TODO return true if member is empty?
        (Datum::ScriptInstanceRef(_), Datum::Int(_)) => Ok(false),
        (Datum::IntPoint(_), Datum::Int(_)) => Ok(false),
        (Datum::PropList(..), Datum::Int(_)) => Ok(false),
        (Datum::BitmapRef(_), Datum::Void) => Ok(false),
        (Datum::Symbol(_), Datum::CastMember(_)) => Ok(false),
        (Datum::PaletteRef(_), Datum::CastMember(_)) => Ok(false), // TODO should we compare the cast member?
        (Datum::Void, Datum::String(_)) => Ok(false),
        (Datum::SpriteRef(_), Datum::Int(_)) => Ok(false),
        (Datum::Symbol(_), Datum::StringChunk(..)) => Ok(false),
        (Datum::Symbol(_), Datum::String(_)) => Ok(false),
        (Datum::Symbol(_), Datum::Void) => Ok(false),
        (Datum::String(_), Datum::Symbol(_)) => Ok(false),
        (Datum::String(left), Datum::Int(right)) => Ok(right.to_string().eq(left)),
        (Datum::List(_, left, _), Datum::List(_, right, _)) => {
            if left.len() != right.len() {
                return Ok(false);
            }
            for (left_item, right_item) in left.iter().zip(right.iter()) {
                let left_item = allocator.get_datum(left_item);
                let right_item = allocator.get_datum(right_item);
                if !datum_equals(left_item, right_item, allocator)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Datum::TimeoutRef(left), Datum::TimeoutRef(right)) => {
            // TODO: they're only equal if the timeout has been scheduled
            Ok(left == right)
        }
        (Datum::SpriteRef(left), Datum::SpriteRef(right)) => Ok(left == right),
        (Datum::CastMember(left), Datum::CastMember(right)) => {
            Ok(CastMemberRefHandlers::get_cast_slot_number(
                left.cast_lib as u32,
                left.cast_member as u32,
            ) == CastMemberRefHandlers::get_cast_slot_number(
                right.cast_lib as u32,
                right.cast_member as u32,
            ))
        }
        (Datum::IntPoint(left), Datum::IntPoint(right)) => Ok(left == right),
        (Datum::Null, Datum::Int(_)) => Ok(false),
        (Datum::PropList(..), Datum::Void) => Ok(false),
        (Datum::Symbol(_), Datum::Int(_)) => Ok(false),
        (Datum::PaletteRef(palette_ref), Datum::Symbol(symbol)) => match palette_ref {
            PaletteRef::BuiltIn(palette) => {
                Ok(palette.symbol_string().eq_ignore_ascii_case(&symbol))
            }
            _ => Ok(false),
        },
        (Datum::String(_), Datum::Void) => Ok(false),
        (Datum::Void, Datum::Symbol(_)) => Ok(false),
        (Datum::IntRect(left), Datum::IntRect(right)) => {
            Ok(left.0 == right.0 && left.1 == right.1 && left.2 == right.2 && left.3 == right.3)
        }
        (Datum::ColorRef(color_ref), Datum::String(string)) => {
            warn!(
                "Datum equals not supported for ColorRef and String: {:?} and {}",
                color_ref, string
            );
            Ok(false)
        }
        _ => {
            warn!(
                "datum_equals not supported for types: {} and {}",
                left.type_str(),
                right.type_str()
            );
            Ok(false)
        }
    }
}

#[allow(dead_code)]
pub fn datum_greater_than(left: &Datum, right: &Datum) -> Result<bool, ScriptError> {
    match (left, right) {
        (Datum::Int(left), Datum::Int(right)) => Ok(*left > *right),
        (Datum::Int(left), Datum::Float(right)) => Ok((*left as f32) > *right), // TODO: is this correct? Flutter compares ints instead
        (Datum::Int(_), Datum::Void) => Ok(false),
        (Datum::Int(left), Datum::String(right)) => {
            if let Ok(right_number) = right.parse::<i32>() {
                Ok(*left > right_number)
            } else {
                Ok(right.is_empty())
            }
        }
        (Datum::Int(_), _) => {
            warn!("Datum isGreaterThan not supported for int");
            Ok(false)
        }
        (Datum::Float(left), Datum::Int(right)) => Ok(*left > (*right as f32)),
        (Datum::Float(left), Datum::Float(right)) => Ok(*left > *right),
        (Datum::IntPoint(left), Datum::IntPoint(right)) => Ok(left.0 > right.0 && left.1 > right.1),
        (Datum::Void, Datum::Int(_)) => Ok(false),
        _ => {
            warn!(
                "datum_greater_than not supported for types: {} and {}",
                left.type_str(),
                right.type_str()
            );
            Ok(false)
        }
    }
}

pub fn datum_less_than(left: &Datum, right: &Datum) -> Result<bool, ScriptError> {
    match (left, right) {
        (Datum::Int(left), Datum::Int(right)) => Ok(*left < *right),
        (Datum::Int(left), Datum::Float(right)) => Ok((*left as f32) < *right), // TODO: is this correct? Flutter compares ints instead
        (Datum::Int(_), Datum::Void) => Ok(false),
        (Datum::Int(left), Datum::String(right)) => {
            if let Ok(right_number) = right.parse::<i32>() {
                Ok(*left < right_number)
            } else {
                Ok(!right.is_empty())
            }
        }
        (Datum::Int(_), _) => {
            warn!("Datum isLessThan not supported for int");
            Ok(false)
        }
        (Datum::Float(left), Datum::Int(right)) => Ok(*left < (*right as f32)),
        (Datum::Float(left), Datum::Float(right)) => Ok(*left < *right),
        (Datum::IntPoint(left), Datum::IntPoint(right)) => Ok(left.0 < right.0 && left.1 < right.1),
        (Datum::String(..), Datum::String(..)) => Ok(false),
        _ => {
            warn!(
                "datum_less_than not supported for types: {} and {}",
                left.type_str(),
                right.type_str()
            );
            Ok(false)
        }
    }
}

pub fn datum_is_zero(datum: &Datum, datums: &DatumAllocator) -> Result<bool, ScriptError> {
    Ok(match datum {
        Datum::Int(value) => *value == 0,
        Datum::Float(value) => *value == 0.0,
        Datum::Void => true,
        Datum::ScriptInstanceRef(_) => false,
        Datum::Null => true,
        Datum::IntPoint(_) => false,
        _ => {
            warn!("datum_is_zero not supported for type: {}", datum.type_str());
            datum.int_value()? == 0
        }
    })
}

pub fn sort_datums(
    datums: &Vec<DatumRef>,
    allocator: &DatumAllocator,
) -> Result<Vec<DatumRef>, ScriptError> {
    let mut sorted_list = datums.clone();
    sorted_list.sort_by(|a, b| {
        let left = allocator.get_datum(a);
        let right = allocator.get_datum(b);

        if datum_equals(left, right, allocator).unwrap() {
            return std::cmp::Ordering::Equal;
        } else if datum_less_than(left, right).unwrap() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });
    Ok(sorted_list)
}
