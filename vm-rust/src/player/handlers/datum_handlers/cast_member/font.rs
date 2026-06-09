use std::collections::VecDeque;

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
            bitmap_font_copy_char_scaled, get_text_index_at_pos, measure_text, BitmapFont, DrawTextParams,
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
use log::debug;
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
    pub kerning: i32,       // Kerning amount (from XMED Section 7 dword98, stored as fixed-point * 65536)
    pub char_spacing: i32,  // Character spacing in pixels (from XMED Section 7 dword9C, stored as fixed-point * 65536)
    /// Director chapter 15 `hyperlink` (`director_reference.md:2348`).
    /// Per-character link target — the Lingo-side data string that
    /// `on hyperlinkClick(me, data, range)` receives. We don't currently
    /// fire hyperlinkClick events, but the field is stored so scripts that
    /// inspect `member.word[N].hyperlink` see the assigned value.
    pub hyperlink: Option<String>,
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
            kerning: 0,
            char_spacing: 0,
            hyperlink: None,
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

/// Caret + selection state to draw on top of native-rendered text. Byte
/// offsets index into the concatenation of all `spans[*].text`.
#[derive(Clone, Debug)]
pub struct NativeCaretOverlay {
    pub sel_start: i32,
    pub sel_end: i32,
    pub caret_blink_on: bool,
}

/// Per-character style for the PFR outline-composition renderer
/// (`render_pfr_outline_text_to_bitmap`). One entry per character of the
/// CRLF-normalised text.
#[derive(Clone, Copy, Debug)]
pub struct OutlineCharStyle {
    pub color: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

pub struct FontMemberHandlers {}

impl FontMemberHandlers {
    pub fn call(
        player: &mut DirPlayer,
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let member_ref = player.get_datum(datum).to_member_ref()?;
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(&member_ref)
            .unwrap();
        let text = member.member_type.as_text().unwrap();
        match handler_name {
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
                let (pt_vals, _flags) = player.get_datum(&args[0]).to_point_inline()?;
                let x = pt_vals[0] as i32;
                let y = pt_vals[1] as i32;

                let params = DrawTextParams {
                    font: &player.font_manager.get_system_font().unwrap(),
                    line_height: None,
                    line_spacing: text.fixed_line_space,
                    top_spacing: text.top_spacing,
                    char_spacing: 0,
                    member_width: None,
                    min_space_advance: None,
                    per_char_advances: None,
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

    /// Measure text dimensions using Canvas2D metrics.
    /// This gives accurate measurements for browser-rendered fonts.
    pub fn measure_text_native(
        text: &str,
        font_name: &str,
        font_size: u16,
        word_wrap: bool,
        max_width: i32,
        top_spacing: i16,
        bottom_spacing: i16,
        fixed_line_space: u16,
    ) -> (u16, u16) {
        Self::measure_text_native_styled(
            text, font_name, font_size, None,
            word_wrap, max_width, top_spacing, bottom_spacing, fixed_line_space,
        )
    }

    /// Styled variant — `font_style` matches the bitflag used by the renderer:
    /// bit 0 = bold, bit 1 = italic, bit 2 = underline. Needed because bold/italic
    /// glyphs are measurably wider than regular, and omitting them underestimates
    /// the wrapped line count.
    pub fn measure_text_native_styled(
        text: &str,
        font_name: &str,
        font_size: u16,
        font_style: Option<u8>,
        word_wrap: bool,
        max_width: i32,
        top_spacing: i16,
        bottom_spacing: i16,
        fixed_line_space: u16,
    ) -> (u16, u16) {
        use wasm_bindgen::JsCast;

        let document = match web_sys::window().and_then(|w| w.document()) {
            Some(d) => d,
            None => return (100, font_size.max(12)),
        };
        let canvas: web_sys::HtmlCanvasElement = match document.create_element("canvas") {
            Ok(el) => match el.dyn_into() {
                Ok(c) => c,
                Err(_) => return (100, font_size.max(12)),
            },
            Err(_) => return (100, font_size.max(12)),
        };
        canvas.set_width(1);
        canvas.set_height(1);
        let ctx: web_sys::CanvasRenderingContext2d = match canvas
            .get_context("2d")
            .ok()
            .flatten()
        {
            Some(c) => match c.dyn_into() {
                Ok(ctx) => ctx,
                Err(_) => return (100, font_size.max(12)),
            },
            None => return (100, font_size.max(12)),
        };

        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = font_style {
            if s & 0x02 != 0 { parts.push("italic".to_string()); }
            if s & 0x01 != 0 { parts.push("bold".to_string()); }
        }
        parts.push(format!("{}px", font_size));
        parts.push(font_name.to_string());
        let font_str = parts.join(" ");
        ctx.set_font(&font_str);
        ctx.set_text_baseline("top");

        let wrap_width = if word_wrap && max_width > 0 {
            max_width as f64
        } else {
            f64::MAX
        };

        let line_height = if fixed_line_space > 0 {
            fixed_line_space as f64
        } else {
            font_size as f64
        };
        let line_step = line_height + bottom_spacing as f64 + top_spacing as f64;

        let mut total_width: f64 = 0.0;
        let mut y = top_spacing.max(0) as f64;

        let raw_lines: Vec<&str> = text.split(|c: char| c == '\r' || c == '\n').collect();
        let mut line_count = 0usize;

        for raw in &raw_lines {
            if word_wrap && max_width > 0 && !raw.is_empty() {
                let words: Vec<&str> = raw.split_whitespace().collect();
                let mut current = String::new();
                for word in words {
                    let candidate = if current.is_empty() {
                        word.to_string()
                    } else {
                        format!("{} {}", current, word)
                    };
                    let w = ctx.measure_text(&candidate).map(|m| m.width()).unwrap_or(0.0);
                    if w > wrap_width && !current.is_empty() {
                        let line_w = ctx.measure_text(&current).map(|m| m.width()).unwrap_or(0.0);
                        total_width = total_width.max(line_w);
                        line_count += 1;
                        current = word.to_string();
                    } else {
                        current = candidate;
                    }
                }
                if !current.is_empty() {
                    let line_w = ctx.measure_text(&current).map(|m| m.width()).unwrap_or(0.0);
                    total_width = total_width.max(line_w);
                    line_count += 1;
                }
            } else {
                let line_w = ctx.measure_text(raw).map(|m| m.width()).unwrap_or(0.0);
                total_width = total_width.max(line_w);
                line_count += 1;
            }
        }

        // Calculate total height
        let total_height = if line_count <= 1 {
            top_spacing.max(0) as f64 + line_height
        } else {
            top_spacing.max(0) as f64 + line_height + (line_count as f64 - 1.0) * line_step
        };

        (total_width.ceil().max(1.0) as u16, total_height.ceil().max(1.0) as u16)
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
        fixed_line_space: u16,
        top_spacing: i16,
        bottom_spacing: i16,
        tab_stops: &[crate::player::cast_member::TabStop],
        // XMED per-paragraph spacing tables. When non-empty, the renderer
        // applies `\sa`/`\sb` (RTF "space after"/"space before", Director's
        // par_info `top_spacing`/`bottom_spacing`) at paragraph boundaries
        // — matching Director's gap between the description and the labeled
        // rows on info-card text members (Junkbot V7 Buggy). Pass empty
        // slices for callers that don't have par_info data; the renderer
        // falls back to the legacy global `top_spacing`/`bottom_spacing`
        // applied per-line.
        par_infos: &[crate::director::chunks::xmedia_styled_text::ParInfo],
        par_runs: &[crate::director::chunks::xmedia_styled_text::ParRun],
    ) -> Result<(), ScriptError> {
        Self::render_native_text_to_bitmap_with_caret(
            bitmap, spans, start_x, start_y, render_width, render_height,
            alignment, max_width, word_wrap, sprite_color, fixed_line_space,
            top_spacing, bottom_spacing, tab_stops, par_infos, par_runs, None,
        )
    }

    /// Same as `render_native_text_to_bitmap` but optionally draws a caret +
    /// selection highlight on top of the rendered text. Positions are computed
    /// from Canvas2D `measureText` (the same metrics used for layout) so the
    /// caret/selection align with what the user sees, regardless of which
    /// system font Canvas2D ends up choosing.
    pub fn render_native_text_to_bitmap_with_caret(
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
        fixed_line_space: u16,
        top_spacing: i16,
        bottom_spacing: i16,
        tab_stops: &[crate::player::cast_member::TabStop],
        par_infos: &[crate::director::chunks::xmedia_styled_text::ParInfo],
        par_runs: &[crate::director::chunks::xmedia_styled_text::ParRun],
        caret_overlay: Option<NativeCaretOverlay>,
    ) -> Result<(), ScriptError> {
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

        // Render at 2x scale for sharper text (Canvas2D anti-aliasing at 1x
        // produces low coverage for small characters like ':'), then downscale.
        let scale_factor = 2u32;
        let canvas_width = render_width.max(1) as u32 * scale_factor;
        let canvas_height = render_height.max(1) as u32 * scale_factor;
        canvas.set_width(canvas_width);
        canvas.set_height(canvas_height);

        let ctx: web_sys::CanvasRenderingContext2d = canvas
            .get_context("2d")
            .map_err(|_| ScriptError::new("Failed to get 2d context".to_string()))?
            .ok_or_else(|| ScriptError::new("No 2d context".to_string()))?
            .dyn_into()
            .map_err(|_| ScriptError::new("Failed to cast context".to_string()))?;

        // Scale the context so text renders at 2x resolution
        let _ = ctx.scale(scale_factor as f64, scale_factor as f64);

        // Render WHITE text on BLACK background to measure pure coverage.
        // This avoids color-dependent anti-aliasing artifacts from Canvas2D.
        // Coverage is then mapped to the bitmap as: black text on white background.
        ctx.set_fill_style_str("rgb(0,0,0)");
        ctx.fill_rect(0.0, 0.0, render_width.max(1) as f64, render_height.max(1) as f64);

        // Set text baseline to top for consistent positioning
        ctx.set_text_baseline("top");

        #[derive(Clone)]
        struct NativeSegmentStyle {
            font: String,
            size_px: f64,
            underline: bool,
            color: (u8, u8, u8),
        }

        #[derive(Clone)]
        struct NativeSegment {
            text: String,
            width: f64,
            style: NativeSegmentStyle,
            is_tab: bool, // Tab marker - width is placeholder, resolved during rendering
            // Byte offset of this segment's first char in the concatenation of
            // all spans[*].text. Used by the optional caret/selection overlay
            // to map byte offsets back to pixel positions.
            start_byte: usize,
        }

        #[derive(Default)]
        struct NativeLine {
            segments: Vec<NativeSegment>,
            width: f64,
            max_font_px: f64,
            // Cumulative text-character position where this line begins.
            // Used to look up which paragraph (par_info) this line belongs
            // to via par_runs. Wrap-induced visual lines get a position
            // somewhere in their source paragraph, which still resolves to
            // the same par_info index — boundary spacing only triggers on
            // genuine source-paragraph transitions.
            start_text_pos: u32,
        }

        let fallback_color = if let Some(color_ref) = sprite_color {
            match color_ref {
                ColorRef::Rgb(r, g, b) => (*r, *g, *b),
                ColorRef::PaletteIndex(idx) => match *idx {
                    0 => (255, 255, 255),
                    255 => (0, 0, 0),
                    _ => (0, 0, 0),
                },
            }
        } else {
            (0, 0, 0)
        };

        let style_from_html = |style: &HtmlStyle| -> NativeSegmentStyle {
            let size_px = style.font_size.filter(|s| *s > 0).unwrap_or(12) as f64;
            let font_face = style
                .font_face
                .clone()
                .filter(|f| !f.is_empty())
                .unwrap_or_else(|| "Arial".to_string());
            let mut font_parts: Vec<String> = Vec::new();
            if style.bold {
                font_parts.push("bold".to_string());
            }
            if style.italic {
                font_parts.push("italic".to_string());
            }
            font_parts.push(format!("{}px", size_px as i32));
            font_parts.push(font_face);

            let color = if let Some(c) = style.color {
                (((c >> 16) & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, (c & 0xFF) as u8)
            } else {
                fallback_color
            };

            NativeSegmentStyle {
                font: font_parts.join(" "),
                size_px,
                underline: style.underline,
                color,
            }
        };

        let mut lines: Vec<NativeLine> = Vec::new();
        let mut current_line = NativeLine::default();
        let wrap_width = if word_wrap && max_width > 0 {
            max_width as f64
        } else {
            f64::MAX
        };
        // Cumulative text-character position across all spans. Used to
        // stamp `start_text_pos` on each new line so we can map lines to
        // par_runs / par_infos at draw time for paragraph-boundary
        // spacing.
        let mut cumulative_pos: u32 = 0;

        let mut push_line = |line: &mut NativeLine, lines_out: &mut Vec<NativeLine>| {
            lines_out.push(std::mem::take(line));
        };

        // Cumulative byte offset across all spans — feeds segment.start_byte
        // so the optional caret/selection overlay can map byte offsets back
        // to pixel positions.
        let mut cur_byte_offset: usize = 0;

        for span in spans {
            if span.text.is_empty() {
                continue;
            }
            let seg_style = style_from_html(&span.style);

            let mut token = String::new();
            let mut token_is_ws: Option<bool> = None;
            let mut token_start_byte: usize = cur_byte_offset;

            let mut flush_token = |token_text: &mut String,
                                   is_ws: Option<bool>,
                                   line: &mut NativeLine,
                                   lines_out: &mut Vec<NativeLine>,
                                   style: &NativeSegmentStyle,
                                   start_byte: usize| {
                if token_text.is_empty() {
                    return;
                }
                let is_whitespace = is_ws.unwrap_or(false);
                ctx.set_font(&style.font);
                let token_width = ctx
                    .measure_text(token_text)
                    .map(|m| m.width())
                    .unwrap_or_else(|_| token_text.chars().count() as f64 * (style.size_px * 0.55));

                // If the line has a tab marker, text after the tab is positioned by
                // the tab stop (e.g. right-aligned), so it doesn't increase line width
                // and should not trigger word wrap.
                let has_tab = line.segments.iter().any(|s| s.is_tab);
                let would_overflow = !has_tab && line.width + token_width > wrap_width;
                let will_wrap = would_overflow && !line.segments.is_empty() && !is_whitespace;

                if will_wrap {
                    lines_out.push(std::mem::take(line));
                }

                if !(is_whitespace && line.segments.is_empty()) {
                    line.max_font_px = line.max_font_px.max(style.size_px);
                    if !has_tab {
                        line.width += token_width;
                    }
                    line.segments.push(NativeSegment {
                        text: token_text.clone(),
                        width: token_width,
                        style: style.clone(),
                        is_tab: false,
                        start_byte,
                    });
                }

                token_text.clear();
            };

            let mut prev_was_cr = false;
            for ch in span.text.chars() {
                let ch_byte_len = ch.len_utf8();
                // Director (Mac origin) uses \r for line breaks.
                // Handle \r, \n, and \r\n without double-breaking.
                if ch == '\n' && prev_was_cr {
                    // Skip \n after \r (already broke on \r) but still
                    // count it toward cumulative text position so the
                    // next line's start_text_pos lines up with par_runs.
                    prev_was_cr = false;
                    cumulative_pos += 1;
                    current_line.start_text_pos = cumulative_pos;
                    cur_byte_offset += ch_byte_len;
                    token_start_byte = cur_byte_offset;
                    continue;
                }
                prev_was_cr = ch == '\r';
                if ch == '\r' || ch == '\n' {
                    flush_token(
                        &mut token,
                        token_is_ws,
                        &mut current_line,
                        &mut lines,
                        &seg_style,
                        token_start_byte,
                    );
                    token_is_ws = None;
                    push_line(&mut current_line, &mut lines);
                    cumulative_pos += 1;
                    current_line.start_text_pos = cumulative_pos;
                    cur_byte_offset += ch_byte_len;
                    token_start_byte = cur_byte_offset;
                    continue;
                }
                if ch == '\t' {
                    flush_token(
                        &mut token,
                        token_is_ws,
                        &mut current_line,
                        &mut lines,
                        &seg_style,
                        token_start_byte,
                    );
                    token_is_ws = None;
                    // Count how many tabs we've seen so far on this line
                    let tab_idx = current_line.segments.iter().filter(|s| s.is_tab).count();
                    // Insert a tab marker segment
                    current_line.segments.push(NativeSegment {
                        text: String::new(),
                        width: 0.0,
                        style: seg_style.clone(),
                        is_tab: true,
                        start_byte: cur_byte_offset,
                    });
                    // Update line.width to the tab stop position so subsequent
                    // flush_token calls don't cause false word-wrap overflow
                    if tab_idx < tab_stops.len() {
                        let stop_pos = tab_stops[tab_idx].position as f64;
                        if stop_pos > current_line.width {
                            current_line.width = stop_pos;
                        }
                    }
                    cumulative_pos += 1;
                    cur_byte_offset += ch_byte_len;
                    token_start_byte = cur_byte_offset;
                    continue;
                }

                let is_ws = ch.is_whitespace();
                if token_is_ws != Some(is_ws) && !token.is_empty() {
                    flush_token(
                        &mut token,
                        token_is_ws,
                        &mut current_line,
                        &mut lines,
                        &seg_style,
                        token_start_byte,
                    );
                    token_start_byte = cur_byte_offset;
                }
                token_is_ws = Some(is_ws);
                token.push(ch);
                cumulative_pos += 1;
                cur_byte_offset += ch_byte_len;
            }

            flush_token(
                &mut token,
                token_is_ws,
                &mut current_line,
                &mut lines,
                &seg_style,
                token_start_byte,
            );
            token_start_byte = cur_byte_offset;
        }

        if !current_line.segments.is_empty() || lines.is_empty() {
            lines.push(current_line);
        }


        // Map a text-character position to its par_info index via par_runs.
        // Returns None when no par_info data is available so callers can
        // skip boundary-spacing logic and use legacy uniform per-line
        // top/bottom spacing.
        let line_par_idx = |pos: u32| -> Option<u16> {
            if par_runs.is_empty() || par_infos.is_empty() {
                return None;
            }
            let mut active: Option<u16> = None;
            for run in par_runs {
                if run.position <= pos {
                    active = Some(run.par_info_index);
                } else {
                    break;
                }
            }
            active
        };

        // Per-line top-y / line-height in 1x logical canvas pixels (same units
        // as the rendering loop's `y`). Captured during render so the optional
        // caret/selection overlay can place rects in the same positions later.
        let mut line_positions: Vec<(f64, f64)> = Vec::new();

        // Per-styled-run color regions, in logical (1x) coords. The canvas is
        // drawn at 2x via ctx.scale, and output pixel (cx,cy) maps back to
        // logical (cx,cy), so these rects can be matched directly against
        // output pixels in the downscale below. This lets each STXT run keep
        // its own color (the red "- 50 POINTS - Get Stung!" line in SpongeBob's
        // Scoring field) instead of the whole field using one color. Font
        // size/face/bold/italic/underline are already per-segment (set_font /
        // underline above); only color needed this per-pixel routing because
        // the text is rendered white-on-black for coverage.
        let mut seg_color_rects: Vec<(f64, f64, f64, f64, (u8, u8, u8))> = Vec::new();

        let mut y = top_spacing.max(0) as f64;
        let mut prev_par_idx: Option<u16> = None;
        for line in &lines {
            if y >= canvas_height as f64 {
                break;
            }

            // Apply paragraph-boundary spacing at par_info transitions.
            // Wrap-induced lines stay within one paragraph (same par_info
            // index from `line_par_idx(line.start_text_pos)`), so this
            // only fires at genuine source-paragraph transitions.
            //
            // Use `prev.bottom_spacing + this.top_spacing` directly from
            // par_infos — the standard RTF interpretation of `\sa`/`\sb`.
            let this_par_idx = line_par_idx(line.start_text_pos);
            if let (Some(prev), Some(this)) = (prev_par_idx, this_par_idx) {
                if prev != this {
                    let prev_sa = par_infos
                        .get(prev as usize)
                        .map(|pi| pi.bottom_spacing)
                        .unwrap_or(0);
                    let this_sb = par_infos
                        .get(this as usize)
                        .map(|pi| pi.top_spacing)
                        .unwrap_or(0);
                    y += (prev_sa + this_sb) as f64;
                }
            }

            // Alignment uses the full canvas width (`render_width`), NOT
            // `max_width`. `max_width` is the wrap fold-point — for a
            // field whose `field.width` is narrower than the sprite
            // display rect (Habbo's hotel-navigator help-text panel,
            // sprite_rect=348 / field.width=324), we want lines to fold
            // at field.width but center within the full sprite width so
            // the text aligns under the field's title bar / box. For
            // non-field text the two widths are equal so behavior is
            // unchanged.
            let align_width = render_width.max(0) as f64;
            let x_start = match alignment {
                TextAlignment::Left => 0.0,
                TextAlignment::Center => {
                    if align_width > 0.0 {
                        (align_width - line.width) / 2.0
                    } else {
                        0.0
                    }
                }
                TextAlignment::Right => {
                    if align_width > 0.0 {
                        align_width - line.width
                    } else {
                        0.0
                    }
                }
                TextAlignment::Justify => 0.0,
            };

            let mut x = x_start.max(0.0);
            let mut tab_index = 0usize;
            for (seg_i, segment) in line.segments.iter().enumerate() {
                if segment.is_tab {
                    // Advance to the next tab stop
                    if tab_index < tab_stops.len() {
                        let stop = &tab_stops[tab_index];
                        match stop.tab_type.as_str() {
                            "right" => {
                                // For right-aligned tabs, measure remaining text after the tab
                                let remaining_width: f64 = line.segments[seg_i + 1..]
                                    .iter()
                                    .filter(|s| !s.is_tab)
                                    .map(|s| s.width)
                                    .sum();
                                x = (stop.position as f64 - remaining_width).max(x);
                            }
                            "center" => {
                                let remaining_width: f64 = line.segments[seg_i + 1..]
                                    .iter()
                                    .filter(|s| !s.is_tab)
                                    .map(|s| s.width)
                                    .sum();
                                x = (stop.position as f64 - remaining_width / 2.0).max(x);
                            }
                            _ => {
                                // Left tab: just advance to position
                                x = (stop.position as f64).max(x);
                            }
                        }
                        tab_index += 1;
                    }
                    continue;
                }

                ctx.set_font(&segment.style.font);
                // Always render in WHITE on the black canvas for coverage measurement
                ctx.set_fill_style_str("rgb(255,255,255)");
                let _ = ctx.fill_text(&segment.text, x, y);

                // Record this segment's color region (logical coords, baseline
                // is "top" so the glyph cell is [y, y+size_px]) for per-run
                // color lookup during the downscale.
                seg_color_rects.push((
                    x,
                    x + segment.width,
                    y,
                    y + segment.style.size_px,
                    segment.style.color,
                ));

                if segment.style.underline {
                    let underline_y = y + segment.style.size_px - 1.0;
                    ctx.begin_path();
                    ctx.move_to(x, underline_y);
                    ctx.line_to(x + segment.width, underline_y);
                    ctx.set_stroke_style_str("rgb(255,255,255)");
                    ctx.stroke();
                }

                x += segment.width;
            }

            let line_font_px = if line.max_font_px > 0.0 {
                line.max_font_px
            } else {
                12.0
            };
            // Per-line stride priority (matches the PFR-styled multi-span
            // path):
            //  1. This line's `par_info.line_spacing` when non-zero. Director
            //     stores per-paragraph stride here (RTF `\sl<twips>`) and
            //     uses it in preference to the global `fixedLineSpace`. For
            //     empty paragraphs (no glyphs on the line) we apply the
            //     authored value verbatim; for content lines we clamp to
            //     the line's font size to avoid clipping characters when a
            //     small `\sl` value sits below the glyph cell — this matches
            //     Director treating `\sl` as a minimum on content lines.
            //     Junkbot V1 brick-info member 139 uses par_info[2] with
            //     line_spacing=6 for the single empty line before "eyeBOT";
            //     without this lookup the stride defaults to 12 and pushes
            //     eyeBOT ~6 px below Director's layout.
            //  2. Global `fixed_line_space` (member-level fixedLineSpace).
            //  3. The line's own `max_font_px` (the natural glyph cell).
            let par_line_spacing = this_par_idx
                .and_then(|idx| par_infos.get(idx as usize))
                .map(|pi| pi.line_spacing)
                .filter(|&s| s > 0)
                .map(|s| s as f64);
            let effective_line_height = if let Some(par_ls) = par_line_spacing {
                if line.segments.is_empty() {
                    par_ls
                } else {
                    par_ls.max(line_font_px)
                }
            } else if fixed_line_space > 0 {
                fixed_line_space as f64
            } else {
                line_font_px
            };
            line_positions.push((y, effective_line_height));
            // Per-line stride is line height + bottom_spacing only.
            // top_spacing is the initial offset before the first line (added
            // once when y was initialized) — adding it again per line stacks
            // extra space between every visual line, which is most visible
            // when a user presses Enter in an editable field and the new
            // line appears way below the previous one instead of one line
            // height down.
            y += effective_line_height + bottom_spacing as f64;
            prev_par_idx = this_par_idx;
        }

        // Get pixel data from canvas
        let image_data = ctx
            .get_image_data(0.0, 0.0, canvas_width as f64, canvas_height as f64)
            .map_err(|_| ScriptError::new("Failed to get image data".to_string()))?;

        let pixels = image_data.data();

        // Debug: check raw canvas pixels for non-black (text) content
        {
            let total_px = (canvas_width * canvas_height) as usize;
            let mut nonblack = 0usize;
            let mut first_nb = String::new();
            for i in 0..total_px {
                let idx = i * 4;
                if idx + 2 < pixels.len() {
                    let r = pixels[idx];
                    let g = pixels[idx + 1];
                    let b = pixels[idx + 2];
                    if r > 0 || g > 0 || b > 0 {
                        nonblack += 1;
                        if first_nb.is_empty() {
                            let x = i % canvas_width as usize;
                            let y = i / canvas_width as usize;
                            first_nb = format!("({},{})=({},{},{})", x, y, r, g, b);
                        }
                    }
                }
            }
            let text_preview = spans.first().map(|s| &s.text[..s.text.len().min(5)]).unwrap_or("?");
            debug!(
                "[canvas-debug] text='{}' canvas={}x{} nonblack={}/{} first={}",
                text_preview, canvas_width, canvas_height, nonblack, total_px,
                if first_nb.is_empty() { "NONE".to_string() } else { first_nb }
            );
        }

        // Downscale 2x canvas (white-on-black) to 1x bitmap (black-on-white).
        // Coverage = average luminance of sf×sf block. Then invert: output = 255 - coverage.
        // This gives black text body on white background with proper anti-aliasing.
        let out_w = render_width.max(1) as usize;
        let out_h = render_height.max(1) as usize;
        let sf = scale_factor as usize;

        for cy in 0..out_h {
            let dest_y = start_y + cy as i32;
            if dest_y < 0 || dest_y >= bitmap.height as i32 {
                continue;
            }

            for cx in 0..out_w {
                let dest_x = start_x + cx as i32;
                if dest_x < 0 || dest_x >= bitmap.width as i32 {
                    continue;
                }

                // Average luminance of the sf×sf pixel block (white text on black bg)
                let mut lum_sum = 0u32;
                let count = (sf * sf) as u32;
                for sy in 0..sf {
                    for sx in 0..sf {
                        let px = cx * sf + sx;
                        let py = cy * sf + sy;
                        let src_idx = (py * canvas_width as usize + px) * 4;
                        if src_idx + 2 < pixels.len() {
                            // Use green channel as luminance proxy (most significant for perception)
                            lum_sum += pixels[src_idx + 1] as u32;
                        }
                    }
                }
                let coverage = (lum_sum / count) as u8; // 0=background, 255=text body

                // Only write pixels where text was actually rendered (coverage > 0).
                // Background pixels stay at the bitmap's pre-fill (transparent for text.image).
                if coverage == 0 { continue; }

                // Write foreColor with coverage-based alpha.
                // This produces: text pixels = foreColor at full/partial alpha,
                // background = transparent (preserved from bitmap pre-fill).
                let dest_idx = (dest_y as usize * bitmap.width as usize + dest_x as usize) * 4;
                if dest_idx + 3 < bitmap.data.len() {
                    // Per-run color: find the styled segment covering this
                    // pixel (output coords == logical coords). segment.style.color
                    // already folds in the field default for runs with no
                    // explicit color, so unmatched pixels fall back to it too.
                    let (fg_r, fg_g, fg_b) = {
                        let fx = cx as f64;
                        let fy = cy as f64;
                        seg_color_rects
                            .iter()
                            .find(|&&(x0, x1, yt, yb, _)| fx >= x0 && fx < x1 && fy >= yt && fy < yb)
                            .map(|&(_, _, _, _, c)| c)
                            .unwrap_or(fallback_color)
                    };
                    bitmap.data[dest_idx] = fg_r;
                    bitmap.data[dest_idx + 1] = fg_g;
                    bitmap.data[dest_idx + 2] = fg_b;
                    // Coverage maps to alpha: text body=255, anti-aliased edges=partial
                    bitmap.data[dest_idx + 3] = coverage.min(255);
                }
            }
        }

        // Optional caret + selection overlay. Drawn with Canvas2D-measured
        // positions so the caret aligns with what was actually rendered,
        // including for center/right alignment where the bitmap font's
        // advances would disagree with the browser font's measureText.
        if let Some(overlay) = caret_overlay {
            let palette_map = PaletteMap::new();
            let sel_lo = overlay.sel_start.min(overlay.sel_end).max(0) as usize;
            let sel_hi = overlay.sel_start.max(overlay.sel_end).max(0) as usize;
            let has_selection = sel_hi > sel_lo;
            let selection_color: (u8, u8, u8) = (164, 205, 255);

            for (line_idx, line) in lines.iter().enumerate() {
                let (line_top_y, line_h) = match line_positions.get(line_idx) {
                    Some(&p) => p,
                    None => continue,
                };
                let x_start = match alignment {
                    TextAlignment::Left => 0.0,
                    TextAlignment::Center => {
                        if max_width > 0 { ((max_width as f64) - line.width) / 2.0 } else { 0.0 }
                    }
                    TextAlignment::Right => {
                        if max_width > 0 { (max_width as f64) - line.width } else { 0.0 }
                    }
                    TextAlignment::Justify => 0.0,
                }.max(0.0);
                let line_byte_start = line.segments.first().map(|s| s.start_byte).unwrap_or(0);
                let line_byte_end = line.segments.last()
                    .map(|s| s.start_byte + s.text.len())
                    .unwrap_or(line_byte_start);

                // Resolve a byte offset within this line to a pixel x by walking
                // segments and using Canvas2D measureText for the prefix inside
                // the segment that contains the offset.
                let pixel_x_for_byte = |byte: usize, ctx: &web_sys::CanvasRenderingContext2d| -> f64 {
                    let mut x = x_start;
                    for segment in &line.segments {
                        if segment.is_tab { continue; }
                        let seg_end = segment.start_byte + segment.text.len();
                        if byte <= segment.start_byte {
                            return x;
                        }
                        if byte >= seg_end {
                            x += segment.width;
                            continue;
                        }
                        let inner = byte - segment.start_byte;
                        // Snap to a UTF-8 char boundary -- selection
                        // indices can land mid-codepoint (e.g. inside ß)
                        // and a raw slice would panic. Rounding DOWN
                        // means the prefix excludes a half-finished char.
                        let mut clamped = inner.min(segment.text.len());
                        while clamped > 0 && !segment.text.is_char_boundary(clamped) { clamped -= 1; }
                        let prefix = &segment.text[..clamped];
                        ctx.set_font(&segment.style.font);
                        let prefix_w = ctx.measure_text(prefix)
                            .map(|m| m.width())
                            .unwrap_or(0.0);
                        return x + prefix_w;
                    }
                    x
                };

                if has_selection {
                    if sel_hi <= line_byte_start || sel_lo > line_byte_end {
                        continue;
                    }
                    let from = sel_lo.max(line_byte_start);
                    let to = sel_hi.min(line_byte_end);
                    let x0 = pixel_x_for_byte(from, &ctx);
                    let mut x1 = pixel_x_for_byte(to, &ctx);
                    if x1 <= x0 && sel_hi > line_byte_end {
                        // Selection wraps to next line — show the trailing space.
                        x1 = x0 + line.max_font_px.max(8.0) * 0.4;
                    }
                    if x1 <= x0 { continue; }
                    let left = (start_x + (x0).round() as i32).max(0);
                    let right = (start_x + (x1).round() as i32).max(0);
                    let top = (start_y + line_top_y.round() as i32).max(0);
                    let bottom = (top + line_h.round() as i32)
                        .min(bitmap.height as i32);
                    let right = right.min(bitmap.width as i32);
                    let left = left.min(right);
                    // Composite the selection BEHIND the already-rasterized
                    // text. The bitmap currently has text glyphs with alpha
                    // 0..=255 (255 = solid glyph body, 0 = transparent
                    // background). For each pixel in the selection rect:
                    //   alpha == 0  → fill with selection color (was empty)
                    //   alpha < 255 → blend selection under the partial glyph
                    //   alpha == 255 → leave the glyph alone
                    // This gives the standard "highlight rect with text on
                    // top" look instead of a translucent overlay tinting the
                    // glyphs.
                    for py in top..bottom {
                        for px in left..right {
                            let idx = ((py as usize) * bitmap.width as usize + px as usize) * 4;
                            if idx + 3 >= bitmap.data.len() { continue; }
                            let a = bitmap.data[idx + 3] as u32;
                            if a == 0 {
                                bitmap.data[idx]     = selection_color.0;
                                bitmap.data[idx + 1] = selection_color.1;
                                bitmap.data[idx + 2] = selection_color.2;
                                bitmap.data[idx + 3] = 255;
                            } else if a < 255 {
                                let inv = 255 - a;
                                bitmap.data[idx]     = ((bitmap.data[idx]     as u32 * a + selection_color.0 as u32 * inv) / 255) as u8;
                                bitmap.data[idx + 1] = ((bitmap.data[idx + 1] as u32 * a + selection_color.1 as u32 * inv) / 255) as u8;
                                bitmap.data[idx + 2] = ((bitmap.data[idx + 2] as u32 * a + selection_color.2 as u32 * inv) / 255) as u8;
                                bitmap.data[idx + 3] = 255;
                            }
                        }
                    }
                } else if overlay.caret_blink_on {
                    let target = overlay.sel_end as usize;
                    if target < line_byte_start || target > line_byte_end {
                        continue;
                    }
                    let cx = pixel_x_for_byte(target, &ctx);
                    let left = start_x + cx.round() as i32;
                    let top = start_y + line_top_y.round() as i32;
                    let bottom = top + line_h.round() as i32;
                    bitmap.fill_rect(left, top, left + 1, bottom, (0, 0, 0), &palette_map, 1.0);
                }
            }
        }

        Ok(())
    }

    /// Sub-pixel outline composition for PFR **outline** fonts.
    ///
    /// Unlike the atlas-copy path (`text.rs::flush_line` → `bitmap_font_copy_char`),
    /// which pre-rasterizes each glyph into an integer-pixel cell and advances
    /// the pen by an integer pixel count, this draws each glyph's actual outline
    /// onto a 2× Canvas2D at a fractional cursor position and advances by the
    /// fractional `set_width`. At small sizes (Coke Studios' Verdana 10px) the
    /// sub-pixel side bearings survive as anti-aliased coverage instead of
    /// quantizing to zero, so adjacent round letters no longer merge
    /// ("London" → "Lurdur"). Mirrors the FontinatorFINAL SkiaSharp reference
    /// (`px = cmd.X·scale + cursorX`, baseline = round(ascender·scale)).
    ///
    /// Draws each char in its own foreColor on a transparent canvas, then
    /// alpha-weighted-downscales into `bitmap` — so per-span colour (CS
    /// audition rows = blue, special = red), bold (double-draw), italic (shear)
    /// and underline are all honoured. `per_char` is indexed by character
    /// position in the CRLF-normalised text (same convention as flush_line).
    pub fn render_pfr_outline_text_to_bitmap(
        bitmap: &mut Bitmap,
        parsed: &crate::director::chunks::pfr1::types::Pfr1ParsedFont,
        font_size: u16,
        text: &str,
        per_char: &[OutlineCharStyle],
        default_style: OutlineCharStyle,
        start_x: i32,
        start_y: i32,
        render_width: i32,
        render_height: i32,
        alignment: TextAlignment,
        max_width: i32,
        word_wrap: bool,
        fixed_line_space: u16,
        top_spacing: i16,
        bottom_spacing: i16,
        char_spacing: i32,
        tab_stops: &[crate::player::cast_member::TabStop],
    ) -> Result<(), ScriptError> {
        use crate::io::encoding::glyph_byte_for;

        let phys = &parsed.physical_font;
        let outline_res = phys.outline_resolution as f64;
        if outline_res <= 0.0 || font_size == 0 {
            return Err(ScriptError::new("PFR outline: bad metrics".to_string()));
        }
        let scale = font_size as f64 / outline_res;
        let m = parsed
            .logical_fonts
            .get(0)
            .map(|l| l.font_matrix)
            .unwrap_or([256, 0, 0, 256]);
        let matrix_sx = m[0] as f64 / 256.0;
        let matrix_sy = m[3] as f64 / 256.0;
        let scale_x = scale * matrix_sx.abs();
        let scale_y_mag = scale * matrix_sy.abs();
        // Same Y orientation rule as the rasterizer: when the font matrix
        // already flips Y (m[3] < 0) the parsed coords are Y-down, so we use a
        // positive scale; otherwise flip.
        let glyph_scale_y = if matrix_sy < 0.0 { scale_y_mag } else { -scale_y_mag };
        let baseline = (phys.metrics.ascender as f64 * scale).round();

        // Per-char advance from the glyph's set_width (fractional → sub-pixel).
        let advance_of = |code: u8| -> f64 {
            let sw = parsed
                .glyphs
                .get(&code)
                .map(|g| g.set_width as f64)
                .unwrap_or(outline_res * 0.5);
            sw * scale + char_spacing as f64
        };
        let style_at = |idx: usize| -> OutlineCharStyle {
            per_char.get(idx).copied().unwrap_or(default_style)
        };

        // --- Build visual lines, tracking each char's original index into
        // `per_char` so styling stays aligned even after word-wrap drops a
        // break space. A line is a Vec of (char, original_index). Tabs are
        // kept inline (their glyph_byte is 9, zero ink).
        let normalised: String = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut lines: Vec<Vec<(char, usize)>> = Vec::new();
        {
            let chars: Vec<char> = normalised.chars().collect();
            let wrap_w = if word_wrap && max_width > 0 { max_width as f64 } else { f64::MAX };
            let mut cur: Vec<(char, usize)> = Vec::new();
            let mut cur_w: f64 = 0.0;
            let mut last_space: Option<usize> = None; // index within `cur`
            let mut width_to: f64 = 0.0; // width up to last_space (exclusive of space)
            for (i, &c) in chars.iter().enumerate() {
                if c == '\n' {
                    lines.push(std::mem::take(&mut cur));
                    cur_w = 0.0; last_space = None; width_to = 0.0;
                    continue;
                }
                let adv = if c == '\t' { 0.0 } else { advance_of(glyph_byte_for(c)) };
                let has_tab = cur.iter().any(|&(ch, _)| ch == '\t');
                if word_wrap && !has_tab && cur_w + adv > wrap_w && !cur.is_empty() {
                    if let Some(sp) = last_space {
                        // Break at the last space: move the tail to a new line.
                        let tail: Vec<(char, usize)> = cur.split_off(sp + 1);
                        cur.pop(); // drop the break space itself
                        lines.push(std::mem::take(&mut cur));
                        cur = tail;
                        cur_w = cur.iter()
                            .map(|&(ch, _)| if ch == '\t' { 0.0 } else { advance_of(glyph_byte_for(ch)) })
                            .sum();
                        let _ = width_to;
                        last_space = None;
                    } else {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0.0; last_space = None;
                    }
                }
                if c == ' ' { last_space = Some(cur.len()); width_to = cur_w; }
                cur.push((c, i));
                cur_w += adv;
            }
            lines.push(cur);
        }

        // --- 5× supersampled software raster buffer (straight RGBA). ---
        // No Canvas2D: rasterizing the outlines in Rust makes the output
        // byte-identical on every platform/browser. The Canvas2D path produced
        // different anti-aliasing on Linux vs Windows (broken kerning + a
        // discontinuous underline) because it fed the browser's platform-
        // specific canvas coverage into the hard LO/HI threshold below.
        // Box-downscaled 4×→1× further down for anti-aliasing.
        let sf = 5u32;
        let cw2 = (render_width.max(1) as u32) * sf;
        let ch2 = (render_height.max(1) as u32) * sf;
        let cw2u = cw2 as usize;
        let ch2u = ch2 as usize;
        let mut buf = vec![0u8; cw2u * ch2u * 4];

        let bold_off = (font_size as f64 * 0.04).max(0.5);

        let line_natural = (((phys.metrics.ascender - phys.metrics.descender) as f64) * scale)
            .round()
            .max(1.0);
        let effective_line_h = if fixed_line_space > 0 {
            fixed_line_space as f64
        } else {
            line_natural
        };
        let line_step = effective_line_h + bottom_spacing as f64 + top_spacing as f64;

        let seg_width = |seg: &[(char, usize)]| -> f64 {
            seg.iter().map(|&(c, _)| advance_of(glyph_byte_for(c))).sum()
        };
        let has_right_tab = tab_stops.iter().any(|t| t.tab_type == "right");

        let mut y_top = top_spacing as f64;
        for line in &lines {
            if y_top >= render_height as f64 { break; }
            let baseline_y = y_top + baseline;

            // Split into tab segments.
            let mut segments: Vec<Vec<(char, usize)>> = vec![Vec::new()];
            for &(c, idx) in line {
                if c == '\t' {
                    segments.push(Vec::new());
                } else {
                    segments.last_mut().unwrap().push((c, idx));
                }
            }

            // Logical width for alignment (when no right-tab anchors the edge).
            let logical_w: f64 = if segments.len() == 1 {
                seg_width(&segments[0])
            } else {
                let mut acc = seg_width(&segments[0]);
                for i in 1..segments.len() {
                    let sw = seg_width(&segments[i]);
                    if let Some(stop) = tab_stops.get(i - 1) {
                        let pos = stop.position as f64;
                        acc = match stop.tab_type.as_str() {
                            "right" => pos,
                            "center" => (pos + sw / 2.0).max(acc),
                            _ => (pos + sw).max(acc + sw),
                        };
                    } else {
                        acc += sw;
                    }
                }
                acc
            };
            let line_offset = if has_right_tab {
                0.0
            } else {
                match alignment {
                    TextAlignment::Center => ((max_width as f64 - logical_w) / 2.0).max(0.0),
                    TextAlignment::Right => (max_width as f64 - logical_w).max(0.0),
                    _ => 0.0,
                }
            };

            // Resolve each segment's start x.
            let mut seg_starts: Vec<f64> = Vec::with_capacity(segments.len());
            seg_starts.push(line_offset);
            let mut cursor = line_offset + seg_width(&segments[0]);
            for i in 1..segments.len() {
                let sw = seg_width(&segments[i]);
                let sx = match tab_stops.get(i - 1) {
                    Some(stop) => {
                        let pos = stop.position as f64;
                        match stop.tab_type.as_str() {
                            "right" => (pos - sw).max(cursor),
                            "center" => (pos - sw / 2.0).max(cursor),
                            _ => pos.max(cursor),
                        }
                    }
                    None => cursor,
                };
                seg_starts.push(sx);
                cursor = sx + sw;
            }

            // Draw each segment.
            for (si, seg) in segments.iter().enumerate() {
                let mut x = seg_starts[si];
                for &(c, idx) in seg {
                    let code = glyph_byte_for(c);
                    let st = style_at(idx);
                    let adv = advance_of(code);
                    // Draw at the *fractional* cursor (not x.round()). The 5×
                    // supersample buffer has the sub-pixel resolution to place
                    // each glyph exactly, so inter-letter spacing matches the
                    // accumulated advance (and the underline). Per-glyph integer
                    // snapping added ±1px jitter that read as uneven gaps —
                    // "You" → "Y o", "create" → "cre ate", "here" → "he re".
                    let draw_x = x;
                    if c != ' ' {
                        if let Some(glyph) = parsed.glyphs.get(&code) {
                            if !glyph.contours.is_empty() {
                                Self::raster_glyph_outline(
                                    &mut buf, cw2u, ch2u, sf as f64, glyph,
                                    draw_x, baseline_y, scale_x, glyph_scale_y, st.italic, st.color,
                                );
                                if st.bold {
                                    Self::raster_glyph_outline(
                                        &mut buf, cw2u, ch2u, sf as f64, glyph,
                                        draw_x + bold_off, baseline_y, scale_x, glyph_scale_y, st.italic, st.color,
                                    );
                                }
                            }
                        }
                    }
                    if st.underline {
                        // Use the float cursor `x` (not the rounded draw_x) for
                        // the underline span so consecutive glyphs' underlines
                        // abut exactly — no gaps. Device coords (×sf).
                        let uy = baseline_y + 1.0;
                        let sfd = sf as f64;
                        Self::fill_solid_rect(
                            &mut buf, cw2u, ch2u,
                            x * sfd, uy * sfd, (x + adv) * sfd, (uy + 1.0) * sfd, st.color,
                        );
                    }
                    x += adv;
                }
            }

            y_top += line_step;
        }

        // --- Alpha-weighted 4×→1× box downscale into the destination bitmap. ---
        let px = &buf;
        let out_w = render_width.max(1) as usize;
        let out_h = render_height.max(1) as usize;
        for cy in 0..out_h {
            let dest_y = start_y + cy as i32;
            if dest_y < 0 || dest_y >= bitmap.height as i32 { continue; }
            for cx in 0..out_w {
                let dest_x = start_x + cx as i32;
                if dest_x < 0 || dest_x >= bitmap.width as i32 { continue; }
                let (mut sr, mut sg, mut sb, mut sa) = (0u32, 0u32, 0u32, 0u32);
                for sy in 0..sf as usize {
                    for sx in 0..sf as usize {
                        let i = ((cy * sf as usize + sy) * cw2u + (cx * sf as usize + sx)) * 4;
                        if i + 3 < px.len() {
                            let a = px[i + 3] as u32;
                            sr += px[i] as u32 * a;
                            sg += px[i + 1] as u32 * a;
                            sb += px[i + 2] as u32 * a;
                            sa += a;
                        }
                    }
                }
                let avg_a = (sa / (sf as u32 * sf as u32)) as u8;
                if avg_a == 0 { continue; }
                // Steepen the coverage ramp for Shockwave-crisp edges (same
                // idea as the atlas path's `steepen_alpha_ramp`): clip the
                // faint outer halo to transparent so edges stay tight, and
                // lift mid coverage so Verdana's thin stems read as solid —
                // instead of the soft grey a plain box-average/gamma produces.
                const LO: f32 = 45.0;
                const HI: f32 = 135.0;
                let out_a = if (avg_a as f32) <= LO {
                    0
                } else if (avg_a as f32) >= HI {
                    255
                } else {
                    (((avg_a as f32 - LO) / (HI - LO)) * 255.0).round() as u8
                };
                if out_a == 0 { continue; }
                let r = (sr / sa) as u8;
                let g = (sg / sa) as u8;
                let b = (sb / sa) as u8;
                let di = (dest_y as usize * bitmap.width as usize + dest_x as usize) * 4;
                if di + 3 < bitmap.data.len() {
                    bitmap.data[di] = r;
                    bitmap.data[di + 1] = g;
                    bitmap.data[di + 2] = b;
                    bitmap.data[di + 3] = out_a;
                }
            }
        }
        Ok(())
    }

    /// Software-rasterize one PFR glyph outline into a 4× straight-RGBA buffer
    /// `buf` (cw×ch device px) using non-zero winding — deterministic, no
    /// Canvas2D, so the output is identical on every platform/browser. Covered
    /// pixels are set to `color` at full alpha; the caller box-downscales for
    /// anti-aliasing.
    fn raster_glyph_outline(
        buf: &mut [u8],
        cw: usize,
        ch: usize,
        sf: f64,
        glyph: &crate::director::chunks::pfr1::types::OutlineGlyph,
        cursor_x: f64,
        baseline_y: f64,
        scale_x: f64,
        glyph_scale_y: f64,
        italic: bool,
        color: (u8, u8, u8),
    ) {
        use crate::director::chunks::pfr1::types::PfrCmdType;
        const SLANT: f64 = 0.21;
        // PFR command coord (render space) → device space (×sf).
        let map = |gx: f32, gy_raw: f32| -> (f64, f64) {
            let gy = gy_raw as f64 * glyph_scale_y;
            let shear = if italic { -gy * SLANT } else { 0.0 };
            ((cursor_x + gx as f64 * scale_x + shear) * sf, (baseline_y + gy) * sf)
        };

        // Flatten contours into device-space edges (curves → line segments).
        let mut edges: Vec<(f64, f64, f64, f64)> = Vec::new();
        for contour in &glyph.contours {
            let mut start: Option<(f64, f64)> = None;
            let mut cur = (0.0_f64, 0.0_f64);
            for cmd in &contour.commands {
                let p = map(cmd.x, cmd.y);
                match cmd.cmd_type {
                    PfrCmdType::MoveTo => {
                        if let Some(s) = start {
                            if cur != s { edges.push((cur.0, cur.1, s.0, s.1)); }
                        }
                        start = Some(p);
                        cur = p;
                    }
                    PfrCmdType::LineTo => {
                        if start.is_none() { start = Some(cur); }
                        edges.push((cur.0, cur.1, p.0, p.1));
                        cur = p;
                    }
                    PfrCmdType::CurveTo => {
                        if start.is_none() { start = Some(cur); }
                        let c1 = map(cmd.x1, cmd.y1);
                        let c2 = map(cmd.x2, cmd.y2);
                        // Segment count from control-net length (device px).
                        let approx = (c1.0 - cur.0).hypot(c1.1 - cur.1)
                            + (c2.0 - c1.0).hypot(c2.1 - c1.1)
                            + (p.0 - c2.0).hypot(p.1 - c2.1);
                        let n = ((approx / 3.0).ceil() as usize).clamp(4, 48);
                        let mut prev = cur;
                        for i in 1..=n {
                            let t = i as f64 / n as f64;
                            let mt = 1.0 - t;
                            let (a, b, cc, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
                            let xp = a * cur.0 + b * c1.0 + cc * c2.0 + d * p.0;
                            let yp = a * cur.1 + b * c1.1 + cc * c2.1 + d * p.1;
                            edges.push((prev.0, prev.1, xp, yp));
                            prev = (xp, yp);
                        }
                        cur = p;
                    }
                    PfrCmdType::Close => {
                        if let Some(s) = start {
                            if cur != s { edges.push((cur.0, cur.1, s.0, s.1)); }
                            cur = s;
                        }
                    }
                }
            }
            if let Some(s) = start {
                if cur != s { edges.push((cur.0, cur.1, s.0, s.1)); }
            }
        }
        if edges.is_empty() { return; }

        // Scanline fill, non-zero winding, sampling each row at its center.
        let mut ymin = f64::MAX;
        let mut ymax = f64::MIN;
        for &(_, y0, _, y1) in &edges {
            ymin = ymin.min(y0.min(y1));
            ymax = ymax.max(y0.max(y1));
        }
        let row0 = ymin.floor().max(0.0) as usize;
        let row1 = (ymax.ceil().max(0.0) as usize).min(ch);
        let mut xs: Vec<(f64, i32)> = Vec::new();
        for row in row0..row1 {
            let yc = row as f64 + 0.5;
            xs.clear();
            for &(x0, y0, x1, y1) in &edges {
                if (y0 <= yc && y1 > yc) || (y1 <= yc && y0 > yc) {
                    let t = (yc - y0) / (y1 - y0);
                    xs.push((x0 + t * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
                }
            }
            if xs.len() < 2 { continue; }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut wind = 0;
            for i in 0..xs.len() - 1 {
                wind += xs[i].1;
                if wind != 0 {
                    let xa = (xs[i].0 - 0.5).ceil().max(0.0) as i64;
                    let xb = (xs[i + 1].0 - 0.5).ceil().max(0.0).min(cw as f64) as i64;
                    let base = row * cw;
                    for px in xa..xb {
                        let di = (base + px as usize) * 4;
                        buf[di] = color.0;
                        buf[di + 1] = color.1;
                        buf[di + 2] = color.2;
                        buf[di + 3] = 255;
                    }
                }
            }
        }
    }

    /// Fill a solid device-space rect into the 4× RGBA buffer. Used for the
    /// underline so it stays continuous across glyphs.
    fn fill_solid_rect(
        buf: &mut [u8],
        cw: usize,
        ch: usize,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        color: (u8, u8, u8),
    ) {
        let px0 = x0.floor().max(0.0) as usize;
        let px1 = (x1.ceil().max(0.0) as usize).min(cw);
        let py0 = y0.floor().max(0.0) as usize;
        let py1 = (y1.ceil().max(0.0) as usize).min(ch);
        for row in py0..py1 {
            let base = row * cw;
            for px in px0..px1 {
                let di = (base + px) * 4;
                buf[di] = color.0;
                buf[di + 1] = color.1;
                buf[di + 2] = color.2;
                buf[di + 3] = 255;
            }
        }
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

        debug!(
            "ðŸ“ Font scaling: requested={}pt, native_char={}x{} -> scaled={}x{}",
            requested_font_size,
            font.char_width, font.char_height,
            scaled_char_width, scaled_char_height
        );

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

            // Calculate line width using proportional per-character advances
            let line_width: i32 = line.chars()
                .map(|c| {
                    let advance = font.get_char_advance_for(c) as i32;
                    (advance * requested_font_size) / native_char_height
                })
                .sum();

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

                // Calculate proportional advance for this character
                let char_advance = ((font.get_char_advance_for(ch) as i32) * requested_font_size / native_char_height).max(1);

                // For space character, just advance position without drawing
                if ch == ' ' {
                    char_x += char_advance;
                    continue;
                }

                if char_x >= bitmap.width as i32 {
                    break;
                }

                // Draw the character with scaling (use full cell width for source mapping)
                bitmap_font_copy_char_scaled(
                    font, font_bitmap, crate::io::encoding::glyph_byte_for(ch), bitmap,
                    char_x, y, char_width, char_height,
                    palettes, &params
                );

                // Simulate bold by drawing again with 1px offset
                if is_bold {
                    bitmap_font_copy_char_scaled(
                        font, font_bitmap, crate::io::encoding::glyph_byte_for(ch), bitmap,
                        char_x + 1, y, char_width, char_height,
                        palettes, &params
                    );
                }

                // Draw underline at the bottom of the scaled character
                if is_underline {
                    let underline_y = y + char_height - 1;
                    if underline_y < bitmap.height as i32 {
                        for ux in char_x..(char_x + char_advance).min(bitmap.width as i32) {
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

                char_x += char_advance;
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
                        skew: 0.0,
                        sprite: None,
                        mask_offset: (0, 0),
                        original_dst_rect: None,
                        bg_color_explicit: false,
                        fore_color_explicit: false,
                        ink9_mask_bitmap: None, ink9_mask_offset: (0, 0),
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
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

        match &member.member_type {
            CastMemberType::Text(text_data) => {
                match prop {
                    "text" => Ok(Datum::String(text_data.text.clone())),
                    "alignment" => Ok(Datum::String(text_data.alignment.clone())),
                    "wordWrap" => Ok(datum_bool(text_data.word_wrap)),
                    "width" => Ok(Datum::Int(text_data.width as i32)),
                    "font" => Ok(Datum::String(text_data.font.clone())),
                    "fontSize" => Ok(Datum::Int(text_data.font_size as i32)),
                    "fontStyle" => {
                        let font_styles: Vec<String> = text_data.font_style.clone();
                        let item_refs: VecDeque<_> = font_styles
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
                            measure_text(&text_clone, &font, None, fixed_line_space, top_spacing, 0);

                        match prop {
                            "rect" => Ok(Datum::Rect([0.0, 0.0, width as f64, height as f64], 0)),
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
                                    skew: 0.0,
                                    sprite: None,
                                    mask_offset: (0, 0),
                                    original_dst_rect: None,
                                    bg_color_explicit: false,
                                    fore_color_explicit: false,
                                    ink9_mask_bitmap: None, ink9_mask_offset: (0, 0),
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

                                // Text member `.image` rasterizes fresh each
                                // call; refcount via DatumRef so the snapshot
                                // is freed when the script's reference drops.
                                let bitmap_ref =
                                    player.bitmap_manager.add_ephemeral_bitmap(bitmap);
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

            CastMemberType::Font(font_data) => match prop {
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
                "fontStyle" => Ok(Datum::List(DatumType::List, VecDeque::new(), false)),
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
