//! Text member drawing for the Canvas2D renderer.
//!
//! The browser's own text rasteriser (`render_native_text_to_bitmap`) lays a
//! member's text out into a bitmap OF ITS OWN: `start_y` is a layout origin
//! inside that bitmap (the text is drawn on a canvas only `render_height`
//! tall) and the glyph pixels are written with their coverage as alpha, not
//! composited. The WebGL2 renderer uses it that way and uploads the bitmap as
//! a texture. Called with stage coordinates on the stage bitmap, a text
//! sprite below the canvas height drew nothing at all and one near the top
//! replaced the stage pixels with half-transparent ones.
//!
//! The Canvas2D renderer therefore lays the text out into a transparent
//! bitmap the size of the sprite and composites that onto the stage here,
//! with the runs chosen by the same rules as the WebGL2 renderer's Text
//! branch, so both renderers draw a member with the same font, colours and
//! styles.

use crate::player::{
    bitmap::{
        bitmap::{resolve_color_ref, get_system_default_palette, Bitmap, PaletteRef},
        palette_map::PaletteMap,
    },
    cast_member::TextMember,
    handlers::datum_handlers::cast_member::font::{HtmlStyle, StyledSpan},
    sprite::ColorRef,
    symbols::builtin::BuiltInSymbol,
};

fn rgb_of(c: &ColorRef, palettes: &PaletteMap) -> (u8, u8, u8) {
    match c {
        ColorRef::Rgb(r, g, b) => (*r, *g, *b),
        ColorRef::PaletteIndex(_) => resolve_color_ref(
            palettes,
            c,
            &PaletteRef::BuiltIn(get_system_default_palette()),
            8,
        ),
    }
}

fn pack((r, g, b): (u8, u8, u8)) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// The colour a text member's runs are drawn in when no run says otherwise,
/// by the WebGL2 renderer's priority: a non-black RGB sprite colour, a
/// non-black RGB member colour, a sprite colour Lingo has set, a non-default
/// sprite colour, the colour of a single run, then the member's colour.
pub fn text_foreground(
    text_member: &TextMember,
    member_color: &ColorRef,
    sprite_color: &ColorRef,
    has_fore_color: bool,
) -> ColorRef {
    let black = ColorRef::Rgb(0, 0, 0);
    let default = ColorRef::PaletteIndex(255);
    if matches!(sprite_color, ColorRef::Rgb(..)) && *sprite_color != black {
        sprite_color.clone()
    } else if matches!(member_color, ColorRef::Rgb(..)) && *member_color != black {
        member_color.clone()
    } else if has_fore_color {
        sprite_color.clone()
    } else if *sprite_color != default && *sprite_color != black {
        sprite_color.clone()
    } else if text_member.html_styled_spans.len() == 1 {
        match text_member.html_styled_spans[0].style.color {
            Some(c) => ColorRef::Rgb(((c >> 16) & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, (c & 0xFF) as u8),
            None if *member_color != default => member_color.clone(),
            None => sprite_color.clone(),
        }
    } else if *member_color != default {
        member_color.clone()
    } else {
        sprite_color.clone()
    }
}

/// The runs the browser draws a text member with.
///
/// The member's styled runs when they still describe its text, with the
/// member's font, its size (a runtime size change scales every run) and the
/// sprite's foreColor combined into each run's colour. Runs that no longer
/// match the text (a script replaced it by a route that did not rebuild
/// them) would draw the previous content, so then, and for a member without
/// runs, the whole text is one run in the member's font, size and style.
/// Ink 4 (Not Copy) inverts the colours, as Director draws it.
pub fn native_text_spans(
    text_member: &TextMember,
    member_color: &ColorRef,
    sprite_color: &ColorRef,
    has_fore_color: bool,
    ink: u32,
    palettes: &PaletteMap,
) -> Vec<StyledSpan> {
    let fg = pack(rgb_of(
        &text_foreground(text_member, member_color, sprite_color, has_fore_color),
        palettes,
    ));
    let spans = &text_member.html_styled_spans;
    let joined: String = spans.iter().map(|s| s.text.as_str()).collect();
    let face = |style_face: &Option<String>| -> String {
        if !text_member.font.is_empty() {
            text_member.font.clone()
        } else {
            style_face.clone().filter(|f| !f.is_empty()).unwrap_or_else(|| "Arial".to_string())
        }
    };

    let mut out: Vec<StyledSpan> = if spans.is_empty() || joined != text_member.text {
        let mut style = HtmlStyle::default();
        style.font_face = Some(face(&spans.first().and_then(|s| s.style.font_face.clone())));
        style.font_size = Some(if text_member.font_size > 0 {
            text_member.font_size as i32
        } else {
            spans.first().and_then(|s| s.style.font_size).filter(|s| *s > 0).unwrap_or(12)
        });
        if let Some(first) = spans.first() {
            style.bold = first.style.bold;
            style.italic = first.style.italic;
            style.underline = first.style.underline;
        } else {
            style.bold = text_member.font_style.contains(&BuiltInSymbol::Bold);
            style.italic = text_member.font_style.contains(&BuiltInSymbol::Italic);
            style.underline = text_member.font_style.contains(&BuiltInSymbol::Underline);
        }
        style.color = Some(fg);
        vec![StyledSpan { text: text_member.text.clone(), style }]
    } else {
        let initial = spans.iter().find_map(|s| s.style.font_size.filter(|sz| *sz > 0)).unwrap_or(0);
        let scale = if text_member.font_size > 0 && initial > 0 && text_member.font_size as i32 != initial {
            Some(text_member.font_size as f32 / initial as f32)
        } else {
            None
        };
        let recolour = has_fore_color
            || (*sprite_color != ColorRef::PaletteIndex(255) && *sprite_color != ColorRef::Rgb(0, 0, 0));
        spans
            .iter()
            .map(|span| {
                let mut style = span.style.clone();
                style.font_face = Some(face(&style.font_face));
                style.font_size = match (scale, style.font_size.filter(|s| *s > 0)) {
                    (Some(k), Some(sz)) => Some(((sz as f32 * k).round() as i32).max(1)),
                    (Some(_), None) => Some(text_member.font_size as i32),
                    (None, Some(sz)) => Some(sz),
                    (None, None) => Some(12),
                };
                // The foreColor combines with a run's own colour per channel
                // (XOR), so the default black foreColor leaves runs as authored.
                if recolour || style.color.is_none() {
                    style.color = Some(match style.color {
                        Some(c) => (c ^ fg) & 0xFFFFFF,
                        None => fg,
                    });
                }
                StyledSpan { text: span.text.clone(), style }
            })
            .collect()
    };

    if ink == 4 {
        for s in out.iter_mut() {
            if let Some(c) = s.style.color {
                s.style.color = Some(!c & 0xFFFFFF);
            }
        }
    }
    out
}

/// A text bitmap laid out by `render_native_text_to_bitmap` (32-bit, the
/// glyph coverage in alpha, everything else alpha 0) composited over `dst`
/// with its top-left at (x, y), every pixel's alpha scaled by `opacity`
/// (the sprite's blend, 0 to 1). Pixels outside `dst` are dropped.
pub fn composite_text_over(
    dst: &mut Bitmap,
    src: &Bitmap,
    x: i32,
    y: i32,
    opacity: f32,
    palettes: &PaletteMap,
) {
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 || src.bit_depth != 32 {
        return;
    }
    let (sw, sh) = (src.width as i32, src.height as i32);
    let (dw, dh) = (dst.width as i32, dst.height as i32);
    for sy in 0..sh {
        let dy = y + sy;
        if dy < 0 || dy >= dh {
            continue;
        }
        for sx in 0..sw {
            let dx = x + sx;
            if dx < 0 || dx >= dw {
                continue;
            }
            let si = ((sy * sw + sx) * 4) as usize;
            let a = src.data[si + 3] as f32 / 255.0 * opacity;
            if a <= 0.0 {
                continue;
            }
            let (sr, sg, sb) = (src.data[si] as f32, src.data[si + 1] as f32, src.data[si + 2] as f32);
            if dst.bit_depth == 32 {
                let di = ((dy * dw + dx) * 4) as usize;
                let da = dst.data[di + 3] as f32 / 255.0;
                let out_a = a + da * (1.0 - a);
                if out_a <= 0.0 {
                    continue;
                }
                let mix = |s: f32, d: u8| ((s * a + d as f32 * da * (1.0 - a)) / out_a).round().clamp(0.0, 255.0) as u8;
                dst.data[di] = mix(sr, dst.data[di]);
                dst.data[di + 1] = mix(sg, dst.data[di + 1]);
                dst.data[di + 2] = mix(sb, dst.data[di + 2]);
                dst.data[di + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            } else {
                let (r, g, b) = dst.get_pixel_color(palettes, dx as u16, dy as u16);
                let m = |s: f32, d: u8| (s * a + d as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                dst.set_pixel(dx, dy, (m(sr, r), m(sg, g), m(sb, b)), palettes);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(w: u16, h: u16) -> Bitmap {
        let mut b = Bitmap::new(w, h, 32, 32, 0, PaletteRef::BuiltIn(get_system_default_palette()));
        b.data.fill(0);
        b
    }

    fn px(b: &Bitmap, x: i32, y: i32) -> [u8; 4] {
        let i = ((y * b.width as i32 + x) * 4) as usize;
        [b.data[i], b.data[i + 1], b.data[i + 2], b.data[i + 3]]
    }

    // A text sprite at (40, 300) on a 100x400 opaque stage: its pixels land
    // at (40, 300), the stage elsewhere is untouched (rows 0..3 included,
    // where the old call wrote them), and partial coverage blends.
    #[test]
    fn text_lands_at_the_sprite_and_blends() {
        let palettes = PaletteMap::new();
        let mut stage = rgba(100, 400);
        for p in stage.data.chunks_mut(4) {
            p.copy_from_slice(&[10, 20, 30, 255]);
        }
        let mut text = rgba(4, 3);
        text.use_alpha = true;
        text.data[0..4].copy_from_slice(&[255, 255, 255, 255]); // (0,0) full
        text.data[4..8].copy_from_slice(&[255, 255, 255, 128]); // (1,0) half
        composite_text_over(&mut stage, &text, 40, 300, 1.0, &palettes);
        assert_eq!(px(&stage, 40, 300), [255, 255, 255, 255]);
        let half = px(&stage, 41, 300);
        assert!((half[0] as i32 - 133).abs() <= 1 && half[3] == 255, "{:?}", half);
        assert_eq!(px(&stage, 42, 300), [10, 20, 30, 255]);
        assert_eq!(px(&stage, 0, 0), [10, 20, 30, 255]);
        assert_eq!(px(&stage, 40, 0), [10, 20, 30, 255]);
    }

    #[test]
    fn blend_scales_and_edges_clip() {
        let palettes = PaletteMap::new();
        let mut stage = rgba(10, 10);
        for p in stage.data.chunks_mut(4) {
            p.copy_from_slice(&[0, 0, 0, 255]);
        }
        let mut text = rgba(4, 4);
        text.use_alpha = true;
        for p in text.data.chunks_mut(4) {
            p.copy_from_slice(&[200, 200, 200, 255]);
        }
        composite_text_over(&mut stage, &text, 8, -2, 0.5, &palettes);
        assert_eq!(px(&stage, 8, 0), [100, 100, 100, 255]);
        assert_eq!(px(&stage, 9, 1), [100, 100, 100, 255]);
        assert_eq!(px(&stage, 7, 0), [0, 0, 0, 255]);
        assert_eq!(px(&stage, 8, 2), [0, 0, 0, 255]);
    }

    fn member(text: &str, spans: Vec<StyledSpan>) -> TextMember {
        let mut m = TextMember::new();
        m.text = text.to_string();
        m.font = "Verdana".to_string();
        m.font_size = 24;
        m.html_styled_spans = spans;
        m
    }

    fn span(text: &str, color: Option<u32>, underline: bool) -> StyledSpan {
        let mut style = HtmlStyle::default();
        style.font_face = Some("Arial".to_string());
        style.font_size = Some(24);
        style.color = color;
        style.underline = underline;
        StyledSpan { text: text.to_string(), style }
    }

    #[test]
    fn spans_keep_their_styles_and_take_the_member_font() {
        let palettes = PaletteMap::new();
        let m = member("Apples", vec![span("A", Some(0xFFFFFF), true), span("pples", Some(0xFFFFFF), false)]);
        let out = native_text_spans(&m, &ColorRef::PaletteIndex(255), &ColorRef::PaletteIndex(255), false, 36, &palettes);
        assert_eq!(out.len(), 2);
        assert!(out[0].style.underline && !out[1].style.underline);
        assert_eq!(out[0].style.font_face.as_deref(), Some("Verdana"));
        assert_eq!(out[1].style.color, Some(0xFFFFFF));
    }

    #[test]
    fn no_runs_or_stale_runs_draw_the_text_as_one_run() {
        let palettes = PaletteMap::new();
        let mut m = member("Pick one:", vec![]);
        m.font_style = vec![BuiltInSymbol::Bold, BuiltInSymbol::Italic];
        let out = native_text_spans(&m, &ColorRef::Rgb(255, 255, 255), &ColorRef::PaletteIndex(255), false, 36, &palettes);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Pick one:");
        assert!(out[0].style.bold && out[0].style.italic);
        assert_eq!(out[0].style.color, Some(0xFFFFFF));
        assert_eq!(out[0].style.font_size, Some(24));

        let stale = member("Pick one:", vec![span("Old heading:", Some(0xFFFFFF), false)]);
        let out = native_text_spans(&stale, &ColorRef::PaletteIndex(255), &ColorRef::PaletteIndex(255), false, 36, &palettes);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Pick one:");
    }
}
