use itertools::Itertools;

use crate::{
    director::lingo::datum::{Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType, datum_bool},
    player::{
        ColorRef, DatumRef, DirPlayer, ScriptError, bitmap::{bitmap::{Bitmap, BuiltInPalette, PaletteRef}, drawing::CopyPixelsParams}, cast_lib::CastMemberRef, cast_member::Media, font::{BitmapFont, measure_text, measure_text_wrapped}, handlers::datum_handlers::{
            cast_member_ref::borrow_member_mut, string::{string_get_lines, string_get_words}, string_chunk::StringChunkUtils
        }
    },
};

pub struct FieldMemberHandlers {}

impl FieldMemberHandlers {
    pub fn call(
        player: &mut DirPlayer,
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "count" => {
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let field = member.member_type.as_field().unwrap();
                let count_of = player.get_datum(&args[0]).string_value()?;
                if args.len() != 1 {
                    return Err(ScriptError::new("count requires 1 argument".to_string()));
                }
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text,
                    StringChunkType::from(&count_of),
                    delimiter,
                )?;
                Ok(player.alloc_datum(Datum::Int(count as i32)))
            }
            "getPropRef" => {
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .unwrap();
                let field = member.member_type.as_field().unwrap();
                let prop_name = player.get_datum(&args[0]).string_value()?;
                let start = player.get_datum(&args[1]).int_value()?;
                let end = if args.len() > 2 {
                    player.get_datum(&args[2]).int_value()?
                } else {
                    start
                };
                let chunk_type = StringChunkType::from(&prop_name);
                let chunk_expr = StringChunkExpr {
                    chunk_type,
                    start,
                    end,
                    item_delimiter: player.movie.item_delimiter,
                };
                let resolved_str =
                    StringChunkUtils::resolve_chunk_expr_string(&field.text, &chunk_expr)?;
                Ok(player.alloc_datum(Datum::StringChunk(
                    StringChunkSource::Member(member_ref),
                    chunk_expr,
                    resolved_str,
                )))
            }
            "setContents" => {
                if args.len() != 1 {
                    return Err(ScriptError::new(
                        "setContents requires 1 argument".to_string(),
                    ));
                }
                let new_contents = player.get_datum(&args[0]).string_value()?;
                let member_ref = player.get_datum(datum).to_member_ref()?;
                let member = player
                    .movie
                    .cast_manager
                    .find_mut_member_by_ref(&member_ref)
                    .unwrap()
                    .member_type
                    .as_field_mut()
                    .unwrap();
                member.text = new_contents.trim_end_matches('\0').to_string();
                Ok(DatumRef::Void)
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for field member type"
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
        let field = member.member_type.as_field().unwrap();

        match prop.as_str() {
            "text" => Ok(Datum::String(field.text.to_owned())),
            "font" => Ok(Datum::String(field.font.to_owned())),
            "fontSize" => Ok(Datum::Int(field.font_size as i32)),
            "fontStyle" => Ok(Datum::String(field.font_style.to_owned())),
            "width" => Ok(Datum::Int(field.width as i32)),
            "alignment" => Ok(Datum::String(field.alignment.to_owned())),
            "wordWrap" => Ok(datum_bool(field.word_wrap)),
            "fixedLineSpace" | "lineHeight" => Ok(Datum::Int(field.fixed_line_space as i32)),
            "topSpacing" => Ok(Datum::Int(field.top_spacing as i32)),
            "boxType" => Ok(Datum::String(field.box_type.to_owned())),
            "antialias" => Ok(datum_bool(field.anti_alias)),
            "autoTab" => Ok(datum_bool(field.auto_tab)),
            "editable" => Ok(datum_bool(field.editable)),
            "border" => Ok(Datum::Int(field.border as i32)),
            "margin" => Ok(Datum::Int(field.margin as i32)),
            "boxDropShadow" => Ok(Datum::Int(field.box_drop_shadow as i32)),
            "dropShadow" => Ok(Datum::Int(field.drop_shadow as i32)),
            "scrollTop" => Ok(Datum::Int(field.scroll_top as i32)),
            "hilite" => Ok(datum_bool(field.hilite)),
            "lineCount" => {
                if field.text.is_empty() {
                    Ok(Datum::Int(0))
                } else {
                    Ok(Datum::Int(field.text.lines().count().max(1) as i32))
                }
            }
            "line" => {
                let lines = string_get_lines(&field.text);
                let line_datums = lines.into_iter().map(Datum::String).map(|d| player.alloc_datum(d)).collect_vec();
                Ok(Datum::List(DatumType::List, line_datums, false))
            }
            "word" => {
                let words = string_get_words(&field.text);
                let word_datums = words.into_iter().map(Datum::String).map(|d| player.alloc_datum(d)).collect_vec();
                Ok(Datum::List(DatumType::List, word_datums, false))
            }
            "pageHeight" => {
                // pageHeight = total height of the text content (including word wrap)
                let text_clone = field.text.clone();
                let font_name = field.font.clone();
                let font_size = Some(field.font_size);
                let fixed_line_space = field.fixed_line_space;
                let top_spacing = field.top_spacing;
                let word_wrap = field.word_wrap;
                let field_width = field.width;

                let font = if !font_name.is_empty() {
                    player.font_manager.get_font_with_cast_and_bitmap(
                        &font_name,
                        &player.movie.cast_manager,
                        &mut player.bitmap_manager,
                        font_size,
                        None,
                    )
                } else {
                    None
                };
                let font = font.or_else(|| player.font_manager.get_system_font())
                    .ok_or_else(|| ScriptError::new("System font not available".to_string()))?;

                let (_, measured_h) = if word_wrap && field_width > 0 {
                    measure_text_wrapped(&text_clone, &font, field_width, true, fixed_line_space, top_spacing, 0)
                } else {
                    measure_text(&text_clone, &font, None, fixed_line_space, top_spacing, 0)
                };
                Ok(Datum::Int(measured_h as i32))
            }
            "foreColor" => {
                match &field.fore_color {
                    Some(ColorRef::Rgb(r, _, _)) => Ok(Datum::Int(*r as i32)),
                    Some(ColorRef::PaletteIndex(idx)) => Ok(Datum::Int(*idx as i32)),
                    None => Ok(Datum::Int(255)), // default: palette index 255 = black
                }
            }
            "backColor" => {
                match &field.back_color {
                    Some(ColorRef::Rgb(r, _, _)) => Ok(Datum::Int(*r as i32)),
                    Some(ColorRef::PaletteIndex(idx)) => Ok(Datum::Int(*idx as i32)),
                    None => Ok(Datum::Int(0)), // default: palette index 0 = white
                }
            }
            "rect" | "height" | "picture" => {
                // Clone data to avoid borrow issues
                let text_clone = field.text.clone();
                let font_name = field.font.clone();
                let font_size = Some(field.font_size);
                let fixed_line_space = field.fixed_line_space;
                let top_spacing = field.top_spacing;
                let alignment = field.alignment.clone();
                let word_wrap = field.word_wrap;
                let fore_color = field.fore_color.clone().unwrap_or(ColorRef::PaletteIndex(255));
                let field_width = field.width;

                // Try to get custom font, fall back to system font
                let font = if !font_name.is_empty() {
                    player.font_manager.get_font_with_cast_and_bitmap(
                        &font_name,
                        &player.movie.cast_manager,
                        &mut player.bitmap_manager,
                        font_size,
                        None,
                    )
                } else {
                    None
                };

                let font = if let Some(f) = font {
                    f
                } else {
                    player
                        .font_manager
                        .get_system_font()
                        .ok_or_else(|| ScriptError::new("System font not available".to_string()))?
                };

                let (measured_w, measured_h) =
                    measure_text(&text_clone, &font, None, fixed_line_space, top_spacing, 0);
                let width = if field_width > 0 {
                    field_width.max(measured_w)
                } else {
                    measured_w
                };
                let height = if fixed_line_space > 0 {
                    fixed_line_space.max(measured_h)
                } else {
                    measured_h
                };

                match prop.as_str() {
                    "rect" => Ok(Datum::Rect([
                        player.alloc_datum(Datum::Int(0)),
                        player.alloc_datum(Datum::Int(0)),
                        player.alloc_datum(Datum::Int(width as i32)),
                        player.alloc_datum(Datum::Int(height as i32))
                    ])),
                    "height" => Ok(Datum::Int(height as i32)),
                    "picture" => {
                        let mut bitmap = Bitmap::new(
                            width,
                            height,
                            32,
                            8,
                            0,
                            PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
                        );

                        // Fill with transparent background
                        for y in 0..height {
                            for x in 0..width {
                                let index =
                                    ((y as usize * width as usize + x as usize) * 4) as usize;
                                if index + 3 < bitmap.data.len() {
                                    bitmap.data[index] = 0;
                                    bitmap.data[index + 1] = 0;
                                    bitmap.data[index + 2] = 0;
                                    bitmap.data[index + 3] = 0;
                                }
                            }
                        }

                        let font_bitmap = player
                            .bitmap_manager
                            .get_bitmap(font.bitmap_ref)
                            .ok_or_else(|| ScriptError::new("Font bitmap not found".to_string()))?;
                        let palettes = player.movie.cast_manager.palettes();

                        let params = CopyPixelsParams {
                            blend: 100,
                            ink: 36,
                            color: fore_color,
                            bg_color: ColorRef::Rgb(255, 255, 255),
                            mask_image: None,
                            is_text_rendering: true,
                            rotation: 0.0,
                            skew: 0.0,
                            sprite: None,
                            original_dst_rect: None,
                        };

                        bitmap.draw_text(
                            &text_clone,
                            &font,
                            &font_bitmap,
                            0,
                            top_spacing as i32,
                            params,
                            &palettes,
                            fixed_line_space,
                            top_spacing,
                        );

                        let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                        Ok(Datum::BitmapRef(bitmap_ref))
                    }
                    _ => unreachable!(),
                }
            }
            "media" => Ok(Datum::Media(Media::Field(field.clone()))),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for field",
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
                    cast_member.member_type.as_field_mut().unwrap().text = value?.trim_end_matches('\0').to_string();
                    Ok(())
                },
            ),
            "rect" => borrow_member_mut(
                member_ref,
                |player| -> Result<(i32, i32, i32, i32), ScriptError> {
                    let rect = value.to_rect()?;

                    let x1 = player.get_datum(&rect[0]).int_value()?;
                    let y1 = player.get_datum(&rect[1]).int_value()?;
                    let x2 = player.get_datum(&rect[2]).int_value()?;
                    let y2 = player.get_datum(&rect[3]).int_value()?;

                    Ok((x1, y1, x2, y2))
                },
                |cast_member, rect_values: Result<(i32, i32, i32, i32), ScriptError>| {
                    let (x1, y1, x2, y2) = rect_values?;
                    let field_data = cast_member.member_type.as_field_mut().unwrap();
                    let w = (x2 - x1).max(0) as u16;
                    let h = (y2 - y1).max(0) as u16;
                    if w > 0 {
                        field_data.width = w;
                    }
                    if h > 0 {
                        field_data.fixed_line_space = h;
                    }

                    Ok(())
                }
            ),
            "alignment" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().alignment = value?;
                    Ok(())
                },
            ),
            "wordWrap" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().word_wrap = value?;
                    Ok(())
                },
            ),
            "width" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().width = value? as u16;
                    Ok(())
                },
            ),
            "height" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().fixed_line_space = value? as u16;
                    Ok(())
                },
            ),
            "font" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().font = value?;
                    Ok(())
                },
            ),
            "fontSize" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let font_size = value? as u16;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    field.font_size = font_size;
                    Ok(())
                },
            ),
            "fontStyle" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().font_style = value?;
                    Ok(())
                },
            ),
            "fixedLineSpace" | "lineHeight" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member
                        .member_type
                        .as_field_mut()
                        .unwrap()
                        .fixed_line_space = value? as u16;
                    Ok(())
                },
            ),
            "topSpacing" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().top_spacing = value? as i16;
                    Ok(())
                },
            ),
            "boxType" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().box_type = value?;
                    Ok(())
                },
            ),
            "antialias" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().anti_alias = value?;
                    Ok(())
                },
            ),
            "autoTab" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().auto_tab = value?;
                    Ok(())
                },
            ),
            "editable" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().editable = value?;
                    Ok(())
                },
            ),
            "border" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().border = value? as u16;
                    Ok(())
                },
            ),
            "margin" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().margin = value? as u16;
                    Ok(())
                },
            ),
            "boxDropShadow" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().box_drop_shadow = value? as u16;
                    Ok(())
                },
            ),
            "dropShadow" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().drop_shadow = value? as u16;
                    Ok(())
                },
            ),
            "scrollTop" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().scroll_top = value? as u16;
                    Ok(())
                },
            ),
            "hilite" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().hilite = value?;
                    Ok(())
                },
            ),
            "foreColor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let v = value? as u8;
                    cast_member.member_type.as_field_mut().unwrap().fore_color = Some(ColorRef::PaletteIndex(v));
                    Ok(())
                },
            ),
            "backColor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let v = value? as u8;
                    cast_member.member_type.as_field_mut().unwrap().back_color = Some(ColorRef::PaletteIndex(v));
                    Ok(())
                },
            ),
            "media" => borrow_member_mut(
                member_ref,
                |player| value.media_value(),
                |cast_member, value| {
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    match value? {
                        Media::Field(new_field) => field.clone_from(&new_field),
                        _ => return Err(ScriptError::new("Invalid media value for field".to_string())),
                    };
                    Ok(())
                },
            ),
            _ => Err(ScriptError::new(format!(
                "Cannot set castMember prop {} for field",
                prop
            ))),
        }
    }
}
