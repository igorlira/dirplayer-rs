use log::debug;
use crate::{
    director::enums::TextInfo,
    director::lingo::datum::{
        datum_bool, Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType,
    },
    player::{
        bitmap::{
            bitmap::{Bitmap, BuiltInPalette, PaletteRef, get_system_default_palette},
            drawing::CopyPixelsParams,
        },
        cast_lib::CastMemberRef,
        font::{get_text_index_at_pos, get_glyph_preference, GlyphPreference, measure_text, DrawTextParams},
        handlers::datum_handlers::{
            cast_member::font::{FontMemberHandlers, HtmlParser, HtmlStyle, StyledSpan, TextAlignment},
            cast_member_ref::borrow_member_mut, string_chunk::StringChunkUtils,
        },
        DatumRef, DirPlayer, ScriptError,
    },
};

pub struct TextMemberHandlers {}
const DEBUG_TEXT_IMAGE: bool = false;

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
            "setProp" => {
                // setProp(#line, index, value) or setProp(#word, index, value) etc.
                let prop_name = player.get_datum(&args[0]).string_value()?;
                let index = player.get_datum(&args[1]).int_value()?;
                let new_value = player.get_datum(&args[2]).string_value()?;
                let chunk_type = StringChunkType::from(&prop_name);
                let chunk_expr = StringChunkExpr {
                    chunk_type,
                    start: index,
                    end: index,
                    item_delimiter: player.movie.item_delimiter,
                };
                let current_text = text.text.clone();
                let new_text = StringChunkUtils::string_by_putting_into_chunk(&current_text, &chunk_expr, &new_value)?;
                // Need mutable access - drop immutable borrows first
                let cast_member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref).unwrap();
                let text_member = cast_member.member_type.as_text_mut().unwrap();
                text_member.text = new_text.trim_end_matches('\0').to_string();
                Ok(DatumRef::Void)
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
        // Director property names are case-insensitive
        let prop_lc = prop.to_ascii_lowercase();
        match prop_lc.as_str() {
            "text" => Ok(Datum::String(text_data.text.to_owned())),
            "alignment" => Ok(Datum::String(text_data.alignment.to_owned())),
            "wordwrap" => Ok(datum_bool(text_data.word_wrap)),
            "width" => Ok(Datum::Int(text_data.width as i32)),
            "font" => Ok(Datum::String(text_data.font.to_owned())),
            "fontsize" => Ok(Datum::Int(text_data.font_size as i32)),
            "fontstyle" => {
                let mut item_refs = Vec::new();
                for item in &text_data.font_style {
                    item_refs.push(player.alloc_datum(Datum::Symbol(item.to_owned())));
                }
                Ok(Datum::List(DatumType::List, item_refs, false))
            }
            "fixedlinespace" => Ok(Datum::Int(text_data.fixed_line_space as i32)),
            "topspacing" => Ok(Datum::Int(text_data.top_spacing as i32)),
            "boxtype" => Ok(Datum::Symbol(text_data.box_type.to_owned())),
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
                let font = if !text_data.font.is_empty() {
                    player
                        .font_manager
                        .get_font_with_cast(
                            &text_data.font,
                            Some(&player.movie.cast_manager),
                            Some(text_data.font_size),
                            None,
                        )
                        .or_else(|| player.font_manager.get_system_font())
                        .unwrap()
                } else {
                    player.font_manager.get_system_font().unwrap()
                };
                let (width, measured_height) = measure_text(
                    &text_data.text,
                    &font,
                    None,
                    text_data.fixed_line_space,
                    text_data.top_spacing,
                    text_data.bottom_spacing,
                );
                // For #adjust, always use measured height. For #fixed/#scroll, use stored height if set.
                let height = if text_data.box_type != "adjust" && text_data.height > 0 {
                    text_data.height
                } else {
                    measured_height
                };
                Ok(Datum::Rect([
                    player.alloc_datum(Datum::Int(0)),
                    player.alloc_datum(Datum::Int(0)),
                    player.alloc_datum(Datum::Int(width as i32)),
                    player.alloc_datum(Datum::Int(height as i32))
                ]))
            }
            "height" => {
                // For #adjust box type, always calculate from text measurement.
                // For #fixed/#scroll, return stored height if set.
                if text_data.box_type != "adjust" && text_data.height > 0 {
                    Ok(Datum::Int(text_data.height as i32))
                } else {
                    let font = if !text_data.font.is_empty() {
                        player
                            .font_manager
                            .get_font_with_cast(
                                &text_data.font,
                                Some(&player.movie.cast_manager),
                                Some(text_data.font_size),
                                None,
                            )
                            .or_else(|| player.font_manager.get_system_font())
                            .unwrap()
                    } else {
                        player.font_manager.get_system_font().unwrap()
                    };
                    let (_, height) = measure_text(
                        &text_data.text,
                        &font,
                        None,
                        text_data.fixed_line_space,
                        text_data.top_spacing,
                        text_data.bottom_spacing,
                    );
                    Ok(Datum::Int(height as i32))
                }
            }
            "forecolor" | "color" => {
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
            "bgcolor" | "backcolor" => {
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
            "lineheight" => {
                // Line height is typically font height + fixed line space
                Ok(Datum::Int(text_data.fixed_line_space as i32))
            }
            // TextInfo (3D text / D6+ text member) properties
            "displayface" => {
                if let Some(ref info) = text_data.info {
                    let faces = info.display_face_list();
                    let mut item_refs = Vec::new();
                    for face in faces {
                        item_refs.push(player.alloc_datum(Datum::Symbol(face.trim_start_matches('#').to_string())));
                    }
                    Ok(Datum::List(DatumType::List, item_refs, false))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "tunneldepth" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Int(info.tunnel_depth as i32))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "beveltype" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Symbol(info.bevel_type_str().trim_start_matches('#').to_string()))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "beveldepth" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Int(info.bevel_depth as i32))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "smoothness" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Int(info.smoothness as i32))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "displaymode" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Symbol(info.display_mode_str().trim_start_matches('#').to_string()))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "directionalpreset" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Symbol(info.directional_preset_str().trim_start_matches('#').to_string()))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "texturetype" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Symbol(info.texture_type_str().trim_start_matches('#').to_string()))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "reflectivity" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Float(info.reflectivity as f64))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "directionalcolor" => {
                if let Some(ref info) = text_data.info {
                    let (r, g, b) = info.directional_color_rgb();
                    let rgb = ((r as i32) << 16) | ((g as i32) << 8) | (b as i32);
                    Ok(Datum::Int(rgb))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "ambientcolor" => {
                if let Some(ref info) = text_data.info {
                    let (r, g, b) = info.ambient_color_rgb();
                    let rgb = ((r as i32) << 16) | ((g as i32) << 8) | (b as i32);
                    Ok(Datum::Int(rgb))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "specularcolor" => {
                if let Some(ref info) = text_data.info {
                    let (r, g, b) = info.specular_color_rgb();
                    let rgb = ((r as i32) << 16) | ((g as i32) << 8) | (b as i32);
                    Ok(Datum::Int(rgb))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "cameraposition" => {
                if let Some(ref info) = text_data.info {
                    // Return as a vector(x, y, z)
                    let x_ref = player.alloc_datum(Datum::Float(info.camera_position_x as f64));
                    let y_ref = player.alloc_datum(Datum::Float(info.camera_position_y as f64));
                    let z_ref = player.alloc_datum(Datum::Float(info.camera_position_z as f64));
                    Ok(Datum::List(DatumType::Vector, vec![x_ref, y_ref, z_ref], false))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "camerarotation" => {
                if let Some(ref info) = text_data.info {
                    // Return as a vector(x, y, z)
                    let x_ref = player.alloc_datum(Datum::Float(info.camera_rotation_x as f64));
                    let y_ref = player.alloc_datum(Datum::Float(info.camera_rotation_y as f64));
                    let z_ref = player.alloc_datum(Datum::Float(info.camera_rotation_z as f64));
                    Ok(Datum::List(DatumType::Vector, vec![x_ref, y_ref, z_ref], false))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "texturemember" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::String(info.texture_member.clone()))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "editable" => {
                if let Some(ref info) = text_data.info {
                    Ok(datum_bool(info.editable))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "autotab" => {
                if let Some(ref info) = text_data.info {
                    Ok(datum_bool(info.auto_tab))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "directtostage" => {
                if let Some(ref info) = text_data.info {
                    Ok(datum_bool(info.direct_to_stage))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "prerender" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Symbol(info.pre_render_str().trim_start_matches('#').to_string()))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "savebitmap" => {
                if let Some(ref info) = text_data.info {
                    Ok(datum_bool(info.save_bitmap))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "kerning" => {
                if let Some(ref info) = text_data.info {
                    Ok(datum_bool(info.kerning))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "kerningthreshold" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Int(info.kerning_threshold as i32))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "usehypertextstyles" => {
                if let Some(ref info) = text_data.info {
                    Ok(datum_bool(info.use_hypertext_styles))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "antialiasthreshold" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Int(info.anti_alias_threshold as i32))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "scrolltop" => {
                if let Some(ref info) = text_data.info {
                    Ok(Datum::Int(info.scroll_top as i32))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "centerregpoint" => {
                if let Some(ref info) = text_data.info {
                    Ok(datum_bool(info.center_reg_point))
                } else {
                    // Older/runtime-created text members may not carry D6+ TextInfo.
                    // Director still treats centerRegPoint as a boolean property; default false.
                    Ok(datum_bool(false))
                }
            }
            "regpoint" => {
                if let Some(ref info) = text_data.info {
                    let x_ref = player.alloc_datum(Datum::Int(info.reg_x));
                    let y_ref = player.alloc_datum(Datum::Int(info.reg_y));
                    Ok(Datum::List(DatumType::Point, vec![x_ref, y_ref], false))
                } else {
                    Err(ScriptError::new("TextInfo not available for this member".to_string()))
                }
            }
            "image" => {
                if DEBUG_TEXT_IMAGE {
                    web_sys::console::log_1(&format!(
                        "[text.image] member={}:{} text_len={} spans={} font='{}' size={} wrap={} align='{}'",
                        cast_member_ref.cast_lib,
                        cast_member_ref.cast_member,
                        text_data.text.len(),
                        text_data.html_styled_spans.len(),
                        text_data.font,
                        text_data.font_size,
                        text_data.word_wrap,
                        text_data.alignment
                    ).into());
                }
                // Use the same rendering approach as sprite display
                // Get dimensions - use styled spans if available for accurate measurement
                let mut preferred_font_name: Option<String> = None;
                let mut preferred_font_size: Option<u16> = None;
                let (mut width, mut height) = if !text_data.html_styled_spans.is_empty() {
                    // Measure based on styled spans
                    let first_style = &text_data.html_styled_spans[0].style;
                    // Filter out 0 font sizes and empty font names
                    let font_size = first_style.font_size
                        .filter(|&s| s > 0)
                        .or_else(|| if text_data.font_size > 0 { Some(text_data.font_size as i32) } else { None })
                        .unwrap_or(12) as u16;
                    // Always prefer text_data.font (may have been changed at runtime via Lingo)
                    let font_name = if !text_data.font.is_empty() {
                        text_data.font.clone()
                    } else {
                        first_style.font_face.clone()
                            .filter(|f| !f.is_empty())
                            .unwrap_or_else(|| "Arial".to_string())
                    };
                    preferred_font_name = Some(font_name.clone());
                    preferred_font_size = Some(font_size);
                    if DEBUG_TEXT_IMAGE {
                        web_sys::console::log_1(&format!(
                            "[text.image] styled first span font='{}' size={}",
                            font_name, font_size
                        ).into());
                    }

                    // Get font for measurement
                    let font = player.font_manager.get_font_with_cast_and_bitmap(
                        &font_name,
                        &player.movie.cast_manager,
                        &mut player.bitmap_manager,
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
                            text_data.bottom_spacing,
                        )
                    } else {
                        (100, 20) // Fallback dimensions
                    }
                } else {
                    if !text_data.font.is_empty() {
                        preferred_font_name = Some(text_data.font.clone());
                        if text_data.font_size > 0 {
                            preferred_font_size = Some(text_data.font_size);
                        }
                    }
                    let font = if let Some(ref name) = preferred_font_name {
                        player
                            .font_manager
                            .get_font_with_cast_and_bitmap(
                                name,
                                &player.movie.cast_manager,
                                &mut player.bitmap_manager,
                                preferred_font_size,
                                None,
                            )
                            .or_else(|| player.font_manager.get_system_font())
                            .unwrap()
                    } else {
                        player.font_manager.get_system_font().unwrap()
                    };
                    measure_text(
                        &text_data.text,
                        &font,
                        None,
                        text_data.fixed_line_space,
                        text_data.top_spacing,
                        text_data.bottom_spacing,
                    )
                };
                let mut box_width = width;
                let mut box_height = height;
                let explicit_box_width = if text_data.width > 0 {
                    Some(text_data.width)
                } else if let Some(ref info) = text_data.info {
                    if info.width > 0 { Some(info.width as u16) } else { None }
                } else {
                    None
                };
                if let Some(w) = explicit_box_width {
                    // For text members with an authored box width, keep wrapping constrained to that box.
                    box_width = w.max(1);
                }
                // For #adjust box type, always use measured height. For #fixed/#scroll, use stored height.
                if text_data.box_type != "adjust" {
                    if text_data.height > 0 {
                        box_height = box_height.max(text_data.height);
                    }
                    if let Some(ref info) = text_data.info {
                        if info.height > 0 {
                            box_height = box_height.max(info.height as u16);
                        }
                    }
                }

                // Create 32-bit RGBA bitmap for proper color and transparency support
                let mut bitmap = Bitmap::new(
                    box_width.max(1),
                    box_height.max(1),
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

                let glyph_pref = get_glyph_preference();
                let is_pfr_font;

                // Load font from font_manager (PFR rasterizer), fall back to system font.
                let font = {
                    let font_name = preferred_font_name.as_deref()
                        .or(if !text_data.font.is_empty() { Some(text_data.font.as_str()) } else { None });
                    let font_size = preferred_font_size
                        .or(if text_data.font_size > 0 { Some(text_data.font_size) } else { None });
                    // Mirror the WebGL2 renderer's font lookup chain:
                    // 1. Name-based lookup
                    let mut loaded = if let Some(name) = font_name {
                        player.font_manager.get_font_with_cast_and_bitmap(
                            name,
                            &player.movie.cast_manager,
                            &mut player.bitmap_manager,
                            font_size,
                            None,
                        )
                    } else {
                        None
                    };
                    // 2. Case-insensitive match in font cache
                    if loaded.is_none() {
                        if let Some(name) = font_name {
                            let name_lower = name.to_lowercase();
                            for (key, font) in player.font_manager.font_cache.iter() {
                                if key.to_lowercase() == name_lower
                                    || key.to_lowercase().starts_with(&format!("{}_", name_lower))
                                {
                                    loaded = Some(font.clone());
                                    break;
                                }
                            }
                        }
                    }
                    loaded.or_else(|| player.font_manager.get_system_font())
                        .ok_or_else(|| ScriptError::new("No font available for text rendering".to_string()))?
                };
                is_pfr_font = font.char_widths.is_some();

                let use_native = match glyph_pref {
                    GlyphPreference::Native => true,
                    GlyphPreference::Bitmap | GlyphPreference::Outline => false,
                    GlyphPreference::Auto => false, // Default: bitmap for image property
                };

                if use_native && !is_pfr_font {
                    // Native Canvas2D rendering for standard fonts
                    let font_name_str = preferred_font_name.as_deref().unwrap_or("Arial");
                    let font_size_val = preferred_font_size.unwrap_or(12);
                    let (r, g, b) = {
                        use crate::player::bitmap::bitmap::resolve_color_ref;
                        let palettes = player.movie.cast_manager.palettes();
                        resolve_color_ref(
                            &palettes,
                            &member.color,
                            &bitmap.palette_ref,
                            bitmap.original_bit_depth,
                        )
                    };
                    let mut style = HtmlStyle::default();
                    style.font_face = Some(font_name_str.to_string());
                    style.font_size = Some(font_size_val as i32);
                    style.color = Some(((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
                    let spans = vec![StyledSpan {
                        text: text_data.text.clone(),
                        style,
                    }];
                    if let Err(e) = FontMemberHandlers::render_native_text_to_bitmap(
                        &mut bitmap,
                        &spans,
                        0,
                        text_data.top_spacing as i32,
                        box_width as i32,
                        box_height as i32,
                        alignment,
                        box_width as i32,
                        text_data.word_wrap,
                        None,
                        text_data.fixed_line_space,
                        text_data.top_spacing,
                        text_data.bottom_spacing,
                    ) {
                        web_sys::console::warn_1(
                            &format!("[text.image] Native render error: {:?}", e).into()
                        );
                    }
                } else if use_native && is_pfr_font {
                    // Native Canvas2D for PFR fonts - use the PFR font name, but Canvas2D
                    // won't have it registered, so it will fall back to a browser default.
                    // Still useful for comparison/debugging purposes.
                    let font_name_str = preferred_font_name.as_deref().unwrap_or(&font.font_name);
                    let font_size_val = preferred_font_size.unwrap_or(font.font_size.max(12));
                    let (r, g, b) = {
                        use crate::player::bitmap::bitmap::resolve_color_ref;
                        let palettes = player.movie.cast_manager.palettes();
                        resolve_color_ref(
                            &palettes,
                            &member.color,
                            &bitmap.palette_ref,
                            bitmap.original_bit_depth,
                        )
                    };
                    let mut style = HtmlStyle::default();
                    style.font_face = Some(font_name_str.to_string());
                    style.font_size = Some(font_size_val as i32);
                    style.color = Some(((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
                    let spans = vec![StyledSpan {
                        text: text_data.text.clone(),
                        style,
                    }];
                    if let Err(e) = FontMemberHandlers::render_native_text_to_bitmap(
                        &mut bitmap,
                        &spans,
                        0,
                        text_data.top_spacing as i32,
                        box_width as i32,
                        box_height as i32,
                        alignment,
                        box_width as i32,
                        text_data.word_wrap,
                        None,
                        text_data.fixed_line_space,
                        text_data.top_spacing,
                        text_data.bottom_spacing,
                    ) {
                        web_sys::console::warn_1(
                            &format!("[text.image] Native render error (PFR): {:?}", e).into()
                        );
                    }
                } else {
                    // Bitmap glyph rendering using PFR rasterizer font
                    let font_bitmap = player
                        .bitmap_manager
                        .get_bitmap(font.bitmap_ref)
                        .ok_or_else(|| ScriptError::new("Font bitmap not found".to_string()))?;
                    let palettes = player.movie.cast_manager.palettes();
                    let params = CopyPixelsParams {
                        blend: 100,
                        ink: 36,
                        color: member.color.clone(),
                        bg_color: crate::player::sprite::ColorRef::Rgb(255, 255, 255),
                        mask_image: None,
                        is_text_rendering: true,
                        rotation: 0.0,
                        skew: 0.0,
                        sprite: None,
                        original_dst_rect: None,
                    };

                    use crate::player::bitmap::bitmap::resolve_color_ref;
                    use crate::player::font::{bitmap_font_copy_char, bitmap_font_copy_char_tight};

                    let text_color = resolve_color_ref(
                        &palettes,
                        &params.color,
                        &bitmap.palette_ref,
                        bitmap.original_bit_depth,
                    );
                    let bold = text_data.font_style.iter().any(|s| s == "bold");
                    let italic = text_data.font_style.iter().any(|s| s == "italic");
                    let underline = text_data.font_style.iter().any(|s| s == "underline");
                    let is_pfr_font = font.char_widths.is_some();

                    let max_width = box_width as i32;
                    let mut y = text_data.top_spacing as i32;
                    let line_height = (font.char_height as i32 - 1).max(1);

                    // Get char_spacing from styled spans (XMED data)
                    let char_spacing: i32 = text_data.html_styled_spans.first()
                        .map(|s| s.style.char_spacing)
                        .unwrap_or(0);

                    let mut flush_line = |line: &str, y_pos: i32, bitmap: &mut Bitmap| {
                        let line_width: i32 = line
                            .chars()
                            .map(|c| font.get_char_advance(c as u8) as i32 + char_spacing)
                            .sum();
                        let start_x = match alignment {
                            TextAlignment::Center => ((max_width - line_width) / 2).max(0),
                            TextAlignment::Right => (max_width - line_width).max(0),
                            _ => 0,
                        };
                        let mut x = start_x;
                        for ch in line.chars() {
                            let adv = font.get_char_advance(ch as u8) as i32;
                            // Use tight copy for PFR fonts when cell width is much larger
                            // than character advance, to prevent transparent cell areas from
                            // overlapping and erasing adjacent characters.
                            let use_tight = is_pfr_font && (font.char_width as i32) > (adv * 2).max(16);
                            if use_tight {
                                bitmap_font_copy_char_tight(
                                    &font, font_bitmap, ch as u8, bitmap,
                                    x, y_pos, &palettes, &params,
                                );
                            } else {
                                bitmap_font_copy_char(
                                    &font, font_bitmap, ch as u8, bitmap,
                                    x, y_pos, &palettes, &params,
                                );
                            }
                            if bold {
                                if use_tight {
                                    bitmap_font_copy_char_tight(
                                        &font, font_bitmap, ch as u8, bitmap,
                                        x + 1, y_pos, &palettes, &params,
                                    );
                                } else {
                                    bitmap_font_copy_char(
                                        &font, font_bitmap, ch as u8, bitmap,
                                        x + 1, y_pos, &palettes, &params,
                                    );
                                }
                            }
                            x += adv + char_spacing;
                        }

                        if underline {
                            let underline_y = y_pos + line_height - 1;
                            for ux in start_x..(start_x + line_width).max(start_x) {
                                bitmap.set_pixel(ux, underline_y, text_color, &palettes);
                            }
                        }
                    };

                    let raw_lines: Vec<&str> = text_data.text.split(|c| c == '\r' || c == '\n').collect();
                    let mut lines_to_draw: Vec<String> = Vec::new();

                    if text_data.word_wrap && max_width > 0 {
                        for raw in raw_lines {
                            if raw.is_empty() {
                                lines_to_draw.push(String::new());
                                continue;
                            }
                            let mut current = String::new();
                            for word in raw.split_whitespace() {
                                let candidate = if current.is_empty() {
                                    word.to_string()
                                } else {
                                    format!("{} {}", current, word)
                                };
                                let candidate_width: i32 = candidate
                                    .chars()
                                    .map(|c| font.get_char_advance(c as u8) as i32 + char_spacing)
                                    .sum();
                                if candidate_width <= max_width || current.is_empty() {
                                    current = candidate;
                                } else {
                                    lines_to_draw.push(current);
                                    current = word.to_string();
                                }
                            }
                            if !current.is_empty() {
                                lines_to_draw.push(current);
                            }
                        }
                    } else {
                        lines_to_draw = raw_lines.iter().map(|s| s.to_string()).collect();
                    }

                    let effective_line_height = if text_data.fixed_line_space > 0 {
                        text_data.fixed_line_space as i32
                    } else {
                        line_height
                    };
                    let line_step = effective_line_height
                        + text_data.bottom_spacing as i32
                        + text_data.top_spacing as i32;
                    for line in lines_to_draw {
                        flush_line(&line, y, &mut bitmap);
                        y += line_step;
                    }

                } // end bitmap glyph else branch

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
        // Director property names are case-insensitive
        let prop_lc = prop.to_ascii_lowercase();
        match prop_lc.as_str() {
            "text" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    let new_text = value?.trim_end_matches('\0').to_string();

                    let old_color = text_member.html_styled_spans.first()
                        .and_then(|s| s.style.color)
                        .map(|c| format!("#{:06X}", c & 0xFFFFFF))
                        .unwrap_or_else(|| "none".to_string());
                    debug!(
                        "[text_setter] member='{}' old_color={} spans={} new_text='{}'",
                        cast_member.name, old_color,
                        text_member.html_styled_spans.len(),
                        &new_text[..new_text.len().min(30)],
                    );

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
            "wordwrap" => borrow_member_mut(
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
                    let font_name = value?;
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    text_member.font = font_name.clone();
                    for span in &mut text_member.html_styled_spans {
                        span.style.font_face = Some(font_name.clone());
                    }
                    Ok(())
                },
            ),
            "fontsize" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let size = value? as u16;
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    text_member.font_size = size;
                    for span in &mut text_member.html_styled_spans {
                        span.style.font_size = Some(size as i32);
                    }
                    Ok(())
                },
            ),
            "fontstyle" => borrow_member_mut(
                member_ref,
                |player| {
                    let mut item_strings = Vec::new();
                    for x in value.to_list().unwrap() {
                        item_strings.push(player.get_datum(x).string_value()?);
                    }
                    Ok(item_strings)
                },
                |cast_member, value| {
                    let styles = value?;
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    let bold = styles.iter().any(|s| s == "bold");
                    let italic = styles.iter().any(|s| s == "italic");
                    let underline = styles.iter().any(|s| s == "underline");
                    text_member.font_style = styles;
                    for span in &mut text_member.html_styled_spans {
                        span.style.bold = bold;
                        span.style.italic = italic;
                        span.style.underline = underline;
                    }
                    Ok(())
                },
            ),
            "fixedlinespace" => borrow_member_mut(
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
            "topspacing" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().top_spacing = value? as i16;
                    Ok(())
                },
            ),
            "boxtype" => borrow_member_mut(
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

                    let old_color = text_member.html_styled_spans.first()
                        .and_then(|s| s.style.color)
                        .map(|c| format!("#{:06X}", c & 0xFFFFFF))
                        .unwrap_or_else(|| "none".to_string());
                    let new_color = spans.first()
                        .and_then(|s| s.style.color)
                        .map(|c| format!("#{:06X}", c & 0xFFFFFF))
                        .unwrap_or_else(|| "none".to_string());
                    debug!(
                        "[html_setter] member='{}' old_color={} new_color={} new_spans={} html='{}'",
                        cast_member.name, old_color, new_color, spans.len(),
                        &html_string[..html_string.len().min(80)],
                    );

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
                    let left = value.1;
                    let top = value.0;
                    let bottom = value.2;
                    let right = value.3;
                    let w = (right - left).max(0) as u16;
                    let h = (bottom - top).max(0) as u16;
                    if w > 0 {
                        text_data.width = w;
                    }
                    // Setting height via rect is a no-op for #adjust box type
                    if h > 0 && text_data.box_type != "adjust" {
                        text_data.height = h;
                    }

                    Ok(())
                },
            ),
            "height" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_data = cast_member.member_type.as_text_mut().unwrap();
                    // Setting height is a no-op for #adjust box type
                    if text_data.box_type != "adjust" {
                        text_data.height = value? as u16;
                    }
                    Ok(())
                },
            ),
            "forecolor" | "color" => borrow_member_mut(
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
            "bgcolor" | "backcolor" => borrow_member_mut(
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
            "lineheight" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    cast_member.member_type.as_text_mut().unwrap().fixed_line_space = value? as u16;
                    Ok(())
                },
            ),
            // TextInfo (3D text / D6+ text member) property setters
            "tunneldepth" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.tunnel_depth = value? as u16;
                    }
                    Ok(())
                },
            ),
            "beveldepth" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.bevel_depth = value? as u16;
                    }
                    Ok(())
                },
            ),
            "smoothness" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.smoothness = value? as u32;
                    }
                    Ok(())
                },
            ),
            "reflectivity" => borrow_member_mut(
                member_ref,
                |player| value.float_value().or_else(|_| value.int_value().map(|i| i as f64)),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.reflectivity = value? as u32;
                    }
                    Ok(())
                },
            ),
            "beveltype" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let val = value?;
                        info.bevel_type = match val.trim_start_matches('#') {
                            "none" => 0,
                            "miter" => 1,
                            "round" => 2,
                            _ => 0,
                        };
                    }
                    Ok(())
                },
            ),
            "displaymode" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let val = value?;
                        info.display_mode = match val.trim_start_matches('#') {
                            "normal" => 0,
                            "mode3d" => 1,
                            _ => 0,
                        };
                    }
                    Ok(())
                },
            ),
            "directionalpreset" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let val = value?;
                        info.directional_preset = match val.trim_start_matches('#') {
                            "none" => 0,
                            "topLeft" => 1,
                            "topCenter" => 2,
                            "topRight" => 3,
                            "middleLeft" => 4,
                            "middleCenter" => 5,
                            "middleRight" => 6,
                            "bottomLeft" => 7,
                            "bottomCenter" => 8,
                            "bottomRight" => 9,
                            _ => 0,
                        };
                    }
                    Ok(())
                },
            ),
            "texturetype" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let val = value?;
                        info.texture_type = match val.trim_start_matches('#') {
                            "none" => 0,
                            "default" => 1,
                            "member" => 2,
                            _ => 0,
                        };
                    }
                    Ok(())
                },
            ),
            "directionalcolor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let color_val = value?;
                        // Convert RGB to format RR GG BB 00
                        let r = ((color_val >> 16) & 0xFF) as u32;
                        let g = ((color_val >> 8) & 0xFF) as u32;
                        let b = (color_val & 0xFF) as u32;
                        info.directional_color = (r << 24) | (g << 16) | (b << 8);
                    }
                    Ok(())
                },
            ),
            "ambientcolor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let color_val = value?;
                        let r = ((color_val >> 16) & 0xFF) as u32;
                        let g = ((color_val >> 8) & 0xFF) as u32;
                        let b = (color_val & 0xFF) as u32;
                        info.ambient_color = (r << 24) | (g << 16) | (b << 8);
                    }
                    Ok(())
                },
            ),
            "specularcolor" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let color_val = value?;
                        let r = ((color_val >> 16) & 0xFF) as u32;
                        let g = ((color_val >> 8) & 0xFF) as u32;
                        let b = (color_val & 0xFF) as u32;
                        info.specular_color = (r << 24) | (g << 16) | (b << 8);
                    }
                    Ok(())
                },
            ),
            "cameraposition" => borrow_member_mut(
                member_ref,
                |player| {
                    let list = value.to_list()?;
                    if list.len() >= 3 {
                        let x = player.get_datum(&list[0]).float_value()?;
                        let y = player.get_datum(&list[1]).float_value()?;
                        let z = player.get_datum(&list[2]).float_value()?;
                        Ok((x, y, z))
                    } else {
                        Err(ScriptError::new("cameraPosition requires a vector with 3 elements".to_string()))
                    }
                },
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let (x, y, z) = value?;
                        info.camera_position_x = x as f32;
                        info.camera_position_y = y as f32;
                        info.camera_position_z = z as f32;
                    }
                    Ok(())
                },
            ),
            "camerarotation" => borrow_member_mut(
                member_ref,
                |player| {
                    let list = value.to_list()?;
                    if list.len() >= 3 {
                        let x = player.get_datum(&list[0]).float_value()?;
                        let y = player.get_datum(&list[1]).float_value()?;
                        let z = player.get_datum(&list[2]).float_value()?;
                        Ok((x, y, z))
                    } else {
                        Err(ScriptError::new("cameraRotation requires a vector with 3 elements".to_string()))
                    }
                },
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let (x, y, z) = value?;
                        info.camera_rotation_x = x as f32;
                        info.camera_rotation_y = y as f32;
                        info.camera_rotation_z = z as f32;
                    }
                    Ok(())
                },
            ),
            "texturemember" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.texture_member = value?;
                    }
                    Ok(())
                },
            ),
            "displayface" => borrow_member_mut(
                member_ref,
                |player| {
                    let list = value.to_list()?;
                    let mut face_mask: i32 = 0;
                    for item_ref in list {
                        let face_str = player.get_datum(&item_ref).string_value()?;
                        match face_str.trim_start_matches('#') {
                            "front" => face_mask |= 1,
                            "tunnel" => face_mask |= 2,
                            "back" => face_mask |= 4,
                            _ => {}
                        }
                    }
                    // If all faces are enabled, use -1
                    if face_mask == 7 {
                        face_mask = -1;
                    }
                    Ok(face_mask)
                },
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.display_face = value?;
                    }
                    Ok(())
                },
            ),
            "editable" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.editable = value?;
                    }
                    Ok(())
                },
            ),
            "autotab" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.auto_tab = value?;
                    }
                    Ok(())
                },
            ),
            "directtostage" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.direct_to_stage = value?;
                    }
                    Ok(())
                },
            ),
            "prerender" => borrow_member_mut(
                member_ref,
                |player| value.string_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let val = value?;
                        info.pre_render = match val.trim_start_matches('#') {
                            "none" => 0,
                            "copyInk" => 1,
                            "otherInk" => 2,
                            _ => 0,
                        };
                    }
                    Ok(())
                },
            ),
            "savebitmap" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.save_bitmap = value?;
                    }
                    Ok(())
                },
            ),
            "kerning" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.kerning = value?;
                    }
                    Ok(())
                },
            ),
            "kerningthreshold" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.kerning_threshold = value? as u32;
                    }
                    Ok(())
                },
            ),
            "usehypertextstyles" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.use_hypertext_styles = value?;
                    }
                    Ok(())
                },
            ),
            "antialiasthreshold" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.anti_alias_threshold = value? as u32;
                    }
                    Ok(())
                },
            ),
            "scrolltop" => borrow_member_mut(
                member_ref,
                |player| value.int_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        info.scroll_top = value? as u32;
                    }
                    Ok(())
                },
            ),
            "centerregpoint" => borrow_member_mut(
                member_ref,
                |player| value.bool_value(),
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if text_member.info.is_none() {
                        text_member.info = Some(TextInfo::default());
                    }
                    if let Some(ref mut info) = text_member.info {
                        info.center_reg_point = value?;
                    }
                    Ok(())
                },
            ),
            "regpoint" => borrow_member_mut(
                member_ref,
                |player| {
                    let list = value.to_list()?;
                    if list.len() >= 2 {
                        let x = player.get_datum(&list[0]).int_value()?;
                        let y = player.get_datum(&list[1]).int_value()?;
                        Ok((x, y))
                    } else {
                        Err(ScriptError::new("regPoint requires a point with 2 elements".to_string()))
                    }
                },
                |cast_member, value| {
                    let text_member = cast_member.member_type.as_text_mut().unwrap();
                    if let Some(ref mut info) = text_member.info {
                        let (x, y) = value?;
                        info.reg_x = x;
                        info.reg_y = y;
                    }
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
