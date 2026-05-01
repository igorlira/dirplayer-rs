use crate::{director::lingo::datum::Datum, player::bitmap::bitmap::PaletteRef, rendering::{render_stage_to_bitmap, with_renderer_mut}};

use super::{
    bitmap::bitmap::{get_system_default_palette, Bitmap},
    DatumRef, DirPlayer, ScriptError,
};

/// Dimensions of the drawn stage: stage_draw_rect if Lingo set one, otherwise
/// the authored movie rect. Independent of the JS host's `set_stage_size`
/// (which tracks the outer container).
pub fn stage_canvas_dims(player: &DirPlayer) -> (u32, u32) {
    draw_rect_dims(player)
}

fn draw_rect_dims(player: &DirPlayer) -> (u32, u32) {
    if let Some(r) = player.stage_draw_rect {
        let w = (r[2] - r[0]).max(1.0) as u32;
        let h = (r[3] - r[1]).max(1.0) as u32;
        (w, h)
    } else {
        (player.movie.rect.width() as u32, player.movie.rect.height() as u32)
    }
}

/// Stage auto-scale factor — ratio of drawRect to the authored movie rect.
/// Returns (1.0, 1.0) if drawRect isn't set or matches movie.rect exactly,
/// so movies that manage their own scaling (e.g. FurniFactory2 via 1_resize
/// scripts) don't get double-scaled. For movies that set drawRect larger
/// than movie.rect without per-sprite scaling (e.g. Coke Studios), this
/// factor is used to auto-stretch sprites at render time.
pub fn stage_scale(player: &DirPlayer) -> (f64, f64) {
    let Some(r) = player.stage_draw_rect else { return (1.0, 1.0); };
    let movie_w = player.movie.rect.width() as f64;
    let movie_h = player.movie.rect.height() as f64;
    if movie_w <= 0.0 || movie_h <= 0.0 { return (1.0, 1.0); }
    let draw_w = (r[2] - r[0]).max(1.0);
    let draw_h = (r[3] - r[1]).max(1.0);
    let sx = draw_w / movie_w;
    let sy = draw_h / movie_h;
    if (sx - 1.0).abs() < 1e-3 && (sy - 1.0).abs() < 1e-3 {
        (1.0, 1.0)
    } else {
        (sx, sy)
    }
}

/// Resize the renderer canvas to match the current drawRect. Sprites are
/// scaled per-rect via `get_concrete_sprite_render_rect` rather than via a
/// global projection transform — keeps text/bitmaps sharp at the target size.
pub fn apply_stage_draw_rect(player: &DirPlayer) {
    let (draw_w, draw_h) = draw_rect_dims(player);
    with_renderer_mut(|renderer_opt| {
        if let Some(renderer) = renderer_opt {
            use crate::rendering_gpu::Renderer;
            renderer.set_size(draw_w, draw_h);
        }
    });
}

/// Convert host-canvas pixel coords to movie-space coords, inverting the
/// drawRect scaling so Lingo's mouseH/mouseV and script-facing APIs see the
/// authored coordinate system.
pub fn canvas_to_movie_coords(player: &DirPlayer, x: f64, y: f64) -> (f64, f64) {
    let (draw_w, draw_h) = draw_rect_dims(player);
    let movie_w = player.movie.rect.width() as f64;
    let movie_h = player.movie.rect.height() as f64;
    if player.stage_draw_rect.is_some()
        && draw_w > 0 && draw_h > 0
        && movie_w > 0.0 && movie_h > 0.0
    {
        (x * movie_w / draw_w as f64, y * movie_h / draw_h as f64)
    } else {
        (x, y)
    }
}

pub fn get_stage_prop(player: &mut DirPlayer, prop: &str) -> Result<Datum, ScriptError> {
    match prop {
        "rect" | "drawRect" => {
            if let Some(r) = player.stage_draw_rect {
                Ok(Datum::Rect(r, 0))
            } else {
                Ok(Datum::Rect([0.0, 0.0, player.movie.rect.width() as f64, player.movie.rect.height() as f64], 0))
            }
        }
        "sourceRect" => {
            // TODO where does this come from?
            Ok(Datum::Rect([0.0, 0.0, player.movie.rect.width() as f64, player.movie.rect.height() as f64], 0))
        }
        "bgColor" => Ok(Datum::ColorRef(player.bg_color.clone())),
        "image" => {
            let mut new_bitmap = Bitmap::new(
                player.movie.rect.width() as u16,
                player.movie.rect.height() as u16,
                32,
                32,
                0,
                PaletteRef::BuiltIn(get_system_default_palette()),
            );
            render_stage_to_bitmap(player, &mut new_bitmap, None);
            // Ephemeral: a fresh stage snapshot per call. Once no DatumRef
            // wraps it (e.g. after the script's `(the stage).image` Lingo
            // expression goes out of scope) the bitmap is freed. RemoteControl
            // CameraScreen scripts call this every frame — without ephemeral
            // tracking each call leaks ~movie_w*movie_h*4 bytes forever.
            let bitmap_id = player.bitmap_manager.add_ephemeral_bitmap(new_bitmap);
            Ok(Datum::BitmapRef(bitmap_id))
        }
        "name" => Ok(Datum::String("stage".to_string())),
        _ => return Err(ScriptError::new(format!("Invalid stage property {}", prop))),
    }
}

pub fn set_stage_prop(
    player: &mut DirPlayer,
    prop: &str,
    value: &DatumRef,
) -> Result<(), ScriptError> {
    match prop {
        "title" => {
            let value = player.get_datum(value).clone();
            player.title = value.string_value()?;
            Ok(())
        }
        "bgColor" => {
            let value = player.get_datum(value).clone();
            match value {
                Datum::ColorRef(color_ref) => {
                    player.bg_color = color_ref;
                }
                Datum::Int(i) => {
                    player.bg_color = super::sprite::ColorRef::PaletteIndex(i as u8);
                }
                _ => {
                    return Err(ScriptError::new(
                        "Color ref or integer expected for stage bgColor".to_string(),
                    ));
                }
            }
            Ok(())
        }
        "drawRect" | "rect" => {
            let value = player.get_datum(value).clone();
            match value {
                Datum::Rect(r, _) => {
                    let w = (r[2] - r[0]).max(1.0) as u32;
                    let h = (r[3] - r[1]).max(1.0) as u32;
                    if prop == "drawRect" {
                        player.stage_draw_rect = Some(r);
                    }
                    player.stage_size = (w, h);
                    apply_stage_draw_rect(player);
                    crate::js_api::JsApi::dispatch_stage_size_changed(w, h, player.center_stage);
                    Ok(())
                }
                _ => Err(ScriptError::new(
                    "Rect expected for stage drawRect".to_string(),
                )),
            }
        }
        "sourceRect" => Ok(()),
        _ => {
            return Err(ScriptError::new(format!(
                "Cannot set stage property {}",
                prop
            )))
        }
    }
}
