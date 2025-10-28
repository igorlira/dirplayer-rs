use crate::PLAYER_OPT;
use crate::{
    director::lingo::datum::{datum_bool, Datum, PropListPair},
    player::{
        allocator::{DatumAllocator, DatumAllocatorTrait},
        compare::{datum_equals, datum_less_than},
        datum_formatting::{format_concrete_datum, format_datum},
        handlers::types::TypeUtils,
        player_duplicate_datum, reserve_player_mut, reserve_player_ref, DatumRef, DirPlayer,
        ScriptError,
    },
};

pub struct PropListDatumHandlers {}

pub struct PropListUtils {}

impl PropListUtils {
    fn find_index_to_add(
        prop_list: &Vec<PropListPair>,
        item: (&DatumRef, &DatumRef),
        allocator: &DatumAllocator,
    ) -> Result<i32, ScriptError> {
        let mut low = 0;
        let mut high = prop_list.len() as i32;
        let key = allocator.get_datum(item.0);

        while low < high {
            let mid = (low + high) / 2;
            let left_key = allocator.get_datum(&prop_list.get(mid as usize).unwrap().0);
            if datum_less_than(left_key, key)? {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        Ok(low)
    }

    fn get_key_index(
        prop_list: &Vec<PropListPair>,
        key: &Datum,
        allocator: &DatumAllocator,
    ) -> Result<i32, ScriptError> {
        for (i, (k, _)) in prop_list.iter().enumerate() {
            let k_datum = allocator.get_datum(k);
            // Lookup: exact match only
            if Self::datum_equals_for_lookup(k_datum, key, allocator)? {
                return Ok(i as i32);
            }
        }
        Ok(-1)
    }

    fn datum_equals_for_lookup(
        left: &Datum,
        right: &Datum,
        allocator: &DatumAllocator,
    ) -> Result<bool, ScriptError> {
        let result = match (left, right) {
            (Datum::String(l), Datum::String(r)) => l == r, // exact (case-sensitive for keys)
            (Datum::String(l), Datum::Symbol(r)) => l == r,
            (Datum::Symbol(l), Datum::String(r)) => l == r,

            // Handle symbol-to-int comparison (e.g., #2 should match key 2)
            (Datum::Symbol(s), Datum::Int(i)) | (Datum::Int(i), Datum::Symbol(s)) => {
                s.parse::<i32>().ok() == Some(*i)
            }

            _ => datum_equals(left, right, allocator)?,
        };

        Ok(result)
    }

    pub fn get_prop_or_built_in(
        player: &mut DirPlayer,
        prop_list: &Vec<PropListPair>,
        key: &String,
    ) -> Result<DatumRef, ScriptError> {
        let key_index =
            Self::get_key_index(prop_list, &Datum::String(key.to_owned()), &player.allocator)?;
        if key_index >= 0 {
            return Ok(prop_list[key_index as usize].1.clone());
        }
        let key_index =
            Self::get_key_index(prop_list, &Datum::Symbol(key.to_owned()), &player.allocator)?;
        if key_index >= 0 {
            return Ok(prop_list[key_index as usize].1.clone());
        }
        return Ok(player.alloc_datum(Self::get_built_in_prop(prop_list, key)?));
    }

    pub fn get_built_in_prop(
        prop_list: &Vec<PropListPair>,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        match prop.as_str() {
            "count" => Ok(Datum::Int(prop_list.len() as i32)),
            "ilk" => Ok(Datum::Symbol("propList".to_owned())),
            _ => {
                return Err(ScriptError::new(format!(
                    "Invalid prop list built-in property {}",
                    prop
                )))
            }
        }
    }

    pub fn get_prop(
        prop_list: &Vec<PropListPair>,
        key_ref: &DatumRef,
        allocator: &DatumAllocator,
        is_required: bool,
        formatted_key: String,
    ) -> Result<DatumRef, ScriptError> {
        let key = allocator.get_datum(&key_ref);
        // First try key-based lookup (works for all types including Int)
        let key_index = Self::get_key_index(prop_list, key, &allocator)?;
        if key_index >= 0 {
            return Ok(prop_list[key_index as usize].1.clone());
        }

        // If not found and key is an Int, try as positional index
        if let Datum::Int(position) = key {
            let index = *position - 1;
            if index >= 0 && index < prop_list.len() as i32 {
                return Ok(prop_list[index as usize].1.clone());
            } else {
                return Err(ScriptError::new(format!("Index out of range: {}", index)));
            }
        }
        if is_required {
            return Err(ScriptError::new(format!(
                "Prop not found: {}",
                formatted_key
            )));
        }
        Ok(DatumRef::Void)
    }

    pub fn set_prop(
        prop_list_ref: &DatumRef,
        key_ref: &DatumRef,
        value_ref: &DatumRef,
        player: &mut DirPlayer,
        is_required: bool,
        formatted_key: String,
    ) -> Result<(), ScriptError> {
        let key = player.get_datum(key_ref);
        let (prop_list, is_sorted) = player.get_datum(prop_list_ref).to_map_tuple()?;
        let key_index = Self::get_key_index(&prop_list, key, &player.allocator)?;
        if is_required && key_index < 0 {
            return Err(ScriptError::new(format!(
                "Prop not found: {}",
                formatted_key
            )));
        }
        let index_to_add =
            PropListUtils::find_index_to_add(&prop_list, (key_ref, value_ref), &player.allocator)?;
        let (prop_list, ..) = player.get_datum_mut(prop_list_ref).to_map_tuple_mut()?;
        if key_index >= 0 {
            prop_list[key_index as usize].1 = value_ref.clone();
        } else if is_sorted {
            prop_list.insert(index_to_add as usize, (key_ref.clone(), value_ref.clone()));
        } else {
            prop_list.push((key_ref.clone(), value_ref.clone()));
        }
        Ok(())
    }

    pub fn get_at(
        prop_list: &Vec<PropListPair>,
        key_ref: &DatumRef,
        allocator: &DatumAllocator,
    ) -> Result<DatumRef, ScriptError> {
        let key = allocator.get_datum(key_ref);
        match key {
            // TODO do same for float
            Datum::Int(index) => {
                let index = (*index as usize) - 1;
                if index < prop_list.len() {
                    Ok(prop_list[index].1.clone())
                } else {
                    Err(ScriptError::new(format!("Index out of range: {}", index)))
                }
            }
            _ => Self::get_by_key(prop_list, key_ref, &allocator),
        }
    }

    pub fn get_by_key(
        prop_list: &Vec<PropListPair>,
        key_ref: &DatumRef,
        allocator: &DatumAllocator,
    ) -> Result<DatumRef, ScriptError> {
        let key = allocator.get_datum(key_ref);
        Self::get_by_concrete_key(prop_list, key, allocator)
    }

    pub fn get_by_concrete_key(
        prop_list: &Vec<PropListPair>,
        key: &Datum,
        allocator: &DatumAllocator,
    ) -> Result<DatumRef, ScriptError> {
        let key_index = Self::get_key_index(prop_list, key, &allocator)?;
        if key_index < 0 {
            return Ok(DatumRef::Void);
        }
        Ok(prop_list[key_index as usize].1.clone())
    }

    pub fn set_at(
        player: &mut DirPlayer,
        prop_list_ref: &DatumRef,
        key_ref: &DatumRef,
        value_ref: &DatumRef,
        formatted_key: String,
    ) -> Result<(), ScriptError> {
        let key = &player.get_datum(key_ref);
        match key {
            // TODO do same for float
            Datum::Int(index) => {
                let index = (*index as usize) - 1;
                let prop_list = player.get_datum_mut(prop_list_ref).to_map_mut()?;
                if index < prop_list.len() {
                    prop_list[index].1 = value_ref.clone();
                } else {
                    return Err(ScriptError::new(format!("Index out of range: {}", index)));
                }
            }
            _ => Self::set_prop(
                prop_list_ref,
                key_ref,
                value_ref,
                player,
                false,
                formatted_key,
            )?,
        }
        Ok(())
    }
}

impl PropListDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "getAt" => Self::get_at(datum, args),
            "setAt" => Self::set_at(datum, args),
            "sort" => Self::sort(datum, args),
            "getPropAt" => Self::get_prop_at(datum, args),
            "addProp" => Self::add_prop(datum, args),
            "setaProp" => Self::set_opt_prop(datum, args),
            "setProp" => {
                if args.len() == 3 {
                    reserve_player_mut(|player| {
                        let prop_key_ref = &args[0];
                        let index_ref = &args[1];
                        let value_ref = &args[2];

                        let prop_list = player.get_datum(datum).to_map()?;
                        let list_ref =
                            PropListUtils::get_by_key(prop_list, prop_key_ref, &player.allocator)?;

                        let index = player.get_datum(index_ref).int_value()? as usize;
                        let adjusted_index = if index == 0 { 0 } else { index - 1 };

                        let list_datum = player.get_datum(&list_ref);
                        if let Datum::List(_, list, _) = list_datum {
                            if adjusted_index < list.len() {
                                let (_, list_vec, _) =
                                    player.get_datum_mut(&list_ref).to_list_mut()?;
                                list_vec[adjusted_index] = value_ref.clone();
                                Ok(DatumRef::Void)
                            } else {
                                Err(ScriptError::new(format!("Index out of bounds: {}", index)))
                            }
                        } else {
                            Err(ScriptError::new(format!(
                                "Property is not a list, it's: {}",
                                list_datum.type_str()
                            )))
                        }
                    })
                } else if args.len() == 2 {
                    Self::set_required_prop(datum, args)
                } else {
                    Err(ScriptError::new(format!(
                        "Invalid number of arguments for setProp: {}",
                        args.len()
                    )))
                }
            }
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
            "getPropRef" => Self::get_prop_ref(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for prop list datum"
            ))),
        }
    }

    fn count(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_list = player.get_datum(datum).to_map()?;
            let count = if args.is_empty() {
                prop_list.len()
            } else if args.len() == 1 {
                let prop_name = &args[0];
                let prop_value =
                    PropListUtils::get_by_key(prop_list, prop_name, &player.allocator)?;
                let prop_value = player.get_datum(&prop_value);
                match prop_value {
                    Datum::List(_, list, _) => list.len(),
                    Datum::PropList(list, ..) => list.len(),
                    _ => return Err(ScriptError::new("Cannot get count of non-list".to_string())),
                }
            } else {
                return Err(ScriptError::new(
                    "Invalid number of arguments for count".to_string(),
                ));
            };
            Ok(player.alloc_datum(Datum::Int(count as i32)))
        })
    }

    pub fn get_one(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let find = player.get_datum(&args[0]);
            let prop_list = player.get_datum(datum);
            let prop_list = match prop_list {
                Datum::PropList(list, ..) => list,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set prop list at non-prop list".to_string(),
                    ))
                }
            };
            let position = prop_list
                .iter()
                .position(|(_, v)| {
                    datum_equals(player.get_datum(&v), find, &player.allocator).unwrap()
                })
                .map(|x| x as i32);

            Ok(player.alloc_datum(Datum::Int(position.unwrap_or(-1) + 1)))
        })
    }

    pub fn find_pos(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let find = player.get_datum(&args[0]);
            let prop_list = player.get_datum(datum);
            let prop_list = match prop_list {
                Datum::PropList(list, ..) => list,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set prop list at non-prop list".to_string(),
                    ))
                }
            };
            let position = prop_list
                .iter()
                .position(|(k, _)| {
                    datum_equals(player.get_datum(&k), find, &player.allocator).unwrap()
                })
                .map(|x| x as i32);
            if let Some(position) = position {
                return Ok(player.alloc_datum(Datum::Int(position as i32 + 1)));
            } else {
                return Ok(DatumRef::Void);
            }
        })
    }

    // Finds position of value
    pub fn get_pos(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let find = player.get_datum(&args[0]);
            let prop_list = player.get_datum(datum).to_map()?;
            let position = prop_list
                .iter()
                .position(|(_, v)| {
                    datum_equals(player.get_datum(&v), find, &player.allocator).unwrap()
                })
                .map(|x| x as i32)
                .unwrap_or(-1);
            return Ok(player.alloc_datum(Datum::Int(position + 1)));
        })
    }

    pub fn get_last(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_list = player.get_datum(datum);
            let prop_list = match prop_list {
                Datum::PropList(list, ..) => list,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set prop list at non-prop list".to_string(),
                    ))
                }
            };
            let last = prop_list.last().map(|(_, v)| v).unwrap();
            Ok(last.clone())
        })
    }

    pub fn duplicate(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Ok(player_duplicate_datum(datum))
    }

    pub fn get_a_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let key = player.get_datum(&args[0]);
            let prop_list = player.get_datum(datum);

            match prop_list {
                Datum::PropList(ref entries, ..) => {
                    let key_index = PropListUtils::get_key_index(entries, key, &player.allocator)?;
                    if key_index >= 0 {
                        Ok(entries[key_index as usize].1.clone())
                    } else {
                        Ok(DatumRef::Void)
                    }
                }
                _ => Err(ScriptError::new(
                    "Cannot get a prop of non-prop list".to_string(),
                )),
            }
        })
    }

    pub fn get_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let base_prop_ref = reserve_player_mut(|player| {
            let key = player.get_datum(&args[0]);
            let prop_list = player.get_datum(datum).to_map()?;
            let key_index = PropListUtils::get_key_index(prop_list, key, &player.allocator)?;
            if key_index >= 0 {
                Ok(prop_list[key_index as usize].1.clone())
            } else {
                let formatted_key = format_concrete_datum(key, player);
                return Err(ScriptError::new(format!(
                    "Unknown prop {} in prop list",
                    formatted_key
                )));
            }
        })?;

        if args.len() == 1 {
            return Ok(base_prop_ref);
        } else if args.len() == 2 {
            return reserve_player_mut(|player| {
                TypeUtils::get_sub_prop(&base_prop_ref, &args[1], player)
            });
        } else {
            return Err(ScriptError::new(
                "Invalid number of arguments for getProp".to_string(),
            ));
        }
    }

    pub fn set_opt_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let formatted_key = format_datum(&args[0], &player);
            let prop_list = player.get_datum(datum);
            match prop_list {
                Datum::PropList(..) => {}
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set prop list at non-prop list".to_string(),
                    ))
                }
            };
            let prop_name_ref = &args[0];
            let value_ref = &args[1];

            PropListUtils::set_prop(
                datum,
                &prop_name_ref,
                &value_ref,
                player,
                false,
                formatted_key,
            )?;
            Ok(DatumRef::Void)
        })
    }

    pub fn add_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name_ref = &args[0];
            let value_ref = &args[1];

            let (prop_list, is_sorted) = player.get_datum(datum).to_map_tuple()?;
            let index_to_add = if is_sorted {
                PropListUtils::find_index_to_add(
                    &prop_list,
                    (&prop_name_ref, &value_ref),
                    &player.allocator,
                )?
            } else {
                prop_list.len() as i32
            };

            let (prop_list, ..) = player.get_datum_mut(datum).to_map_tuple_mut()?;
            if is_sorted {
                prop_list.insert(
                    index_to_add as usize,
                    (prop_name_ref.clone(), value_ref.clone()),
                );
            } else {
                prop_list.push((prop_name_ref.clone(), value_ref.clone()));
            }

            Ok(DatumRef::Void)
        })
    }

    fn set_required_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let formatted_key = format_datum(&args[0], &player);
            let prop_list = player.get_datum(datum);
            match prop_list {
                Datum::PropList(..) => {}
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set prop list at non-prop list".to_string(),
                    ))
                }
            };
            let prop_name_ref = &args[0];
            let value_ref = &args[1];

            PropListUtils::set_prop(
                datum,
                &prop_name_ref,
                &value_ref,
                player,
                true,
                formatted_key,
            )?;
            Ok(DatumRef::Void)
        })
    }

    pub fn set_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let formatted_key = format_datum(&args[0], &player);
            let prop_list = player.get_datum(datum);
            match prop_list {
                Datum::PropList(..) => {}
                _ => {
                    return Err(ScriptError::new(
                        "Cannot set prop list at non-prop list".to_string(),
                    ))
                }
            };
            let prop_name_ref = &args[0];
            let value_ref = &args[1];

            PropListUtils::set_at(player, datum, &prop_name_ref, &value_ref, formatted_key)?;
            Ok(DatumRef::Void)
        })
    }

    pub fn get_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_list = player.get_datum(datum);
            let prop_list = match prop_list {
                Datum::PropList(prop_list, ..) => prop_list,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot get prop list at non-prop list".to_string(),
                    ))
                }
            };
            let prop_name_ref = &args[0];
            PropListUtils::get_at(&prop_list, &prop_name_ref, &player.allocator)
        })
    }

    pub fn delete_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let position = player.get_datum(&args[0]).int_value()?;
            let prop_list = player.get_datum_mut(datum);
            match prop_list {
                Datum::PropList(prop_list, ..) => {
                    prop_list.remove((position - 1) as usize);
                    Ok(())
                }
                _ => Err(ScriptError::new(
                    "Cannot get prop list at non-prop list".to_string(),
                )),
            }?;
            Ok(DatumRef::Void)
        })
    }

    pub fn get_prop_at(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_list = player.get_datum(datum);
            let prop_list = match prop_list {
                Datum::PropList(prop_list, ..) => prop_list,
                _ => {
                    return Err(ScriptError::new(
                        "Cannot get prop list at non-prop list".to_string(),
                    ))
                }
            };
            let position = player.get_datum(&args[0]).int_value()?;
            Ok(prop_list.get((position - 1) as usize).unwrap().0.clone())
        })
    }

    pub fn sort(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let sorted_prop_list = reserve_player_ref(|player| {
            let mut sorted_prop_list = player.get_datum(datum).to_map()?.clone();
            sorted_prop_list.sort_by(|a, b| {
                let (left_key_ref, _) = a;
                let (right_key_ref, _) = b;

                let left = player.get_datum(left_key_ref);
                let right = player.get_datum(right_key_ref);

                if datum_equals(left, right, &player.allocator).unwrap() {
                    return std::cmp::Ordering::Equal;
                } else if datum_less_than(left, right).unwrap() {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            });
            Ok(sorted_prop_list)
        })?;

        reserve_player_mut(|player| {
            let (list_vec, is_sorted) = player.get_datum_mut(datum).to_map_tuple_mut()?;
            list_vec.clear();
            list_vec.extend(sorted_prop_list);
            *is_sorted = true;

            Ok(DatumRef::Void)
        })
    }

    pub fn delete_prop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let prop_name = player.get_datum(&args[0]);
            if prop_name.is_string() || prop_name.is_symbol() {
                // let prop_name = prop_name.string_value()?;
                let prop_list = player.get_datum(datum).to_map()?;
                let index = PropListUtils::get_key_index(&prop_list, prop_name, &player.allocator)?;
                if index >= 0 {
                    let prop_list = player.get_datum_mut(datum).to_map_mut()?;
                    prop_list.remove(index as usize);
                    Ok(player.alloc_datum(datum_bool(true)))
                } else {
                    Ok(player.alloc_datum(datum_bool(false)))
                }
            } else if prop_name.is_int() {
                let position = player.get_datum(&args[0]).int_value()?;
                let prop_list = player.get_datum_mut(datum).to_map_mut()?;
                if position >= 1 && position <= prop_list.len() as i32 {
                    prop_list.remove((position - 1) as usize);
                    Ok(player.alloc_datum(datum_bool(true)))
                } else {
                    Ok(player.alloc_datum(datum_bool(false)))
                }
            } else if prop_name.is_void() {
                Ok(player.alloc_datum(datum_bool(false)))
            } else {
                Err(ScriptError::new(format!(
                    "Prop name must be a string, int or symbol (is {})",
                    prop_name.type_str()
                )))
            }
        })
    }

    pub fn get_prop_ref(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new(
                "getPropRef requires at least one argument".to_string(),
            ));
        }

        let key = args[0].clone();
        let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
        let base = player.get_datum(datum);

        let result = match base {
            Datum::PropList(prop_list, _is_sorted) => {
                // Get the property from the prop list
                let prop_value = PropListUtils::get_by_key(&prop_list, &key, &player.allocator)?;

                // If there's a second argument and the property is a list, index into it
                if args.len() >= 2 {
                    let index_ref = &args[1];
                    let index = player.get_datum(index_ref).int_value()?;
                    let prop_datum = player.get_datum(&prop_value);

                    match prop_datum {
                        Datum::List(_, items, _) => {
                            // Support both 0-based and 1-based indexing
                            let actual_index = if index == 0 {
                                0
                            } else if index >= 1 {
                                (index - 1) as usize
                            } else {
                                return Err(ScriptError::new(format!(
                                    "Index out of bounds: {}",
                                    index
                                )));
                            };

                            if actual_index >= items.len() {
                                return Err(ScriptError::new(format!(
                                    "Index out of bounds: {}",
                                    index
                                )));
                            }

                            items[actual_index].clone()
                        }
                        _ => return Err(ScriptError::new(
                            "Second argument to getPropRef requires first property to be a list"
                                .to_string(),
                        )),
                    }
                } else {
                    prop_value
                }
            }
            _ => {
                return Err(ScriptError::new(
                    "getPropRef: datum is not a propList".to_string(),
                ))
            }
        };

        // If there are more keys, recursively resolve
        if args.len() > 2 {
            TypeUtils::get_sub_prop(&result, &args[2], player)
        } else {
            Ok(result)
        }
    }
}
