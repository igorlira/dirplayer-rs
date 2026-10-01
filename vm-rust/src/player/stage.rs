use crate::{director::lingo::datum::Datum, player::{bitmap::bitmap::PaletteRef, symbols::{builtin::BuiltInSymbol, symbol::Symbol}}, rendering::{render_stage_to_bitmap, with_renderer_mut}, rendering_gpu::Renderer};

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
    // The WebGL2 renderer is shared and belongs to the HOST stage. A nested
    // `#movie` sub-player is headless (rendered via render_stage_to_bitmap into
    // a bitmap), so it must never resize the shared renderer — doing so
    // reprojected the host's content to the sub's dimensions, zooming the whole
    // stage. Only the host (active id 0) owns the on-screen renderer.
    if unsafe { crate::player::ACTIVE_PLAYER_ID } != 0 {
        return;
    }
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

/// Inverse of `canvas_to_movie_coords` for a whole rect: a movie-space
/// [left, top, right, bottom] mapped into host-canvas coordinates through the
/// drawRect scaling.
pub fn movie_rect_to_canvas(
    rect: [f64; 4],
    draw_rect: [f64; 4],
    movie_w: f64,
    movie_h: f64,
) -> [f64; 4] {
    let draw_w = (draw_rect[2] - draw_rect[0]).max(1.0);
    let draw_h = (draw_rect[3] - draw_rect[1]).max(1.0);
    if movie_w <= 0.0 || movie_h <= 0.0 {
        return rect;
    }
    let sx = draw_w / movie_w;
    let sy = draw_h / movie_h;
    [
        draw_rect[0] + rect[0] * sx,
        draw_rect[1] + rect[1] * sy,
        draw_rect[0] + rect[2] * sx,
        draw_rect[1] + rect[3] * sy,
    ]
}

pub fn get_stage_prop(player: &mut DirPlayer, prop: Symbol) -> Result<Datum, ScriptError> {
    match prop.into_builtin() {
        // A window's `movie` property is the Movie playing in it (Director 11.5
        // Scripting Dictionary, Window object). The Stage is a window — see
        // `windowList`: "The Stage is also considered a window" — and it plays
        // the current movie, so `_player.windowList[1].movie` resolves to the
        // Movie object. AreaZero's `[M] Main.InitGlobals` stores it as
        // `gSystem[#parent]`.
        Some(BuiltInSymbol::Movie) => Ok(Datum::MovieRef),
        Some(BuiltInSymbol::Rect) => Ok(Datum::Rect(stage_layout(player).stage_rect, 0)),
        Some(BuiltInSymbol::DrawRect) => Ok(Datum::Rect(stage_layout(player).draw_rect, 0)),
        Some(BuiltInSymbol::SourceRect) => {
            // TODO where does this come from?
            Ok(Datum::Rect([0.0, 0.0, player.movie.rect.width() as f64, player.movie.rect.height() as f64], 0))
        }
        Some(BuiltInSymbol::BgColor) => Ok(Datum::ColorRef(player.bg_color.clone())),
        Some(BuiltInSymbol::Image) => {
            // `(the stage).image` is a *live, writable* handle to the stage
            // framebuffer (Director 11.5 Scripting Dictionary — drawing into
            // it via draw()/copyPixels()/fill() appears on screen). We back it
            // with one persistent bitmap reused across calls, so a cached
            // `theStage = (the stage).image` keeps accumulating draws (the
            // "imaging Lingo" engine pattern used by spectral-wizard et al.).
            //
            // While no script has drawn into it (`stage_image_dirty == false`),
            // we refresh its contents from the current render on every access
            // — this preserves the read-only snapshot behavior camera-capture
            // movies rely on. Once a draw marks it dirty, the renderer
            // composites it over the sprite output and we leave its pixels
            // alone here.
            let has_clean_existing = match player.stage_image {
                Some(existing) => {
                    player.bitmap_manager.get_bitmap(existing).is_some()
                        && player.stage_image_dirty
                }
                None => false,
            };
            // A dirty existing image is returned as-is; only build a fresh
            // render snapshot when we need to create or refresh (clean) it.
            // (capture_stage_bitmap runs a full draw_frame, so skip it when
            // possible.)
            if has_clean_existing {
                return Ok(Datum::BitmapRef(player.stage_image.unwrap()));
            }

            let mut snapshot = None;
            with_renderer_mut(|renderer_opt| {
                if let Some(renderer) = renderer_opt {
                    snapshot = Some(renderer.capture_stage_bitmap(player));
                }
            });
            let mut snapshot = snapshot.unwrap_or_else(|| {
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
            });
            // The renderer draws a stretched stage at the canvas's size, but
            // `(the stage).image` is the movie's own stage: scripts crop it in
            // movie coordinates. Captured at a 1.6x canvas, a crop of the
            // stage centre came from the wrong place and at the wrong scale.
            let mut snapshot = stage_snapshot_at_movie_size(player, snapshot);
            // The stage framebuffer is OPAQUE — Director's `(the stage).image`
            // has no alpha channel. `capture_stage_bitmap` flags its result
            // use_alpha=true, but if the persistent stage image keeps that
            // flag, `copyPixels` of an alpha source onto it writes transparent
            // pixels verbatim (as black) instead of skipping them — imaging-
            // Lingo movies that blit an alpha bubble/dialog onto the stage
            // (spectral-wizard: `theStage.copyPixels(talkBoxBuffer)`) then get
            // a black box around the shape. Force opaque so the alpha source's
            // transparent pixels are skipped and the scene shows through.
            snapshot.use_alpha = false;

            match player.stage_image {
                Some(existing) if player.bitmap_manager.get_bitmap(existing).is_some() => {
                    // Clean existing image — refresh from the live render so
                    // camera-capture reads see current sprite content.
                    if let Some(dst) = player.bitmap_manager.get_bitmap_mut(existing) {
                        *dst = snapshot;
                    }
                    Ok(Datum::BitmapRef(existing))
                }
                _ => {
                    let bitmap_id = player.bitmap_manager.add_bitmap(snapshot);
                    player.stage_image = Some(bitmap_id);
                    player.stage_image_dirty = false;
                    Ok(Datum::BitmapRef(bitmap_id))
                }
            }
        }
        Some(BuiltInSymbol::Name) => Ok(Datum::String("stage".to_string())),
        _ => return Err(ScriptError::new(format!("Invalid stage property {}", prop))),
    }
}

/// Bring a canvas-sized stage capture back to the movie's size: the part of
/// the canvas the movie is drawn into (`draw_rect`), box-averaged down to one
/// pixel per movie pixel. A capture already at movie size is returned as is.
fn stage_snapshot_at_movie_size(player: &DirPlayer, snapshot: Bitmap) -> Bitmap {
    let mw = player.movie.rect.width().max(1) as u32;
    let mh = player.movie.rect.height().max(1) as u32;
    if snapshot.width as u32 == mw && snapshot.height as u32 == mh {
        return snapshot;
    }
    let layout = stage_layout(player);
    let [x0, y0, x1, y1] = layout.draw_rect;
    let sx = (x1 - x0).max(1.0) / mw as f64;
    let sy = (y1 - y0).max(1.0) / mh as f64;
    resample_region(&snapshot, x0, y0, sx, sy, mw, mh)
}

/// Box-average a region of a 32-bit bitmap into a `w` x `h` bitmap, where
/// each target pixel covers `sx` x `sy` source pixels starting at `(x0, y0)`.
fn resample_region(src: &Bitmap, x0: f64, y0: f64, sx: f64, sy: f64, w: u32, h: u32) -> Bitmap {
    let mut out = Bitmap::new(w as u16, h as u16, 32, 32, 0, src.palette_ref.clone());
    out.use_alpha = src.use_alpha;
    let (sw, sh) = (src.width as i64, src.height as i64);
    for ty in 0..h as i64 {
        let ya = (y0 + ty as f64 * sy).floor() as i64;
        let yb = ((y0 + (ty + 1) as f64 * sy).ceil() as i64).max(ya + 1);
        for tx in 0..w as i64 {
            let xa = (x0 + tx as f64 * sx).floor() as i64;
            let xb = ((x0 + (tx + 1) as f64 * sx).ceil() as i64).max(xa + 1);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in ya.max(0)..yb.min(sh) {
                for x in xa.max(0)..xb.min(sw) {
                    let i = ((y * sw + x) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += src.data[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((ty * w as i64 + tx) * 4) as usize;
            if n > 0 {
                for c in 0..4 {
                    out.data[o + c] = (acc[c] / n) as u8;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod snapshot_tests {
    use super::resample_region;
    use crate::player::bitmap::bitmap::{Bitmap, BuiltInPalette, PaletteRef};

    #[test]
    fn a_doubled_capture_comes_back_at_movie_size() {
        // 4x2 capture of a 2x1 stage drawn at 2x: left half red, right blue.
        let mut src = Bitmap::new(4, 2, 32, 32, 0, PaletteRef::BuiltIn(BuiltInPalette::SystemWin));
        for y in 0..2usize {
            for x in 0..4usize {
                let px = if x < 2 { [200, 0, 0, 255] } else { [0, 0, 200, 255] };
                let i = (y * 4 + x) * 4;
                src.data[i..i + 4].copy_from_slice(&px);
            }
        }
        let out = resample_region(&src, 0.0, 0.0, 2.0, 2.0, 2, 1);
        assert_eq!((out.width, out.height), (2, 1));
        assert_eq!(&out.data[0..4], &[200, 0, 0, 255]);
        assert_eq!(&out.data[4..8], &[0, 0, 200, 255]);
    }
}

pub fn set_stage_prop(
    player: &mut DirPlayer,
    prop: Symbol,
    value: &DatumRef,
) -> Result<(), ScriptError> {
    match prop.into_builtin() {
        Some(BuiltInSymbol::Title) => {
            let value = player.get_datum(value).clone();
            player.title = value.string_value()?;
            Ok(())
        }
        Some(BuiltInSymbol::BgColor) => {
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
        Some(BuiltInSymbol::DrawRect | BuiltInSymbol::Rect) => {
            let value = player.get_datum(value).clone();
            match value {
                Datum::Rect(r, _) => {
                    let w = (r[2] - r[0]).max(1.0) as u32;
                    let h = (r[3] - r[1]).max(1.0) as u32;
                    if prop.into_builtin().unwrap() == BuiltInSymbol::DrawRect {
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
        Some(BuiltInSymbol::SourceRect) => Ok(()),
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
    use super::{compute_stage_layout, movie_rect_to_canvas, StretchStyle};

    #[test]
    fn movie_rect_maps_through_a_letterboxed_draw_rect() {
        // 640x480 movie drawn at 1000x750 starting 125 px down.
        let r = movie_rect_to_canvas([64.0, 48.0, 320.0, 96.0], [0.0, 125.0, 1000.0, 875.0], 640.0, 480.0);
        assert_eq!(r, [100.0, 200.0, 500.0, 275.0]);
    }

    #[test]
    fn movie_rect_is_unchanged_at_one_to_one() {
        let r = movie_rect_to_canvas([10.0, 20.0, 30.0, 40.0], [0.0, 0.0, 640.0, 480.0], 640.0, 480.0);
        assert_eq!(r, [10.0, 20.0, 30.0, 40.0]);
    }

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
