use std::collections::VecDeque;
use itertools::Itertools;

use crate::{
    director::lingo::datum::{Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType, datum_bool},
    player::{
        ColorRef, DatumRef, DirPlayer, ScriptError, bitmap::{bitmap::{Bitmap, BuiltInPalette, PaletteRef}, drawing::CopyPixelsParams}, cast_lib::CastMemberRef, cast_member::Media, font::{measure_text, measure_text_wrapped}, handlers::datum_handlers::{
            cast_member_ref::borrow_member_mut, string::{string_get_lines, string_get_words}, string_chunk::StringChunkUtils
        }
    },
};

pub struct FieldMemberHandlers {}

impl FieldMemberHandlers {
    pub fn call(
        player: &mut DirPlayer,
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
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
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let field = member.member_type.as_field().unwrap();

        match prop {
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
                let line_datums: VecDeque<_> = lines.into_iter().map(Datum::String).map(|d| player.alloc_datum(d)).collect();
                Ok(Datum::List(DatumType::List, line_datums, false))
            }
            "word" => {
                let words = string_get_words(&field.text);
                let word_datums: VecDeque<_> = words.into_iter().map(Datum::String).map(|d| player.alloc_datum(d)).collect();
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
                    measure_text_wrapped(&text_clone, &font, field_width, true, fixed_line_space, top_spacing, 0, 0)
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

                // Use wrap-aware measurement when word_wrap is on so the
                // rect/height getter reports the wrapped layout — Director's
                // `the rect of member` for a wrapped field returns the
                // authored width and the height of the wrapped text. Without
                // this, our rect/height returned `measure_text(...)` which
                // measures unwrapped (single-line) — a long-line field
                // reported a too-wide rect (right = unwrapped line width
                // instead of field.width) and a too-short height (line
                // count of unwrapped vs wrapped). `pageHeight` already does
                // the wrap-aware path.
                let (measured_w, measured_h) = if word_wrap && field_width > 0 {
                    measure_text_wrapped(
                        &text_clone, &font, field_width, true,
                        fixed_line_space, top_spacing, 0, 0,
                    )
                } else {
                    measure_text(&text_clone, &font, None, fixed_line_space, top_spacing, 0)
                };
                let width = if field_width > 0 {
                    // Clamp to authored field.width when set — a wrapped
                    // field's display width is the authored width, not the
                    // measured (which may be the longest unwrapped line).
                    if word_wrap { field_width } else { field_width.max(measured_w) }
                } else {
                    measured_w
                };
                let height = if fixed_line_space > 0 {
                    fixed_line_space.max(measured_h)
                } else {
                    measured_h
                };

                match prop {
                    "rect" => Ok(Datum::Rect([0.0, 0.0, width as f64, height as f64], 0)),
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
                            mask_offset: (0, 0),
                            original_dst_rect: None,
                            bg_color_explicit: false,
                            fore_color_explicit: false,
                            ink9_mask_bitmap: None, ink9_mask_offset: (0, 0),
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

                        // Field `.image` returns a freshly rasterized snapshot
                        // of the text. Each call rasterizes again, so the
                        // bitmap isn't anchored — let the DatumRef refcount
                        // free it when the script drops the value.
                        let bitmap_ref = player.bitmap_manager.add_ephemeral_bitmap(bitmap);
                        Ok(Datum::BitmapRef(bitmap_ref))
                    }
                    _ => unreachable!(),
                }
            }
            "media" => Ok(Datum::Media(Media::Field(field.clone()))),
            // Chunk count shortcuts — computed from text string.
            "charCount" => {
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text, StringChunkType::Char, delimiter,
                )?;
                Ok(Datum::Int(count as i32))
            }
            "wordCount" => {
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text, StringChunkType::Word, delimiter,
                )?;
                Ok(Datum::Int(count as i32))
            }
            "paragraphCount" => {
                // Director treats paragraphs as \r-delimited, same as lines.
                let delimiter = player.movie.item_delimiter;
                let count = StringChunkUtils::resolve_chunk_count(
                    &field.text, StringChunkType::Line, delimiter,
                )?;
                Ok(Datum::Int(count as i32))
            }
            "tabCount" => Ok(Datum::Int(0)), // Fields don't have tab stops in our model
            // Runtime selection state (stored, no editor integration yet)
            "selStart" => Ok(Datum::Int(field.sel_start)),
            "selEnd" => Ok(Datum::Int(field.sel_end)),
            // Text rendering config
            "kerning" => Ok(datum_bool(field.kerning)),
            "kerningThreshold" => Ok(Datum::Int(field.kerning_threshold as i32)),
            "useHypertextStyles" => Ok(datum_bool(field.use_hypertext_styles)),
            "antiAliasType" => Ok(Datum::Symbol(field.anti_alias_type.clone())),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for field",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop {
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
                |_player| -> Result<(i32, i32, i32, i32), ScriptError> {
                    let (vals, _flags) = value.to_rect_inline()?;

                    let x1 = vals[0] as i32;
                    let y1 = vals[1] as i32;
                    let x2 = vals[2] as i32;
                    let y2 = vals[3] as i32;

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
                        // Field BOX height — NOT line stride. The earlier
                        // assignment `fixed_line_space = h` clobbered the
                        // per-line stride with the box height (Coke Studios
                        // InfoStandDescription: rect=(0,0,190,55) made
                        // fixed_line_space=55 which the renderer then used
                        // as the line stride, putting each glyph at the
                        // bottom of a 55-px cell — only line 1 fit in the
                        // texture and showed at the bottom of the field).
                        field_data.height = h;
                    }
                    // Keep rect_* in sync so callers like
                    // get_concrete_sprite_rect (which uses these for
                    // bounding-box arithmetic) see the script-authored
                    // rect, not the FieldMember::new() defaults.
                    field_data.rect_left = x1 as i16;
                    field_data.rect_top = y1 as i16;
                    field_data.rect_right = x2 as i16;
                    field_data.rect_bottom = y2 as i16;

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
                    let w = value? as u16;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    field.width = w;
                    // Mirror into rect_* (right = left + w) so consumers
                    // that read rect_right/rect_left stay consistent.
                    field.rect_right = field.rect_left.saturating_add(w as i16);
                    Ok(())
                },
            ),
            "height" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let h = value? as u16;
                    let field = cast_member.member_type.as_field_mut().unwrap();
                    // Field BOX height. Do NOT touch `fixed_line_space` —
                    // that's the per-line stride (e.g. 13 for 12pt) and
                    // setting it to the box height (e.g. 55) makes the
                    // renderer place each glyph at the bottom of a tall
                    // cell. See `rect` setter above for the same bug fix.
                    field.height = h;
                    field.rect_bottom = field.rect_top.saturating_add(h as i16);
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
            "selStart" => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().sel_start = value?;
                    Ok(())
                },
            ),
            "selEnd" => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().sel_end = value?;
                    Ok(())
                },
            ),
            "kerning" => borrow_member_mut(
                member_ref,
                |_player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().kerning = value?;
                    Ok(())
                },
            ),
            "kerningThreshold" => borrow_member_mut(
                member_ref,
                |_player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().kerning_threshold = value? as u16;
                    Ok(())
                },
            ),
            "useHypertextStyles" => borrow_member_mut(
                member_ref,
                |_player| value.bool_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().use_hypertext_styles = value?;
                    Ok(())
                },
            ),
            "antiAliasType" => borrow_member_mut(
                member_ref,
                |_player| value.string_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().anti_alias_type = value?;
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
