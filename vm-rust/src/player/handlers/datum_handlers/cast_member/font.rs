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
            bitmap_font_copy_char, bitmap_font_copy_char_scaled, get_text_index_at_pos, measure_text, BitmapFont, DrawTextParams,
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
use wasm_bindgen::JsCast;

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

#[derive(Debug, Clone, Copy)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
    Justify,
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
        // Track depth inside non-content tags (head, title, script, style)
        let mut skip_content_depth = 0;

        while pos < chars.len() {
            if chars[pos] == '<' {
                // Find tag end
                if let Some(end) = chars[pos..].iter().position(|&c| c == '>').map(|p| p + pos) {
                    let tag = chars[pos + 1..end].iter().collect::<String>();
                    let tag_lower = tag.to_lowercase();

                    // Handle closing tags
                    if tag.starts_with('/') {
                        let closing_tag = tag_lower[1..].split_whitespace().next().unwrap_or("");
                        // Check if closing a non-content tag
                        if closing_tag == "head" || closing_tag == "title" || closing_tag == "script" || closing_tag == "style" {
                            if skip_content_depth > 0 {
                                skip_content_depth -= 1;
                            }
                        } else if skip_content_depth == 0 && style_stack.len() > 1 {
                            style_stack.pop();
                        }
                    } else {
                        // Handle opening tags
                        let mut new_style = style_stack.last().unwrap().clone();

                        // Get the tag name, handling self-closing tags like <br/> or <br />
                        let tag_name = tag_lower.split_whitespace().next().unwrap_or("")
                            .trim_end_matches('/');

                        // Check if entering a non-content tag
                        if tag_name == "head" || tag_name == "title" || tag_name == "script" || tag_name == "style" {
                            skip_content_depth += 1;
                        } else if skip_content_depth == 0 {
                            // Only process content tags when not inside non-content section
                            match tag_name {
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
                                "p" => {
                                    // Handle paragraph tag - may have align attribute
                                    // Add newline before paragraph if not at start
                                    if !spans.is_empty() && !spans.last().unwrap().text.ends_with('\n')
                                    {
                                        spans.push(StyledSpan {
                                            text: "\n".to_string(),
                                            style: new_style.clone(),
                                        });
                                    }
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
                                // Skip structural tags that don't affect content
                                "html" | "body" | "!doctype" => {}
                                _ => {}
                            }

                            if !tag.ends_with('/') && tag_name != "br" {
                                style_stack.push(new_style);
                            }
                        }
                    }

                    pos = end + 1;
                    continue;
                }
            }

            // Collect text content (only if not inside non-content tags)
            let mut text = String::new();
            while pos < chars.len() && chars[pos] != '<' {
                text.push(chars[pos]);
                pos += 1;
            }

            // Only add text if not inside head/title/script/style and text is not empty
            if skip_content_depth == 0 && !text.is_empty() {
                spans.push(StyledSpan {
                    text,
                    style: style_stack.last().unwrap().clone(),
                });
            }
        }
    }

    pub fn extract_tag_attr(tag: &str, attr: &str) -> Option<String> {
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
                let point_ref = player.get_datum(&args[0]).to_point()?;
                let x = player.get_datum(&point_ref[0]).int_value()?;
                let y = player.get_datum(&point_ref[1]).int_value()?;

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
        // Call the extended version with default alignment and no word wrap
        Self::render_html_text_to_bitmap_styled(
            bitmap,
            spans,
            font,
            font_bitmap,
            palettes,
            fixed_line_space,
            start_x,
            start_y,
            params,
            TextAlignment::Left,
            0,     // max_width = 0 means no limit
            false, // word_wrap
        )
    }

    /// Render styled text using browser's native Canvas2D fillText() for smooth, anti-aliased text
    /// This produces much better quality than bitmap font scaling
    /// Returns the rendered text as RGBA pixel data that can be composited onto the destination
    /// sprite_color: Optional sprite foreground color that overrides the styled span color
    pub fn render_native_text_to_bitmap(
        bitmap: &mut Bitmap,
        spans: &[StyledSpan],
        start_x: i32,
        start_y: i32,
        render_width: i32,
        render_height: i32,
        alignment: TextAlignment,
        max_width: i32,
        word_wrap: bool,
        sprite_color: Option<&ColorRef>,
    ) -> Result<(), ScriptError> {
        // Get style from first span
        let first_style = spans.first().map(|s| &s.style);
        let is_bold = first_style.map(|s| s.bold).unwrap_or(false);
        let is_italic = first_style.map(|s| s.italic).unwrap_or(false);
        let is_underline = first_style.map(|s| s.underline).unwrap_or(false);

        // Get font_size - treat 0 as "not set" and use default
        let font_size = first_style
            .and_then(|s| s.font_size)
            .filter(|&size| size > 0)  // Treat 0 as None
            .unwrap_or(12) as i32;

        // Get font_face - treat empty string as "not set" and use default
        let font_face = first_style
            .and_then(|s| s.font_face.clone())
            .filter(|face| !face.is_empty())  // Treat "" as None
            .unwrap_or_else(|| "Arial".to_string());

        // Get text color - styled span color takes precedence if available,
        // otherwise fall back to sprite color
        let styled_span_color = first_style.and_then(|s| s.color);

        let (text_r, text_g, text_b) = if let Some(color_u32) = styled_span_color {
            // Use styled span color (from HTML font color attribute)
            (
                ((color_u32 >> 16) & 0xFF) as u8,
                ((color_u32 >> 8) & 0xFF) as u8,
                (color_u32 & 0xFF) as u8,
            )
        } else if let Some(color_ref) = sprite_color {
            // Fall back to sprite color
            match color_ref {
                ColorRef::Rgb(r, g, b) => (*r, *g, *b),
                ColorRef::PaletteIndex(idx) => {
                    // Convert palette index to RGB (common palette mappings)
                    match *idx {
                        0 => (255, 255, 255),   // White
                        255 => (0, 0, 0),       // Black
                        _ => (0, 0, 0),         // Default to black for other indices
                    }
                }
            }
        } else {
            // Default to black
            (0, 0, 0)
        };

        // Collect all text
        let full_text: String = spans.iter().map(|s| s.text.as_str()).collect();

        // Create temporary canvas for text rendering (size of text area only)
        let document = web_sys::window()
            .ok_or_else(|| ScriptError::new("No window".to_string()))?
            .document()
            .ok_or_else(|| ScriptError::new("No document".to_string()))?;

        let canvas: web_sys::HtmlCanvasElement = document
            .create_element("canvas")
            .map_err(|_| ScriptError::new("Failed to create canvas".to_string()))?
            .dyn_into()
            .map_err(|_| ScriptError::new("Failed to cast canvas".to_string()))?;

        let canvas_width = render_width.max(1) as u32;
        let canvas_height = render_height.max(1) as u32;
        canvas.set_width(canvas_width);
        canvas.set_height(canvas_height);

        let ctx: web_sys::CanvasRenderingContext2d = canvas
            .get_context("2d")
            .map_err(|_| ScriptError::new("Failed to get 2d context".to_string()))?
            .ok_or_else(|| ScriptError::new("No 2d context".to_string()))?
            .dyn_into()
            .map_err(|_| ScriptError::new("Failed to cast context".to_string()))?;

        // Clear canvas to transparent (don't fill with white)
        // The browser's fillText will render with proper alpha for anti-aliasing
        ctx.clear_rect(0.0, 0.0, canvas_width as f64, canvas_height as f64);

        // Build font string: "bold italic 12px Arial"
        let font_size_str = format!("{}px", font_size);
        let mut font_parts = Vec::new();
        if is_bold {
            font_parts.push("bold");
        }
        if is_italic {
            font_parts.push("italic");
        }
        font_parts.push(&font_size_str);
        font_parts.push(&font_face);

        let font_string = font_parts.join(" ");
        ctx.set_font(&font_string);

        // Set text color
        ctx.set_fill_style_str(&format!("rgb({},{},{})", text_r, text_g, text_b));

        // Set text baseline to top for consistent positioning
        ctx.set_text_baseline("top");

        // Split text into lines
        let raw_lines: Vec<&str> = full_text.split(|c| c == '\n' || c == '\r').collect();

        // Process lines with word wrap if enabled
        let lines: Vec<String> = if word_wrap && max_width > 0 {
            let mut wrapped_lines = Vec::new();
            for raw_line in raw_lines {
                wrapped_lines.extend(Self::word_wrap_native_with_size(raw_line, max_width as f64, font_size));
            }
            wrapped_lines
        } else {
            raw_lines.iter().map(|s| s.to_string()).collect()
        };

        // Render each line (relative to canvas origin, not bitmap origin)
        let line_height = (font_size as f64 * 1.2).ceil();
        let mut y = 0.0;

        for line in &lines {
            if y >= canvas_height as f64 {
                break;
            }

            // Calculate x position based on alignment
            // Estimate text width: average char width is ~0.55x font height for proportional fonts like Arial
            let text_width = line.chars().count() as f64 * (font_size as f64 * 0.55);
            let x = match alignment {
                TextAlignment::Left => 0.0,
                TextAlignment::Center => {
                    if max_width > 0 {
                        (max_width as f64 - text_width) / 2.0
                    } else {
                        0.0
                    }
                }
                TextAlignment::Right => {
                    if max_width > 0 {
                        max_width as f64 - text_width
                    } else {
                        0.0
                    }
                }
                TextAlignment::Justify => 0.0, // TODO: implement justify
            };

            // Draw text
            let _ = ctx.fill_text(line, x, y);

            // Draw underline if needed
            if is_underline {
                let underline_y = y + font_size as f64 - 1.0;
                ctx.begin_path();
                ctx.move_to(x, underline_y);
                ctx.line_to(x + text_width, underline_y);
                ctx.set_stroke_style_str(&format!("rgb({},{},{})", text_r, text_g, text_b));
                ctx.stroke();
            }

            y += line_height;
        }

        // Get pixel data from canvas
        let image_data = ctx
            .get_image_data(0.0, 0.0, canvas_width as f64, canvas_height as f64)
            .map_err(|_| ScriptError::new("Failed to get image data".to_string()))?;

        let pixels = image_data.data();

        // Copy pixels to bitmap, converting white to transparent
        // This allows proper compositing with background
        for cy in 0..canvas_height as i32 {
            let dest_y = start_y + cy;
            if dest_y < 0 || dest_y >= bitmap.height as i32 {
                continue;
            }

            for cx in 0..canvas_width as i32 {
                let dest_x = start_x + cx;
                if dest_x < 0 || dest_x >= bitmap.width as i32 {
                    continue;
                }

                let src_idx = ((cy as usize * canvas_width as usize) + cx as usize) * 4;
                let dest_idx = ((dest_y as usize * bitmap.width as usize) + dest_x as usize) * 4;

                if src_idx + 3 >= pixels.len() || dest_idx + 3 >= bitmap.data.len() {
                    continue;
                }

                let r = pixels[src_idx];
                let g = pixels[src_idx + 1];
                let b = pixels[src_idx + 2];
                let a = pixels[src_idx + 3];

                // Copy all pixels with their alpha channel from the browser's fillText
                // Background pixels will have alpha=0, text pixels will have alpha>0
                bitmap.data[dest_idx] = r;
                bitmap.data[dest_idx + 1] = g;
                bitmap.data[dest_idx + 2] = b;
                bitmap.data[dest_idx + 3] = a;
            }
        }

        Ok(())
    }

    /// Word wrap helper for native text rendering
    fn word_wrap_native_with_size(text: &str, max_width: f64, font_size: i32) -> Vec<String> {
        let mut lines = Vec::new();
        let words: Vec<&str> = text.split_whitespace().collect();

        if words.is_empty() {
            return vec![String::new()];
        }

        // Estimate char width: ~0.55x font height for proportional fonts
        let char_width = font_size as f64 * 0.55;

        let mut current_line = String::new();

        for word in words {
            let test_line = if current_line.is_empty() {
                word.to_string()
            } else {
                format!("{} {}", current_line, word)
            };

            let width = test_line.chars().count() as f64 * char_width;

            if width <= max_width || current_line.is_empty() {
                current_line = test_line;
            } else {
                lines.push(current_line);
                current_line = word.to_string();
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        if lines.is_empty() {
            lines.push(String::new());
        }

        lines
    }

    /// Extended text rendering with alignment, word wrap, and bold simulation
    pub fn render_html_text_to_bitmap_styled(
        bitmap: &mut Bitmap,
        spans: &[StyledSpan],
        font: &BitmapFont,
        font_bitmap: &Bitmap,
        palettes: &PaletteMap,
        fixed_line_space: u16,
        start_x: i32,
        start_y: i32,
        params: CopyPixelsParams,
        alignment: TextAlignment,
        max_width: i32,
        word_wrap: bool,
    ) -> Result<(), ScriptError> {
        // Get style from first span (for bold/italic simulation and font size)
        let first_style = spans.first().map(|s| &s.style);
        let is_bold = first_style.map(|s| s.bold).unwrap_or(false);
        let _is_italic = first_style.map(|s| s.italic).unwrap_or(false);
        let is_underline = first_style.map(|s| s.underline).unwrap_or(false);

        // Get requested font size from style
        // Font size in points approximately equals pixel height at 96 DPI
        let requested_font_size = first_style
            .and_then(|s| s.font_size)
            .unwrap_or(12) as i32; // Default to 12pt if not specified

        // Scale based on actual character height vs requested pixel height
        // The requested font_size (in points) should map to approximately that many pixels in height
        let native_char_height = font.char_height.max(1) as i32;

        // Calculate scale factor: requested_font_size / native_char_height
        // This ensures a 12pt request scales the font to ~12 pixels tall
        let scaled_char_height = requested_font_size;
        let scaled_char_width = (font.char_width as i32 * requested_font_size) / native_char_height;

        web_sys::console::log_1(&format!(
            "📏 Font scaling: requested={}pt, native_char={}x{} -> scaled={}x{}",
            requested_font_size,
            font.char_width, font.char_height,
            scaled_char_width, scaled_char_height
        ).into());

        // Use scaled dimensions for layout
        let char_width = scaled_char_width.max(1);
        let char_height = scaled_char_height.max(1);

        let line_height = if fixed_line_space > 0 {
            fixed_line_space as i32
        } else {
            char_height
        };

        // Collect all text into a single string for processing
        let full_text: String = spans.iter().map(|s| s.text.as_str()).collect();

        // Split text into lines (respecting existing line breaks)
        let raw_lines: Vec<&str> = full_text.split(|c| c == '\n' || c == '\r').collect();

        // Process lines with word wrap if enabled
        let mut lines: Vec<String> = Vec::new();
        for raw_line in raw_lines {
            if word_wrap && max_width > 0 {
                // Word wrap this line using scaled char width
                let wrapped = Self::word_wrap_line(raw_line, char_width, max_width);
                lines.extend(wrapped);
            } else {
                lines.push(raw_line.to_string());
            }
        }

        // Render each line with alignment
        let mut y = start_y;
        for line in &lines {
            if y >= bitmap.height as i32 {
                break;
            }

            let line_width = line.chars().count() as i32 * char_width;

            // Calculate x position based on alignment
            let x = match alignment {
                TextAlignment::Left => start_x,
                TextAlignment::Center => {
                    if max_width > 0 {
                        start_x + (max_width - line_width) / 2
                    } else {
                        start_x
                    }
                }
                TextAlignment::Right => {
                    if max_width > 0 {
                        start_x + max_width - line_width
                    } else {
                        start_x
                    }
                }
                TextAlignment::Justify => start_x, // TODO: implement justify
            };

            // Render the line
            let mut char_x = x;
            for ch in line.chars() {
                // Skip control characters
                if ch < ' ' {
                    continue;
                }

                // For space character, just advance position without drawing
                if ch == ' ' {
                    char_x += char_width;
                    continue;
                }

                if char_x >= bitmap.width as i32 {
                    break;
                }

                // Draw the character with scaling
                bitmap_font_copy_char_scaled(
                    font, font_bitmap, ch as u8, bitmap,
                    char_x, y, char_width, char_height,
                    palettes, &params
                );

                // Simulate bold by drawing again with 1px offset
                if is_bold {
                    bitmap_font_copy_char_scaled(
                        font, font_bitmap, ch as u8, bitmap,
                        char_x + 1, y, char_width, char_height,
                        palettes, &params
                    );
                }

                // Draw underline at the bottom of the scaled character
                if is_underline {
                    let underline_y = y + char_height - 1;
                    if underline_y < bitmap.height as i32 {
                        for ux in char_x..(char_x + char_width).min(bitmap.width as i32) {
                            if ux >= 0 {
                                let idx = (underline_y as usize * bitmap.width as usize + ux as usize) * 4;
                                if idx + 3 < bitmap.data.len() {
                                    // Draw underline pixel (use foreground color)
                                    bitmap.data[idx] = 0;     // R
                                    bitmap.data[idx + 1] = 0; // G
                                    bitmap.data[idx + 2] = 0; // B
                                    bitmap.data[idx + 3] = 255; // A
                                }
                            }
                        }
                    }
                }

                char_x += char_width;
            }

            y += line_height;
        }

        Ok(())
    }

    /// Word wrap a single line to fit within max_width
    fn word_wrap_line(text: &str, char_width: i32, max_width: i32) -> Vec<String> {
        let mut lines = Vec::new();
        let max_chars = (max_width / char_width).max(1) as usize;

        let words: Vec<&str> = text.split_whitespace().collect();
        let mut current_line = String::new();

        for word in words {
            let word_with_space = if current_line.is_empty() {
                word.to_string()
            } else {
                format!(" {}", word)
            };

            if current_line.len() + word_with_space.len() <= max_chars {
                current_line.push_str(&word_with_space);
            } else {
                if !current_line.is_empty() {
                    lines.push(current_line);
                    current_line = String::new();
                }
                // Handle very long words
                if word.len() > max_chars {
                    let mut remaining = word;
                    while remaining.len() > max_chars {
                        lines.push(remaining[..max_chars].to_string());
                        remaining = &remaining[max_chars..];
                    }
                    current_line = remaining.to_string();
                } else {
                    current_line = word.to_string();
                }
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        if lines.is_empty() {
            lines.push(String::new());
        }

        lines
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
                        is_text_rendering: true,
                        rotation: 0.0,
                        sprite: None,
                        original_dst_rect: None,
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

    fn parse_alignment(value: Datum) -> Result<TextAlignment, ScriptError> {
        let s = value.string_value()?.to_ascii_lowercase();
        match s.as_str() {
            "left" => Ok(TextAlignment::Left),
            "center" => Ok(TextAlignment::Center),
            "right" => Ok(TextAlignment::Right),
            "justify" => Ok(TextAlignment::Justify),
            _ => Err(ScriptError::new(format!(
                "Invalid alignment '{}'",
                s
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
                            "rect" => Ok(Datum::Rect([
                                player.alloc_datum(Datum::Int(0)),
                                player.alloc_datum(Datum::Int(0)),
                                player.alloc_datum(Datum::Int(width as i32)),
                                player.alloc_datum(Datum::Int(height as i32)
                            )])),
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
                                    is_text_rendering: true,
                                    rotation: 0.0,
                                    sprite: None,
                                    original_dst_rect: None,
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
                "text" => Ok(Datum::String(font_data.preview_text.clone())),
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
                    let prop = prop.to_ascii_lowercase();

                    match prop.as_str() {
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
                        "fixedlinespace" => {
                            font_member.fixed_line_space = value.int_value()? as u16
                        }
                        "alignment" => {
                            font_member.alignment = Self::parse_alignment(value)?;
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
