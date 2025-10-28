use crate::{
    director::lingo::datum::{datum_bool, Datum, StringChunkType},
    player::{
        bitmap::bitmap::{Bitmap, BuiltInPalette, PaletteRef},
        bitmap::drawing::CopyPixelsParams,
        cast_lib::CastMemberRef,
        font::{measure_text, BitmapFont},
        handlers::datum_handlers::{
            cast_member_ref::borrow_member_mut, string_chunk::StringChunkUtils,
        },
        ColorRef, DatumRef, DirPlayer, ScriptError,
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
        let member_ref = player.get_datum(datum).to_member_ref()?;
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(&member_ref)
            .unwrap();
        let field = member.member_type.as_field().unwrap();
        match handler_name.as_str() {
            "count" => {
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
            "fixedLineSpace" => Ok(Datum::Int(field.fixed_line_space as i32)),
            "topSpacing" => Ok(Datum::Int(field.top_spacing as i32)),
            "boxType" => Ok(Datum::String(field.box_type.to_owned())),
            "antialias" => Ok(datum_bool(field.anti_alias)),
            "autoTab" => Ok(datum_bool(field.auto_tab)),
            "editable" => Ok(datum_bool(field.editable)),
            "border" => Ok(Datum::Int(field.border as i32)),
            "backColor" => Ok(Datum::Int(field.back_color as i32)),
            "rect" | "height" | "image" => {
                // Clone data to avoid borrow issues
                let text_clone = field.text.clone();
                let font_name = field.font.clone();
                let font_size = Some(field.font_size);
                let fixed_line_space = field.fixed_line_space;
                let top_spacing = field.top_spacing;

                // Try to get custom font, fall back to system font
                let font = if !font_name.is_empty() {
                    player.font_manager.get_font_with_cast(
                        &font_name,
                        Some(&player.movie.cast_manager),
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

                web_sys::console::log_1(
                    &format!(
                        "Field using font: '{}' (size: {})",
                        font.font_name, font.font_size
                    )
                    .into(),
                );

                let (width, height) =
                    measure_text(&text_clone, &font, None, fixed_line_space, top_spacing);

                match prop.as_str() {
                    "rect" => Ok(Datum::IntRect((0, 0, width as i32, height as i32))),
                    "height" => Ok(Datum::Int(height as i32)),
                    "image" => {
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
                            color: bitmap.get_fg_color_ref(),
                            bg_color: ColorRef::PaletteIndex(0),
                            mask_image: None,
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
                    cast_member.member_type.as_field_mut().unwrap().text = value?;
                    Ok(())
                },
            ),
            "rect" => borrow_member_mut(
                member_ref,
                |_| value.to_int_rect(),
                |cast_member, value| {
                    let value = value?;
                    let field_data = cast_member.member_type.as_field_mut().unwrap();
                    field_data.width = value.2 as u16;
                    Ok(())
                },
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
                    cast_member.member_type.as_field_mut().unwrap().font_size = value? as u16;
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
            "fixedLineSpace" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
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
            "backColor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_field_mut().unwrap().back_color = value? as u16;
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
