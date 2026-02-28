use itertools::Itertools;
use log::{debug, warn};

use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType},
    player::{
        allocator::ScriptInstanceAllocatorTrait,
        bitmap::bitmap::{get_system_default_palette, Bitmap, BuiltInPalette, PaletteRef},
        ci_string::CiStr,
        compare::sort_datums,
        datum_formatting::format_datum,
        eval::eval_lingo_expr_runtime,
        geometry::IntRect,
        reserve_player_mut, reserve_player_ref,
        sprite::{ColorRef, CursorRef},
        xtra::manager::{create_xtra_instance, is_xtra_registered},
        DatumRef, DirPlayer, MathObject, ScriptError, XmlDocument,
    },
};

use super::datum_handlers::{
    date::DateObject,
    list_handlers::ListDatumHandlers,
    player_call_datum_handler,
    prop_list::{PropListDatumHandlers, PropListUtils},
    rect::RectUtils,
    script_instance::ScriptInstanceDatumHandlers,
    sound_channel::{SoundChannelDatumHandlers, SoundStatus},
};

pub struct TypeHandlers {}
pub struct TypeUtils {}

impl TypeUtils {
    pub fn get_datum_ilks(datum: &Datum) -> Result<Vec<&str>, ScriptError> {
        match datum {
            Datum::List(..) => Ok(vec!["list", "linearlist"]),
            Datum::Int(..) => Ok(vec!["integer"]),
            Datum::Float(..) => Ok(vec!["float"]),
            Datum::String(..) => Ok(vec!["string"]),
            Datum::Symbol(..) => Ok(vec!["symbol"]),
            Datum::Void | Datum::Null => Ok(vec!["void"]),
            Datum::PropList(..) => Ok(vec!["proplist", "list"]),
            Datum::ScriptInstanceRef(..) => Ok(vec!["instance"]),
            Datum::ScriptRef(..) => Ok(vec!["script"]),
            Datum::CastMember(member_ref) => Ok(vec![if member_ref.is_valid() {
                "member"
            } else {
                "void"
            }]),
            Datum::ColorRef(..) => Ok(vec!["color"]),
            Datum::TimeoutRef(..) => Ok(vec!["timeout"]),
            Datum::TimeoutFactory => Ok(vec!["timeout"]),
            Datum::TimeoutInstance { .. } => Ok(vec!["timeout"]),
            Datum::BitmapRef(..) => Ok(vec!["image"]),
            Datum::Rect(..) => Ok(vec!["rect"]),
            Datum::Point(..) => Ok(vec!["point"]),
            Datum::SpriteRef(..) => Ok(vec!["sprite"]),
            Datum::PaletteRef(..) => Ok(vec!["palette"]),
            Datum::Vector(..) => Ok(vec!["vector"]),
            Datum::StringChunk(..) => Ok(vec!["string"]),
            Datum::CastLib(..) => Ok(vec!["castlib"]),
            Datum::Stage => Ok(vec!["stage"]),
            Datum::SoundChannel(..) => Ok(vec!["instance"]),
            Datum::SoundRef(..) => Ok(vec!["sound"]),
            Datum::CursorRef(..) => Ok(vec!["cursor"]),
            Datum::Xtra(..) => Ok(vec!["xtra"]),
            Datum::XtraInstance(..) => Ok(vec!["instance"]),
            Datum::Matte(..) => Ok(vec!["image"]),
            Datum::PlayerRef => Ok(vec!["player"]),
            Datum::MovieRef => Ok(vec!["movie"]),
            Datum::XmlRef(..) => Ok(vec!["xml"]),
            Datum::DateRef(..) => Ok(vec!["date"]),
            Datum::MathRef(..) => Ok(vec!["math"]),
            Datum::VarRef(..) => Ok(vec!["void"]), // VarRef should be dereferenced before checking ilk

            _ => Err(ScriptError::new(format!(
                "Getting ilk for unknown type: {}",
                datum.type_str()
            )))?,
        }
    }

    pub fn get_datum_ilk(datum: &Datum) -> Result<&str, ScriptError> {
        Ok(Self::get_datum_ilks(datum)?.get(0).unwrap())
    }

    fn is_datum_ilk(datum: &Datum, ilk: &str) -> Result<bool, ScriptError> {
        Ok(Self::get_datum_ilks(datum)?
            .iter()
            .any(|x| x.eq_ignore_ascii_case(ilk)))
    }

    pub fn get_sub_prop(
        datum_ref: &DatumRef,
        prop_key_ref: &DatumRef,
        player: &mut DirPlayer,
    ) -> Result<DatumRef, ScriptError> {
        let datum = player.get_datum(datum_ref);
        let prop_key = player.get_datum(prop_key_ref);

        let formatted_key = format_datum(prop_key_ref, player);
        let result = match datum {
            Datum::PropList(prop_list, ..) => PropListUtils::get_prop(
                prop_list,
                prop_key_ref,
                &player.allocator,
                false,
                formatted_key.clone(),
            )?,
            Datum::Rect(arr) => {
                let index = prop_key.int_value()?; // 1..4
                let idx = index - 1;

                if !(0..4).contains(&idx) {
                    return Err(ScriptError::new(format!(
                        "Rect index {} out of bounds (must be 1-4)",
                        index
                    )));
                }

                let val = player.get_datum(&arr[idx as usize]).float_value()?;
                if val.fract() == 0.0 {
                    player.alloc_datum(Datum::Int(val as i32))
                } else {
                    player.alloc_datum(Datum::Float(val))
                }
            }
            Datum::List(_, list, _) => {
                let position = prop_key.int_value()?;
                let index = position - 1;
                if index < 0 || index >= list.len() as i32 {
                    return Err(ScriptError::new(format!("Index out of bounds: {index}")));
                }
                list[index as usize].clone()
            }
            Datum::Point(arr) => {
                let index = prop_key.int_value()?;

                let (idx, label) = match index {
                    1 => (0usize, "x"),
                    2 => (1usize, "y"),
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Invalid sub-prop position for point: {}",
                            index
                        )))
                    }
                };

                let val = player.get_datum(&arr[idx]).float_value()?;
                if val.fract() == 0.0 {
                    player.alloc_datum(Datum::Int(val as i32))
                } else {
                    player.alloc_datum(Datum::Float(val))
                }
            }
            Datum::ScriptInstanceRef(instance_ref) => {
                // Numeric index
                if let Ok(index) = prop_key.int_value() {
                    let instance = player.allocator.get_script_instance(instance_ref);
                    let mut property_names: Vec<String> =
                        instance.properties.keys().map(|k| k.as_str().to_owned()).collect();
                    property_names.sort();
                    let zero_based_index = (index - 1) as usize;

                    if zero_based_index < property_names.len() {
                        let prop_name = &property_names[zero_based_index];
                        if let Some(prop_ref) = instance.properties.get(CiStr::new(prop_name)) {
                            return Ok(prop_ref.clone());
                        }
                    }
                    return Ok(DatumRef::Void);
                }

                // String key
                if let Ok(prop_name) = prop_key.string_value() {
                    let instance = player.allocator.get_script_instance(instance_ref);
                    if let Some(prop_ref) = instance.properties.get(CiStr::new(&prop_name)) {
                        return Ok(prop_ref.clone());
                    }
                }

                // Symbol key
                if let Datum::Symbol(prop_name) = prop_key {
                    let instance = player.allocator.get_script_instance(instance_ref);
                    if let Some(prop_ref) = instance.properties.get(CiStr::new(prop_name)) {
                        return Ok(prop_ref.clone());
                    }
                }

                return Ok(DatumRef::Void);
            }
            Datum::Int(i) => {
                let prop_name = player.get_datum(prop_key_ref).string_value()?;
                match prop_name.as_str() {
                    "abs" => {
                        let result = i.abs();
                        player.alloc_datum(Datum::Int(result))
                    }
                    "integer" => {
                        datum_ref.clone()
                    }
                    "float" => {
                        player.alloc_datum(Datum::Float(*i as f64))
                    }
                    "char" => {
                        // Convert integer to character
                        if *i >= 0 && *i <= 255 {
                            let ch = char::from_u32(*i as u32).unwrap_or('?');
                            player.alloc_datum(Datum::String(ch.to_string()))
                        } else {
                            return Err(ScriptError::new(format!("Integer {} out of range for char", i)));
                        }
                    }
                    "string" => {
                        player.alloc_datum(Datum::String(i.to_string()))
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Unknown property '{}' for integer",
                            prop_name
                        )));
                    }
                }
            }
            Datum::Float(f) => {
                let prop_name = player.get_datum(prop_key_ref).string_value()?;
                match prop_name.as_str() {
                    "abs" => {
                        let result = f.abs();
                        player.alloc_datum(Datum::Float(result))
                    }
                    "integer" => {
                        let result = f.round() as i32;
                        player.alloc_datum(Datum::Int(result))
                    }
                    "float" => {
                        datum_ref.clone()
                    }
                    "string" => {
                        player.alloc_datum(Datum::String(f.to_string()))
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Unknown property '{}' for float",
                            prop_name
                        )));
                    }
                }
            }
            _ => {
                web_sys::console::log_1(
                    &format!(
                        "  ❌ Cannot get sub-prop '{}' from type {}",
                        formatted_key,
                        datum.type_str()
                    )
                    .into(),
                );
                return Err(ScriptError::new(format!(
                    "Cannot get sub-prop `{}` from prop of type {}",
                    formatted_key,
                    datum.type_str()
                )));
            }
        };
        Ok(result)
    }

    pub fn set_sub_prop(
        datum_ref: &DatumRef,
        prop_key_ref: &DatumRef,
        value_ref: &DatumRef,
        player: &mut DirPlayer,
    ) -> Result<(), ScriptError> {
        let datum_type = player.get_datum(datum_ref).type_enum();
        let formatted_key = format_datum(prop_key_ref, player);
        match datum_type {
            DatumType::PropList => PropListUtils::set_prop(
                datum_ref,
                prop_key_ref,
                value_ref,
                player,
                false,
                formatted_key,
            ),
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
            _ => {
                return Err(ScriptError::new(format!(
                    "Cannot set sub-prop `{}` on prop of type {}",
                    formatted_key,
                    datum_type.type_str()
                )))
            }
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

    pub async fn value(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let eval_expr = reserve_player_mut(|player| {
            let datum = player.get_datum(&args[0]);
            match datum {
                Datum::String(s) => Some(s.clone()),
                _ => None,
            }
        });
        match eval_expr {
            Some(s) => eval_lingo_expr_runtime(s.to_owned()).await.or_else(|err| {
                warn!("value() eval error, returning Void: {}", &err.message);
                Ok(DatumRef::Void)
            }),
            _ => Ok(args[0].clone()),
        }
    }

    pub fn void(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Ok(DatumRef::Void)
    }

    pub fn ilk(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let obj = player.get_datum(&args[0]);
            let ilk_type = args.get(1).map(|d| player.get_datum(d));

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
                }
                // special_symbols
                '.' => return None,
                '-' => {
                    if result.is_empty() {
                        result.push(char);
                    } else {
                        return None;
                    }
                }
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
                }
                Datum::Void => Datum::Void,
                _ => {
                    return Err(ScriptError::new(format!(
                        "Cannot convert datum of type {} to integer",
                        value.type_str()
                    )))
                }
            };
            Ok(player.alloc_datum(result))
        })
    }

    pub fn float(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(&args[0]);
            let result = match value {
                Datum::Float(f) => Datum::Float(*f),
                Datum::Int(i) => Datum::Float(*i as f64),
                Datum::SpriteRef(sprite_num) => Datum::Float(*sprite_num as f64),
                Datum::String(s) => {
                    if let Ok(float_value) = s.parse::<f64>() {
                        Datum::Float(float_value)
                    } else {
                        value.to_owned()
                    }
                }
                Datum::StringChunk(_, _, s) => {
                    if let Ok(float_value) = s.parse::<f64>() {
                        Datum::Float(float_value)
                    } else {
                        value.to_owned()
                    }
                }
                Datum::Void => Datum::Void,
                _ => {
                    return Err(ScriptError::new(format!(
                        "Cannot convert datum of type {} to float",
                        value.type_str()
                    )))
                }
            };
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
                return Err(ScriptError::new(format!(
                    "Cannot convert datum of type {} to symbol",
                    symbol_name.type_str()
                )));
            };
            Ok(player.alloc_datum(result))
        })
    }

    pub fn point(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() != 2 {
                return Err(ScriptError::new("point() requires exactly 2 arguments".to_string()));
            }

            let x = player.get_datum(&args[0]).clone();
            let y = player.get_datum(&args[1]).clone();

            let x_ref = match x {
                Datum::Int(n) => player.alloc_datum(Datum::Int(n)),
                Datum::Float(f) => player.alloc_datum(Datum::Float(f)),
                other => return Err(ScriptError::new(format!(
                    "Point component must be numeric, got {}",
                    other.type_str()
                ))),
            };

            let y_ref = match y {
                Datum::Int(n) => player.alloc_datum(Datum::Int(n)),
                Datum::Float(f) => player.alloc_datum(Datum::Float(f)),
                other => return Err(ScriptError::new(format!(
                    "Point component must be numeric, got {}",
                    other.type_str()
                ))),
            };

            Ok(player.alloc_datum(Datum::Point([x_ref, y_ref])))
        })
    }

    pub fn rect(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() != 2 && args.len() != 4 {
                return Err(ScriptError::new("rect() requires 2 or 4 arguments".to_string()));
            }

            // Helper (normal function, NOT a closure)
            fn preserve_numeric(
                player: &mut DirPlayer,
                dref: &DatumRef,
            ) -> Result<DatumRef, ScriptError> {
                let d = player.get_datum(dref).clone();

                match d {
                    Datum::Int(n) => Ok(player.alloc_datum(Datum::Int(n))),
                    Datum::Float(f) => Ok(player.alloc_datum(Datum::Float(f))),
                    other => Err(ScriptError::new(format!(
                        "Rect component must be numeric, got {}",
                        other.type_str()
                    ))),
                }
            }

            // Case 1: rect(left, top, right, bottom)
            if args.len() == 4 && player.get_datum(&args[0]).is_number() {
                let left   = preserve_numeric(player, &args[0])?;
                let top    = preserve_numeric(player, &args[1])?;
                let right  = preserve_numeric(player, &args[2])?;
                let bottom = preserve_numeric(player, &args[3])?;

                return Ok(player.alloc_datum(Datum::Rect([left, top, right, bottom])));
            }

            // Case 2: rect(Point, Point)
            if args.len() == 2 {
                let p1 = player.get_datum(&args[0]).to_point()?.clone();
                let p2 = player.get_datum(&args[1]).to_point()?.clone();

                let left   = preserve_numeric(player, &p1[0])?;
                let top    = preserve_numeric(player, &p1[1])?;
                let right  = preserve_numeric(player, &p2[0])?;
                let bottom = preserve_numeric(player, &p2[1])?;

                return Ok(player.alloc_datum(Datum::Rect([left, top, right, bottom])));
            }

            Err(ScriptError::new("Invalid rect() arguments".to_string()))
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
                Err(ScriptError::new(
                    "Invalid number of arguments for cursor".to_string(),
                ))
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
                        let member_ref = cast.create_member_at(
                            cast.first_free_member_id(),
                            &s,
                            &mut player.bitmap_manager,
                        )?;
                        Ok(player.alloc_datum(Datum::CastMember(member_ref)))
                    }
                    _ => Err(ScriptError::new(format!(
                        "Unsupported call location type: {}",
                        location.type_str()
                    )))?,
                }
            }),
            DatumType::ScriptRef => {
                Ok(
                    player_call_datum_handler(&args[0], &"new".to_owned(), &args[1..].to_vec())
                        .await?,
                )
            }
            DatumType::Xtra => {
                let xtra_name = reserve_player_ref(|player| {
                    player
                        .get_datum(&args[0])
                        .to_xtra_name()
                        .unwrap()
                        .to_owned()
                });
                let result_id = create_xtra_instance(&xtra_name, args)?;
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::XtraInstance(xtra_name, result_id)))
                })
            }
            _ => Err(ScriptError::new(format!(
                "Unsupported new call with subject type: {}",
                obj_type.type_str()
            ))),
        }?;
        Ok(result)
    }

    pub fn timeout(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.is_empty() {
                // Called without arguments: return the timeout factory
                Ok(player.alloc_datum(Datum::TimeoutFactory))
            } else {
                // Called with a name argument: return a timeout reference
                let name = player.get_datum(&args[0]).string_value()?;
                Ok(player.alloc_datum(Datum::TimeoutRef(name)))
            }
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
                    if hex_str.len() != 6 {
                        log::warn!("Invalid hex color string: {}", hex_str);
                        Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(0, 0, 0))))
                    } else {
                        let r = u8::from_str_radix(&hex_str[0..2], 16).unwrap();
                        let g = u8::from_str_radix(&hex_str[2..4], 16).unwrap();
                        let b = u8::from_str_radix(&hex_str[4..6], 16).unwrap();
                        Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(r, g, b))))
                    }
                } else {
                    Err(ScriptError::new(
                        "Invalid number of arguments for rgb".to_string(),
                    ))
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
            // TODO: Palette ref can be on args[3], need to handle it
            if args.len() < 3 {
                return Err(ScriptError::new(
          format!("image() expects at least 3 arguments: width, height, bitDepth, optional alphaDepth, got {}", args.len())
        ));
            }

            let width_datum = player.get_datum(&args[0]);
            let height_datum = player.get_datum(&args[1]);

            let width = match width_datum {
                Datum::Int(i) => *i as u16,
                Datum::Float(f) => {
                    let rounded = f.round() as u16;
                    rounded
                }
                _ => {
                    let val = width_datum.int_value()? as u16;
                    val
                }
            };

            let height = match height_datum {
                Datum::Int(i) => *i as u16,
                Datum::Float(f) => {
                    let rounded = f.round() as u16;
                    rounded
                }
                _ => {
                    let val = height_datum.int_value()? as u16;
                    val
                }
            };

            let bit_depth = player.get_datum(&args[2]).int_value()? as u8;
            let mut palette_ref = PaletteRef::BuiltIn(get_system_default_palette());
            let mut alpha_depth = 0;
            if args.len() >= 4 {
                let arg3 = player.get_datum(&args[3]);
                match arg3.type_enum() {
                    DatumType::Int => {
                        alpha_depth = arg3.int_value()? as u8;
                    }
                    DatumType::Symbol => {
                        palette_ref = match arg3 {
                            Datum::Symbol(s) => {
                                PaletteRef::BuiltIn(BuiltInPalette::from_symbol_string(s).unwrap())
                            }
                            _ => {
                                return Err(ScriptError::new(format!(
                                    "Invalid 4th argument type for image(): {}, expected symbol",
                                    arg3.type_str()
                                )))
                            }
                        };
                    }
                    DatumType::PaletteRef => {
                        // If the 4th argument is a palette, then there's no alpha depth specified
                        palette_ref = match arg3 {
                            Datum::PaletteRef(p) => p.clone(),
                            _ => {
                                return Err(ScriptError::new(format!(
                                    "Invalid 4th argument type for image(): {}, expected palette",
                                    arg3.type_str()
                                )))
                            }
                        };
                    }
                    DatumType::CastMemberRef => {
                        // If the 4th argument is a cast member, then there's no alpha depth specified
                        palette_ref = match arg3 {
              Datum::CastMember(m) => PaletteRef::Member(m.clone()),
              _ => return Err(ScriptError::new(
                format!("Invalid 4th argument type for image(): {}, expected int or palette", arg3.type_str())
              )),
            };
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Invalid 4th argument type for image(): {}, expected int or palette",
                            arg3.type_str()
                        )));
                    }
                }
            }

            let bitmap = Bitmap::new(
                width,
                height,
                bit_depth,
                bit_depth,
                alpha_depth,
                palette_ref,
            );
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
                _ => {
                    return Err(ScriptError::new(format!(
                        "Cannot get abs of type: {}",
                        value.type_str()
                    )))
                }
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
                Err(ScriptError::new(format!(
                    "Xtra {} is not registered",
                    xtra_name
                )))
            }
        })
    }

    pub fn union(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() != 2 {
                return Err(ScriptError::new("Union requires 2 arguments".to_string()));
            }

            let left_refs = player.get_datum(&args[0]).to_rect()?;
            let right_refs = player.get_datum(&args[1]).to_rect()?;

            let rect_to_tuple = |rect: [DatumRef; 4]| -> Result<(i32, i32, i32, i32), ScriptError> {
                let l = Datum::to_f64(player, &rect[0])? as i32;
                let t = Datum::to_f64(player, &rect[1])? as i32;
                let r = Datum::to_f64(player, &rect[2])? as i32;
                let b = Datum::to_f64(player, &rect[3])? as i32;
                Ok((l, t, r, b))
            };

            let left_tuple = rect_to_tuple(left_refs)?;
            let right_tuple = rect_to_tuple(right_refs)?;

            let (l, t, r, b) = RectUtils::union(left_tuple, right_tuple);
            let rect = IntRect { left: l, top: t, right: r, bottom: b };

            Ok(player.alloc_datum(rect.to_datum()))
        })
    }

    pub fn bit_xor(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() != 2 {
                return Err(ScriptError::new(
                    "Bitwise XOR requires 2 arguments".to_string(),
                ));
            }
            let left = player.get_datum(&args[0]).int_value()?;
            let right = player.get_datum(&args[1]).int_value()?;

            Ok(player.alloc_datum(Datum::Int(left ^ right)))
        })
    }

    /// vector() or vector(x, y, z)
    pub fn vector(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (x, y, z) = match args.len() {
                0 => (0.0, 0.0, 0.0),
                3 => (
                    player.get_datum(&args[0]).to_float()? as f64,
                    player.get_datum(&args[1]).to_float()? as f64,
                    player.get_datum(&args[2]).to_float()? as f64,
                ),
                _ => {
                    return Err(ScriptError::new(
                        "vector() expects 0 or 3 arguments".to_string(),
                    ))
                }
            };
            Ok(player.alloc_datum(Datum::Vector([x, y, z])))
        })
    }

    pub fn color(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            match args.len() {
                1 => {
                    // color(paletteIndex) - single argument is palette index
                    let index = player.get_datum(&args[0]).int_value()? as u8;
                    Ok(player.alloc_datum(Datum::ColorRef(ColorRef::PaletteIndex(index))))
                }
                2 => {
                    // color(#rgb, "RRGGBB") or color(#paletteIndex, index)
                    let first = player.get_datum(&args[0]);
                    if let Datum::Symbol(sym) = first {
                        match sym.to_lowercase().as_str() {
                            "rgb" => {
                                let hex_str = player.get_datum(&args[1]).string_value()?.replace("#", "");
                                let r = u8::from_str_radix(&hex_str[0..2], 16).unwrap_or(0);
                                let g = u8::from_str_radix(&hex_str[2..4], 16).unwrap_or(0);
                                let b = u8::from_str_radix(&hex_str[4..6], 16).unwrap_or(0);
                                Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(r, g, b))))
                            }
                            "paletteindex" => {
                                let index = player.get_datum(&args[1]).int_value()? as u8;
                                Ok(player.alloc_datum(Datum::ColorRef(ColorRef::PaletteIndex(index))))
                            }
                            _ => Err(ScriptError::new(format!(
                                "color(): unknown color type symbol #{}",
                                sym
                            ))),
                        }
                    } else {
                        Err(ScriptError::new(
                            "color() with 2 arguments expects first argument to be a symbol".to_string(),
                        ))
                    }
                }
                3 => {
                    // color(r, g, b)
                    let r = player.get_datum(&args[0]).int_value()? as u8;
                    let g = player.get_datum(&args[1]).int_value()? as u8;
                    let b = player.get_datum(&args[2]).int_value()? as u8;
                    Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(r, g, b))))
                }
                4 => {
                    // color(#rgb, r, g, b) - first argument is symbol, skip it
                    let r = player.get_datum(&args[1]).int_value()? as u8;
                    let g = player.get_datum(&args[2]).int_value()? as u8;
                    let b = player.get_datum(&args[3]).int_value()? as u8;
                    Ok(player.alloc_datum(Datum::ColorRef(ColorRef::Rgb(r, g, b))))
                }
                _ => Err(ScriptError::new(format!(
                    "color() expects 1, 2, 3, or 4 arguments, got {}",
                    args.len()
                ))),
            }
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
                }
                (Datum::Float(base), Datum::Float(exponent)) => {
                    Ok(player.alloc_datum(Datum::Float(base.powf(*exponent))))
                }
                (Datum::Float(base), Datum::Int(exponent)) => {
                    Ok(player.alloc_datum(Datum::Float(base.powf(*exponent as f64))))
                }
                (Datum::Int(base), Datum::Float(exponent)) => {
                    Ok(player.alloc_datum(Datum::Float((*base as f64).powf(*exponent))))
                }
                _ => Err(ScriptError::new("Power requires two numbers".to_string())),
            }
        })
    }

    pub fn add(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.len() != 2 {
            return Err(ScriptError::new("Add requires 2 arguments".to_string()));
        }
        let left_type = reserve_player_ref(|player| player.get_datum(&args[0]).type_enum());

        if left_type == DatumType::Void {
            // Operations on void return void
            return Ok(DatumRef::Void);
        }

        match left_type {
            DatumType::List => {
                ListDatumHandlers::add(args.get(0).unwrap(), &vec![args.get(1).unwrap().clone()])
            }
            _ => Err(ScriptError::new(format!(
                "Add not supported for {}",
                left_type.type_str()
            ))),
        }
    }

    pub fn nothing(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Ok(DatumRef::Void)
    }

    pub fn get_a_prop(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        let datum_ref = args.get(0).unwrap();
        let (datum_type, datum_debug) = reserve_player_mut(|player| {
            let datum = player.get_datum(&args[0]);
            let debug_str = match datum {
                Datum::Symbol(s) => format!("#{}", s),
                Datum::String(s) => format!("\"{}\"", s),
                Datum::Int(i) => format!("{}", i),
                _ => format!("{:?}", datum.type_enum()),
            };
            (datum.type_enum(), debug_str)
        });
        match datum_type {
            DatumType::PropList => {
                PropListDatumHandlers::get_a_prop(datum_ref, &vec![args.get(1).unwrap().clone()])
            }
            DatumType::ScriptInstanceRef => {
                ScriptInstanceDatumHandlers::get_a_prop(datum_ref, &vec![args.get(1).unwrap().clone()])
            }
            _ => Err(ScriptError::new(format!(
                "Cannot getaProp prop of type: {} (value: {})",
                datum_type.type_str(),
                datum_debug
            ))),
        }
    }

    pub fn min(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() == 0 {
                return Ok(player.alloc_datum(Datum::Int(0)));
            }
            let args = if player.get_datum(&args[0]).is_list() {
                player.get_datum(&args[0]).to_list()?
            } else {
                args
            };
            if args.len() == 0 {
                // TODO this returns [] instead
                return Ok(player.alloc_datum(Datum::Int(0)));
            }

            let sorted_list = sort_datums(args, &player.allocator)?;
            return Ok(sorted_list.first().unwrap().clone());
        })
    }

    pub fn max(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() == 0 {
                return Ok(player.alloc_datum(Datum::Int(0)));
            }
            let args = if player.get_datum(&args[0]).is_list() {
                player.get_datum(&args[0]).to_list()?
            } else {
                args
            };
            if args.len() == 0 {
                // TODO this returns [] instead
                return Ok(player.alloc_datum(Datum::Int(0)));
            }

            let sorted_list = sort_datums(args, &player.allocator)?;
            return Ok(sorted_list.last().unwrap().clone());
        })
    }

    pub fn sort(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let datum_ref = &args[0];
            match player.get_datum(datum_ref) {
                Datum::PropList(_, _) => PropListDatumHandlers::sort(datum_ref, &vec![]),
                _ => ListDatumHandlers::sort(datum_ref, &vec![]),
            }
        })
    }

    pub fn intersect(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() != 2 {
                return Err(ScriptError::new("Intersect requires 2 arguments".to_string()));
            }

            let left_refs = player.get_datum(&args[0]).to_rect()?;
            let right_refs = player.get_datum(&args[1]).to_rect()?;

            let left = (
                player.get_datum(&left_refs[0]).int_value()?,
                player.get_datum(&left_refs[1]).int_value()?,
                player.get_datum(&left_refs[2]).int_value()?,
                player.get_datum(&left_refs[3]).int_value()?,
            );
            let right = (
                player.get_datum(&right_refs[0]).int_value()?,
                player.get_datum(&right_refs[1]).int_value()?,
                player.get_datum(&right_refs[2]).int_value()?,
                player.get_datum(&right_refs[3]).int_value()?,
            );

            let (l, t, r, b) = RectUtils::intersect(left, right);
            let rect = IntRect { left: l, top: t, right: r, bottom: b };

            Ok(player.alloc_datum(rect.to_datum()))
        })
    }

    // pub fn get_prop_at(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    //     let datum_ref = args.get(0).unwrap();
    //     let prop_key_ref = args.get(1).unwrap();
    //     reserve_player_mut(|player| TypeUtils::get_sub_prop(datum_ref, prop_key_ref, player))
    // }

    pub fn get_prop_at(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        use crate::player::datum_formatting::format_concrete_datum;
        reserve_player_ref(|player| {
            let prop_list_ref = &args[0];
            let position = player.get_datum(&args[1]).int_value()?;
            let index = (position - 1) as usize;
            
            let prop_list = player.get_datum(prop_list_ref);
            
            debug!(
                "🔍 getPropAt: proplist={}, index={}", 
                format_concrete_datum(prop_list, player),
                position
            );
            
            match prop_list {
                Datum::PropList(entries, _) => {
                    if index >= entries.len() {
                        return Err(ScriptError::new(format!(
                            "Index {} out of bounds for proplist of length {}", 
                            position, 
                            entries.len()
                        )));
                    }
                    // Return the KEY at this position, not the value!
                    let key_ref = entries[index].0.clone();
                    
                    debug!(
                        "✅ getPropAt returned key: {}", 
                        format_concrete_datum(player.get_datum(&key_ref), player)
                    );
                    
                    Ok(key_ref)
                }
                _ => Err(ScriptError::new(
                    "getPropAt requires a property list".to_string()
                )),
            }
        })
    }

    pub fn pi(_: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Float(std::f64::consts::PI))))
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

    pub fn sqrt(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(&args[0]);
            
            let num = if let Ok(f) = value.float_value() {
                f
            } else if let Ok(i) = value.int_value() {
                i as f64
            } else {
                return Err(ScriptError::new("sqrt requires a number".to_string()));
            };
            
            if num < 0.0 {
                return Err(ScriptError::new("sqrt of negative number".to_string()));
            }
            
            let result = num.sqrt();
            Ok(player.alloc_datum(Datum::Float(result)))
        })
    }

    pub fn tan(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(&args[0]).to_float()?;
            Ok(player.alloc_datum(Datum::Float(value.tan())))
        })
    }

    pub fn atan(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let value = player.get_datum(&args[0]);
            
            let num = if let Ok(f) = value.float_value() {
                f
            } else if let Ok(i) = value.int_value() {
                i as f64
            } else {
                return Err(ScriptError::new("atan requires a number".to_string()));
            };
            
            let result = num.atan();
            Ok(player.alloc_datum(Datum::Float(result)))
        })
    }

    pub fn sound(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let first_arg = player.get_datum(&args[0]).clone();
            // Command form: sound(#verb, channelNum, ...args)
            // e.g. sound #stop, 3  or  sound #play, 1, member("snd")
            if let Datum::Symbol(verb) = &first_arg {
                let verb = verb.to_lowercase();
                let channel_num = if args.len() > 1 {
                    player.get_datum(&args[1]).int_value()? as u16
                } else {
                    1 // default to channel 1
                };
                if channel_num == 0 || channel_num as usize > player.sound_manager.num_channels() {
                    return Err(ScriptError::new(format!(
                        "Invalid sound channel: {}",
                        channel_num
                    )));
                }
                let channel_datum = player.alloc_datum(Datum::SoundChannel(channel_num));
                let remaining_args: Vec<DatumRef> = args[2..].to_vec();
                SoundChannelDatumHandlers::call(player, &channel_datum, &verb, &remaining_args)
            } else {
                // Function form: sound(channelNum) - returns a SoundChannel datum
                let channel_num = first_arg.int_value()? as u16;
                if channel_num == 0 || channel_num as usize > player.sound_manager.num_channels() {
                    return Err(ScriptError::new(format!(
                        "Invalid sound channel: {}",
                        channel_num
                    )));
                }
                Ok(player.alloc_datum(Datum::SoundChannel(channel_num)))
            }
        })
    }

    pub async fn call_ancestor(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // callAncestor(#handler, me, arg1, arg2, ...)
        //
        // In Director, callAncestor:
        // 1. Finds the ancestor of the 'me' argument (args[1])
        // 2. Looks up the handler in the ancestor's SCRIPT
        // 3. Executes the handler with 'me' still being the ORIGINAL instance
        //
        // The key insight: when inside an ancestor's handler (due to callAncestor),
        // a nested callAncestor should use the CURRENT SCOPE's receiver to determine
        // which ancestor to call next, NOT args[1] (which is still the original me).
        //
        // Example: if A has ancestor B, B has ancestor C:
        // - A::start calls callAncestor(#start, me, ...)
        //   -> current receiver is A, ancestor is B
        //   -> B::start runs with receiver=A (so 'me' properties come from A)
        // - Inside B::start, callAncestor(#start, me, ...) is called
        //   -> current receiver is A, but we need B's ancestor (C)
        //   -> We look at the scope's script_ref to find B, then get B's ancestor
        let (ancestor_list, original_me_list, instance_datum_refs, handler_name, extra_args) = reserve_player_mut(|player| {
            let handler_name = player.get_datum(&args[0]).string_value()?;

            // Get the current scope's script_ref to determine which script we're currently in
            let current_scope_ref = player.current_scope_ref();
            let current_script_ref = player.scopes.get(current_scope_ref)
                .map(|scope| scope.script_ref.clone());

            let list_or_script_instance = player.get_datum(&args[1]);
            let instance_list = match list_or_script_instance {
                Datum::List(_, list, _) => list.to_owned(),
                Datum::ScriptInstanceRef(_) => {
                    vec![args[1].clone()]
                }
                _ => {
                    return Err(ScriptError::new(format!(
                        "Can only callAncestor on script instances and lists"
                    )))
                }
            };

            let mut ancestor_list = vec![];
            let mut original_me_list = vec![];
            let mut instance_datum_refs = vec![];
            for instance_ref in instance_list {
                let original_me_ref = player.get_datum(&instance_ref).to_script_instance_ref()?;
                original_me_list.push(original_me_ref.clone());
                instance_datum_refs.push(instance_ref.clone());

                // Determine which instance's ancestor to use:
                //
                // The key insight: we need to find the instance in the ancestor chain
                // whose SCRIPT matches the current scope's script_ref. That tells us
                // "which level" of the ancestor chain we're currently executing in.
                // Then we get THAT instance's ancestor.
                //
                // Example: A (script=A) -> B (script=B) -> C (script=C)
                // When A::start calls callAncestor(#start, me, ...):
                //   - current_script_ref = A's script
                //   - We find A in chain, get A's ancestor = B
                // When B::start calls callAncestor(#start, me, ...):
                //   - current_script_ref = B's script (because that's what we're executing)
                //   - We find B in chain, get B's ancestor = C
                let ancestor_source = if let Some(ref script_ref) = current_script_ref {
                    // Walk the ancestor chain to find which instance has the script
                    // we're currently executing
                    let mut walk_ref = original_me_ref.clone();
                    let mut found = false;
                    for _ in 0..100 { // Safety limit
                        let walk_instance = player.allocator.get_script_instance(&walk_ref);
                        if walk_instance.script == *script_ref {
                            found = true;
                            break;
                        }
                        if let Some(ref next_ancestor) = walk_instance.ancestor {
                            walk_ref = next_ancestor.clone();
                        } else {
                            break;
                        }
                    }
                    if found {
                        walk_ref
                    } else {
                        // Fallback: use original_me
                        original_me_ref.clone()
                    }
                } else {
                    // No script_ref, use original_me
                    original_me_ref.clone()
                };

                let instance = player.allocator.get_script_instance(&ancestor_source);
                let ancestor = instance.ancestor.as_ref().ok_or_else(|| {
                    ScriptError::new("Instance has no ancestor".to_string())
                })?;
                ancestor_list.push(ancestor.clone());
            }
            // Get extra arguments beyond the instance list (args[2..])
            // The instance itself will be prepended in each iteration
            let extra_args = args[2..].to_vec();
            Ok((ancestor_list, original_me_list, instance_datum_refs, handler_name, extra_args))
        })?;

        let mut result = DatumRef::Void;
        for ((ancestor_ref, original_me_ref), instance_datum_ref) in ancestor_list
            .into_iter()
            .zip(original_me_list.into_iter())
            .zip(instance_datum_refs.into_iter())
        {
            // Walk up the ancestor chain to find a script that has the handler.
            // For example, if A->B->C and B doesn't have the handler but C does,
            // we should call C's handler.
            let handler_and_instance = reserve_player_ref(|player| {
                let mut walk_ref = ancestor_ref.clone();
                for _ in 0..100 { // Safety limit
                    let walk_instance = player.allocator.get_script_instance(&walk_ref);
                    let script = player.movie.cast_manager.get_script_by_ref(&walk_instance.script);
                    if let Some(script) = script {
                        if let Some(handler_ref) = script.get_own_handler_ref(&handler_name) {
                            return Some((handler_ref, walk_ref.clone()));
                        }
                    }
                    // Handler not found in this script, try the next ancestor
                    if let Some(ref next_ancestor) = walk_instance.ancestor {
                        walk_ref = next_ancestor.clone();
                    } else {
                        // No more ancestors
                        break;
                    }
                }
                None
            });

            if let Some((handler_ref, _handler_instance_ref)) = handler_and_instance {
                // Build call_args with the individual instance first, then extra args.
                // This ensures that when callAncestor is called with a list like [me],
                // the handler receives 'me' as its first argument, not '[me]'.
                let mut call_args = vec![instance_datum_ref.clone()];
                call_args.extend(extra_args.clone());

                // Call with the ORIGINAL me as receiver (for property access),
                // but use the handler we found in the ancestor chain.
                // use_raw_arg_list=true so args are used as-is
                let scope_result = crate::player::player_call_script_handler_raw_args(
                    Some(original_me_ref.clone()),  // receiver = original me, for property access
                    handler_ref,                    // handler from ancestor's script
                    &call_args,
                    true,  // use_raw_arg_list = true: don't prepend receiver to args
                ).await?;
                crate::player::player_handle_scope_return(&scope_result);
                result = scope_result.return_value;
            }
        }
        Ok(result)
    }

    pub async fn new_object(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::new(
                "newObject requires at least one argument".to_string(),
            ));
        }

        let object_type = reserve_player_ref(|player| player.get_datum(&args[0]).string_value())?;

        match object_type.to_lowercase().as_str() {
            "xml" => reserve_player_mut(|player| {
                let xml_id = player.allocator.get_free_script_instance_id();
                let xml_doc = XmlDocument {
                    id: xml_id,
                    root_element: None,
                    content: String::new(),
                    ignore_white: false,
                };
                player.xml_documents.insert(xml_id, xml_doc);
                Ok(player.alloc_datum(Datum::XmlRef(xml_id)))
            }),
            "date" => reserve_player_mut(|player| {
                let date_id = player.allocator.get_free_script_instance_id();
                let date_obj = DateObject::new(date_id);
                player.date_objects.insert(date_id, date_obj);
                Ok(player.alloc_datum(Datum::DateRef(date_id)))
            }),
            "math" => reserve_player_mut(|player| {
                let math_id = player.allocator.get_free_script_instance_id();
                let math_obj = MathObject::new(math_id);
                player.math_objects.insert(math_id, math_obj);
                Ok(player.alloc_datum(Datum::MathRef(math_id)))
            }),
            "object" => {
                reserve_player_mut(|player| {
                    // Allocate an empty prop list, unsorted
                    let obj = Datum::PropList(Vec::new(), false);
                    Ok(player.alloc_datum(obj))
                })
            }
            "string" => {
                let value = if args.len() > 1 {
                    reserve_player_ref(|player| player.get_datum(&args[1]).string_value())?
                } else {
                    String::new()
                };
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(value))))
            }
            _ => Err(ScriptError::new(format!(
                "newObject: Unsupported object type '{}'",
                object_type
            ))),
        }
    }

    pub fn sound_busy(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {       
        reserve_player_mut(|player| {
            if args.is_empty() {
                return Err(ScriptError::new("soundBusy requires a channel number".to_string()));
            }
            
            let channel_num_ref = &args[0];
            let channel_num = player.get_datum(channel_num_ref).int_value()?;
            
            // Convert to 0-based index
            let channel_idx = (channel_num - 1) as usize;
            
            // Get the channel directly from sound manager
            let channel_rc = player.sound_manager.get_channel(channel_idx)
                .ok_or_else(|| ScriptError::new(format!(
                    "Sound channel {} out of range",
                    channel_num
                )))?;
            
            let channel = channel_rc.borrow();
            
            // soundBusy returns true (1) if the channel is Playing, Loading, or Queued
            let is_busy = matches!(
                channel.status, 
                SoundStatus::Playing | SoundStatus::Loading | SoundStatus::Queued
            );
            debug!("🔍 soundBusy({}) = {} (status: {:?})", 
                channel_num, is_busy, channel.status);
            
            Ok(player.alloc_datum(Datum::Int(if is_busy { 1 } else { 0 })))
        })
    }
}
