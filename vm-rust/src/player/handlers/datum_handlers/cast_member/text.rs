use crate::{
    director::lingo::datum::{
        datum_bool, Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType,
    },
    player::{
        bitmap::{
            bitmap::{Bitmap, BuiltInPalette, PaletteRef, get_system_default_palette},
            drawing::CopyPixelsParams,
        },
        cast_lib::CastMemberRef,
        font::{get_text_index_at_pos, measure_text, DrawTextParams},
        handlers::datum_handlers::{
            cast_member::font::{FontMemberHandlers, HtmlParser, StyledSpan, HtmlStyle, TextAlignment},
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
                let point = player.get_datum(&args[0]).to_point()?;
                let x = player.get_datum(&point[0]).int_value()?;
                let y = player.get_datum(&point[1]).int_value()?;

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
            "html" => {
                // Generate Director-style HTML from current text member state
                // Director always generates HTML from current properties, not stored HTML
                let mut html = String::new();

                // Get colors for body tag
                let bg_color = match member.bg_color {
                    crate::player::sprite::ColorRef::PaletteIndex(idx) => format!("#{:06X}", idx as u32),
                    crate::player::sprite::ColorRef::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
                };
                let text_color = match member.color {
                    crate::player::sprite::ColorRef::PaletteIndex(idx) => format!("#{:06X}", idx as u32),
                    crate::player::sprite::ColorRef::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
                };

                // Build body tag with color attributes (Director style uses bg= not bgcolor=)
                html.push_str(&format!(
                    "<html><body bg={} text={} link={} alink={} vlink={}>",
                    bg_color, text_color, text_color, text_color, text_color
                ));

                // Add alignment wrapper
                let alignment_start = match text_data.alignment.as_str() {
                    "center" => "<center>",
                    "right" => "<p align=right>",
                    _ => "",
                };
                let alignment_end = match text_data.alignment.as_str() {
                    "center" => "</center>",
                    "right" => "</p>",
                    _ => "",
                };
                html.push_str(alignment_start);

                // Add font tag if font is set
                if !text_data.font.is_empty() {
                    html.push_str(&format!("<font face=\"{}\">", text_data.font));
                }

                // Add text content
                html.push_str(&text_data.text);

                // Close tags
                if !text_data.font.is_empty() {
                    html.push_str("</font>");
                }
                html.push_str(alignment_end);
                html.push_str("</body></html>");

                Ok(Datum::String(html))
            }
            "rect" => {
                let font = player.font_manager.get_system_font().unwrap();
                let (width, height) = measure_text(
                    &text_data.text,
                    &font,
                    None,
                    text_data.fixed_line_space,
                    text_data.top_spacing,
                );
                Ok(Datum::Rect([
                    player.alloc_datum(Datum::Int(0)),
                    player.alloc_datum(Datum::Int(0)),
                    player.alloc_datum(Datum::Int(width as i32)),
                    player.alloc_datum(Datum::Int(height as i32))
                ]))
            }
            "height" => {
                // Return the stored height if set, otherwise calculate from text
                if text_data.height > 0 {
                    Ok(Datum::Int(text_data.height as i32))
                } else {
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
            }
            "foreColor" | "color" => {
                // Get foreground color from cast member
                match member.color {
                    crate::player::sprite::ColorRef::PaletteIndex(idx) => Ok(Datum::Int(idx as i32)),
                    crate::player::sprite::ColorRef::Rgb(r, g, b) => {
                        // Convert RGB to a packed integer
                        let rgb = ((r as i32) << 16) | ((g as i32) << 8) | (b as i32);
                        Ok(Datum::Int(rgb))
                    }
                }
            }
            "bgColor" | "backColor" => {
                // Get background color from cast member
                match member.bg_color {
                    crate::player::sprite::ColorRef::PaletteIndex(idx) => Ok(Datum::Int(idx as i32)),
                    crate::player::sprite::ColorRef::Rgb(r, g, b) => {
                        // Convert RGB to a packed integer
                        let rgb = ((r as i32) << 16) | ((g as i32) << 8) | (b as i32);
                        Ok(Datum::Int(rgb))
                    }
                }
            }
            "lineHeight" => {
                // Line height is typically font height + fixed line space
                Ok(Datum::Int(text_data.fixed_line_space as i32))
            }
            "image" => {
                // Use the same rendering approach as sprite display
                // Get dimensions - use styled spans if available for accurate measurement
                let (width, height) = if !text_data.html_styled_spans.is_empty() {
                    // Measure based on styled spans
                    let first_style = &text_data.html_styled_spans[0].style;
                    // Filter out 0 font sizes and empty font names
                    let font_size = first_style.font_size
                        .filter(|&s| s > 0)
                        .or_else(|| if text_data.font_size > 0 { Some(text_data.font_size as i32) } else { None })
                        .unwrap_or(12) as u16;
                    let font_name = first_style.font_face.clone()
                        .filter(|f| !f.is_empty())
                        .or_else(|| if !text_data.font.is_empty() { Some(text_data.font.clone()) } else { None })
                        .unwrap_or_else(|| "Arial".to_string());

                    // Get font for measurement
                    let font = player.font_manager.get_font_with_cast(
                        &font_name,
                        Some(&player.movie.cast_manager),
                        Some(font_size),
                        None,
                    ).or_else(|| player.font_manager.get_system_font());

                    if let Some(font) = font {
                        measure_text(
                            &text_data.text,
                            &font,
                            None,
                            text_data.fixed_line_space,
                            text_data.top_spacing,
                        )
                    } else {
                        (100, 20) // Fallback dimensions
                    }
                } else {
                    let font = player.font_manager.get_system_font().unwrap();
                    measure_text(
                        &text_data.text,
                        &font,
                        None,
                        text_data.fixed_line_space,
                        text_data.top_spacing,
                    )
                };

                // Create 32-bit RGBA bitmap for proper color and transparency support
                let mut bitmap = Bitmap::new(
                    width.max(1),
                    height.max(1),
                    32,
                    32,
                    8, // alpha_depth for transparency
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                bitmap.use_alpha = true;
                // Clear to transparent
                bitmap.data.fill(0);

                // Determine alignment
                let alignment = match text_data.alignment.to_lowercase().as_str() {
                    "center" | "#center" => TextAlignment::Center,
                    "right" | "#right" => TextAlignment::Right,
                    "justify" | "#justify" => TextAlignment::Justify,
                    _ => TextAlignment::Left,
                };

                // Get text color from cast member
                let text_color = Some(&member.color);

                // Use styled spans if available, otherwise create a basic span
                if !text_data.html_styled_spans.is_empty() {
                    // Clone spans and ALWAYS apply text_member's current properties
                    // The movie can set font, fontSize, fontStyle at runtime, so these
                    // should override whatever was in the original styled spans
                    let spans_with_defaults: Vec<StyledSpan> = text_data.html_styled_spans.iter().map(|span| {
                        let mut style = span.style.clone();

                        // ALWAYS use text_member's font if set (movie may have changed it)
                        if !text_data.font.is_empty() {
                            style.font_face = Some(text_data.font.clone());
                        } else if style.font_face.as_ref().map_or(true, |f| f.is_empty()) {
                            style.font_face = Some("Arial".to_string());
                        }

                        // ALWAYS use text_member's font_size if set (movie may have changed it)
                        if text_data.font_size > 0 {
                            style.font_size = Some(text_data.font_size as i32);
                        } else if style.font_size.map_or(true, |s| s <= 0) {
                            style.font_size = Some(12);
                        }

                        // Use cast member color if span doesn't have color
                        if style.color.is_none() {
                            style.color = match member.color {
                                crate::player::sprite::ColorRef::Rgb(r, g, b) => {
                                    Some(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
                                }
                                crate::player::sprite::ColorRef::PaletteIndex(idx) => {
                                    match idx {
                                        0 => Some(0xFFFFFF),
                                        255 => Some(0x000000),
                                        _ => Some(0x000000),
                                    }
                                }
                            };
                        }

                        // ALWAYS apply text_member's fontStyle (movie may have changed it)
                        // This replaces any per-span bold/italic/underline settings
                        if !text_data.font_style.is_empty() {
                            style.bold = text_data.font_style.iter().any(|s| s == "bold");
                            style.italic = text_data.font_style.iter().any(|s| s == "italic");
                            style.underline = text_data.font_style.iter().any(|s| s == "underline");
                        }

                        StyledSpan {
                            text: span.text.clone(),
                            style,
                        }
                    }).collect();

                    // Use native browser text rendering with styled spans
                    let _ = FontMemberHandlers::render_native_text_to_bitmap(
                        &mut bitmap,
                        &spans_with_defaults,
                        0,
                        text_data.top_spacing as i32,
                        width as i32,
                        height as i32,
                        alignment,
                        width as i32,
                        text_data.word_wrap,
                        None, // Don't pass text_color - it's now in the span
                    );
                } else {
                    // Create a basic styled span from text member properties
                    // Use None for empty/zero values so renderer uses proper defaults
                    let style = HtmlStyle {
                        font_face: if !text_data.font.is_empty() {
                            Some(text_data.font.clone())
                        } else {
                            None
                        },
                        font_size: if text_data.font_size > 0 {
                            Some(text_data.font_size as i32)
                        } else {
                            None
                        },
                        color: match member.color {
                            crate::player::sprite::ColorRef::Rgb(r, g, b) => {
                                Some(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
                            }
                            crate::player::sprite::ColorRef::PaletteIndex(idx) => {
                                // Convert common palette indices to RGB
                                match idx {
                                    0 => Some(0xFFFFFF), // White
                                    255 => Some(0x000000), // Black
                                    _ => Some(0x000000), // Default to black
                                }
                            }
                        },
                        bg_color: None, // Transparent background
                        bold: text_data.font_style.iter().any(|s| s == "bold"),
                        italic: text_data.font_style.iter().any(|s| s == "italic"),
                        underline: text_data.font_style.iter().any(|s| s == "underline"),
                    };
                    let spans = vec![StyledSpan {
                        text: text_data.text.clone(),
                        style,
                    }];

                    let _ = FontMemberHandlers::render_native_text_to_bitmap(
                        &mut bitmap,
                        &spans,
                        0,
                        text_data.top_spacing as i32,
                        width as i32,
                        height as i32,
                        alignment,
                        width as i32,
                        text_data.word_wrap,
                        None, // Don't pass text_color since we set it in the span
                    );
                }

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
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    let new_text = value?;

                    // Update the plain text
                    text_member.text = new_text.clone();

                    // Update html_styled_spans to contain a single span with the new text
                    // This ensures the styled_spans_hash changes when text changes
                    if !text_member.html_styled_spans.is_empty() {
                        // Create a new span with existing style from first span
                        let style = text_member.html_styled_spans[0].style.clone();
                        text_member.html_styled_spans = vec![
                            crate::player::handlers::datum_handlers::cast_member::font::StyledSpan {
                                text: new_text,
                                style,
                            }
                        ];
                    }

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
            "html" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let html_string = value?;
                    let spans = HtmlParser::parse_html(&html_string).map_err(|e| {
                        ScriptError::new(format!("Failed to parse HTML: {}", e))
                    })?;
                    let text_member = cast_member.member_type.as_text_mut().unwrap();

                    // Store original HTML source
                    text_member.html_source = html_string.clone();

                    // Extract plain text from all spans
                    text_member.text = spans.iter().map(|s| s.text.clone()).collect();

                    // Extract alignment from <p align="..."> or <center> tag
                    let html_lower = html_string.to_lowercase();
                    if html_lower.contains("align=\"center\"") || html_lower.contains("align='center'") || html_lower.contains("<center") {
                        text_member.alignment = "center".to_string();
                    } else if html_lower.contains("align=\"right\"") || html_lower.contains("align='right'") {
                        text_member.alignment = "right".to_string();
                    } else if html_lower.contains("align=\"left\"") || html_lower.contains("align='left'") {
                        text_member.alignment = "left".to_string();
                    }

                    // Extract bgcolor and text color from body tag
                    // Director uses both standard "bgcolor" and short "bg" forms
                    if let Some(body_start) = html_lower.find("<body") {
                        if let Some(body_end) = html_lower[body_start..].find('>') {
                            let body_tag = &html_string[body_start..body_start + body_end];

                            // Try bgcolor="..." or bg="..."
                            let bg_color_str = HtmlParser::extract_tag_attr(body_tag, "bgcolor")
                                .or_else(|| HtmlParser::extract_tag_attr(body_tag, "bg"));
                            if let Some(color_str) = bg_color_str {
                                if let Some(color) = HtmlParser::parse_color(&color_str) {
                                    cast_member.bg_color = crate::player::sprite::ColorRef::Rgb(
                                        ((color >> 16) & 0xFF) as u8,
                                        ((color >> 8) & 0xFF) as u8,
                                        (color & 0xFF) as u8,
                                    );
                                }
                            }

                            // Try text="..." for foreground color
                            if let Some(color_str) = HtmlParser::extract_tag_attr(body_tag, "text") {
                                if let Some(color) = HtmlParser::parse_color(&color_str) {
                                    cast_member.color = crate::player::sprite::ColorRef::Rgb(
                                        ((color >> 16) & 0xFF) as u8,
                                        ((color >> 8) & 0xFF) as u8,
                                        (color & 0xFF) as u8,
                                    );
                                }
                            }
                        }
                    }

                    // Apply style properties from the first span to the text member
                    if let Some(first_span) = spans.first() {
                        let style = &first_span.style;

                        // Apply font face if specified
                        if let Some(ref font_face) = style.font_face {
                            text_member.font = font_face.clone();
                        }

                        // Apply font size if specified
                        if let Some(font_size) = style.font_size {
                            text_member.font_size = font_size as u16;
                        }

                        // Apply font color if specified
                        if let Some(color) = style.color {
                            cast_member.color = crate::player::sprite::ColorRef::Rgb(
                                ((color >> 16) & 0xFF) as u8,
                                ((color >> 8) & 0xFF) as u8,
                                (color & 0xFF) as u8,
                            );
                        }

                        // Build font_style list from bold/italic/underline flags
                        let mut font_styles = Vec::new();
                        if style.bold {
                            font_styles.push("bold".to_string());
                        }
                        if style.italic {
                            font_styles.push("italic".to_string());
                        }
                        if style.underline {
                            font_styles.push("underline".to_string());
                        }
                        text_member.font_style = font_styles;
                    }

                    // Store all styled spans for rendering
                    text_member.html_styled_spans = spans;
                    Ok(())
                },
            ),
            "rect" => borrow_member_mut(
                member_ref,
                |player| {
                    let rect = value.to_rect()?;

                    let r1 = player.get_datum(&rect[1]).int_value()? as i16;
                    let r0 = player.get_datum(&rect[0]).int_value()? as i16;
                    let r3 = player.get_datum(&rect[3]).int_value()? as i16;
                    let r2 = player.get_datum(&rect[2]).int_value()? as i16;

                    Ok((r1, r0, r3, r2))
                },
                |cast_member, value| {
                    let value = value?;
                    let text_data = cast_member.member_type.as_text_mut().unwrap();
                    text_data.width = value.2 as u16;
                    Ok(())
                },
            ),
            "height" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().height = value? as u16;
                    Ok(())
                },
            ),
            "foreColor" | "color" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let color_val = value?;
                    // If value > 255, treat as RGB, otherwise as palette index
                    if color_val > 255 {
                        let r = ((color_val >> 16) & 0xFF) as u8;
                        let g = ((color_val >> 8) & 0xFF) as u8;
                        let b = (color_val & 0xFF) as u8;
                        cast_member.color = crate::player::sprite::ColorRef::Rgb(r, g, b);
                    } else {
                        cast_member.color = crate::player::sprite::ColorRef::PaletteIndex(color_val as u8);
                    }
                    Ok(())
                },
            ),
            "bgColor" | "backColor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let color_val = value?;
                    // If value > 255, treat as RGB, otherwise as palette index
                    if color_val > 255 {
                        let r = ((color_val >> 16) & 0xFF) as u8;
                        let g = ((color_val >> 8) & 0xFF) as u8;
                        let b = (color_val & 0xFF) as u8;
                        cast_member.bg_color = crate::player::sprite::ColorRef::Rgb(r, g, b);
                    } else {
                        cast_member.bg_color = crate::player::sprite::ColorRef::PaletteIndex(color_val as u8);
                    }
                    Ok(())
                },
            ),
            "lineHeight" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().fixed_line_space = value? as u16;
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
