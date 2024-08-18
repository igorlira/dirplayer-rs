use itertools::Itertools;

use crate::{director::lingo::datum::{datum_bool, Datum, DatumType}, player::{allocator::ScriptInstanceAllocatorTrait, bitmap::bitmap::{get_system_default_palette, Bitmap, BuiltInPalette, PaletteRef}, compare::sort_datums, datum_formatting::format_datum, eval::eval_lingo, geometry::IntRect, reserve_player_mut, reserve_player_ref, sprite::{ColorRef, CursorRef}, xtra::manager::{create_xtra_instance, is_xtra_registered}, DatumRef, DirPlayer, ScriptError}};

use super::datum_handlers::{list_handlers::ListDatumHandlers, player_call_datum_handler, prop_list::{PropListDatumHandlers, PropListUtils}, rect::RectUtils};


pub struct TypeHandlers {}
pub struct TypeUtils {}

impl TypeUtils {
  pub fn get_datum_ilks(datum: &Datum) -> Result<Vec<&str>, ScriptError> {
    match datum {
      Datum::List(..) => Ok(vec!["list", "linearlist"]),
      Datum::Int(..) => Ok(vec!["integer"]),
      Datum::String(..) => Ok(vec!["string"]),
      Datum::Symbol(..) => Ok(vec!["symbol"]),
      Datum::Void | Datum::Null => Ok(vec!["void"]),
      Datum::PropList(..) => Ok(vec!["proplist", "list"]),
      Datum::ScriptInstanceRef(..) => Ok(vec!["instance"]),
      Datum::ScriptRef(..) => Ok(vec!["script"]),
      Datum::CastMember(member_ref) => Ok(vec![if member_ref.is_valid() { "member" } else { "void" }]),
      Datum::ColorRef(..) => Ok(vec!["color"]),
      Datum::TimeoutRef(..) => Ok(vec!["timeout"]), // TODO verify this
      Datum::BitmapRef(..) => Ok(vec!["image"]),
      Datum::IntRect(..) => Ok(vec!["rect"]),
      Datum::IntPoint(..) => Ok(vec!["point"]),
      Datum::SpriteRef(..) => Ok(vec!["sprite"]),
      Datum::PaletteRef(..) => Ok(vec!["palette"]),
      _ => Err(ScriptError::new(format!("Getting ilk for unknown type: {}", datum.type_str())))?,
    }
  }

  pub fn get_datum_ilk(datum: &Datum) -> Result<&str, ScriptError> {
    Ok(Self::get_datum_ilks(datum)?.get(0).unwrap())
  }

  fn is_datum_ilk(datum: &Datum, ilk: &str) -> Result<bool, ScriptError> {
    Ok(Self::get_datum_ilks(datum)?.iter().any(|x| x.eq_ignore_ascii_case(ilk)))
  }

  pub fn get_sub_prop(datum_ref: &DatumRef, prop_key_ref: &DatumRef, player: &mut DirPlayer) -> Result<DatumRef, ScriptError> {
    let datum = player.get_datum(datum_ref);
    let formatted_key = format_datum(prop_key_ref, player);
    let result = match datum {
      Datum::PropList(prop_list, ..) => {
        PropListUtils::get_prop(prop_list, prop_key_ref, &player.allocator, false, formatted_key)?
      },
      Datum::List(_, list, _) => {
        let position = player.get_datum(prop_key_ref).int_value()?;
        let index = position - 1;
        if index < 0 || index >= list.len() as i32 {
          return Err(ScriptError::new(format!("Index out of bounds: {index}")));
        }
        list[index as usize].clone()
      }
      Datum::IntPoint((x, y)) => {
        let prop_key = player.get_datum(prop_key_ref);
        player.alloc_datum(match prop_key {
          Datum::Int(position) => {
            match position {
              1 => Datum::Int(*x as i32),
              2 => Datum::Int(*y as i32),
              _ => return Err(ScriptError::new(format!("Invalid sub-prop position for point: {position}"))),
            }
          },
          _ => return Err(ScriptError::new(format!("Invalid sub-prop type for point: {}", prop_key.type_str()))),
        })
      }
      _ => return Err(ScriptError::new(format!("Cannot get sub-prop `{}` from prop of type {}", formatted_key, datum.type_str()))),
    };
    Ok(result)
  }

  pub fn set_sub_prop(datum_ref: &DatumRef, prop_key_ref: &DatumRef, value_ref: &DatumRef, player: &mut DirPlayer) -> Result<(), ScriptError> { 
    let datum_type = player.get_datum(datum_ref).type_enum();
    let formatted_key = format_datum(prop_key_ref, player);
    match datum_type {
      DatumType::PropList => {
        PropListUtils::set_prop(datum_ref, prop_key_ref, value_ref, player, false, formatted_key)
      }
      DatumType::List => {
        let position = player.get_datum(prop_key_ref).int_value()?;
        let index = position - 1;
        let (_, list, _) = player.get_datum_mut(datum_ref).to_list_mut().unwrap();
        if index < 0 {
          return Err(ScriptError::new(format!("Index out of bounds: {index}")));
        } else if index < list.len() as i32 {
          list[index as usize] = value_ref.clone();
        } else {
          // FIXME this is not the same as Director, which would fill in the list with zeros
          list.resize((index as usize + 1).max(list.len()), DatumRef::Void);
          list[index as usize] = value_ref.clone();
        }
        Ok(())
      }
      _ => return Err(ScriptError::new(format!("Cannot set sub-prop `{}` on prop of type {}", formatted_key, datum_type.type_str()))),
    }
  }
}

impl TypeHandlers {
  pub fn objectp(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let is_object = match obj {
        Datum::Void => false,
        Datum::Float(_) => false,
        Datum::Int(_) => false,
        Datum::Symbol(_) => false,
        Datum::String(_) => false,
        _ => true,
      };
      Ok(player.alloc_datum(datum_bool(is_object)))
    })
  }

  pub fn voidp(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let is_void = match obj {
        Datum::Void => true,
        _ => false,
      };
      Ok(player.alloc_datum(datum_bool(is_void)))
    })
  }

  pub fn listp(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let is_list = match obj {
        Datum::List(..) => true,
        Datum::PropList(..) => true,
        _ => false,
      };
      Ok(player.alloc_datum(datum_bool(is_list)))
    })
  }

  pub fn symbolp(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let is_symbol = match obj {
        Datum::Symbol(_) => true,
        _ => false,
      };
      Ok(player.alloc_datum(datum_bool(is_symbol)))
    })
  }

  pub fn stringp(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let is_string = match obj {
        Datum::String(_) => true,
        Datum::StringChunk(..) => true,
        _ => false,
      };
      Ok(player.alloc_datum(datum_bool(is_string)))
    })
  }

  pub fn integerp(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let is_integer = match obj {
        Datum::Int(_) => true,
        _ => false,
      };
      Ok(player.alloc_datum(datum_bool(is_integer)))
    })
  }

  pub fn floatp(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let is_float = match obj {
        Datum::Float(_) => true,
        _ => false,
      };
      Ok(player.alloc_datum(datum_bool(is_float)))
    })
  }

  pub fn value(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let expr = player.get_datum(&args[0]);
      match expr {
        Datum::String(s) => eval_lingo(s.to_owned(), player),
        _ => Ok(args[0].clone()),
      }
    })
  }

  pub fn void(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    Ok(DatumRef::Void)
  }

  pub fn ilk(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      let ilk_type = args
        .get(1)
        .map(|d| player.get_datum(d));

      let result_datum = if let Some(query) = ilk_type {
        let query = query.string_value()?;
        datum_bool(TypeUtils::is_datum_ilk(&obj, &query)?)
      } else {
        Datum::Symbol(TypeUtils::get_datum_ilk(&obj)?.to_string())
      };
      Ok(player.alloc_datum(result_datum))
    })
  }

  fn integer_impl(input: &str) -> Option<i32> {
    if input.is_empty() {
      return None;
    }

    // Remove leading and trailing whitespace
    let trimmed_input = input.trim();

    if trimmed_input.is_empty() {
      return Some(0);
    }

    if trimmed_input == "-" {
      return Some(0);
    }

    let mut result = String::new();
    let mut found_valid_digit = false;

    for char in trimmed_input.chars() {
      match char {
        // numeric_chars
        '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' => {
          result.push(char);
          found_valid_digit = true;
        },
        // special_symbols
        '.' => return None,
        '-' => {
          if result.is_empty() {
            result.push(char);
          } else {
            return None;
          }
        },
        // unknown
        _ => {
          if !found_valid_digit {
            return None;
          }
        }
      };
    }

    if !found_valid_digit {
      return None;
    }

    // Convert result to integer
    if let Ok(final_result) = result.parse::<i32>() {
      return Some(final_result);
    }

    None
  }

  pub fn integer(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let value = player.get_datum(&args[0]);
      let result = match value {
        Datum::Int(i) => Datum::Int(*i),
        Datum::Float(f) => Datum::Int(f.round() as i32),
        Datum::SpriteRef(sprite_num) => Datum::Int(*sprite_num as i32),
        Datum::String(s) => {
          let result = Self::integer_impl(&s);
          if let Some(int_value) = result {
            Datum::Int(int_value)
          } else {
            return Ok(DatumRef::Void);
          }
        },
        Datum::Void => Datum::Void,
        _ => return Err(ScriptError::new(format!("Cannot convert datum of type {} to integer", value.type_str()))),
      };
      Ok(player.alloc_datum(result))
    })
  }

  pub fn float(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let value = player.get_datum(&args[0]);
      let result = if value.is_number() {
        Ok(Datum::Float(value.to_float()?))
      } else if value.is_string() {
        if let Ok(float_value) = value.string_value()?.parse::<f32>() {
          Ok(Datum::Float(float_value))
        } else {
          Ok(value.to_owned())
        }
      } else if value.is_void() {
        Ok(Datum::Void)
      } else {
        Err(ScriptError::new(format!("Cannot create float from datum of type {}", value.type_str())))
      }?;
      Ok(player.alloc_datum(result))
    })
  }

  pub fn symbol(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let symbol_name = player.get_datum(&args[0]);
      let result = if let Datum::Symbol(_) = symbol_name {
        symbol_name.clone()
      } else if symbol_name.is_string() {
        let str_value = symbol_name.string_value()?;
        if str_value.is_empty() {
          Datum::Symbol("".to_string())
        } else if str_value.starts_with("#") {
          Datum::Symbol("#".to_string())
        } else {
          Datum::Symbol(str_value)
        }
      } else {
        return Err(ScriptError::new(format!("Cannot convert datum of type {} to symbol", symbol_name.type_str())));
      };
      Ok(player.alloc_datum(result))
    })
  }

  pub fn point(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let x = player.get_datum(&args[0]).int_value()?;
      let y = player.get_datum(&args[1]).int_value()?;
      Ok(player.alloc_datum(Datum::IntPoint((x, y))))
    })
  }

  pub fn rect(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let first_arg_is_num = player.get_datum(&args[0]).is_number();
      let (left, top, right, bottom) = if args.len() == 4 && first_arg_is_num {
        let left = player.get_datum(&args[0]).int_value()?;
        let top = player.get_datum(&args[1]).int_value()?;
        let right = player.get_datum(&args[2]).int_value()?;
        let bottom = player.get_datum(&args[3]).int_value()?;
        (left, top, right, bottom)
      } else if args.len() == 4 && !first_arg_is_num {
        let top_left = player.get_datum(&args[0]).to_int_point()?;
        let top_right = player.get_datum(&args[1]).to_int_point()?;
        let bottom_right = player.get_datum(&args[2]).to_int_point()?;
        let bottom_left = player.get_datum(&args[3]).to_int_point()?;
        let rect = IntRect::from_quad(top_left, top_right, bottom_right, bottom_left);
        (rect.left, rect.top, rect.right, rect.bottom)
      } else {
        let left_top = player.get_datum(&args[0]).to_int_point()?;
        let right_bottom = player.get_datum(&args[1]).to_int_point()?;
        (left_top.0, left_top.1, right_bottom.0, right_bottom.1)
      };

      Ok(player.alloc_datum(Datum::IntRect((left, top, right, bottom))))
    })
  }

  pub fn cursor(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() == 1 {
        let arg = player.get_datum(&args[0]);
        if arg.is_int() {
          player.cursor = CursorRef::System(arg.int_value()?);
          Ok(DatumRef::Void)
        } else if arg.is_list() {
          let list = arg.to_list()?;
          // TODO why not: let members = list.clone().iter().map(|x| player.get_datum(x).int_value()).collect_vec();
          let members = list.clone().iter().map(|x| x.unwrap() as i32).collect_vec();
          player.cursor = CursorRef::Member(members);
          Ok(DatumRef::Void)
        } else {
          Err(ScriptError::new("Invalid argument for cursor".to_string()))
        }
      } else if args.len() == 2 {
        Err(ScriptError::new("Cursor call not implemented".to_string()))
      } else {
        Err(ScriptError::new("Invalid number of arguments for cursor".to_string()))
      }
    })
  }

  pub async fn new(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let obj_type = reserve_player_mut(|player| {
      let obj = player.get_datum(&args[0]);
      obj.type_enum()
    });
    let result = match obj_type {
      DatumType::Symbol => reserve_player_mut(|player| {
        let location = player.get_datum(&args[1]);
        match location {
          Datum::CastLib(cast_num) => {
            let s = player.get_datum(&args[0]).string_value()?;
            let cast = player.movie.cast_manager.get_cast_mut(*cast_num);
            let member_ref = cast.create_member_at(cast.first_free_member_id(), &s, &mut player.bitmap_manager)?;
            Ok(player.alloc_datum(Datum::CastMember(member_ref)))
          },
          _ => Err(ScriptError::new(format!("Unsupported call location type: {}", location.type_str())))?,
        }
      }),
      DatumType::ScriptRef => {
        Ok(
          player_call_datum_handler(&args[0], &"new".to_owned(), &args[1..].to_vec()).await?
        )
      },
      DatumType::Xtra => {
        let xtra_name = reserve_player_ref(|player| {
          player.get_datum(&args[0]).to_xtra_name().unwrap().to_owned()
        });
        let result_id = create_xtra_instance(&xtra_name, args)?;
        reserve_player_mut(|player| {
          Ok(player.alloc_datum(Datum::XtraInstance(xtra_name, result_id)))
        })
      }
      _ => Err(ScriptError::new(format!("Unsupported new call with subject type: {}", obj_type.type_str()))),
    }?;
    Ok(result)
  }

  pub fn timeout(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let name = player.get_datum(&args[0]).string_value()?;
      Ok(player.alloc_datum(Datum::TimeoutRef(name)))
    })
  }

  pub fn rgb(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() == 3 {
        let r = player.get_datum(&args[0]).int_value()? as u8;
        let g = player.get_datum(&args[1]).int_value()? as u8;
        let b = player.get_datum(&args[2]).int_value()? as u8;
        Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(r, g, b))))
      } else {
        let first_arg = player.get_datum(&args[0]);
        if first_arg.is_string() {
          let hex_str = first_arg.string_value()?.replace("#", "");
          let r = u8::from_str_radix(&hex_str[0..2], 16).unwrap();
          let g = u8::from_str_radix(&hex_str[2..4], 16).unwrap();
          let b = u8::from_str_radix(&hex_str[4..6], 16).unwrap();
          Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(r, g, b))))
        } else {
          Err(ScriptError::new("Invalid number of arguments for rgb".to_string()))
        }
      }
    })
  }

  pub fn palette_index(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let color = player.get_datum(&args[0]).int_value()?;
      Ok(player.alloc_datum(Datum::ColorRef(ColorRef::PaletteIndex(color as u8))))
    })
  }

  pub fn list(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      Ok(player.alloc_datum(Datum::List(DatumType::List, args.clone(), false)))
    })
  }

  pub fn image(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let width = player.get_datum(&args[0]).int_value()?;
      let height = player.get_datum(&args[1]).int_value()?;
      let bit_depth = player.get_datum(&args[2]).int_value()?;
      let palette_ref = match args.get(3) {
        Some(palette_ref) => {
          let palette_ref = player.get_datum(palette_ref);
          match palette_ref {
            Datum::Symbol(s) => PaletteRef::BuiltIn(BuiltInPalette::from_symbol_string(s).unwrap()),
            Datum::PaletteRef(palette_ref) => palette_ref.clone(),
            Datum::CastMember(member_ref) => PaletteRef::Member(member_ref.clone()),
            _ => return Err(ScriptError::new(format!("Invalid palette argument of type {} for image", palette_ref.type_str()))),
          }
        },
        None => PaletteRef::BuiltIn(get_system_default_palette()),
      };

      let bitmap = Bitmap::new(width as u16, height as u16, bit_depth as u8, palette_ref, None);
      let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
      Ok(player.alloc_datum(Datum::BitmapRef(bitmap_ref)))
    })
  }

  pub fn abs(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let value = player.get_datum(&args[0]);
      let result = match value {
        Datum::Int(i) => Datum::Int(i.abs()),
        Datum::Float(f) => Datum::Float(f.abs()),
        _ => return Err(ScriptError::new(format!("Cannot get abs of type: {}", value.type_str()))),
      };
      Ok(player.alloc_datum(result))
    })
  }

  pub fn xtra(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let xtra_name = player.get_datum(&args[0]).string_value()?;
      if is_xtra_registered(&xtra_name) {
        Ok(player.alloc_datum(Datum::Xtra(xtra_name)))
      } else {
        Err(ScriptError::new(format!("Xtra {} is not registered", xtra_name)))
      }
    })
  }

  pub fn union(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() != 2 {
        return Err(ScriptError::new("Union requires 2 arguments".to_string()));
      }
      let left = player.get_datum(&args[0]).to_int_rect()?;
      let right = player.get_datum(&args[1]).to_int_rect()?;

      Ok(player.alloc_datum(Datum::IntRect(RectUtils::union(left, right))))
    })
  }

  pub fn bit_xor(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() != 2 {
        return Err(ScriptError::new("Bitwise XOR requires 2 arguments".to_string()));
      }
      let left = player.get_datum(&args[0]).int_value()?;
      let right = player.get_datum(&args[1]).int_value()?;

      Ok(player.alloc_datum(Datum::Int(left ^ right)))
    })
  }

  pub fn power(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() != 2 {
        return Err(ScriptError::new("Power requires 2 arguments".to_string()));
      }
      let base = player.get_datum(&args[0]);
      let exponent = player.get_datum(&args[1]);

      match (base, exponent) {
        (Datum::Int(base), Datum::Int(exponent)) => {
          Ok(player.alloc_datum(Datum::Int(base.pow(*exponent as u32))))
        },
        (Datum::Float(base), Datum::Float(exponent)) => {
          Ok(player.alloc_datum(Datum::Float(base.powf(*exponent))))
        },
        (Datum::Float(base), Datum::Int(exponent)) => {
          Ok(player.alloc_datum(Datum::Float(base.powf(*exponent as f32))))
        },
        _ => Err(ScriptError::new("Power requires two numbers".to_string())),
      }
    })
  }

  pub fn add(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    if args.len() != 2 {
      return Err(ScriptError::new("Add requires 2 arguments".to_string()));
    }
    let left_type = reserve_player_ref(|player| {
      player.get_datum(&args[0]).type_enum()
    });
    match left_type {
      DatumType::List => ListDatumHandlers::add(args.get(0).unwrap(), &vec![args.get(1).unwrap().clone()]),
      _ => Err(ScriptError::new(format!("Add not supported for {}", left_type.type_str()))),
    }
  }

  pub fn nothing(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    Ok(DatumRef::Void)
  }

  pub fn get_a_prop(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let datum_ref = args.get(0).unwrap();
    let datum_type = reserve_player_mut(|player| {
      player.get_datum(&args[0]).type_enum()
    });
    match datum_type {
      DatumType::PropList => {
        PropListDatumHandlers::get_a_prop(datum_ref, &vec![args.get(1).unwrap().clone()])
      },
      _ => Err(ScriptError::new(format!("Cannot getaProp prop of type: {}", datum_type.type_str()))),
    }
  }

  pub fn min(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() == 0 {
        return Ok(player.alloc_datum(Datum::Int(0)))
      }
      let args = if player.get_datum(&args[0]).is_list() {
        player.get_datum(&args[0]).to_list()?
      } else {
        args
      };
      if args.len() == 0 {
        // TODO this returns [] instead
        return Ok(player.alloc_datum(Datum::Int(0)))
      }

      let sorted_list = sort_datums(args, &player.allocator)?;
      return Ok(sorted_list.first().unwrap().clone())
    })
  }

  pub fn max(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() == 0 {
        return Ok(player.alloc_datum(Datum::Int(0)))
      }
      let args = if player.get_datum(&args[0]).is_list() {
        player.get_datum(&args[0]).to_list()?
      } else {
        args
      };
      if args.len() == 0 {
        // TODO this returns [] instead
        return Ok(player.alloc_datum(Datum::Int(0)))
      }

      let sorted_list = sort_datums(args, &player.allocator)?;
      return Ok(sorted_list.last().unwrap().clone())
    })
  }

  pub fn sort(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    ListDatumHandlers::sort(&args[0], &vec![])
  }

  pub fn intersect(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      if args.len() != 2 {
        return Err(ScriptError::new("Intersect requires 2 arguments".to_string()));
      }
      let left = player.get_datum(&args[0]).to_int_rect()?;
      let right = player.get_datum(&args[1]).to_int_rect()?;

      Ok(player.alloc_datum(Datum::IntRect(RectUtils::intersect(left, right))))
    })
  }

  pub fn get_prop_at(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let datum_ref = args.get(0).unwrap();
    let prop_key_ref = args.get(1).unwrap();
    reserve_player_mut(|player| {
      TypeUtils::get_sub_prop(datum_ref, prop_key_ref, player)
    })
  }

  pub fn pi(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      Ok(player.alloc_datum(Datum::Float(std::f32::consts::PI)))
    })
  }

  pub fn sin(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let value = player.get_datum(&args[0]).to_float()?;
      Ok(player.alloc_datum(Datum::Float(value.sin())))
    })
  }

  pub fn cos(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let value = player.get_datum(&args[0]).to_float()?;
      Ok(player.alloc_datum(Datum::Float(value.cos())))
    })
  }

  pub fn sound(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let channel_num = player.get_datum(&args[0]).int_value()? as u16;
      Ok(player.alloc_datum(Datum::SoundRef(channel_num)))
    })
  }

  pub async fn call_ancestor(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let (ref_list, handler_name, args) = reserve_player_mut(|player| {
      let handler_name = player.get_datum(&args[0]).string_value()?;
      let instance_list = player.get_datum(&args[1]).to_list()?.clone();
      let mut ref_list = vec![];
      for instance_ref in instance_list {
        let instance_ref = player.get_datum(&instance_ref).to_script_instance_ref()?;
        let instance = player.allocator.get_script_instance(instance_ref);
        let ancestor = instance.ancestor.as_ref().unwrap();
        ref_list.push(player.alloc_datum(Datum::ScriptInstanceRef(ancestor.clone())));
      }
      let args = args[2..].to_vec();
      Ok((ref_list, handler_name, args))
    })?;
    let mut result = DatumRef::Void;
    for ref_item in ref_list {
      result = player_call_datum_handler(&ref_item, &handler_name.to_string(), &args).await?;
    }
    Ok(result)
  }
}
