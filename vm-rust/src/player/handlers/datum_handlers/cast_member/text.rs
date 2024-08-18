use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType}, player::{
        bitmap::{bitmap::{resolve_color_ref, Bitmap}, manager::BitmapManager, palette_map::PaletteMap}, cast_lib::CastMemberRef, cast_member::TextData, font::{get_text_index_at_pos, measure_text, DrawTextParams, FontManager}, handlers::datum_handlers::{cast_member_ref::borrow_member_mut, string_chunk::StringChunkUtils}, reserve_player_mut, DatumRef, DirPlayer, ScriptError
    }
};

pub struct TextMemberHandlers {}

impl TextMemberHandlers {
    pub fn call(player: &mut DirPlayer, datum: &DatumRef, handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "count" => {
                let count = {
                    let member_ref = player.get_datum(datum).to_member_ref()?;
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref).unwrap();
                    let text = member.member_type.as_text().unwrap();
                    let text = text.text_data.borrow();

                    let count_of = player.get_datum(&args[0]).string_value()?;
                    if args.len() != 1 {
                        return Err(ScriptError::new("count requires 1 argument".to_string()));
                    }
                    let delimiter = &player.movie.item_delimiter;
                    let count = StringChunkUtils::resolve_chunk_count(&text.text, StringChunkType::from(&count_of), delimiter)?;
                    count
                };
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            "getPropRef" => {
                let result = {
                    let member_ref = player.get_datum(datum).to_member_ref()?;
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref).unwrap();
                    let text = member.member_type.as_text().unwrap();
                    let text = text.text_data.borrow();

                    let prop_name = player.get_datum(&args[0]).string_value()?;
                    let start = player.get_datum(&args[1]).int_value()?;
                    let end = if args.len() > 2 { player.get_datum(&args[2]).int_value()? } else { start };
                    let chunk_expr = StringChunkType::from(&prop_name);
                    let chunk_expr = StringChunkExpr {
                        chunk_type: chunk_expr,
                        start,
                        end,
                        item_delimiter: player.movie.item_delimiter.clone(),
                    };
                    let resolved_str = StringChunkUtils::resolve_chunk_expr_string(&text.text, &chunk_expr)?;
                    Datum::StringChunk(StringChunkSource::Member(member_ref), chunk_expr, resolved_str)
                };
                Ok(player.alloc_datum(result))
            }
            "locToCharPos" => {
                let result = {
                    let member_ref = player.get_datum(datum).to_member_ref()?;
                    let member = player.movie.cast_manager.find_member_by_ref(&member_ref).unwrap();
                    let text = member.member_type.as_text().unwrap();
                    let text = text.text_data.borrow();

                    let (x, y) = player.get_datum(&args[0]).to_int_point()?;
                    let params = DrawTextParams {
                        font: player.font_manager.get_system_font().unwrap(),
                        line_height: None,
                        line_spacing: text.fixed_line_space,
                        top_spacing: text.top_spacing,
                    };
                    let index = get_text_index_at_pos(&text.text, &params, x, y);
                    Datum::Int((index + 1) as i32)
                };
                Ok(player.alloc_datum(result))
            }
            _ => Err(ScriptError::new(format!("No handler {handler_name} for text member type")))
          }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let text_data = member.member_type.as_text().unwrap().clone();
        let text_data = text_data.text_data.borrow_mut();
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
            }
            "fixedLineSpace" => Ok(Datum::Int(text_data.fixed_line_space as i32)),
            "topSpacing" => Ok(Datum::Int(text_data.top_spacing as i32)),
            "boxType" => Ok(Datum::Symbol(text_data.box_type.to_owned())),
            "antialias" => Ok(datum_bool(text_data.anti_alias)),
            "rect" => {
                let font = player.font_manager.get_system_font().unwrap();
                let (width, height) = measure_text(
                    &text_data.text,
                    &font,
                    None,
                    text_data.fixed_line_space,
                    text_data.top_spacing,
                );
                Ok(Datum::IntRect((0, 0, width as i32, height as i32)))
            }
            "height" => {
                let font = player.font_manager.get_system_font().unwrap();
                let (_, height) = measure_text(
                    &text_data.text,
                    &font,
                    None,
                    text_data.fixed_line_space,
                    text_data.top_spacing,
                );
                Ok(Datum::Int(height as i32))
            }
            "image" => {
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(cast_member_ref)
                    .unwrap();
                let text_data = member.member_type.as_text().unwrap();
                Ok(Datum::BitmapRef(text_data.image_ref))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for text",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &String,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop.as_str() {
            "text" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.text = value?;
                    Ok(())
                },
            ),
            "alignment" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.alignment = value?;
                    Ok(())
                },
            ),
            "wordWrap" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.word_wrap = value?;
                    Ok(())
                },
            ),
            "width" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.width = value? as u16;
                    Ok(())
                },
            ),
            "font" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.font = value?;
                    Ok(())
                },
            ),
            "fontSize" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.font_size = value? as u16;
                    Ok(())
                },
            ),
            "fontStyle" => borrow_member_mut(
                member_ref,
                |player| {
                    let mut item_strings = Vec::new();
                    for x in value.to_list().unwrap() {
                        item_strings.push(player.get_datum(x).string_value()?);
                    }
                    Ok(item_strings)
                },
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.font_style = value?;
                    Ok(())
                },
            ),
            "fixedLineSpace" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.fixed_line_space = value? as u16;
                    Ok(())
                },
            ),
            "topSpacing" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.top_spacing = value? as i16;
                    Ok(())
                },
            ),
            "boxType" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.box_type = value?;
                    Ok(())
                },
            ),
            "antialias" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.anti_alias = value?;
                    Ok(())
                },
            ),
            "rect" => borrow_member_mut(
                member_ref,
                |player| {
                    let rect = value.to_int_rect()?;
                    let rect: (i16, i16, i16, i16) =
                        (rect.0 as i16, rect.1 as i16, rect.2 as i16, rect.3 as i16);
                    Ok(rect)
                },
                |cast_member, value| {
                    let value = value?;
                    let text_data = cast_member.member_type.as_text().unwrap();
                    let mut text_data = text_data.text_data.borrow_mut();
                    text_data.width = value.2 as u16;
                    Ok(())
                },
            ),
            _ => Err(ScriptError::new(format!(
                "Cannot set castMember prop {} for text",
                prop
            ))),
        }?;
        reserve_player_mut(|player| {
            let cast_member_ref = member_ref;
            TextMemberHandlers::invalidate_bitmap(player, cast_member_ref)?;
            Ok(())
        })
    }

    pub fn invalidate_bitmap(player: &mut DirPlayer, cast_member_ref: &CastMemberRef) -> Result<(), ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let text_member = member.member_type.as_text().unwrap();
        let text_data = text_member.text_data.borrow();
        let width = text_data.width;
        
        let image_ref = text_member.image_ref;
        let bitmap = player.bitmap_manager.get_bitmap(image_ref).unwrap();
        if !bitmap.is_dirty() {
            bitmap.set_dirty(true);
        }

        let font = player.font_manager.get_system_font().unwrap();
        let (_, height) = measure_text(
            &text_data.text,
            &font,
            None,
            text_data.fixed_line_space,
            text_data.top_spacing,
        );
        if width != bitmap.width() || height != bitmap.height() {
            bitmap.set_size(width, height, false);
        }
        Ok(())
    }

    pub fn render_to_bitmap(
        palettes: &PaletteMap,
        font_manager: &FontManager,
        bitmap_manager: &BitmapManager,
        text_data: &TextData,
        bitmap: &Bitmap,
    ) -> Result<(), ScriptError> {
        let font = font_manager.get_system_font().unwrap();
        let (width, height) = measure_text(
            &text_data.text,
            &font,
            None,
            text_data.fixed_line_space,
            text_data.top_spacing,
        );
        let draw_x = match text_data.alignment.as_str() {
            "left" => 0,
            "center" => (text_data.width as i32 - width as i32) / 2,
            "right" => text_data.width as i32 - width as i32,
            _ => 0,
        };
        // TODO use 32 bits
        if bitmap.buffer_width() != text_data.width || bitmap.buffer_height() != height {
            bitmap.set_size(text_data.width, height, true);
        } else {
            let bg_color = bitmap.get_bg_color_ref();
            let bg_color = resolve_color_ref(&palettes, &bg_color, &bitmap.palette_ref);
            bitmap.clear_rect(0, 0, text_data.width as i32, height as i32, bg_color, &palettes);
        }

        let font_bitmap = bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();

        let ink = 0;//36;
        bitmap.draw_text(
            &text_data.text,
            font,
            font_bitmap,
            draw_x,
            text_data.top_spacing as i32,
            ink,
            bitmap.get_bg_color_ref(), // TODO use chunk color
            &palettes,
            text_data.fixed_line_space,
            text_data.top_spacing,
        );
        Ok(())
    }
}
