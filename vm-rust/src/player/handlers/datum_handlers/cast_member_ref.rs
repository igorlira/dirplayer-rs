use crate::{console_warn, director::lingo::datum::{datum_bool, Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType}, js_api::JsApi, player::{bitmap::bitmap::{Bitmap, BuiltInPalette, PaletteRef}, cast_lib::CastMemberRef, cast_member::{CastMember, CastMemberType, CastMemberTypeId, TextMember}, font::measure_text, handlers::types::TypeUtils, reserve_player_mut, reserve_player_ref, DatumRef, DirPlayer, ScriptError, VOID_DATUM_REF}};

use super::string_chunk::StringChunkUtils;
use num::FromPrimitive;

pub struct CastMemberRefHandlers {}

pub fn borrow_member_mut<T1, F1, T2, F2>(
  member_ref: &CastMemberRef,
  player_f: F2,
  f: F1,
) -> T1 where F1 : FnOnce(&mut CastMember, T2) -> T1, F2 : FnOnce(&mut DirPlayer) -> T2 {
  reserve_player_mut(|player| {
    let arg = player_f(player);
    let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref).unwrap();
    f(member, arg)
  })
}

fn get_text_member_line_height(text_data: &TextMember) -> u16 {
  return text_data.font_size + 3; // TODO: Implement text line height
}

impl CastMemberRefHandlers {
  pub fn get_cast_slot_number(cast_lib: u32, cast_member: u32) -> u32 {
    (cast_lib << 16) | (cast_member & 0xFFFF)
  }

  pub fn member_ref_from_slot_number(slot_number: u32) -> CastMemberRef {
    CastMemberRef { 
      cast_lib: (slot_number >> 16) as i32,
      cast_member: (slot_number & 0xFFFF) as i32,
    }
  }

  pub fn call(datum: DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match handler_name.as_str() {
      "duplicate" => Self::duplicate(datum, args),
      "erase" => Self::erase(datum, args),
      "charPosToLoc" => {
        reserve_player_mut(|player| {
          let cast_member_ref = match player.get_datum(datum) {
            Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
            _ => return Err(ScriptError::new("Cannot call charPosToLoc on non-cast-member".to_string())),
          };
          let cast_member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref).unwrap();
          let text_data = cast_member.member_type.as_text().unwrap();
          let char_pos = player.get_datum(args[0]).int_value(&player.datums)? as u16;
          let char_width: u16 = 7; // TODO: Implement char width
          let line_height = get_text_member_line_height(&text_data);
          let result = if text_data.text.is_empty() || char_pos <= 0 {
            Datum::IntPoint((0, 0))
          } else if char_pos > text_data.text.len() as u16 {
            Datum::IntPoint(((char_width * (text_data.text.len() as u16)) as i16, line_height as i16))
          } else {
            Datum::IntPoint(((char_width * (char_pos - 1)) as i16, line_height as i16))
          };
          // TODO this is a stub!
          Ok(player.alloc_datum(result))
        })
      },
      "getProp" => {
        let result_ref = reserve_player_mut(|player| {
          let cast_member_ref = match player.get_datum(datum) {
            Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
            _ => return Err(ScriptError::new("Cannot call getProp on non-cast-member".to_string())),
          };
          let prop = player.get_datum(args[0]).string_value(&player.datums)?;
          let result = Self::get_prop(player, &cast_member_ref, &prop)?;
          Ok(player.alloc_datum(result))
        })?;
        if args.len() > 1 {
          reserve_player_mut(|player| {
            TypeUtils::get_sub_prop(result_ref, args[1], player)
          })
        } else {
          Ok(result_ref)
        }
      }
      _ => Self::call_member_type(datum, handler_name, args),
    }
  }

  fn call_member_type(datum: DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let member_ref = match player.get_datum(datum) {
        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
        _ => return Err(ScriptError::new("Cannot call_member_type on non-cast-member".to_string())),
      };
      let cast_member = player.movie.cast_manager.find_member_by_ref(&member_ref).unwrap();
      match &cast_member.member_type {
        CastMemberType::Field(field) => {
          match handler_name.as_str() {
            "count" => {
              let count_of = player.get_datum(args[0]).string_value(&player.datums)?;
              if args.len() != 1 {
                return Err(ScriptError::new("count requires 1 argument".to_string()));
              }
              let delimiter = &player.movie.item_delimiter;
              let count = StringChunkUtils::resolve_chunk_count(&field.text, StringChunkType::from(&count_of), delimiter)?;
              Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            _ => Err(ScriptError::new(format!("No handler {handler_name} for field member type")))
          }
        }
        CastMemberType::Text(text) => {
          match handler_name.as_str() {
            "count" => {
              let count_of = player.get_datum(args[0]).string_value(&player.datums)?;
              if args.len() != 1 {
                return Err(ScriptError::new("count requires 1 argument".to_string()));
              }
              let delimiter = &player.movie.item_delimiter;
              let count = StringChunkUtils::resolve_chunk_count(&text.text, StringChunkType::from(&count_of), delimiter)?;
              Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            "getPropRef" => {
              let prop_name = player.get_datum(args[0]).string_value(&player.datums)?;
              let start = player.get_datum(args[1]).int_value(&player.datums)?;
              let end = if args.len() > 2 { player.get_datum(args[2]).int_value(&player.datums)? } else { start };
              let chunk_expr = StringChunkType::from(&prop_name);
              let chunk_expr = StringChunkExpr {
                chunk_type: chunk_expr,
                start,
                end,
                item_delimiter: player.movie.item_delimiter.clone(),
              };
              let resolved_str = StringChunkUtils::resolve_chunk_expr_string(&text.text, &chunk_expr)?;
              Ok(player.alloc_datum(Datum::StringChunk(StringChunkSource::Member(member_ref), chunk_expr, resolved_str)))
            }
            _ => Err(ScriptError::new(format!("No handler {handler_name} for text member type")))
          }
        }
        _ => Err(ScriptError::new(format!("No handler {handler_name} for member type")))
      }
    })
  }

  fn erase(datum: DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let cast_member_ref = match player.get_datum(datum) {
        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
        _ => return Err(ScriptError::new("Cannot erase non-cast-member".to_string())),
      };
      player.movie.cast_manager.remove_member_with_ref(&cast_member_ref)?;
      Ok(VOID_DATUM_REF)
    })
  }

  fn duplicate(datum: DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
      let cast_member_ref = match player.get_datum(datum) {
        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
        _ => return Err(ScriptError::new("Cannot duplicate non-cast-member".to_string())),
      };
      let dest_slot_number = args.get(0)
        .map(|x| player.get_datum(*x).int_value(&player.datums));

      if dest_slot_number.is_none() {
        return Err(ScriptError::new("Cannot duplicate cast member without destination slot number".to_string()));
      }
      let dest_slot_number = dest_slot_number.unwrap()?;
      let dest_ref = Self::member_ref_from_slot_number(dest_slot_number as u32);
      
      let mut new_member = {
        let src_member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
        if src_member.is_none() {
          return Err(ScriptError::new("Cannot duplicate non-existent cast member reference".to_string()));
        }
        src_member.unwrap().clone()
      };
      new_member.number = dest_ref.cast_member as u32;

      let dest_cast = player.movie.cast_manager.get_cast_mut(dest_ref.cast_lib as u32);
      dest_cast.insert_member(dest_ref.cast_member as u32, new_member);

      Ok(player.alloc_datum(Datum::Int(dest_slot_number)))
    })
  }

  fn get_invalid_member_prop(
    _: &DirPlayer,
    member_ref: &CastMemberRef,
    prop: &String,
  ) -> Result<Datum, ScriptError> {
    match prop.as_str() {
      "name" => Ok(Datum::String("".to_string())),
      "number" => Ok(Datum::Int(-1)),
      "type" => Ok(Datum::String("empty".to_string())),
      "castLibNum" => Ok(Datum::Int(-1)),
      "width" => Ok(Datum::Void),
      "height" => Ok(Datum::Void),
      "rect" => Ok(Datum::Void),
      _ => Err(ScriptError::new(format!("Cannot get prop {} of invalid cast member ({}, {})", prop, member_ref.cast_lib, member_ref.cast_member))),
    }
  }

  fn get_member_type_prop(
    player: &mut DirPlayer,
    cast_member_ref: &CastMemberRef,
    member_type: &CastMemberTypeId,
    prop: &String,
  ) -> Result<Datum, ScriptError> {
    match &member_type {
      CastMemberTypeId::Bitmap => {
        let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref).unwrap();
        let bitmap_member = member.member_type.as_bitmap().unwrap();
        let bitmap_ref = bitmap_member.image_ref;
        let bitmap = player.bitmap_manager.get_bitmap(bitmap_member.image_ref);
        if !bitmap.is_some() {
          return Err(ScriptError::new(format!("Cannot get prop of invalid bitmap ref")));
        }
        match prop.as_str() {
          "width" => Ok(Datum::Int(bitmap.map(|x| x.width as i32).unwrap_or(0))),
          "height" => Ok(Datum::Int(bitmap.map(|x| x.height as i32).unwrap_or(0))),
          "image" => Ok(Datum::BitmapRef(bitmap_ref)),
          "paletteRef" => Ok(Datum::PaletteRef(bitmap.map(|x| x.palette_ref.clone()).unwrap_or(PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))),
          "regPoint" => Ok(Datum::IntPoint(bitmap_member.reg_point)),
          "rect" => {
            let width = bitmap.map(|x| x.width as i16).unwrap_or(0);
            let height = bitmap.map(|x| x.height as i16).unwrap_or(0);
            Ok(Datum::IntRect((0, 0, width, height)))
          },
          _ => {
            Err(ScriptError::new(format!("Cannot get castMember property {} for bitmap", prop)))
          }
        }
      }
      CastMemberTypeId::Field => {
        let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref).unwrap();
        let field = member.member_type.as_field().unwrap();
        match prop.as_str() {
          "text" => Ok(Datum::String(field.text.to_owned())),
          _ => {
            Err(ScriptError::new(format!("Cannot get castMember property {} for field", prop)))
          }
        }
      }
      CastMemberTypeId::Text => {
        let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref).unwrap();
        let text_data = member.member_type.as_text().unwrap().clone();
        match prop.as_str() {
          "text" => Ok(Datum::String(text_data.text.to_owned())),
          "alignment" => Ok(Datum::String(text_data.alignment.to_owned())),
          "wordWrap" => Ok(datum_bool(text_data.word_wrap)),
          "width" => Ok(Datum::Int(text_data.width as i32)),
          "font" => Ok(Datum::String(text_data.font.to_owned())),
          "fontSize" => Ok(Datum::Int(text_data.font_size as i32)),
          "fontStyle" => {
            let mut item_refs = Vec::new();
            for item in &text_data.font_style {
              item_refs.push(player.alloc_datum(Datum::Symbol(item.to_owned())));
            }
            Ok(Datum::List(DatumType::List, item_refs, false))
          },
          "fixedLineSpace" => Ok(Datum::Int(text_data.fixed_line_space as i32)),
          "topSpacing" => Ok(Datum::Int(text_data.top_spacing as i32)),
          "boxType" => Ok(Datum::Symbol(text_data.box_type.to_owned())),
          "antialias" => Ok(datum_bool(text_data.anti_alias)),
          "rect" => {
            let font = player.font_manager.get_system_font().unwrap();
            let (width, height) = measure_text(&text_data.text, &font, None, text_data.fixed_line_space, text_data.top_spacing);
            Ok(Datum::IntRect((0, 0, width as i16, height as i16)))
          },
          "height" => {
            let font = player.font_manager.get_system_font().unwrap();
            let (_, height) = measure_text(&text_data.text, &font, None, text_data.fixed_line_space, text_data.top_spacing);
            Ok(Datum::Int(height as i32))
          },
          "image" => {
            // TODO: alignment
            let font = player.font_manager.get_system_font().unwrap();
            let (width, height) = measure_text(&text_data.text, &font, None, text_data.fixed_line_space, text_data.top_spacing);
            // TODO use 32 bits
            let mut bitmap = Bitmap::new(width, height, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale));
            let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();
            let palettes = player.movie.cast_manager.palettes();

            let ink = 36;
            bitmap.draw_text(
              &text_data.text, 
              font, 
              font_bitmap, 
              0, 
              text_data.top_spacing, 
              ink, 
              bitmap.get_bg_color_ref(), 
              &palettes,
              text_data.fixed_line_space,
              text_data.top_spacing,
            );

            let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
            Ok(Datum::BitmapRef(bitmap_ref))
          }
          _ => {
            Err(ScriptError::new(format!("Cannot get castMember property {} for text", prop)))
          }
        }
      }
      _ => {
        Err(ScriptError::new(format!("Cannot get castMember prop {} for member of type {:?}", prop, member_type)))
      }
    }
  }

  fn set_member_type_prop(
    member_ref: &CastMemberRef,
    prop: &String,
    value: Datum,
  ) -> Result<(), ScriptError> {
    let member_type = reserve_player_ref(|player| {
      let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref);
      match cast_member {
        Some(cast_member) => Ok(cast_member.member_type.member_type_id()),
        None => Err(ScriptError::new(format!("Setting prop of invalid castMember reference"))),
      }
    })?;

    match member_type {
      CastMemberTypeId::Field => {
        match prop.as_str() {
          "text" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().text = value?;
              Ok(())
            }
          ),
          "rect" => {
            borrow_member_mut(
              member_ref, 
              |_| {
                value.to_int_rect()
              }, 
              |cast_member, value| {
                let value = value?;
                let field_data = cast_member.member_type.as_field_mut().unwrap();
                field_data.width = value.2 as u16;
                Ok(())
              }
            )
          },
          "alignment" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().alignment = value?;
              Ok(())
            }
          ),
          "wordWrap" => borrow_member_mut(
            member_ref, 
            |player| value.bool_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().word_wrap = value?;
              Ok(())
            }
          ),
          "width" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().width = value? as u16;
              Ok(())
            }
          ),
          "font" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().font = value?;
              Ok(())
            }
          ),
          "fontSize" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().font_size = value? as u16;
              Ok(())
            }
          ),
          "fontStyle" => {
            borrow_member_mut(
              member_ref, 
              |player| {
                value.string_value(&player.datums)
              }, 
              |cast_member, value| {
                cast_member.member_type.as_field_mut().unwrap().font_style = value?;
                Ok(())
              }
            )
          },
          "fixedLineSpace" => borrow_member_mut(
            member_ref, 
            |player| value.bool_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().fixed_line_space = value? as u16;
              Ok(())
            }
          ),
          "topSpacing" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().top_spacing = value? as i16;
              Ok(())
            }
          ),
          "boxType" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().box_type = value?;
              Ok(())
            }
          ),
          "antialias" => borrow_member_mut(
            member_ref, 
            |player| value.bool_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().anti_alias = value?;
              Ok(())
            }
          ),
          "autoTab" => borrow_member_mut(
            member_ref, 
            |player| value.bool_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().auto_tab = value?;
              Ok(())
            }
          ),
          "editable" => borrow_member_mut(
            member_ref, 
            |player| value.bool_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().editable = value?;
              Ok(())
            }
          ),
          "border" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_field_mut().unwrap().border = value? as u16;
              Ok(())
            }
          ),
          _ => {
            Err(ScriptError::new(format!("Cannot set castMember prop {} for field", prop)))
          }
        }
      }
      CastMemberTypeId::Text => {
        match prop.as_str() {
          "text" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().text = value?;
              Ok(())
            }
          ),
          "alignment" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().alignment = value?;
              Ok(())
            }
          ),
          "wordWrap" => borrow_member_mut(
            member_ref, 
            |player| value.bool_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().word_wrap = value?;
              Ok(())
            }
          ),
          "width" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().width = value? as u16;
              Ok(())
            }
          ),
          "font" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().font = value?;
              Ok(())
            }
          ),
          "fontSize" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().font_size = value? as u16;
              Ok(())
            }
          ),
          "fontStyle" => {
            borrow_member_mut(
              member_ref, 
              |player| {
                let mut item_strings = Vec::new();
                for x in value.to_list().unwrap() {
                  item_strings.push(player.get_datum(*x).string_value(&player.datums)?);
                }
                Ok(item_strings)
              }, 
              |cast_member, value| {
                cast_member.member_type.as_text_mut().unwrap().font_style = value?;
                Ok(())
              }
            )
          },
          "fixedLineSpace" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().fixed_line_space = value? as u16;
              Ok(())
            }
          ),
          "topSpacing" => borrow_member_mut(
            member_ref, 
            |player| value.int_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().top_spacing = value? as i16;
              Ok(())
            }
          ),
          "boxType" => borrow_member_mut(
            member_ref, 
            |player| value.string_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().box_type = value?;
              Ok(())
            }
          ),
          "antialias" => borrow_member_mut(
            member_ref, 
            |player| value.bool_value(&player.datums), 
            |cast_member, value| {
              cast_member.member_type.as_text_mut().unwrap().anti_alias = value?;
              Ok(())
            }
          ),
          "rect" => {
            borrow_member_mut(
              member_ref, 
              |player| {
                let rect = value.to_int_rect()?;
                let rect: (i16, i16, i16, i16) = (rect.1 as i16, rect.0 as i16, rect.3 as i16, rect.2 as i16);
                Ok(rect)
              }, 
              |cast_member, value| {
                let value = value?;
                let text_data = cast_member.member_type.as_text_mut().unwrap();
                text_data.width = value.2 as u16;
                Ok(())
              }
            )
          },
          _ => {
            Err(ScriptError::new(format!("Cannot set castMember prop {} for text", prop)))
          }
        }
      }
      CastMemberTypeId::Bitmap => {
        match prop.as_str() {
          "image" => {
            let bitmap_ref = value.to_bitmap_ref()?;
            reserve_player_mut(|player| {
              let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();
              let clone = bitmap.clone();

              let member_image_ref = {
                let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref).unwrap();
                let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                bitmap_member.image_ref
              };
              player.bitmap_manager.replace_bitmap(member_image_ref, clone);
              Ok(())
            })
          }
          "regPoint" => {
            borrow_member_mut(
              member_ref, 
              |_| {}, 
              |cast_member, _| {
                let value = value.to_int_point()?;
                let value: (i16, i16) = (value.0 as i16, value.1 as i16);
                cast_member.member_type.as_bitmap_mut().unwrap().reg_point = value;
                Ok(())
              }
            )
          },
          "paletteRef" => {
            let bitmap_id = borrow_member_mut(
              member_ref, 
              |_| {}, 
              |cast_member, _| {
                let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                let bitmap = bitmap_member.image_ref;
                Ok(bitmap)
              }
            )?;
            match value {
              Datum::Symbol(name) => {
                let palette_ref = BuiltInPalette::from_symbol_string(&name).unwrap();
                reserve_player_mut(|player| {
                  let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                  bitmap.palette_ref = PaletteRef::BuiltIn(palette_ref);
                  Ok(())
                })?;
              },
              Datum::CastMember(member_ref) => {
                reserve_player_mut(|player| {
                  let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                  bitmap.palette_ref = PaletteRef::from(member_ref.cast_member as i16, member_ref.cast_lib as u32);
                  Ok(())
                })?;
              },
              _ => {
                return Err(ScriptError::new(format!("Cannot set bitmap member paletteRef to type {}", value.type_str())))
              }
            }
            Ok(())
          }
          "palette" => {
            let bitmap_id = borrow_member_mut(
              member_ref, 
              |_| {}, 
              |cast_member, _| {
                let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                let bitmap = bitmap_member.image_ref;
                Ok(bitmap)
              }
            )?;
            match value {
              Datum::Int(palette_ref) => {
                let member = CastMemberRefHandlers::member_ref_from_slot_number(palette_ref as u32);
                reserve_player_mut(|player| {
                  let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                  if palette_ref < 0 {
                    bitmap.palette_ref = PaletteRef::BuiltIn(BuiltInPalette::from_i16(palette_ref as i16).unwrap())
                  } else {
                    bitmap.palette_ref = PaletteRef::from(member.cast_member as i16, member.cast_lib as u32);
                  }
                  Ok(())
                })?;
              },
              Datum::CastMember(member_ref) => {
                reserve_player_mut(|player| {
                  let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                  bitmap.palette_ref = PaletteRef::from(member_ref.cast_member as i16, member_ref.cast_lib as u32);
                  Ok(())
                })?;
              },
              _ => {
                return Err(ScriptError::new(format!("Cannot set bitmap member palette to type {}", value.type_str())))
              }
            }
            Ok(())
          }
          _ => {
            Err(ScriptError::new(format!("Cannot set castMember prop {} for bitmap", prop)))
          }
        }
      }
      _ => {
        Err(ScriptError::new(format!("Cannot set castMember prop {} for member of type {:?}", prop, member_type)))
      }
    }
  }

  pub fn get_prop(
    player: &mut DirPlayer,
    cast_member_ref: &CastMemberRef,
    prop: &String,
  ) -> Result<Datum, ScriptError> {
    let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
    if is_invalid {
      return Self::get_invalid_member_prop(player, cast_member_ref, prop);
    }
    let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref);
    let (name, slot_number, member_type, color, bg_color) = match cast_member {
      Some(cast_member) => {
        let name = cast_member.name.to_owned();
        let slot_number = Self::get_cast_slot_number(cast_member_ref.cast_lib as u32, cast_member_ref.cast_member as u32) as i32;
        let member_type = cast_member.member_type.member_type_id();
        let color = cast_member.color.to_owned();
        let bg_color = cast_member.bg_color.to_owned();
        (name, slot_number, member_type, color, bg_color)
      },
      None => {
        console_warn!("Getting prop {} of non-existent castMember reference {}, {}", prop, cast_member_ref.cast_lib, cast_member_ref.cast_member);
        return Self::get_invalid_member_prop(player, cast_member_ref, prop);
      }
    };

    match prop.as_str() {
      "name" => Ok(Datum::String(name)),
      "number" => Ok(Datum::Int(slot_number)),
      "type" => Ok(Datum::Symbol(member_type.symbol_string()?.to_string())),
      "castLibNum" => Ok(Datum::Int(cast_member_ref.cast_lib as i32)),
      "color" => Ok(Datum::ColorRef(color)),
      "bgColor" => Ok(Datum::ColorRef(bg_color)),
      _ => Self::get_member_type_prop(player, cast_member_ref, &member_type, prop),
    }
  }

  pub fn set_prop(
    cast_member_ref: &CastMemberRef,
    prop: &String,
    value: Datum,
  ) -> Result<(), ScriptError> {
    let is_invalid = cast_member_ref.cast_lib < 0 || cast_member_ref.cast_member < 0;
    if is_invalid {
      return Err(ScriptError::new(format!("Setting prop of invalid castMember reference")));
    }
    let exists = reserve_player_ref(|player| {
      player.movie.cast_manager.find_member_by_ref(cast_member_ref).is_some()
    });
    let result = if exists {
      match prop.as_str() {
        "name" => borrow_member_mut(
          cast_member_ref, 
          |player| value.string_value(&player.datums), 
          |cast_member, value| {
            cast_member.name = value?;
            Ok(())
          }
        ),
        "color" => borrow_member_mut(
          cast_member_ref, 
          |_| {}, 
          |cast_member, _| {
            cast_member.color = value.to_color_ref()?.to_owned();
            Ok(())
          }
        ),
        "bgColor" => borrow_member_mut(
          cast_member_ref, 
          |_| {}, 
          |cast_member, _| {
            cast_member.bg_color = value.to_color_ref()?.to_owned();
            Ok(())
          }
        ),
        _ => Self::set_member_type_prop(cast_member_ref, prop, value)
      }
    } else {
      Err(ScriptError::new(format!("Setting prop of invalid castMember reference")))
    };
    if result.is_ok() {
      JsApi::dispatch_cast_member_changed(cast_member_ref.to_owned());
    }
    result
  }
}