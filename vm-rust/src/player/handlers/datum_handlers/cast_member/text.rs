use crate::{
    director::lingo::datum::{
        datum_bool, Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType,
    },
    player::{
        bitmap::{
            bitmap::{Bitmap, BuiltInPalette, PaletteRef},
            drawing::CopyPixelsParams,
        },
        cast_lib::CastMemberRef,
        font::{get_text_index_at_pos, measure_text, DrawTextParams},
        handlers::datum_handlers::{
            cast_member_ref::borrow_member_mut, string_chunk::StringChunkUtils,
        },
        DatumRef, DirPlayer, ScriptError,
    },
};

pub struct TextMemberHandlers {}

impl TextMemberHandlers {
    pub fn call(
        player: &mut DirPlayer,
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let member_ref = player.get_datum(datum).to_member_ref()?;
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(&member_ref)
            .unwrap();
        let text = member.member_type.as_text().unwrap();
        match handler_name.as_str() {
            "count" => {
                let count_of = player.get_datum(&args[0]).string_value()?;
                if args.len() != 1 {
                    return Err(ScriptError::new("count requires 1 argument".to_string()));
                }
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &text.text,
                    StringChunkType::from(&count_of),
                    delimiter,
                )?;
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            "getPropRef" => {
                let prop_name = player.get_datum(&args[0]).string_value()?;
                let start = player.get_datum(&args[1]).int_value()?;
                let end = if args.len() > 2 {
                    player.get_datum(&args[2]).int_value()?
                } else {
                    start
                };
                let chunk_expr = StringChunkType::from(&prop_name);
                let chunk_expr = StringChunkExpr {
                    chunk_type: chunk_expr,
                    start,
                    end,
                    item_delimiter: player.movie.item_delimiter.clone(),
                };
                let resolved_str =
                    StringChunkUtils::resolve_chunk_expr_string(&text.text, &chunk_expr)?;
                Ok(player.alloc_datum(Datum::StringChunk(
                    StringChunkSource::Member(member_ref),
                    chunk_expr,
                    resolved_str,
                )))
            }
            "locToCharPos" => {
                let (x, y) = player.get_datum(&args[0]).to_int_point()?;
                let params = DrawTextParams {
                    font: &player.font_manager.get_system_font().unwrap(),
                    line_height: None,
                    line_spacing: text.fixed_line_space,
                    top_spacing: text.top_spacing,
                };
                let index = get_text_index_at_pos(&text.text, &params, x, y);
                Ok(player.alloc_datum(Datum::Int((index + 1) as i32)))
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for text member type"
            ))),
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
                // TODO: alignment
                let font = player.font_manager.get_system_font().unwrap();
                let (width, height) = measure_text(
                    &text_data.text,
                    &font,
                    None,
                    text_data.fixed_line_space,
                    text_data.top_spacing,
                );
                // TODO use 32 bits
                let mut bitmap = Bitmap::new(
                    width,
                    height,
                    8,
                    8,
                    0,
                    PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
                );
                let font_bitmap = player.bitmap_manager.get_bitmap(font.bitmap_ref).unwrap();
                let palettes = player.movie.cast_manager.palettes();

                let params = CopyPixelsParams {
                    blend: 100,
                    ink: 36,
                    color: bitmap.get_fg_color_ref(),
                    bg_color: bitmap.get_bg_color_ref(),
                    mask_image: None,
                };

                bitmap.draw_text(
                    &text_data.text,
                    &font,
                    font_bitmap,
                    0,
                    text_data.top_spacing as i32,
                    params,
                    &palettes,
                    text_data.fixed_line_space,
                    text_data.top_spacing,
                );

                let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                Ok(Datum::BitmapRef(bitmap_ref))
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
                    cast_member.member_type.as_text_mut().unwrap().text = value?;
                    Ok(())
                },
            ),
            "alignment" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().alignment = value?;
                    Ok(())
                },
            ),
            "wordWrap" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().word_wrap = value?;
                    Ok(())
                },
            ),
            "width" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().width = value? as u16;
                    Ok(())
                },
            ),
            "font" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().font = value?;
                    Ok(())
                },
            ),
            "fontSize" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().font_size = value? as u16;
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
                    cast_member.member_type.as_text_mut().unwrap().font_style = value?;
                    Ok(())
                },
            ),
            "fixedLineSpace" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member
                        .member_type
                        .as_text_mut()
                        .unwrap()
                        .fixed_line_space = value? as u16;
                    Ok(())
                },
            ),
            "topSpacing" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().top_spacing = value? as i16;
                    Ok(())
                },
            ),
            "boxType" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().box_type = value?;
                    Ok(())
                },
            ),
            "antialias" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().anti_alias = value?;
                    Ok(())
                },
            ),
            "rect" => borrow_member_mut(
                member_ref,
                |player| {
                    let rect = value.to_int_rect()?;
                    let rect: (i16, i16, i16, i16) =
                        (rect.1 as i16, rect.0 as i16, rect.3 as i16, rect.2 as i16);
                    Ok(rect)
                },
                |cast_member, value| {
                    let value = value?;
                    let text_data = cast_member.member_type.as_text_mut().unwrap();
                    text_data.width = value.2 as u16;
                    Ok(())
                },
            ),
            _ => Err(ScriptError::new(format!(
                "Cannot set castMember prop {} for text",
                prop
            ))),
        }
    }
}
