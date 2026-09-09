use web_sys::HtmlCanvasElement;

use crate::player::{
    score::get_sprite_at,
    sprite::CursorRef,
    DirPlayer,
};

/// Cache key for the native cursor: either a cast-member cursor
/// (bitmap_ref, mask_bitmap_ref, reg_point, scale_factor) or one of Director's
/// numbered built-ins. Kept so the CSS property is only written when it changes.
///
/// The scale factor is part of the MEMBER key because the cursor is rasterised
/// at the stage scale — without it, going fullscreen would keep serving the
/// cached 1x image. A system cursor is a CSS keyword, which the browser scales
/// itself, so it needs no factor.
#[derive(PartialEq, Clone, Debug)]
pub enum CursorCacheKey {
    Member(Option<u32>, Option<u32>, (i16, i16), u32),
    System(i32),
}
pub type NativeCursorCache = Option<CursorCacheKey>;

/// Director's numbered cursors, as the CSS keyword that looks the same.
///
/// A movie sets these from rollover behaviours: `cursor(280)` on entering a
/// button and `cursor(0)` on leaving it. Checked against the Windows
/// projector's live cursor handle: over a button it is a pointing hand from
/// Director's own bitmap, matching no stock Windows cursor, and over the
/// backdrop the plain arrow. An unknown number falls back to the arrow.
fn system_cursor_css(id: i32) -> Option<&'static str> {
    match id {
        -1 | 0 => Some("default"),
        1 => Some("text"),        // I-Beam
        2 => Some("crosshair"),   // Crosshair
        3 => Some("cell"),        // Crossbar
        4 => Some("wait"),        // Watch
        200 => Some("none"),      // Blank
        254 => Some("help"),      // Help
        280 => Some("pointer"),   // Finger
        _ => Some("default"),
    }
}

/// Resolve and apply the custom cursor as a native CSS cursor on `canvas` (and
/// `document.body` so it persists during pointer-capture drag). Returns without
/// doing anything if the cursor hasn't changed since the last call (cache hit).
pub fn update_native_cursor(
    player: &mut DirPlayer,
    canvas: &HtmlCanvasElement,
    cache: &mut NativeCursorCache,
) {
    let hovered_sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
    let cursor_ref = if let Some(hovered_sprite) = hovered_sprite {
        let sprite = player.movie.score.get_sprite(hovered_sprite as i16);
        sprite.and_then(|s| s.cursor_ref.clone())
    } else {
        None
    };
    let cursor_ref = cursor_ref.as_ref().unwrap_or(&player.cursor);
    // A numbered cursor is a CSS keyword; only a member cursor needs a bitmap
    // built into a data URL below.
    if let CursorRef::System(id) = cursor_ref {
        let key = CursorCacheKey::System(*id);
        if cache.as_ref() == Some(&key) {
            return;
        }
        match system_cursor_css(*id) {
            Some("default") => {
                let _ = canvas.style().remove_property("cursor");
                set_body_cursor(None);
            }
            Some(css) => {
                let _ = canvas.style().set_property("cursor", css);
                set_body_cursor(Some(css));
            }
            None => {
                let _ = canvas.style().remove_property("cursor");
                set_body_cursor(None);
            }
        }
        *cache = Some(key);
        return;
    }
    let cursor_list = match cursor_ref {
        CursorRef::Member(ids) => Some(ids),
        _ => None,
    };

    let cursor_bitmap_member = cursor_list
        .and_then(|ids| ids.first().copied())
        .and_then(|id| player.movie.cast_manager.find_member_by_slot_number(id as u32))
        .and_then(|m| m.member_type.as_bitmap().cloned());

    let cursor_mask_member = cursor_list
        .and_then(|ids| ids.get(1).copied())
        .and_then(|id| player.movie.cast_manager.find_member_by_slot_number(id as u32))
        .and_then(|m| m.member_type.as_bitmap().cloned());

    let cursor_bitmap_member = match cursor_bitmap_member {
        Some(m) => m,
        None => {
            if cache.is_some() {
                let _ = canvas.style().remove_property("cursor");
                set_body_cursor(None);
                *cache = None;
            }
            return;
        }
    };

    // A custom cursor is a CSS image, so the browser draws it at device pixels
    // and it does NOT inherit the stage's scale — on a scaled stage Habbo's
    // cursor stayed its authored size while everything it points at grew.
    //
    // Integer factors only: these are pixel-art cursors and the canvas is
    // `image-rendering: pixelated`, so a fractional resample would make the
    // cursor the one soft thing on screen. Browsers also REFUSE a custom cursor
    // larger than 128px in either axis (they silently fall back to the default
    // arrow), so the factor is clamped to keep both axes inside that — better a
    // 2x cursor than no cursor.
    const MAX_CURSOR_PX: u32 = 128;
    let scale_factor = {
        // `stage_scale` is movie -> DEVICE pixels, because `stage_layout` folds
        // in `stage_pixel_ratio`. A CSS cursor is not: the browser takes the
        // image's intrinsic pixels as CSS pixels and applies the device ratio
        // itself. Using the device scale therefore multiplied by the ratio
        // twice, and on a Retina Mac (dpr 2) every custom cursor came out at
        // double size. What is wanted is the movie -> CSS scale.
        let (sx, sy) = crate::player::stage::stage_scale(player);
        let dpr = crate::player::stage::stage_pixel_ratio(player).max(1.0);
        let want = (sx.min(sy) / dpr).round().max(1.0) as u32;
        let w = (cursor_bitmap_member.info.width as u32).max(1);
        let h = (cursor_bitmap_member.info.height as u32).max(1);
        let fit_w = (MAX_CURSOR_PX / w).max(1);
        let fit_h = (MAX_CURSOR_PX / h).max(1);
        want.min(fit_w).min(fit_h).max(1)
    };

    let cache_key = CursorCacheKey::Member(
        Some(cursor_bitmap_member.image_ref),
        cursor_mask_member.as_ref().map(|m| m.image_ref),
        cursor_bitmap_member.reg_point,
        scale_factor,
    );
    if cache.as_ref() == Some(&cache_key) {
        return;
    }

    let cursor_bitmap = match player.bitmap_manager.get_bitmap(cursor_bitmap_member.image_ref) {
        Some(b) => b,
        None => {
            let _ = canvas.style().remove_property("cursor");
            set_body_cursor(None);
            *cache = None;
            return;
        }
    };

    let palettes = player.movie.cast_manager.palettes();
    let w = cursor_bitmap.width as u32;
    let h = cursor_bitmap.height as u32;
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = cursor_bitmap.get_pixel_color(&palettes, x as u16, y as u16);
            let idx = ((y * w + x) * 4) as usize;
            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = 255;
        }
    }

    if let Some(mask_member) = cursor_mask_member {
        if let Some(mask_bitmap) = player.bitmap_manager.get_bitmap(mask_member.image_ref) {
            for y in 0..h.min(mask_bitmap.height as u32) {
                for x in 0..w.min(mask_bitmap.width as u32) {
                    let (mr, mg, mb) = mask_bitmap.get_pixel_color(&palettes, x as u16, y as u16);
                    let idx = ((y * w + x) * 4) as usize;
                    // White mask pixels = transparent, black = opaque
                    if mr > 127 && mg > 127 && mb > 127 {
                        rgba[idx + 3] = 0;
                    }
                }
            }
        }
    }

    // Nearest-neighbour upscale to the stage scale. Done here rather than via a
    // CSS size because a data-URL cursor has no CSS box to size — the browser
    // uses the image's own pixels.
    let (out_w, out_h, out_rgba) = if scale_factor > 1 {
        let (nw, nh) = (w * scale_factor, h * scale_factor);
        let mut up = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            let sy = y / scale_factor;
            for x in 0..nw {
                let sx = x / scale_factor;
                let src = ((sy * w + sx) * 4) as usize;
                let dst = ((y * nw + x) * 4) as usize;
                up[dst..dst + 4].copy_from_slice(&rgba[src..src + 4]);
            }
        }
        (nw, nh, up)
    } else {
        (w, h, rgba)
    };

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        use image::codecs::png::PngEncoder;
        use image::ImageEncoder;
        let encoder = PngEncoder::new(&mut png_bytes);
        if encoder
            .write_image(&out_rgba, out_w, out_h, image::ExtendedColorType::Rgba8)
            .is_err()
        {
            return;
        }
    }
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    let data_url = format!("url(\"data:image/png;base64,{b64}\")");

    // The hotspot is in the cursor image's own pixels, so it scales with it.
    let hx = cursor_bitmap_member.reg_point.0 as i32 * scale_factor as i32;
    let hy = cursor_bitmap_member.reg_point.1 as i32 * scale_factor as i32;
    let cursor_css = format!("{data_url} {hx} {hy}, auto");

    let _ = canvas.style().set_property("cursor", &cursor_css);
    // Also set on document.body so the cursor persists when the outer container
    // holds pointer capture during drag (browsers use the capturing element's
    // computed cursor, which inherits from body if not explicitly set).
    set_body_cursor(Some(&cursor_css));
    *cache = Some(cache_key);
}

fn set_body_cursor(cursor_css: Option<&str>) {
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    let Some(body) = document.body() else { return };
    match cursor_css {
        Some(css) => { let _ = body.style().set_property("cursor", css); }
        None => { let _ = body.style().remove_property("cursor"); }
    }
}
