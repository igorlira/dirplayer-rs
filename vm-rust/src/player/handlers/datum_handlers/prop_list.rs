use crate::{director::lingo::datum::{datum_bool, Datum, PropListPair}, player::{compare::datum_equals, datum_formatting::{format_concrete_datum, format_datum}, get_datum, handlers::types::TypeUtils, player_duplicate_datum, reserve_player_mut, DatumRef, DatumRefMap, DirPlayer, ScriptError, VOID_DATUM_REF}};

pub struct PropListDatumHandlers {}

pub struct PropListUtils {}

impl PropListUtils {
  fn get_key_index(prop_list: &Vec<PropListPair>, key: &Datum, datums: &DatumRefMap) -> Result<i32, ScriptError> {
    let mut pos = -1;
    for (i, (k, _)) in prop_list.iter().enumerate() {
      let k_datum = get_datum(*k, datums);
      if ((key.is_string() && k_datum.is_symbol()) || (key.is_symbol() && k_datum.is_string())) && key.string_value(datums)? == k_datum.string_value(datums)? {
        pos = i as i32;
        break;
      } else if datum_equals(k_datum, key, datums)? {
        pos = i as i32;
        break;
      }
    }
    Ok(pos)
  }

  pub fn get_prop_or_built_in(
    player: &mut DirPlayer,
    prop_list: &Vec<PropListPair>, 
    key: &String,
  ) -> Result<DatumRef, ScriptError> {
    let key_index = Self::get_key_index(prop_list, &Datum::String(key.to_owned()), &player.datums)?;
    if key_index >= 0 {
      return Ok(prop_list[key_index as usize].1)
    }
    let key_index = Self::get_key_index(prop_list, &Datum::Symbol(key.to_owned()), &player.datums)?;
    if key_index >= 0 {
      return Ok(prop_list[key_index as usize].1)
    }
    return Ok(player.alloc_datum(Self::get_built_in_prop(prop_list, key)?))
  }

  pub fn get_built_in_prop(
    prop_list: &Vec<PropListPair>,
    prop: &String,
  ) -> Result<Datum, ScriptError> {
    match prop.as_str() {
      "count" => Ok(Datum::Int(prop_list.len() as i32)),
      "ilk" => Ok(Datum::Symbol("propList".to_owned())),
      _ => {
        return Err(ScriptError::new(format!("Invalid prop list built-in property {}", prop)))
      },
    }
  }

  pub fn get_prop(
    prop_list: &Vec<PropListPair>, 
    key_ref: DatumRef, 
    datums: &DatumRefMap,
    is_required: bool,
    formatted_key: String,
  ) -> Result<DatumRef, ScriptError> {
    let key = datums.get(&key_ref).unwrap();
    if let Datum::Int(position) = key {
      let index = *position - 1;
      if index >= 0 && index < prop_list.len() as i32 {
        return Ok(prop_list[index as usize].1);
      } else {
        return Err(ScriptError::new(format!("Index out of range: {}", index)));
      }
    }
    let key_index = Self::get_key_index(prop_list, key, &datums)?;
    if is_required && key_index < 0 {
      return Err(ScriptError::new(format!("Prop not found: {}", formatted_key)));
    }
    if key_index >= 0 {
      Ok(prop_list[key_index as usize].1)
    } else {
      Ok(VOID_DATUM_REF)
    }
  }

  pub fn set_prop(
    prop_list_ref: DatumRef,
    key_ref: DatumRef, 
    value_ref: DatumRef, 
    player: &mut DirPlayer,
    is_required: bool,
    formatted_key: String,
  ) -> Result<(), ScriptError> {
    let key = player.datums.get(&key_ref).unwrap();
    let prop_list = player.get_datum(prop_list_ref).to_map()?;
    let key_index = Self::get_key_index(&prop_list, key, &player.datums)?;
    if is_required && key_index < 0 {
      return Err(ScriptError::new(format!("Prop not found: {}", formatted_key)));
    }
    let prop_list = player.get_datum_mut(prop_list_ref).to_map_mut()?;
    if key_index >= 0 {
      prop_list[key_index as usize].1 = value_ref;
    } else {
      prop_list.push((key_ref, value_ref));
    }
    Ok(())
  }

  pub fn get_at(
    prop_list: &Vec<PropListPair>, 
    key_ref: DatumRef, 
    datums: &DatumRefMap,
  ) -> Result<DatumRef, ScriptError> {
    let key = get_datum(key_ref, datums);
    match key {
      // TODO do same for float
      Datum::Int(index) => {
        let index = (*index as usize) - 1;
        if index < prop_list.len() {
          Ok(prop_list[index].1)
        } else {
          Err(ScriptError::new(format!("Index out of range: {}", index)))
        }
      }
      _ => {
        Self::get_by_key(prop_list, key_ref, &datums)
      }
    }
  }

  pub fn get_by_key(
    prop_list: &Vec<PropListPair>, 
    key_ref: DatumRef, 
    datums: &DatumRefMap,
  ) -> Result<DatumRef, ScriptError> {
    let key = get_datum(key_ref, datums);
    Self::get_by_concrete_key(prop_list, key, datums)
  }

  pub fn get_by_concrete_key(
    prop_list: &Vec<PropListPair>, 
    key: &Datum, 
    datums: &DatumRefMap,
  ) -> Result<DatumRef, ScriptError> {
    let key_index = Self::get_key_index(prop_list, key, &datums)?;
    if key_index < 0 {
      return Ok(VOID_DATUM_REF);
    }
    Ok(prop_list[key_index as usize].1)
  }

  pub fn set_at(
    player: &mut DirPlayer,
    prop_list_ref: DatumRef,
    key_ref: DatumRef, 
    value_ref: DatumRef, 
    formatted_key: String,
  ) -> Result<(), ScriptError> {
    let key = &player.get_datum(key_ref);
    match key {
      // TODO do same for float
      Datum::Int(index) => {
        let index = (*index as usize) - 1;
        let prop_list = player.get_datum_mut(prop_list_ref).to_map_mut()?;
        if index < prop_list.len() {
          prop_list[index].1 = value_ref;
        } else {
          return Err(ScriptError::new(format!("Index out of range: {}", index)));
        }
      }
      _ => {
        Self::set_prop(prop_list_ref, key_ref, value_ref, player, false, formatted_key)?
      }
    }
    Ok(())
  }
}

impl PropListDatumHandlers {
  pub fn call(datum: DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      "getAt" => Self::get_at(datum, args),
      "setAt" => Self::set_at(datum, args),
      "sort" => Self::sort(datum, args),
      "getPropAt" => Self::get_prop_at(datum, args),
      "addProp" => Self::add_prop(datum, args),
      "setaProp" => Self::set_opt_prop(datum, args),
      "setProp" => Self::set_required_prop(datum, args),
      "getProp" => Self::get_prop(datum, args),
      "getaProp" => Self::get_a_prop(datum, args),
      "deleteProp" => Self::delete_prop(datum, args),
      "deleteAt" => Self::delete_at(datum, args),
      "getOne" => Self::get_one(datum, args),
      "findPos" => Self::find_pos(datum, args),
      "getPos" => Self::get_pos(datum, args),
      "duplicate" => Self::duplicate(datum, args),
      "getLast" => Self::get_last(datum, args),
      "count" => Self::count(datum, args),
      _ => Err(ScriptError::new(format!("No handler {handler_name} for prop list datum")))
    }
  }

  fn count(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let prop_list = player.get_datum(datum).to_map()?;
      let count = if args.is_empty() {
        prop_list.len()
      } else if args.len() == 1 {
        let prop_name = args[0];
        let prop_value = PropListUtils::get_by_key(prop_list, prop_name, &player.datums)?;
        let prop_value = player.get_datum(prop_value);
        match prop_value {
          Datum::List(_, list, _) => list.len(),
          Datum::PropList(list) => list.len(),
          _ => return Err(ScriptError::new("Cannot get count of non-list".to_string())),
        }
      } else {
        return Err(ScriptError::new("Invalid number of arguments for count".to_string()));
      };
      Ok(player.alloc_datum(Datum::Int(count as i32)))
    })
  }

  pub fn get_one(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let find = player.get_datum(args[0]);
      let prop_list = player.get_datum(datum);
      let prop_list = match prop_list {
        Datum::PropList(list) => list,
        _ => return Err(ScriptError::new("Cannot set prop list at non-prop list".to_string())),
      };
      let position = prop_list.iter()
        .position(|&(_, v)| 
          datum_equals(player.get_datum(v), find, &player.datums).unwrap()
        )
        .map(|x| x as i32);

      Ok(player.alloc_datum(Datum::Int(position.unwrap_or(-1) + 1)))
    })
  }

  pub fn find_pos(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let find = player.get_datum(args[0]);
      let prop_list = player.get_datum(datum);
      let prop_list = match prop_list {
        Datum::PropList(list) => list,
        _ => return Err(ScriptError::new("Cannot set prop list at non-prop list".to_string())),
      };
      let position = prop_list.iter()
        .position(|&(k, _)| 
          datum_equals(player.get_datum(k), find, &player.datums).unwrap()
        )
        .map(|x| x as i32);
      if let Some(position) = position {
        return Ok(player.alloc_datum(Datum::Int(position as i32 + 1)));
      } else {
        return Ok(VOID_DATUM_REF);
      }
    })
  }

  // Finds position of value
  pub fn get_pos(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let find = player.get_datum(args[0]);
      let prop_list = player.get_datum(datum).to_map()?;
      let position = prop_list.iter()
        .position(|&(_, v)| 
          datum_equals(player.get_datum(v), find, &player.datums).unwrap()
        )
        .map(|x| x as i32)
        .unwrap_or(-1);
      return Ok(player.alloc_datum(Datum::Int(position + 1)));
    })
  }

  pub fn get_last(datum: DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let prop_list = player.get_datum(datum);
      let prop_list = match prop_list {
        Datum::PropList(list) => list,
        _ => return Err(ScriptError::new("Cannot set prop list at non-prop list".to_string())),
      };
      let last = prop_list.last().map(|(_, v)| v).unwrap();
      Ok(*last)
    })
  }

  fn duplicate(datum: DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    Ok(player_duplicate_datum(datum))
  }

  pub fn get_a_prop(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let key = player.get_datum(args[0]);
      let prop_list = player.get_datum(datum);
      match prop_list {
        Datum::PropList(prop_list) => {
          let key_index = PropListUtils::get_key_index(prop_list, key, &player.datums)?;
          if key_index >= 0 {
            return Ok(prop_list[key_index as usize].1);
          } else {
            return Ok(VOID_DATUM_REF);
          }
        },
        _ => return Err(ScriptError::new("Cannot get a prop of non-prop list".to_string())),
      };
    })
  }

  fn get_prop(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let base_prop_ref = reserve_player_mut(|player| {
      let key = player.get_datum(args[0]);
      let prop_list = player.get_datum(datum).to_map()?;
      let key_index = PropListUtils::get_key_index(prop_list, key, &player.datums)?;
      if key_index >= 0 {
        Ok(prop_list[key_index as usize].1)
      } else {
        let formatted_key = format_concrete_datum(key, player);
        return Err(ScriptError::new(format!("Unknown prop {} in prop list", formatted_key)));
      }
    })?;

    if args.len() == 1 {
      return Ok(base_prop_ref);
    } else if args.len() == 2 {
      return reserve_player_mut(|player| {
        TypeUtils::get_sub_prop(base_prop_ref, args[1], player)
      });
    } else {
      return Err(ScriptError::new("Invalid number of arguments for getProp".to_string()));
    }
  }

  fn set_opt_prop(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let formatted_key = format_datum(args[0], &player);
      let prop_list = player.get_datum(datum);
      match prop_list {
        Datum::PropList(_) => {},
        _ => return Err(ScriptError::new("Cannot set prop list at non-prop list".to_string())),
      };
      let prop_name_ref = args[0];
      let value_ref = args[1];
      
      PropListUtils::set_prop(datum, prop_name_ref, value_ref, player, false, formatted_key)?;
      Ok(VOID_DATUM_REF)
    })
  }

  fn add_prop(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let prop_list = player.get_datum_mut(datum).to_map_mut()?;
      
      let prop_name_ref = args[0];
      let value_ref = args[1];
      prop_list.push((prop_name_ref, value_ref));
      
      Ok(VOID_DATUM_REF)
    })
  }

  fn set_required_prop(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let formatted_key = format_datum(args[0], &player);
      let prop_list = player.get_datum(datum);
      match prop_list {
        Datum::PropList(_) => {},
        _ => return Err(ScriptError::new("Cannot set prop list at non-prop list".to_string())),
      };
      let prop_name_ref = args[0];
      let value_ref = args[1];
      
      PropListUtils::set_prop(datum, prop_name_ref, value_ref, player, true, formatted_key)?;
      Ok(VOID_DATUM_REF)
    })
  }

  pub fn set_at(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let formatted_key = format_datum(args[0], &player);
      let prop_list = player.get_datum(datum);
      match prop_list {
        Datum::PropList(_) => {},
        _ => return Err(ScriptError::new("Cannot set prop list at non-prop list".to_string())),
      };
      let prop_name_ref = args[0];
      let value_ref = args[1];
      
      PropListUtils::set_at(player, datum, prop_name_ref, value_ref, formatted_key)?;
      Ok(VOID_DATUM_REF)
    })
  }

  pub fn get_at(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let prop_list = player.get_datum(datum);
      let prop_list = match prop_list {
        Datum::PropList(prop_list) => prop_list,
        _ => return Err(ScriptError::new("Cannot get prop list at non-prop list".to_string())),
      };
      let prop_name_ref = args[0];
      PropListUtils::get_at(&prop_list, prop_name_ref, &player.datums)
    })
  }

  pub fn delete_at(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let position = player.get_datum(args[0]).int_value(&player.datums)?;
      let prop_list = player.get_datum_mut(datum);
      match prop_list {
        Datum::PropList(prop_list) => {
          prop_list.remove((position - 1) as usize);
          Ok(())
        },
        _ => Err(ScriptError::new("Cannot get prop list at non-prop list".to_string())),
      }?;
      Ok(VOID_DATUM_REF)
    })
  }

  pub fn get_prop_at(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let prop_list = player.get_datum(datum);
      let prop_list = match prop_list {
        Datum::PropList(prop_list) => prop_list,
        _ => return Err(ScriptError::new("Cannot get prop list at non-prop list".to_string())),
      };
      let position = player.get_datum(args[0]).int_value(&player.datums)?;
      Ok(prop_list.get((position - 1) as usize).unwrap().0)
    })
  }

  pub fn sort(datum: DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let prop_list = player.get_datum_mut(datum).to_map()?;
      if prop_list.len() > 0 {
        Err(ScriptError::new("Cannot sort non-empty prop list".to_string()))
      } else {
        Ok(VOID_DATUM_REF)
      }
    })
  }

  pub fn delete_prop(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let prop_name = player.get_datum(args[0]);
      if prop_name.is_string() || prop_name.is_symbol() {
        // let prop_name = prop_name.string_value(&player.datums)?;
        let prop_list = player.get_datum(datum).to_map()?;
        let index = PropListUtils::get_key_index(&prop_list, prop_name, &player.datums)?;
        if index >= 0  {
          let prop_list = player.get_datum_mut(datum).to_map_mut()?;
          prop_list.remove(index as usize);
          Ok(player.alloc_datum(datum_bool(true)))
        } else {
          Ok(player.alloc_datum(datum_bool(false)))
        }
      } else if prop_name.is_int() {
        let position = player.get_datum(args[0]).int_value(&player.datums)?;
        let prop_list = player.get_datum_mut(datum).to_map_mut()?;
        if position >= 1 && position <= prop_list.len() as i32 {
          prop_list.remove((position - 1) as usize);
          Ok(player.alloc_datum(datum_bool(true)))
        } else {
          Ok(player.alloc_datum(datum_bool(false)))
        }
      } else {
        Err(ScriptError::new(format!("Prop name must be a string, int or symbol (is {})", prop_name.type_str())))
      }
    })
  }
}
