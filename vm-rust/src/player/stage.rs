use crate::{director::lingo::datum::Datum, player::bitmap::bitmap::PaletteRef, rendering::{render_stage_to_bitmap, with_renderer_mut}, rendering_gpu::Renderer};

use super::{
    bitmap::bitmap::{get_system_default_palette, Bitmap},
    DatumRef, DirPlayer, ScriptError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StretchStyle {
    Meet,
    Fill,
    Stage,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageLayout {
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub stage_rect: [f64; 4],
    pub draw_rect: [f64; 4],
}

impl StageLayout {
    pub fn scale_x(&self, movie_width: f64) -> f64 {
        if movie_width <= 0.0 {
            1.0
        } else {
            (self.draw_rect[2] - self.draw_rect[0]).max(1.0) / movie_width
        }
    }

    pub fn scale_y(&self, movie_height: f64) -> f64 {
        if movie_height <= 0.0 {
            1.0
        } else {
            (self.draw_rect[3] - self.draw_rect[1]).max(1.0) / movie_height
        }
    }
}

fn stretch_style(player: &DirPlayer) -> StretchStyle {
    match player
        .external_params
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("swStretchStyle"))
        .map(|(_, value)| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("meet") => StretchStyle::Meet,
        Some("fill") => StretchStyle::Fill,
        Some("stage") => StretchStyle::Stage,
        _ => StretchStyle::None,
    }
}

fn compute_stage_layout(
    movie_width: f64,
    movie_height: f64,
    stage_width: u32,
    stage_height: u32,
    style: StretchStyle,
) -> StageLayout {
    let movie_width = movie_width.max(1.0);
    let movie_height = movie_height.max(1.0);
    let stage_width = stage_width.max(1);
    let stage_height = stage_height.max(1);

    match style {
        StretchStyle::Meet => {
            let scale = f64::min(stage_width as f64 / movie_width, stage_height as f64 / movie_height);
            let draw_width = movie_width * scale;
            let draw_height = movie_height * scale;
            let left = ((stage_width as f64 - draw_width) / 2.0).max(0.0);
            let top = ((stage_height as f64 - draw_height) / 2.0).max(0.0);
            StageLayout {
                canvas_width: stage_width,
                canvas_height: stage_height,
                stage_rect: [0.0, 0.0, stage_width as f64, stage_height as f64],
                draw_rect: [left, top, left + draw_width, top + draw_height],
            }
        }
        StretchStyle::Fill => StageLayout {
            canvas_width: stage_width,
            canvas_height: stage_height,
            stage_rect: [0.0, 0.0, stage_width as f64, stage_height as f64],
            draw_rect: [0.0, 0.0, stage_width as f64, stage_height as f64],
        },
        StretchStyle::Stage => StageLayout {
            canvas_width: stage_width,
            canvas_height: stage_height,
            stage_rect: [0.0, 0.0, stage_width as f64, stage_height as f64],
            draw_rect: [0.0, 0.0, movie_width, movie_height],
        },
        StretchStyle::None => StageLayout {
            canvas_width: movie_width as u32,
            canvas_height: movie_height as u32,
            stage_rect: [0.0, 0.0, movie_width, movie_height],
            draw_rect: [0.0, 0.0, movie_width, movie_height],
        },
    }
}

pub fn stage_layout(player: &DirPlayer) -> StageLayout {
    if let Some(r) = player.stage_draw_rect {
        let width = (r[2] - r[0]).max(1.0) as u32;
        let height = (r[3] - r[1]).max(1.0) as u32;
        StageLayout {
            canvas_width: width,
            canvas_height: height,
            stage_rect: r,
            draw_rect: r,
        }
    } else {
        compute_stage_layout(
            player.movie.rect.width() as f64,
            player.movie.rect.height() as f64,
            player.stage_size.0,
            player.stage_size.1,
            stretch_style(player),
        )
    }
}

/// Dimensions of the stage canvas: explicit drawRect if Lingo set one,
/// otherwise the effective layout derived from `swStretchStyle`.
pub fn stage_canvas_dims(player: &DirPlayer) -> (u32, u32) {
    let layout = stage_layout(player);
    (layout.canvas_width, layout.canvas_height)
}

/// Stage content scale — ratio of the effective draw rect to the authored
/// movie rect.
pub fn stage_scale(player: &DirPlayer) -> (f64, f64) {
    let layout = stage_layout(player);
    let movie_w = player.movie.rect.width() as f64;
    let movie_h = player.movie.rect.height() as f64;
    if movie_w <= 0.0 || movie_h <= 0.0 { return (1.0, 1.0); }
    let sx = layout.scale_x(movie_w);
    let sy = layout.scale_y(movie_h);
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
    let (draw_w, draw_h) = stage_canvas_dims(player);
    // 1x1 only occurs before any movie has loaded (movie.rect is 0x0, clamped).
    // Skip resizing to avoid triggering external canvas-size observers (e.g.
    // third-party embed wrappers that read the first canvas resize to infer
    // the player dimensions) before the real movie dimensions are known.
    if draw_w <= 1 && draw_h <= 1 {
        return;
    }
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
    let layout = stage_layout(player);
    let draw_w = (layout.draw_rect[2] - layout.draw_rect[0]).max(1.0);
    let draw_h = (layout.draw_rect[3] - layout.draw_rect[1]).max(1.0);
    let movie_w = player.movie.rect.width() as f64;
    let movie_h = player.movie.rect.height() as f64;
    if draw_w > 0.0 && draw_h > 0.0
        && movie_w > 0.0 && movie_h > 0.0
    {
        (
            (x - layout.draw_rect[0]) * movie_w / draw_w,
            (y - layout.draw_rect[1]) * movie_h / draw_h,
        )
    } else {
        (x, y)
    }
}

pub fn get_stage_prop(player: &mut DirPlayer, prop: &str) -> Result<Datum, ScriptError> {
    match prop {
        "rect" => Ok(Datum::Rect(stage_layout(player).stage_rect, 0)),
        "drawRect" => Ok(Datum::Rect(stage_layout(player).draw_rect, 0)),
        "sourceRect" => {
            // TODO where does this come from?
            Ok(Datum::Rect([0.0, 0.0, player.movie.rect.width() as f64, player.movie.rect.height() as f64], 0))
        }
        "bgColor" => Ok(Datum::ColorRef(player.bg_color.clone())),
        "image" => {
            let mut captured_bitmap = None;
            with_renderer_mut(|renderer_opt| {
                if let Some(renderer) = renderer_opt {
                    captured_bitmap = Some(renderer.capture_stage_bitmap(player));
                }
            });

            let new_bitmap = if let Some(bitmap) = captured_bitmap {
                bitmap
            } else {
                let layout = stage_layout(player);
                let w = layout.stage_rect[2] - layout.stage_rect[0];
                let h = layout.stage_rect[3] - layout.stage_rect[1];
                let mut bitmap = Bitmap::new(
                    w as u16,
                    h as u16,
                    32,
                    32,
                    0,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                render_stage_to_bitmap(player, &mut bitmap, None);
                bitmap
            };
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

#[cfg(test)]
mod tests {
    use super::{compute_stage_layout, StretchStyle};

    #[test]
    fn stretch_meet_letterboxes_inside_stage() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 1000, StretchStyle::Meet);
        assert_eq!(layout.canvas_width, 1000);
        assert_eq!(layout.canvas_height, 1000);
        assert_eq!(layout.stage_rect, [0.0, 0.0, 1000.0, 1000.0]);
        assert_eq!(layout.draw_rect, [0.0, 125.0, 1000.0, 875.0]);
    }

    #[test]
    fn stretch_fill_matches_container() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 600, StretchStyle::Fill);
        assert_eq!(layout.canvas_width, 1000);
        assert_eq!(layout.canvas_height, 600);
        assert_eq!(layout.stage_rect, [0.0, 0.0, 1000.0, 600.0]);
        assert_eq!(layout.draw_rect, [0.0, 0.0, 1000.0, 600.0]);
    }

    #[test]
    fn stretch_stage_resizes_stage_without_scaling_content() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 600, StretchStyle::Stage);
        assert_eq!(layout.canvas_width, 1000);
        assert_eq!(layout.canvas_height, 600);
        assert_eq!(layout.stage_rect, [0.0, 0.0, 1000.0, 600.0]);
        assert_eq!(layout.draw_rect, [0.0, 0.0, 640.0, 480.0]);
    }

    #[test]
    fn stretch_none_keeps_authored_movie_size() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 600, StretchStyle::None);
        assert_eq!(layout.canvas_width, 640);
        assert_eq!(layout.canvas_height, 480);
        assert_eq!(layout.stage_rect, [0.0, 0.0, 640.0, 480.0]);
        assert_eq!(layout.draw_rect, [0.0, 0.0, 640.0, 480.0]);
    }
}
