use log::{warn, debug};

use crate::{
    director::{
        enums::ShapeType,
        lingo::datum::{datum_bool, Datum, DatumType},
    },
    js_api::JsApi,
    player::{
        cast_lib::CastMemberRef,
        cast_member::{BitmapMember, CastMember, CastMemberType, CastMemberTypeId, TextMember},
        handlers::types::TypeUtils,
        reserve_player_mut, reserve_player_ref, DatumRef, DirPlayer, ScriptError,
        sprite::ColorRef,
    },
};

use super::cast_member::{
    bitmap::BitmapMemberHandlers, button::ButtonMemberHandlers, field::FieldMemberHandlers,
    film_loop::FilmLoopMemberHandlers, font::FontMemberHandlers, sound::SoundMemberHandlers,
    text::TextMemberHandlers, palette::PaletteMemberHandlers,
};

pub struct CastMemberRefHandlers {}

pub fn borrow_member_mut<T1, F1, T2, F2>(member_ref: &CastMemberRef, player_f: F2, f: F1) -> T1
where
    F1: FnOnce(&mut CastMember, T2) -> T1,
    F2: FnOnce(&mut DirPlayer) -> T2,
{
    reserve_player_mut(|player| {
        let arg = player_f(player);
        let member = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(&member_ref)
            .unwrap();
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

    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "duplicate" => Self::duplicate(datum, args),
            "erase" => Self::erase(datum, args),
            "charPosToLoc" => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call charPosToLoc on non-cast-member".to_string(),
                            ))
                        }
                    };
                    let cast_member = player
                        .movie
                        .cast_manager
                        .find_member_by_ref(&cast_member_ref)
                        .unwrap();
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let char_pos = player.get_datum(&args[0]).int_value()? as u16;
                    let char_width: i32 = 7;
                    let line_height: i32 = get_text_member_line_height(&text_data) as i32;

                    let (x, y) = if text_data.text.is_empty() || char_pos <= 0 {
                        (0, 0)
                    } else if char_pos > text_data.text.len() as u16 {
                        (char_width * text_data.text.len() as i32, line_height)
                    } else {
                        (char_width * (char_pos - 1) as i32, line_height)
                    };

                    let x_ref = player.alloc_datum(Datum::Int(x));
                    let y_ref = player.alloc_datum(Datum::Int(y));
                    Ok(player.alloc_datum(Datum::Point([x_ref, y_ref])))
                })
            }
            "getProp" => {
                let result_ref = reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call getProp on non-cast-member".to_string(),
                            ))
                        }
                    };
                    let prop = player.get_datum(&args[0]).string_value()?;
                    let result = Self::get_prop(player, &cast_member_ref, &prop)?;
                    Ok(player.alloc_datum(result))
                })?;
                if args.len() > 1 {
                    reserve_player_mut(|player| {
                        TypeUtils::get_sub_prop(&result_ref, &args[1], player)
                    })
                } else {
                    Ok(result_ref)
                }
            }
            "count" => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                        _ => {
                            return Err(ScriptError::new(
                                "Cannot call count on non-cast-member".to_string(),
                            ))
                        }
                    };
                    
                    if args.is_empty() {
                        return Err(ScriptError::new("count requires 1 argument".to_string()));
                    }
                    
                    let count_of = player.get_datum(&args[0]).string_value()?;
                    
                    // Try to get the member's text
                    // First try "text" property, then fallback to "previewText" for Font members
                    let text = match Self::get_prop(player, &cast_member_ref, &"text".to_string()) {
                        Ok(datum) => datum.string_value()?,
                        Err(_) => {
                            // Try previewText for Font members
                            match Self::get_prop(player, &cast_member_ref, &"previewText".to_string()) {
                                Ok(datum) => datum.string_value()?,
                                Err(_) => {
                                    return Err(ScriptError::new(format!(
                                        "Member type does not support count operation"
                                    )));
                                }
                            }
                        }
                    };
                    
                    let delimiter = player.movie.item_delimiter;
                    let count = crate::player::handlers::datum_handlers::string_chunk::StringChunkUtils::resolve_chunk_count(
                        &text,
                        crate::director::lingo::datum::StringChunkType::from(&count_of),
                        delimiter,
                    )?;
                    Ok(player.alloc_datum(Datum::Int(count as i32)))
                })
            }
            _ => Self::call_member_type(datum, handler_name, args),
        }
    }

    fn call_member_type(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot call_member_type on non-cast-member".to_string(),
                    ))
                }
            };
            let cast_member = player
                .movie
                .cast_manager
                .find_member_by_ref(&member_ref)
                .unwrap();
            match &cast_member.member_type {
                CastMemberType::Field(_) => {
                    FieldMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::Text(_) => {
                    TextMemberHandlers::call(player, datum, handler_name, args)
                }
                CastMemberType::Button(_) => {
                    ButtonMemberHandlers::call(player, datum, handler_name, args)
                }
                _ => Err(ScriptError::new(format!(
                    "No handler {handler_name} for member type"
                ))),
            }
        })
    }

    fn erase(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => return Err(ScriptError::new("Cannot erase non-cast-member".to_string())),
            };
            player
                .movie
                .cast_manager
                .remove_member_with_ref(&cast_member_ref)?;
            Ok(DatumRef::Void)
        })
    }

    fn duplicate(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let cast_member_ref = match player.get_datum(datum) {
                Datum::CastMember(cast_member_ref) => cast_member_ref.to_owned(),
                _ => {
                    return Err(ScriptError::new(
                        "Cannot duplicate non-cast-member".to_string(),
                    ))
                }
            };
            let dest_slot_number = args.get(0).map(|x| player.get_datum(x).int_value());

            if dest_slot_number.is_none() {
                return Err(ScriptError::new(
                    "Cannot duplicate cast member without destination slot number".to_string(),
                ));
            }
            let dest_slot_number = dest_slot_number.unwrap()?;
            let dest_ref = Self::member_ref_from_slot_number(dest_slot_number as u32);

            let mut new_member = {
                let src_member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&cast_member_ref);
                if src_member.is_none() {
                    return Err(ScriptError::new(
                        "Cannot duplicate non-existent cast member reference".to_string(),
                    ));
                }
                src_member.unwrap().clone()
            };
            new_member.number = dest_ref.cast_member as u32;

            let dest_cast = player
                .movie
                .cast_manager
                .get_cast_mut(dest_ref.cast_lib as u32);
            dest_cast.insert_member(dest_ref.cast_member as u32, new_member);

            Ok(player.alloc_datum(Datum::Int(dest_slot_number)))
        })
    }

    fn get_invalid_member_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        match prop.as_str() {
            "name" => Ok(Datum::String("".to_string())),
            "number" => Ok(Datum::Int(-1)),
            "type" => Ok(Datum::String("empty".to_string())),
            "castLibNum" => Ok(Datum::Int(-1)),
            "memberNum" => Ok(Datum::Int(-1)),
            "width" | "height" | "rect" | "duration" => Ok(Datum::Void),
            "image" => Ok(Datum::Void),
            "regPoint" => Ok(Datum::Point([
                player.alloc_datum(Datum::Int(0)),
                player.alloc_datum(Datum::Int(0)),
            ])),
            _ => Err(ScriptError::new(format!(
                "Cannot get prop {} of invalid cast member ({}, {})",
                prop, member_ref.cast_lib, member_ref.cast_member
            ))),
        }
    }

    fn get_member_type_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        member_type: &CastMemberTypeId,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        debug!("Getting prop '{}' for member type {:?}", prop, member_type);
        match &member_type {
            CastMemberTypeId::Bitmap => {
                BitmapMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Field => FieldMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Text => TextMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Button => ButtonMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::FilmLoop => {
                FilmLoopMemberHandlers::get_prop(player, cast_member_ref, prop)
            }
            CastMemberTypeId::Sound => SoundMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Font => FontMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Palette => PaletteMemberHandlers::get_prop(player, cast_member_ref, prop),
            CastMemberTypeId::Script => {
                if prop == "text" {
                    let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                        .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

                    if let CastMemberType::Script(script_data) = &cast_member.member_type {
                        // Scripts in Director typically don't have editable text
                        // This member might be misclassified
                        web_sys::console::log_1(&format!("⚠️ Trying to get .text from Script member #{}", cast_member.number).into());

                        // Return empty string for now, but this suggests the member type is wrong
                        Ok(Datum::String("".to_string()))
                    } else {
                        Err(ScriptError::new("Script member has no text".to_string()))
                    }
                } else if prop == "script" {
                    Ok(Datum::ScriptRef(cast_member_ref.clone()))
                } else {
                    Err(ScriptError::new(format!("Script members don't support property {}", prop)))
                }
            }
            CastMemberTypeId::Shape => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

                if let CastMemberType::Shape(shape_member) = &cast_member.member_type {
                    let info = &shape_member.shape_info;
                    match prop.as_str() {
                        "rect" => {
                            let width = info.width() as i32;
                            let height = info.height() as i32;
                            Ok(Datum::Rect([
                                player.alloc_datum(Datum::Int(0)),
                                player.alloc_datum(Datum::Int(0)),
                                player.alloc_datum(Datum::Int(width)),
                                player.alloc_datum(Datum::Int(height)),
                            ]))
                        }
                        "width" => Ok(Datum::Int(info.width() as i32)),
                        "height" => Ok(Datum::Int(info.height() as i32)),
                        "shapeType" => {
                            let symbol = match info.shape_type {
                                ShapeType::Rect => "rect",
                                ShapeType::OvalRect => "roundRect",
                                ShapeType::Oval => "oval",
                                ShapeType::Line => "line",
                                ShapeType::Unknown => "rect",
                            };
                            Ok(Datum::Symbol(symbol.to_string()))
                        }
                        "filled" => Ok(datum_bool(info.fill_type != 0)),
                        "lineSize" => Ok(Datum::Int(info.line_thickness as i32)),
                        "pattern" => Ok(Datum::Int(info.pattern as i32)),
                        "foreColor" => Ok(Datum::Int(info.fore_color as i32)),
                        "backColor" => Ok(Datum::Int(info.back_color as i32)),
                        _ => Err(ScriptError::new(format!(
                            "Shape members don't support property {}", prop
                        ))),
                    }
                } else {
                    Err(ScriptError::new("Expected shape member".to_string()))
                }
            }
            CastMemberTypeId::VectorShape => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

                if let CastMemberType::VectorShape(vs) = &cast_member.member_type {
                    // Extract data we need before dropping the borrow on player
                    let result: Result<Datum, ScriptError> = match prop.as_str() {
                        "width" => Ok(Datum::Int(vs.width().ceil() as i32)),
                        "height" => Ok(Datum::Int(vs.height().ceil() as i32)),
                        "strokeColor" => {
                            let (r, g, b) = vs.stroke_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        "strokeWidth" => Ok(Datum::Float(vs.stroke_width as f64)),
                        "closed" => Ok(datum_bool(vs.closed)),
                        "fillMode" => {
                            let sym = match vs.fill_mode {
                                0 => "none",
                                1 => "solid",
                                2 => "gradient",
                                _ => "none",
                            };
                            Ok(Datum::Symbol(sym.to_string()))
                        }
                        "fillColor" => {
                            let (r, g, b) = vs.fill_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        "backgroundColor" => {
                            let (r, g, b) = vs.bg_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        "endColor" => {
                            let (r, g, b) = vs.end_color;
                            Ok(Datum::ColorRef(ColorRef::Rgb(r, g, b)))
                        }
                        _ => Err(ScriptError::new(format!(
                            "VectorShape members don't support property {}", prop
                        ))),
                    };
                    // Handle props that need alloc_datum separately (to avoid borrow conflict)
                    if prop == "rect" {
                        let w = vs.width().ceil() as i32;
                        let h = vs.height().ceil() as i32;
                        drop(cast_member);
                        Ok(Datum::Rect([
                            player.alloc_datum(Datum::Int(0)),
                            player.alloc_datum(Datum::Int(0)),
                            player.alloc_datum(Datum::Int(w)),
                            player.alloc_datum(Datum::Int(h)),
                        ]))
                    } else if prop == "vertexList" {
                        let vert_data: Vec<(i32, i32, i32, i32, i32, i32)> = vs.vertices.iter()
                            .map(|v| (
                                v.x as i32, v.y as i32,
                                v.handle1_x as i32, v.handle1_y as i32,
                                v.handle2_x as i32, v.handle2_y as i32,
                            ))
                            .collect();
                        drop(cast_member);
                        let list: Vec<DatumRef> = vert_data.iter().map(|(vx, vy, h1x, h1y, h2x, h2y)| {
                            let vertex_key = player.alloc_datum(Datum::Symbol("vertex".to_string()));
                            let vx_ref = player.alloc_datum(Datum::Int(*vx));
                            let vy_ref = player.alloc_datum(Datum::Int(*vy));
                            let vertex_val = player.alloc_datum(Datum::Point([vx_ref, vy_ref]));

                            let h1_key = player.alloc_datum(Datum::Symbol("handle1".to_string()));
                            let h1x_ref = player.alloc_datum(Datum::Int(*h1x));
                            let h1y_ref = player.alloc_datum(Datum::Int(*h1y));
                            let h1_val = player.alloc_datum(Datum::Point([h1x_ref, h1y_ref]));

                            let h2_key = player.alloc_datum(Datum::Symbol("handle2".to_string()));
                            let h2x_ref = player.alloc_datum(Datum::Int(*h2x));
                            let h2y_ref = player.alloc_datum(Datum::Int(*h2y));
                            let h2_val = player.alloc_datum(Datum::Point([h2x_ref, h2y_ref]));

                            let prop_list = Datum::PropList(vec![
                                (vertex_key, vertex_val),
                                (h1_key, h1_val),
                                (h2_key, h2_val),
                            ], false);
                            player.alloc_datum(prop_list)
                        }).collect();
                        Ok(Datum::List(DatumType::List, list, false))
                    } else {
                        result
                    }
                } else {
                    Err(ScriptError::new("Expected vectorShape member".to_string()))
                }
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember prop {} for member of type {:?}",
                prop, member_type
            ))),
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
                Some(cast_member) => Ok(Some(cast_member.member_type.member_type_id())),
                None => {
                    // Silently ignore setting props on erased members
                    web_sys::console::warn_1(&format!(
                        "Ignoring set prop {} on erased member {} of castLib {}",
                        prop, member_ref.cast_member, member_ref.cast_lib
                    ).into());
                    Ok(None)
                }
            }
        })?;

        let member_type = match member_type {
            Some(t) => t,
            None => return Ok(()), // Member was erased, silently ignore
        };

        match member_type {
            CastMemberTypeId::Field => FieldMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Text => TextMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Button => ButtonMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Font => reserve_player_mut(|player| {
                FontMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            CastMemberTypeId::Bitmap => BitmapMemberHandlers::set_prop(member_ref, prop, value),
            CastMemberTypeId::Palette => reserve_player_mut(|player| {
                PaletteMemberHandlers::set_prop(player, member_ref, prop, value)
            }),
            _ => {
                // Check if this is a bitmap-specific property being set on a non-bitmap
                if prop == "image"
                    || prop == "regPoint"
                    || prop == "paletteRef"
                    || prop == "palette"
                {
                    // Director allows setting bitmap properties on non-bitmap members
                    // by implicitly converting them to bitmap members
                    reserve_player_mut(|player| {
                        let cast_member = player
                            .movie
                            .cast_manager
                            .find_mut_member_by_ref(member_ref)
                            .unwrap();

                        // If not already a bitmap, convert it
                        if cast_member.member_type.as_bitmap().is_none() {
                            // Create a new empty/default bitmap member
                            let new_bitmap = BitmapMember::default();

                            // Replace the member type
                            cast_member.member_type = CastMemberType::Bitmap(new_bitmap);
                        }

                        Ok(())
                    })?;
                    // Now try setting the property again
                    BitmapMemberHandlers::set_prop(member_ref, prop, value)
                } else {
                    Err(ScriptError::new(format!(
                        "Cannot set castMember prop {} for member of type {:?}",
                        prop, member_type
                    )))
                }
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
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref);
        let (name, slot_number, member_type, color, bg_color, member_num) = match cast_member {
            Some(cast_member) => {
                let name = cast_member.name.to_owned();
                let slot_number = Self::get_cast_slot_number(
                    cast_member_ref.cast_lib as u32,
                    cast_member_ref.cast_member as u32,
                ) as i32;
                let member_type = cast_member.member_type.member_type_id();
                let member_num = cast_member.number;
                let color = cast_member.color.to_owned();
                let bg_color = cast_member.bg_color.to_owned();
                (name, slot_number, member_type, color, bg_color, member_num)
            }
            None => {
                warn!(
                    "Getting prop {} of non-existent castMember reference {}, {}",
                    prop, cast_member_ref.cast_lib, cast_member_ref.cast_member
                );
                return Self::get_invalid_member_prop(player, cast_member_ref, prop);
            }
        };

        match prop.as_str() {
            "name" => Ok(Datum::String(name)),
            "memberNum" => Ok(Datum::Int(member_num as i32)),
            "number" => {
                if player.movie.dir_version >= 600 {
                    Ok(Datum::Int(slot_number))
                } else {
                    Ok(Datum::Int(member_num as i32))
                }
            }
            "type" => Ok(Datum::Symbol(member_type.symbol_string()?.to_string())),
            "castLibNum" => Ok(Datum::Int(cast_member_ref.cast_lib as i32)),
            "color" => Ok(Datum::ColorRef(color)),
            "bgColor" => Ok(Datum::ColorRef(bg_color)),
            "mediaReady" => Ok(Datum::Int(1)),
            // In Director, member.member returns the member reference itself
            "member" => Ok(Datum::CastMember(cast_member_ref.clone())),
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
            return Err(ScriptError::new(format!(
                "Setting prop {} of invalid castMember reference (member {} of castLib {})",
                prop, cast_member_ref.cast_member, cast_member_ref.cast_lib
            )));
        }
        let exists = reserve_player_ref(|player| {
            player
                .movie
                .cast_manager
                .find_member_by_ref(cast_member_ref)
                .is_some()
        });
        let result = if exists {
            match prop.as_str() {
                "name" => borrow_member_mut(
                    cast_member_ref,
                    |_player| value.string_value(),
                    |cast_member, value| {
                        cast_member.name = value?;
                        Ok(())
                    },
                ),
                "color" => borrow_member_mut(
                    cast_member_ref,
                    |_| {},
                    |cast_member, _| {
                        cast_member.color = value.to_color_ref()?.to_owned();
                        Ok(())
                    },
                ),
                "bgColor" => borrow_member_mut(
                    cast_member_ref,
                    |_| {},
                    |cast_member, _| {
                        cast_member.bg_color = value.to_color_ref()?.to_owned();
                        Ok(())
                    },
                ),
                _ => Self::set_member_type_prop(cast_member_ref, prop, value),
            }
        } else {
            // Silently ignore setting props on non-existent members
            // This can happen when a script erases a member but still holds a reference
            // Director silently ignores this case
            web_sys::console::warn_1(&format!(
                "Ignoring set prop {} on erased member {} of castLib {}",
                prop, cast_member_ref.cast_member, cast_member_ref.cast_lib
            ).into());
            Ok(())
        };
        if result.is_ok() {
            JsApi::dispatch_cast_member_changed(cast_member_ref.to_owned());
        }
        result
    }
}
