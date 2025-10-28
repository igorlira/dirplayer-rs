use crate::{
    director::lingo::datum::{
        datum_bool, Datum, DatumType, StringChunkExpr, StringChunkSource, StringChunkType,
    },
    player::{
        bitmap::bitmap,
        bitmap::bitmap::{Bitmap, BuiltInPalette, PaletteRef},
        bitmap::drawing::CopyPixelsParams,
        bitmap::mask::BitmapMask,
        bitmap::palette_map::PaletteMap,
        cast_lib::CastMemberRef,
        font::{
            bitmap_font_copy_char, get_text_index_at_pos, measure_text, BitmapFont, DrawTextParams,
        },
        handlers::datum_handlers::{
            cast_member_ref::borrow_member_mut, string_chunk::StringChunkUtils,
        },
        DatumRef, DirPlayer, ScriptError,
    },
};

use crate::player::cast_member::CastMemberType;
use crate::player::ColorRef;
use std::borrow::Borrow;
use std::convert::TryInto;

// Simple HTML parser without external dependencies
#[derive(Clone, Debug)]
pub struct HtmlStyle {
    pub font_face: Option<String>,
    pub font_size: Option<i32>,
    pub color: Option<u32>,
    pub bg_color: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Default for HtmlStyle {
    fn default() -> Self {
        HtmlStyle {
            font_face: None,
            font_size: None,
            color: None,
            bg_color: None,
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StyledSpan {
    pub text: String,
    pub style: HtmlStyle,
}

pub struct HtmlParser;

impl HtmlParser {
    /// Parse HTML into styled spans without external dependencies
    pub fn parse_html(html: &str) -> Result<Vec<StyledSpan>, String> {
        let mut spans = Vec::new();
        let mut default_style = HtmlStyle::default();

        // Extract body attributes for global styling
        Self::extract_body_style(html, &mut default_style);

        // Simple regex-free HTML parsing
        Self::parse_html_recursive(html, &mut spans, default_style);

        Ok(spans)
    }

    fn extract_body_style(html: &str, style: &mut HtmlStyle) {
        let lower = html.to_lowercase();

        // Extract text color from body tag
        if let Some(text_attr) = Self::extract_attr(&lower, "body", "text") {
            if let Some(color) = Self::parse_color(&text_attr) {
                style.color = Some(color);
            }
        }

        // Extract background color from body tag
        if let Some(bg_attr) = Self::extract_attr(&lower, "body", "bg")
            .or_else(|| Self::extract_attr(&lower, "body", "bgcolor"))
        {
            if let Some(color) = Self::parse_color(&bg_attr) {
                style.bg_color = Some(color);
            }
        }
    }

    fn extract_attr(html: &str, tag: &str, attr: &str) -> Option<String> {
        let tag_start = format!("<{}", tag);
        if let Some(start_idx) = html.find(&tag_start) {
            if let Some(end_idx) = html[start_idx..].find('>') {
                let tag_content = &html[start_idx..start_idx + end_idx];
                let attr_pattern = format!("{}=", attr);

                if let Some(attr_idx) = tag_content.find(&attr_pattern) {
                    let after_eq = &tag_content[attr_idx + attr_pattern.len()..];
                    let quote_char = if after_eq.starts_with('"') { '"' } else { '\'' };

                    if let Some(start) = after_eq.find(quote_char) {
                        if let Some(end) = after_eq[start + 1..].find(quote_char) {
                            return Some(after_eq[start + 1..start + 1 + end].to_string());
                        }
                    }
                }
            }
        }
        None
    }

    fn parse_html_recursive(html: &str, spans: &mut Vec<StyledSpan>, current_style: HtmlStyle) {
        let mut pos = 0;
        let mut style_stack = vec![current_style];
        let chars: Vec<char> = html.chars().collect();

        while pos < chars.len() {
            if chars[pos] == '<' {
                // Find tag end
                if let Some(end) = chars[pos..].iter().position(|&c| c == '>').map(|p| p + pos) {
                    let tag = chars[pos + 1..end].iter().collect::<String>();
                    let tag_lower = tag.to_lowercase();

                    // Handle closing tags
                    if tag.starts_with('/') {
                        if style_stack.len() > 1 {
                            style_stack.pop();
                        }
                    } else {
                        // Handle opening tags
                        let mut new_style = style_stack.last().unwrap().clone();

                        match tag_lower.split_whitespace().next().unwrap_or("") {
                            "font" => {
                                if let Some(face) = Self::extract_tag_attr(&tag, "face") {
                                    new_style.font_face = Some(face);
                                }
                                if let Some(size_str) = Self::extract_tag_attr(&tag, "size") {
                                    if let Ok(size) = size_str.parse::<i32>() {
                                        new_style.font_size = Some(size);
                                    }
                                }
                                if let Some(color_str) = Self::extract_tag_attr(&tag, "color") {
                                    if let Some(color) = Self::parse_color(&color_str) {
                                        new_style.color = Some(color);
                                    }
                                }
                            }
                            "b" | "strong" => new_style.bold = true,
                            "i" | "em" => new_style.italic = true,
                            "u" => new_style.underline = true,
                            "br" => {
                                spans.push(StyledSpan {
                                    text: "\n".to_string(),
                                    style: new_style.clone(),
                                });
                            }
                            "center" => {
                                if !spans.is_empty() && !spans.last().unwrap().text.ends_with('\n')
                                {
                                    spans.push(StyledSpan {
                                        text: "\n".to_string(),
                                        style: new_style.clone(),
                                    });
                                }
                            }
                            _ => {}
                        }

                        if !tag.ends_with('/') && tag_lower != "br" {
                            style_stack.push(new_style);
                        }
                    }

                    pos = end + 1;
                    continue;
                }
            }

            // Collect text content
            let mut text = String::new();
            while pos < chars.len() && chars[pos] != '<' {
                text.push(chars[pos]);
                pos += 1;
            }

            if !text.trim().is_empty() {
                spans.push(StyledSpan {
                    text,
                    style: style_stack.last().unwrap().clone(),
                });
            }
        }
    }

    fn extract_tag_attr(tag: &str, attr: &str) -> Option<String> {
        let lower = tag.to_lowercase();
        let attr_pattern = format!("{}=", attr.to_lowercase());

        if let Some(idx) = lower.find(&attr_pattern) {
            let after_eq = &tag[idx + attr_pattern.len()..];
            let quote_char = if after_eq.starts_with('"') {
                '"'
            } else if after_eq.starts_with('\'') {
                '\''
            } else {
                ' '
            };

            if quote_char != ' ' {
                if let Some(start) = after_eq.find(quote_char) {
                    if let Some(end) = after_eq[start + 1..].find(quote_char) {
                        return Some(after_eq[start + 1..start + 1 + end].to_string());
                    }
                }
            } else {
                // Unquoted attribute
                let end_pos = after_eq.find(' ').unwrap_or(after_eq.len());
                return Some(after_eq[..end_pos].to_string());
            }
        }
        None
    }

    pub fn parse_color(color_str: &str) -> Option<u32> {
        let color_str = color_str.trim().to_lowercase();

        if color_str.starts_with('#') {
            let hex = &color_str[1..];
            if hex.len() == 6 {
                return u32::from_str_radix(hex, 16).ok();
            }
        } else if color_str.starts_with("0x") {
            let hex = &color_str[2..];
            if hex.len() == 6 {
                return u32::from_str_radix(hex, 16).ok();
            }
        }

        let rgb = match color_str.as_str() {
            "black" => 0x000000,
            "white" => 0xFFFFFF,
            "red" => 0xFF0000,
            "green" => 0x00FF00,
            "blue" => 0x0000FF,
            "yellow" => 0xFFFF00,
            "cyan" => 0x00FFFF,
            "magenta" => 0xFF00FF,
            "gray" | "grey" => 0x808080,
            "silver" => 0xC0C0C0,
            "maroon" => 0x800000,
            "olive" => 0x808000,
            "lime" => 0x00FF00,
            "aqua" => 0x00FFFF,
            "teal" => 0x008080,
            "navy" => 0x000080,
            "purple" => 0x800080,
            _ => return None,
        };

        Some(rgb)
    }
}

pub struct FontMemberHandlers {}

impl FontMemberHandlers {
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

    pub fn render_html_text_to_bitmap(
        bitmap: &mut Bitmap,
        spans: &[StyledSpan],
        font: &BitmapFont,
        font_bitmap: &Bitmap,
        palettes: &PaletteMap,
        fixed_line_space: u16,
        start_x: i32,
        start_y: i32,
        params: CopyPixelsParams,
    ) -> Result<(), ScriptError> {
        let mut x = start_x;
        let mut y = start_y;
        let line_height = if fixed_line_space > 0 {
            fixed_line_space as i32
        } else {
            font.char_height as i32
        };

        for span in spans {
            for ch in span.text.chars() {
                if ch == '\n' || ch == '\r' {
                    x = start_x;
                    y += line_height;
                    continue;
                }

                if ch < ' ' {
                    continue;
                }

                // Determine the color for this span
                let fg_color = if let Some(rgb) = span.style.color {
                    let r = ((rgb >> 16) & 0xFF) as u8;
                    let g = ((rgb >> 8) & 0xFF) as u8;
                    let b = (rgb & 0xFF) as u8;
                    ColorRef::Rgb(r, g, b)
                } else {
                    // Use sprite's foreground color if no HTML color specified
                    params.color.clone()
                };

                // For background, only set if HTML explicitly specifies it
                // Otherwise use sprite's bg_color (which for matte ink should be transparent)
                let bg_color = if let Some(rgb) = span.style.bg_color {
                    let r = ((rgb >> 16) & 0xFF) as u8;
                    let g = ((rgb >> 8) & 0xFF) as u8;
                    let b = (rgb & 0xFF) as u8;
                    ColorRef::Rgb(r, g, b)
                } else {
                    params.bg_color.clone()
                };

                if x >= bitmap.width as i32 {
                    x = start_x;
                    y += line_height;
                }

                if y >= bitmap.height as i32 {
                    break;
                }

                // Draw single character using bitmap_font_copy_char
                bitmap_font_copy_char(font, font_bitmap, ch as u8, bitmap, x, y, palettes, &params);

                x += font.char_width as i32;
            }
        }

        Ok(())
    }

    // Convert RGB color to palette index without accessing private fields
    fn rgb_to_palette_index_safe(rgb: u32, palettes: &PaletteMap) -> u32 {
        // Simple fallback: just return 36 (white) for now
        // You can implement a more advanced mapping if needed
        36
    }

    // Helper function to determine ink mode from style
    fn determine_ink_from_style(style: &HtmlStyle) -> i32 {
        let mut ink = 36; // Default ink

        // You can customize ink based on style properties
        if style.bold {
            ink = 36; // or a different ink for bold
        }

        if style.italic {
            // Apply italic transformation if needed
        }

        ink
    }

    // Helper function to convert RGB color to palette index
    fn rgb_to_palette_index(rgb: u32, palettes: &PaletteMap) -> u8 {
        let r = ((rgb >> 16) & 0xFF) as u8;
        let g = ((rgb >> 8) & 0xFF) as u8;
        let b = (rgb & 0xFF) as u8;

        if let Some(palette_entry) = palettes.palettes.first() {
            let mut best_index = 0u8;
            let mut best_distance = f32::MAX;

            for (i, color) in palette_entry.member.colors.iter().enumerate() {
                let distance = ((color.0 as i32 - r as i32).pow(2)
                    + (color.1 as i32 - g as i32).pow(2)
                    + (color.2 as i32 - b as i32).pow(2)) as f32;

                if distance < best_distance {
                    best_distance = distance;
                    best_index = i as u8;
                }
            }

            best_index
        } else {
            0
        }
    }

    // Helper function to draw a single styled character
    fn draw_styled_character(
        bitmap: &mut Bitmap,
        ch: char,
        x: i32,
        y: i32,
        font: BitmapFont,
        font_bitmap: &Bitmap,
        ink: i32,
        bg_color: u8,
        color_override: Option<u8>,
        palettes: PaletteMap,
    ) -> Result<(), ScriptError> {
        // Get the glyph from the font bitmap
        let glyph_index = ch as usize;

        // This depends on your font implementation
        // For now, we'll assume you have a method to draw a character
        // You may need to adjust this based on your font system

        let final_color = color_override.unwrap_or({
            // Use default text color from ink
            let palette_index = match ink {
                36 => 0, // Default to first color (usually white or black)
                _ => 0,
            };
            palette_index
        });

        // Draw the character at (x, y) with the appropriate color
        // This is a placeholder - adjust based on your actual bitmap drawing code
        if let Some(palette) = palettes.palettes.first() {
            // Draw character using your existing text rendering system
            // but with the color override applied
        }

        Ok(())
    }

    fn color_ref_to_u8(color: ColorRef) -> u8 {
        match color {
            ColorRef::PaletteIndex(idx) => idx,
            ColorRef::Rgb(r, g, b) => {
                // fallback: find closest palette index
                0
            }
            _ => 0, // fallback
        }
    }

    // Alternative simpler approach - if you have a draw_text that takes a color parameter:
    fn render_html_text_simple(
        bitmap: &mut Bitmap,
        spans: &[StyledSpan],
        font: BitmapFont,
        font_bitmap: &Bitmap,
        palettes: PaletteMap,
        fixed_line_space: u16,
        top_spacing: i16,
    ) -> Result<(), ScriptError> {
        // Group consecutive spans with the same style
        let mut x = 0i32;
        let mut y = top_spacing as i32;
        let line_height = fixed_line_space as i32;

        for span in spans {
            // Handle newlines
            let lines: Vec<&str> = span.text.split('\n').collect();

            for (line_idx, line) in lines.iter().enumerate() {
                if line_idx > 0 {
                    y += line_height;
                    x = 0;
                }

                if !line.is_empty() {
                    let color_idx = if let Some(color) = span.style.color {
                        Self::rgb_to_palette_index(color, &palettes)
                    } else {
                        255 // Default (usually white)
                    };

                    let bg_color = if let Some(bg) = span.style.bg_color {
                        Self::rgb_to_palette_index(bg, &palettes)
                    } else {
                        Self::color_ref_to_u8(bitmap.get_bg_color_ref())
                    };

                    let params = CopyPixelsParams {
                        blend: 100,
                        ink: 36,
                        color: ColorRef::PaletteIndex(color_idx),
                        bg_color: bitmap.get_bg_color_ref(),
                        mask_image: None,
                    };

                    bitmap.draw_text(
                        line,
                        &font,
                        font_bitmap,
                        x,
                        y,
                        params,
                        &palettes,
                        line_height as u16,
                        top_spacing,
                    );

                    x += line.len() as i32 * 8; // Rough estimate
                }
            }
        }

        Ok(())
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
            .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

        match &member.member_type {
            CastMemberType::Text(text_data) => {
                match prop.as_str() {
                    "text" => Ok(Datum::String(text_data.text.clone())),
                    "alignment" => Ok(Datum::String(text_data.alignment.clone())),
                    "wordWrap" => Ok(datum_bool(text_data.word_wrap)),
                    "width" => Ok(Datum::Int(text_data.width as i32)),
                    "font" => Ok(Datum::String(text_data.font.clone())),
                    "fontSize" => Ok(Datum::Int(text_data.font_size as i32)),
                    "fontStyle" => {
                        let font_styles: Vec<String> = text_data.font_style.clone();
                        let item_refs: Vec<_> = font_styles
                            .into_iter()
                            .map(|s| player.alloc_datum(Datum::Symbol(s)))
                            .collect();
                        Ok(Datum::List(DatumType::List, item_refs, false))
                    }
                    "fixedLineSpace" => Ok(Datum::Int(text_data.fixed_line_space as i32)),
                    "topSpacing" => Ok(Datum::Int(text_data.top_spacing as i32)),
                    "boxType" => Ok(Datum::Symbol(text_data.box_type.clone())),
                    "antialias" => Ok(datum_bool(text_data.anti_alias)),
                    "rect" | "height" | "image" => {
                        // Clone necessary data to avoid borrow issues
                        let text_clone = text_data.text.clone();
                        let font_name = text_data.font.clone();
                        let font_size = Some(text_data.font_size);
                        let fixed_line_space = text_data.fixed_line_space;
                        let top_spacing = text_data.top_spacing;
                        let has_html = text_data.has_html_styling();
                        let html_spans = if has_html {
                            Some(text_data.html_styled_spans.clone())
                        } else {
                            None
                        };

                        // Try to get custom font, fall back to system font
                        let font = if !font_name.is_empty() {
                            // Try to load from cast with size
                            player.font_manager.get_font_with_cast(
                                &font_name,
                                Some(&player.movie.cast_manager),
                                font_size,
                                None, // style
                            )
                        } else {
                            None
                        };

                        // Fall back to system font if custom font not found
                        let font = if let Some(f) = font {
                            f
                        } else {
                            player.font_manager.get_system_font().ok_or_else(|| {
                                ScriptError::new("System font not available".to_string())
                            })?
                        };

                        web_sys::console::log_1(
                            &format!(
                                "Using font: '{}' (size: {}, char_width: {}, char_height: {})",
                                font.font_name, font.font_size, font.char_width, font.char_height
                            )
                            .into(),
                        );

                        let (width, height) =
                            measure_text(&text_clone, &font, None, fixed_line_space, top_spacing);

                        match prop.as_str() {
                            "rect" => Ok(Datum::IntRect((0, 0, width as i32, height as i32))),
                            "height" => Ok(Datum::Int(height as i32)),
                            "image" => {
                                // Create 32-bit bitmap for proper transparency
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
                                        let index = ((y as usize * width as usize + x as usize) * 4)
                                            as usize;
                                        if index + 3 < bitmap.data.len() {
                                            bitmap.data[index] = 0;
                                            bitmap.data[index + 1] = 0;
                                            bitmap.data[index + 2] = 0;
                                            bitmap.data[index + 3] = 0;
                                        }
                                    }
                                }

                                let font_bitmap: &mut bitmap::Bitmap = player
                                    .bitmap_manager
                                    .get_bitmap_mut(font.bitmap_ref)
                                    .unwrap();

                                let palettes = player.movie.cast_manager.palettes();

                                if font_bitmap.matte.is_none() {
                                    font_bitmap.create_matte_text(&palettes);
                                }
                                let mask = Some(font_bitmap.matte.as_ref().unwrap());

                                let mut params = CopyPixelsParams {
                                    blend: 100,
                                    ink: 8,
                                    color: font_bitmap.get_fg_color_ref(),
                                    bg_color: font_bitmap.get_bg_color_ref(),
                                    mask_image: None,
                                };

                                if let Some(mask) = mask {
                                    let mask_bitmap: &BitmapMask = mask.borrow();
                                    params.mask_image = Some(mask_bitmap);
                                }

                                if let Some(spans) = html_spans {
                                    FontMemberHandlers::render_html_text_to_bitmap(
                                        &mut bitmap,
                                        &spans,
                                        &font,
                                        &font_bitmap,
                                        &palettes,
                                        fixed_line_space as u16,
                                        0,
                                        top_spacing as i32,
                                        params,
                                    )?;
                                } else {
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
                                }

                                let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                                Ok(Datum::BitmapRef(bitmap_ref))
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => Err(ScriptError::new(format!(
                        "Cannot get castMember property {} for Text member",
                        prop
                    ))),
                }
            }

            CastMemberType::Font(font_data) => match prop.as_str() {
                "previewText" => Ok(Datum::String(font_data.preview_text.clone())),
                "previewHtml" => {
                    let html_string: String = font_data
                        .preview_html_spans
                        .iter()
                        .map(|s| s.text.clone())
                        .collect();
                    Ok(Datum::String(html_string))
                }
                "fontStyle" => Ok(Datum::List(DatumType::List, vec![], false)),
                "name" => Ok(Datum::String(font_data.font_info.name.clone())),
                "size" => Ok(Datum::Int(font_data.font_info.size as i32)),
                _ => Err(ScriptError::new(format!(
                    "Cannot get castMember property {} for Font member",
                    prop
                ))),
            },

            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for this member type",
                prop
            ))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        borrow_member_mut(
            member_ref,
            |player| (), // no extra data needed
            |cast_member, _| {
                if let CastMemberType::Font(font_member) = &mut cast_member.member_type {
                    match prop {
                        "text" => font_member.preview_text = value.string_value()?,
                        "html" => {
                            let html_string = value.string_value()?;
                            let spans = HtmlParser::parse_html(&html_string).map_err(|e| {
                                ScriptError::new(format!("Failed to parse HTML: {}", e))
                            })?;
                            font_member.preview_text =
                                spans.iter().map(|s| s.text.clone()).collect();
                            font_member.preview_html_spans = spans;
                        }
                        _ => {
                            return Err(ScriptError::new(format!(
                                "Cannot set castMember prop '{}' for Font member",
                                prop
                            )))
                        }
                    }
                } else {
                    return Err(ScriptError::new(format!(
                        "Cannot set castMember prop '{}' for non-Font member",
                        prop
                    )));
                }
                Ok(())
            },
        )
    }
}
