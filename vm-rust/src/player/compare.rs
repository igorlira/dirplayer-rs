use crate::{console_warn, director::lingo::datum::Datum};

use super::{bitmap::bitmap::PaletteRef, get_datum, handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers, DatumRef, DatumRefMap, ScriptError};

pub fn datum_equals(left: &Datum, right: &Datum, datum: &DatumRefMap) -> Result<bool, ScriptError> {
  match (left, right) {
    (Datum::Int(left), Datum::Int(right)) => Ok(*left == *right),
    (Datum::Int(left), Datum::Float(right)) => Ok((*left as f32) == *right), // TODO: is this correct? Flutter compares ints instead
    (Datum::Int(left), Datum::Void) => Ok(*left == 0),
    (Datum::Int(left), Datum::String(right)) => {
      if let Ok(right_number) = right.parse::<i32>() {
        Ok(*left == right_number)
      } else {
        Ok(false)
      }
    }
    (Datum::Float(left), Datum::Int(right)) => Ok(*left == (*right as f32)),
    (Datum::Float(left), Datum::Float(right)) => Ok(*left == *right),
    (Datum::String(left), Datum::String(right)) => Ok(left == right),
    (Datum::String(left), Datum::StringChunk(..)) => {
      let right = right.string_value(datum)?;
      Ok(left.eq(&right))
    },
    (Datum::StringChunk(..), Datum::String(right)) => {
      let left = left.string_value(datum)?;
      Ok(left.eq(right))
    },
    (Datum::StringChunk(..), Datum::StringChunk(..)) => {
      let left = left.string_value(datum)?;
      let right = right.string_value(datum)?;
      Ok(left == right)
    },
    (Datum::ScriptInstanceRef(left), Datum::ScriptInstanceRef(right)) => Ok(*left == *right),
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
    (Datum::String(left), Datum::Int(right)) => {
      Ok(right.to_string().eq(left))
    }
    (Datum::List(_, left, _), Datum::List(_, right, _)) => {
      if left.len() != right.len() {
        return Ok(false);
      }
      for (left_item, right_item) in left.iter().zip(right.iter()) {
        let left_item = get_datum(*left_item, datum);
        let right_item = get_datum(*right_item, datum);
        if !datum_equals(left_item, right_item, datum)? {
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
    (Datum::CastMember(left), Datum::CastMember(right)) => Ok(
      CastMemberRefHandlers::get_cast_slot_number(left.cast_lib as u32, left.cast_member as u32) == CastMemberRefHandlers::get_cast_slot_number(right.cast_lib as u32, right.cast_member as u32)
    ),
    (Datum::IntPoint(left), Datum::IntPoint(right)) => Ok(left == right),
    (Datum::Null, Datum::Int(_)) => Ok(false),
    (Datum::PropList(..), Datum::Void) => Ok(false),
    (Datum::Symbol(_), Datum::Int(_)) => Ok(false),
    (Datum::PaletteRef(palette_ref), Datum::Symbol(symbol)) => match palette_ref {
      PaletteRef::BuiltIn(palette) => Ok(palette.symbol_string().eq_ignore_ascii_case(&symbol)),
      _ => Ok(false)
    }
    (Datum::String(_), Datum::Void) => Ok(false),
    (Datum::Void, Datum::Symbol(_)) => Ok(false),
    (Datum::ColorRef(color_ref), Datum::String(string)) => {
      console_warn!("Datum equals not supported for ColorRef and String: {:?} and {}", color_ref, string);
      Ok(false)
    }
    _ => {
      console_warn!("datum_equals not supported for types: {} and {}", left.type_str(), right.type_str());
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
      console_warn!("Datum isGreaterThan not supported for int");
      Ok(false)
    }
    (Datum::Float(left), Datum::Int(right)) => Ok(*left > (*right as f32)),
    (Datum::Float(left), Datum::Float(right)) => Ok(*left > *right),
    (Datum::IntPoint(left), Datum::IntPoint(right)) => Ok(left.0 > right.0 && left.1 > right.1),
    (Datum::Void, Datum::Int(_)) => Ok(false),
    _ => {
      console_warn!("datum_greater_than not supported for types: {} and {}", left.type_str(), right.type_str());
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
      console_warn!("Datum isLessThan not supported for int");
      Ok(false)
    }
    (Datum::Float(left), Datum::Int(right)) => Ok(*left < (*right as f32)),
    (Datum::Float(left), Datum::Float(right)) => Ok(*left < *right),
    (Datum::IntPoint(left), Datum::IntPoint(right)) => Ok(left.0 < right.0 && left.1 < right.1),
    _ => {
      console_warn!("datum_less_than not supported for types: {} and {}", left.type_str(), right.type_str());
      Ok(false)
    }
  }
}

pub fn datum_is_zero(datum: &Datum, datums: &DatumRefMap) -> Result<bool, ScriptError>{
  Ok(match datum {
    Datum::Int(value) => *value == 0,
    Datum::Float(value) => *value == 0.0,
    Datum::Void => true,
    Datum::ScriptInstanceRef(_) => false,
    Datum::Null => true,
    Datum::IntPoint(_) => false,
    _ => {
      console_warn!("datum_is_zero not supported for type: {}", datum.type_str());
      datum.int_value(&datums)? == 0
    }
  })
}

pub fn sort_datums(datums: &Vec<DatumRef>, all_datums: &DatumRefMap) -> Result<Vec<DatumRef>, ScriptError> {
  let mut sorted_list = datums.clone();
  sorted_list.sort_by(|a, b| {
    let left = get_datum(*a, all_datums);
    let right = get_datum(*b, all_datums);

    if datum_equals(left, right, &all_datums).unwrap() {
      return std::cmp::Ordering::Equal
    } else if datum_less_than(left, right).unwrap() {
      std::cmp::Ordering::Less
    } else {
      std::cmp::Ordering::Greater
    }
  });
  Ok(sorted_list)
}
