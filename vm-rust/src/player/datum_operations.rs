use std::cmp::min;

use crate::director::lingo::datum::{Datum, DatumType};

use super::{get_datum, sprite::ColorRef, DirPlayer, ScriptError};

pub fn add_datums(left: Datum, right: Datum, player: &mut DirPlayer) -> Result<Datum, ScriptError> {
  match (&left, &right) {
    (Datum::Void, some) => Ok(some.clone()),
    (some, Datum::Void) => Ok(some.clone()),
    (Datum::Int(a), Datum::Int(b)) => Ok(Datum::Int(a + b)),
    (Datum::Float(a), Datum::Float(b)) => Ok(Datum::Float(a + b)),
    (Datum::Float(a), Datum::Int(b)) => Ok(Datum::Float(a + (*b as f32))),
    (Datum::Int(a), Datum::Float(b)) => Ok(Datum::Float((*a as f32) + b)),
    (Datum::IntRect(a), Datum::IntRect(b)) => Ok(Datum::IntRect((a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3))),
    (Datum::IntRect(a), Datum::List(_, ref_list, _)) => {
      if ref_list.len() == 4 {
        let b = ref_list.iter()
          .map(|r|
            get_datum(r, &player.datums).int_value()
              .map(|x| x as i32)
          ).collect::<Result<Vec<i32>, ScriptError>>()?;
        Ok(Datum::IntRect((a.0 + b[0], a.1 + b[1], a.2 + b[2], a.3 + b[3])))
      } else {
        Err(ScriptError::new(format!("Invalid list length for add_datums: {}", ref_list.len())))
      }
    },
    (Datum::List(_, list_a, _), Datum::List(_, list_b, _)) => {
      let intersection_count = min(list_a.len(), list_b.len());
      let mut result = Vec::with_capacity(intersection_count);
      for i in 0..intersection_count {
        let a = get_datum(&list_a[i], &player.datums).clone();
        let b = get_datum(&list_b[i], &player.datums).clone();
        let result_datum = add_datums(a, b, player)?;
        result.push(player.alloc_datum(result_datum));
      }
      Ok(Datum::List(DatumType::List, result, false))
    },
    (Datum::List(_, list, _), Datum::Int(i)) => {
      let mut result_refs = vec![];
      for r in list {
        let datum = get_datum(r, &player.datums);
        let result_datum = match datum {
          Datum::Int(n) => Datum::Int(n + i),
          Datum::Float(n) => Datum::Float(n + *i as f32),
          _ => return Err(ScriptError::new(format!("Invalid list element for add_datums: {}", r))),
        };
        result_refs.push(player.alloc_datum(result_datum));
      }
      Ok(Datum::List(DatumType::List, result_refs, false))
    },
    (Datum::IntPoint(a), Datum::IntPoint(b)) => Ok(Datum::IntPoint((a.0 + b.0, a.1 + b.1))),
    (Datum::IntPoint(a), Datum::List(_, ref_list, _)) => {
      if ref_list.len() == 2 {
        let b = ref_list.iter()
          .map(|r| 
            get_datum(r, &player.datums).int_value()
              .map(|x| x as i32)
          )
          .collect::<Result<Vec<i32>, ScriptError>>()?;
        Ok(Datum::IntPoint((a.0 + b[0], a.1 + b[1])))
      } else {
        Err(ScriptError::new(format!("Invalid list length for add_datums: {}", ref_list.len())))
      }
    },
    (Datum::IntPoint(a), Datum::Int(b)) => Ok(Datum::IntPoint((a.0 + *b as i32, a.1 + *b as i32))),
    (Datum::ColorRef(a), Datum::ColorRef(b)) => {
      match (a, b) {
        (ColorRef::PaletteIndex(a), ColorRef::PaletteIndex(b)) => Ok(Datum::ColorRef(ColorRef::PaletteIndex(a + b))),
        (ColorRef::Rgb(a_r, a_g, a_b), ColorRef::Rgb(b_r, b_g, b_b)) => Ok(Datum::ColorRef(ColorRef::Rgb(a_r + b_r, a_g + b_g, a_b + b_b))),
        _ => Err(ScriptError::new(format!("Invalid operands for add_datums: {:?}, {:?}", a, b))),
      }
    },
    _ => Err(ScriptError::new(format!("Invalid operands for add_datums: {}, {}", left.type_str(), right.type_str()))),
  }
}

pub fn subtract_datums(left: Datum, right: Datum, player: &mut DirPlayer) -> Result<Datum, ScriptError> {
  match (&left, &right) {
    (Datum::Int(left), Datum::Int(right)) => Ok(Datum::Int(left.wrapping_sub(*right))),
    (Datum::Float(left), Datum::Float(right)) => Ok(Datum::Float(left - right)),
    (Datum::Float(left), Datum::Int(right)) => Ok(Datum::Float(left - (*right as f32))),
    (Datum::Int(left), Datum::Float(right)) => Ok(Datum::Float((*left as f32) - right)),
    (Datum::IntRect(a), Datum::IntRect(b)) => Ok(Datum::IntRect((a.0.wrapping_sub(b.0), a.1.wrapping_sub(b.1), a.2.wrapping_sub(b.2), a.3.wrapping_sub(b.3)))),
    (Datum::IntRect(a), Datum::List(_, ref_list, _)) => {
      if ref_list.len() == 4 {
        let b = ref_list.iter()
          .map(|r| 
            get_datum(r, &player.datums).int_value()
              .map(|x| x as i32)
          )
          .collect::<Result<Vec<i32>, ScriptError>>()?;
        Ok(Datum::IntRect((a.0.wrapping_sub(b[0]), a.1.wrapping_sub(b[1]), a.2.wrapping_sub(b[2]), a.3.wrapping_sub(b[3]))))
      } else {
        Err(ScriptError::new(format!("Invalid list length for subtract_datums: {}", ref_list.len())))
      }
    },
    (Datum::List(_, list_a, _), Datum::List(_, list_b, _)) => {
      let intersection_count = min(list_a.len(), list_b.len());
      let mut result = Vec::with_capacity(intersection_count);
      for i in 0..intersection_count {
        let a = get_datum(&list_a[i], &player.datums).clone();
        let b = get_datum(&list_b[i], &player.datums).clone();
        let result_datum = subtract_datums(a, b, player)?;
        result.push(player.alloc_datum(result_datum));
      }
      Ok(Datum::List(DatumType::List, result, false))
    },
    (Datum::IntPoint(a), Datum::IntPoint(b)) => Ok(Datum::IntPoint((a.0.wrapping_sub(b.0), a.1.wrapping_sub(b.1)))),
    (Datum::IntPoint(a), Datum::List(_, ref_list, _)) => {
      if ref_list.len() == 2 {
        let b = ref_list.iter()
          .map(|r| get_datum(r, &player.datums).int_value().map(|x| x as i32)).collect::<Result<Vec<i32>, ScriptError>>()?;
        Ok(Datum::IntPoint((a.0.wrapping_sub(b[0]), a.1.wrapping_sub(b[1]))))
      } else {
        Err(ScriptError::new(format!("Invalid list length for subtract_datums: {}", ref_list.len())))
      }
    },
    (Datum::Int(a), Datum::IntPoint(b)) => Ok(Datum::IntPoint(((*a as i32).wrapping_sub(b.0), (*a as i32).wrapping_sub(b.1)))),
    (Datum::ColorRef(a), Datum::ColorRef(b)) => {
      match (a, b) {
        (ColorRef::PaletteIndex(a), ColorRef::PaletteIndex(b)) => Ok(Datum::ColorRef(ColorRef::PaletteIndex(a.wrapping_sub(*b)))),
        (ColorRef::Rgb(a_r, a_g, a_b), ColorRef::Rgb(b_r, b_g, b_b)) => Ok(Datum::ColorRef(ColorRef::Rgb(a_r.wrapping_sub(*b_r), a_g.wrapping_sub(*b_g), a_b.wrapping_sub(*b_b)))),
        _ => Err(ScriptError::new(format!("Invalid operands for subtract_datums: {:?}, {:?}", a, b))),
      }
    },
    (Datum::String(_), Datum::Int(_)) => {
      // returns junk data
      Ok(Datum::Int(0xFFFF))
    },
    (left, Datum::Void) => Ok(left.clone()),
    _ => Err(ScriptError::new(format!("Invalid operands for subtract_datums: {}, {}", left.type_str(), right.type_str()))),
  }
}
