use web_sys::HtmlCanvasElement;

use crate::player::{
    score::get_sprite_at,
    sprite::CursorRef,
    DirPlayer,
};

/// Cache key for the native cursor: (bitmap_ref, mask_bitmap_ref, reg_point).
pub type NativeCursorCache = Option<(Option<u32>, Option<u32>, (i16, i16))>;

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

    let cache_key = (
        Some(cursor_bitmap_member.image_ref),
        cursor_mask_member.as_ref().map(|m| m.image_ref),
        cursor_bitmap_member.reg_point,
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

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        use image::codecs::png::PngEncoder;
        use image::ImageEncoder;
        let encoder = PngEncoder::new(&mut png_bytes);
        if encoder
            .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
            .is_err()
        {
            return;
        }
    }
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    let data_url = format!("url(\"data:image/png;base64,{b64}\")");

    let hx = cursor_bitmap_member.reg_point.0;
    let hy = cursor_bitmap_member.reg_point.1;
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
