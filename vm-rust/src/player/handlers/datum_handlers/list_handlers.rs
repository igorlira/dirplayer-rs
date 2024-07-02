use crate::{director::lingo::datum::{datum_bool, Datum}, player::{compare::{datum_equals, datum_less_than}, get_datum, player_duplicate_datum, reserve_player_mut, reserve_player_ref, DatumRef, DatumRefMap, ScriptError, VOID_DATUM_REF}};

pub struct ListDatumHandlers {}
pub struct ListDatumUtils {}

impl ListDatumUtils {
  fn find_index_to_add(list_vec: &Vec<DatumRef>, item: &DatumRef, datums: &DatumRefMap) -> Result<i32, ScriptError> {
    let mut low = 0;
    let mut high = list_vec.len() as i32;
    let item = get_datum(item, datums);

    while low < high {
      let mid = (low + high) / 2;
      let left = get_datum(list_vec.get(mid as usize).unwrap(), datums);
      if datum_less_than(left, item)? {
        low = mid + 1;
      } else {
        high = mid;
      }
    }

    Ok(low)
  }

  pub fn get_prop(list_vec: &Vec<DatumRef>, prop_name: &String, _datums: &DatumRefMap) -> Result<Datum, ScriptError> {
    match prop_name.as_str() {
      "count" => Ok(Datum::Int(list_vec.len() as i32)),
      "ilk" => Ok(Datum::Symbol("list".to_string())),
      _ => Err(ScriptError::new(format!("No property {prop_name} for list datum")))
    }
  }
}

impl ListDatumHandlers {
  pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let list_vec = player.get_datum(datum).to_list()?;
      let position = player.get_datum(&args[0]).int_value()? - 1;
      if position < 0 || position >= list_vec.len() as i32 {
        return Err(ScriptError::new(format!("Index out of bounds: {}", position)))
      }

      Ok(list_vec[position as usize].clone())
    })
  }

  pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let position = player.get_datum(&args[0]).int_value()?;
      let (_, list_vec, ..) = player.get_datum_mut(datum).to_list_mut()?;
      let index = position - 1;
      let item_ref = &args[1];

      if index < list_vec.len() as i32 {
        list_vec[index as usize] = item_ref.clone();
      } else {
        let padding_size = index - list_vec.len() as i32;
        for _ in 0..padding_size {
          // TODO: should this be filled with zeroes instead?
          list_vec.push(VOID_DATUM_REF.clone());
        }
        list_vec.push(item_ref.clone());
      }
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn call(datum: &DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      "count" => Self::count(datum, args),
      "getAt" => Self::get_at(datum, args),
      "setAt" => Self::set_at(datum, args),
      "sort" => Self::sort(datum, args),
      "getOne" => Self::get_one(datum, args),
      "add" => Self::add(datum, args),
      "duplicate" => Self::duplicate(datum, args),
      "addAt" => Self::add_at(datum, args),
      "getLast" => Self::get_last(datum, args),
      "append" => Self::append(datum, args),
      "deleteOne" => Self::delete_one(datum, args),
      "deleteAt" => Self::delete_at(datum, args),
      "findPos" => Self::find_pos(datum, args),
      "getPos" => Self::find_pos(datum, args),
      _ => Err(ScriptError::new(format!("No handler {handler_name} for list datum")))
    }
  }

  fn count(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let list_vec = player.get_datum(datum).to_list()?;
      Ok(player.alloc_datum(Datum::Int(list_vec.len() as i32)))
    })
  }

  fn get_last(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let list_vec = player.get_datum(datum).to_list()?;
      let last = list_vec.last().map(|x| x.clone()).unwrap_or(VOID_DATUM_REF.clone());
      Ok(last)
    })
  }

  pub fn get_one(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let find = player.get_datum(&args[0]);
      let list_vec = player.get_datum(datum).to_list()?;
      let position = list_vec.iter().position(|x| datum_equals(player.get_datum(&x), find, &player.datums).unwrap()).map(|x| x as i32);

      Ok(player.alloc_datum(Datum::Int(position.unwrap_or(-1) + 1)))
    })
  }

  pub fn find_pos(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    // TODO: why is this exactly the same as get_one?
    reserve_player_mut(|player| {
      let find = player.get_datum(&args[0]);
      let list_vec = player.get_datum(datum).to_list()?;
      let position = list_vec.iter().position(|x| datum_equals(player.get_datum(&x), find, &player.datums).unwrap()).map(|x| x as i32);
      Ok(player.alloc_datum(Datum::Int(position.unwrap_or(-1) + 1)))
    })
  }

  pub fn add(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let item = &args[0];
    reserve_player_mut(|player| {
      let (_, list_vec, is_sorted) = player.get_datum(datum).to_list_tuple()?;
      let index_to_add = if is_sorted {
        ListDatumUtils::find_index_to_add(&list_vec, &item, &player.datums)?
      } else {
        list_vec.len() as i32
      };
      
      let (_, list_vec, _) = player.get_datum_mut(datum).to_list_mut()?;
      if is_sorted {
        list_vec.insert(index_to_add as usize, item.clone());
      } else {
        list_vec.push(item.clone());
      }
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn delete_one(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let index = reserve_player_ref(|player| {
      let item = player.get_datum(&args[0]);
      let list_vec = player.get_datum(datum).to_list()?;
      let index = list_vec.iter().position(|x| datum_equals(player.get_datum(&x), item, &player.datums).unwrap());
      Ok(index)
    })?;

    reserve_player_mut(|player| {
      let (_, list_vec, _) = player.get_datum_mut(datum).to_list_mut()?;
      if let Some(index) = index {
        list_vec.remove(index);
      }
      Ok(player.alloc_datum(datum_bool(index.is_some())))
    })
  }

  pub fn delete_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let position = player.get_datum(&args[0]).int_value()?;
      let (_, list_vec, _) = player.get_datum_mut(datum).to_list_mut()?;
      if position <= list_vec.len() as i32 {
        let index = (position - 1) as usize;
        list_vec.remove(index);
        Ok(VOID_DATUM_REF.clone())
      } else {
        Err(ScriptError::new("Index out of bounds".to_string()))
      }
    })
  }

  pub fn add_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let position = player.get_datum(&args[0]).int_value()? - 1;
      let item_ref = &args[1];

      let (_, list_vec, _) = player.get_datum_mut(datum).to_list_mut()?;
      list_vec.insert(position as usize, item_ref.clone());
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn append(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let item = &args[0];
    reserve_player_mut(|player| {
      let (_, list_vec, _) = player.get_datum_mut(datum).to_list_mut()?;
      list_vec.push(item.clone());
      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn sort(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let sorted_list = reserve_player_ref(|player| {
      let list_vec = player.get_datum(datum).to_list()?;
      let mut sorted_list = list_vec.clone();
      sorted_list.sort_by(|a, b| {
        let left = player.get_datum(a);
        let right = player.get_datum(b);

        if datum_equals(left, right, &player.datums).unwrap() {
          return std::cmp::Ordering::Equal
        } else if datum_less_than(left, right).unwrap() {
          std::cmp::Ordering::Less
        } else {
          std::cmp::Ordering::Greater
        }
      });

      Ok(sorted_list)
    })?;

    reserve_player_mut(|player| {
      let (_, list_vec, is_sorted) = player.get_datum_mut(datum).to_list_mut()?;
      list_vec.clear();
      list_vec.extend(sorted_list);
      *is_sorted = true;

      Ok(VOID_DATUM_REF.clone())
    })
  }

  pub fn duplicate(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    Ok(player_duplicate_datum(datum))
  }
}
