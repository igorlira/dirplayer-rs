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
        // Fullscreen with no explicit style asked for: lay the stage out to the
        // container, aspect preserved. Without this the canvas stays the movie's
        // own size (`None`) and the frontend has to CSS-upscale it to fill the
        // screen — a bitmap blow-up, so a 640x480 movie on a 1920x1440 screen is
        // three times as blurry as it needs to be. `Meet` instead resizes the
        // canvas and scales the sprite rects, so 3D re-renders at the real
        // resolution and text is re-rasterised rather than stretched.
        //
        // An EXPLICIT swStretchStyle wins even in fullscreen — a host that asked
        // for `fill` or `stage` means it, and silently overriding would change
        // the aspect ratio it chose.
        _ if player.fullscreen_active => StretchStyle::Meet,
        _ => StretchStyle::None,
    }
}

fn compute_stage_layout(
    movie_width: f64,
    movie_height: f64,
    stage_width: u32,
    stage_height: u32,
    style: StretchStyle,
    snap_integer: bool,
) -> StageLayout {
    let movie_width = movie_width.max(1.0);
    let movie_height = movie_height.max(1.0);
    let stage_width = stage_width.max(1);
    let stage_height = stage_height.max(1);

    match style {
        StretchStyle::Meet => {
            let scale = f64::min(stage_width as f64 / movie_width, stage_height as f64 / movie_height);
            // Optionally snap the magnification to a whole number.
            //
            // The exact aspect fit is almost always fractional — 640x480 on a
            // 1920x1080 screen is 2.25x — and everything the renderer can only
            // MAGNIFY (bitmap art, 3D camera overlays, film-loop offscreens, the
            // cursor) is then resampled unevenly: a source pixel becomes two
            // destination pixels here and three there, which is what reads as
            // "pixelated" even though no detail was lost. At a whole factor every
            // source pixel becomes the same square and the result is uniform.
            //
            // Text is unaffected either way — it is re-rasterised at the scale
            // rather than magnified — so this trades sharper ART against a
            // smaller picture: 2x of a 640x480 movie is 1280x960, so the
            // letterbox grows on both axes instead of only the sides. Which
            // reads better depends on the movie, hence a toggle and not a policy.
            //
            // Only ever snaps DOWN, and never below 1.0 — a container smaller
            // than the movie is a minification, where none of this applies and
            // flooring would crop the movie.
            let scale = if snap_integer && scale >= 1.0 { scale.floor() } else { scale };
            let draw_width = movie_width * scale;
            let draw_height = movie_height * scale;
            let left = ((stage_width as f64 - draw_width) / 2.0).max(0.0);
            let top = ((stage_height as f64 - draw_height) / 2.0).max(0.0);
            StageLayout {
                canvas_width: stage_width,
                canvas_height: stage_height,
                // The MOVIE's rect, not the container's. `stage_rect` is what
                // Lingo sees as `the stage.rect` / stageLeft / stageTop /
                // stageRight / stageBottom, and every other coordinate a script
                // handles is in movie space — sprite locs, member rects, mouseH.
                // Reporting the container here hands scripts a number in a
                // different unit from everything around it. Habbo v7's
                // `Window Instance Class` centres every window with
                //     tX = ((the stageRight - the stageLeft) / 2) - (pwidth / 2)
                // so on a scaled stage the help window, catalogue, purse, trade
                // and alert all centred against 1920x1080 in a 720x540 movie and
                // flew off the bottom-right. Director's fullscreen kept the
                // stage the same size and changed the SCREEN mode; the scaling
                // lives in `draw_rect`, which is where the renderer reads it.
                stage_rect: [0.0, 0.0, movie_width, movie_height],
                draw_rect: [left, top, left + draw_width, top + draw_height],
            }
        }
        StretchStyle::Fill => StageLayout {
            canvas_width: stage_width,
            canvas_height: stage_height,
            // Movie rect — same reasoning as Meet. Fill stretches the movie to
            // the container, so scripts still work in movie coordinates.
            stage_rect: [0.0, 0.0, movie_width, movie_height],
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

/// How many DEVICE pixels the canvas gets per CSS pixel of the stage
/// container. 1.0 unless the frontend has reported a hi-DPI display.
///
/// Clamped rather than trusted: `devicePixelRatio` is whatever the page says it
/// is, and a nonsense value here would size a GPU texture.
pub fn stage_pixel_ratio(player: &DirPlayer) -> f64 {
    let r = player.stage_pixel_ratio;
    if r.is_finite() && r > 0.0 { r.clamp(1.0, 4.0) } else { 1.0 }
}

/// The stage layout in CSS pixels — the unit `stage_size` arrives in and the
/// unit the page lays the canvas out in.
fn stage_layout_css(player: &DirPlayer) -> StageLayout {
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
            player.stage_scale_snap_integer,
        )
    }
}

/// The stage layout the RENDERER works in: the CSS layout above in device
/// pixels.
///
/// A phone hands the page ~873x393 CSS pixels and puts two or three physical
/// pixels behind each one. Sizing the canvas in CSS pixels throws that away:
/// the movie is rendered into an 873x393 buffer and the compositor blows it up
/// to the 2400x1080 the screen actually has, so a 760x520 movie is first
/// MINIFIED to 0.756 and then magnified back — nothing the renderer can
/// re-rasterise (text, 3D) ever sees the resolution it is being shown at, and
/// anything the movie composes itself into a bitmap (the Coke Studios navigator
/// room list) is baked at 0.756 and unreadable. Rendering at the device size
/// instead turns that same phone into a 2.08x MAGNIFICATION, which is a scale
/// every part of the pipeline already handles.
///
/// `stage_rect` is deliberately NOT scaled: it is what Lingo sees as
/// `the stage.rect`, and every script-facing coordinate is in movie units.
pub fn stage_layout(player: &DirPlayer) -> StageLayout {
    let layout = stage_layout_css(player);
    let dpr = stage_pixel_ratio(player);
    if (dpr - 1.0).abs() < 1e-6 {
        return layout;
    }
    StageLayout {
        canvas_width: ((layout.canvas_width as f64 * dpr).round() as u32).max(1),
        canvas_height: ((layout.canvas_height as f64 * dpr).round() as u32).max(1),
        stage_rect: layout.stage_rect,
        draw_rect: [
            layout.draw_rect[0] * dpr,
            layout.draw_rect[1] * dpr,
            layout.draw_rect[2] * dpr,
            layout.draw_rect[3] * dpr,
        ],
    }
}

/// Dimensions of the stage canvas' BACKING STORE, in device pixels: explicit
/// drawRect if Lingo set one, otherwise the effective layout derived from
/// `swStretchStyle`.
pub fn stage_canvas_dims(player: &DirPlayer) -> (u32, u32) {
    let layout = stage_layout(player);
    (layout.canvas_width, layout.canvas_height)
}

/// Dimensions the canvas must be LAID OUT at, in CSS pixels. On a hi-DPI
/// display this is smaller than `stage_canvas_dims` by the pixel ratio, and it
/// is what the frontend sizes `#stage_canvas_container` to — without it the
/// canvas would take its intrinsic device-pixel size as its CSS size and
/// overflow the viewport by that same ratio.
pub fn stage_css_dims(player: &DirPlayer) -> (u32, u32) {
    let layout = stage_layout_css(player);
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
    let (css_w, css_h) = stage_css_dims(player);
    with_renderer_mut(|renderer_opt| {
        if let Some(renderer) = renderer_opt {
            use crate::rendering_gpu::Renderer;
            renderer.set_size(draw_w, draw_h);
            // `set_size` sets the BACKING STORE. On a hi-DPI display that is
            // larger than the canvas' CSS box by the pixel ratio, and a canvas
            // with no CSS size lays itself out at its backing size — so without
            // this the stage would overflow its container by exactly the ratio
            // that was supposed to make it sharper.
            if css_w != draw_w || css_h != draw_h {
                let style = renderer.canvas().style();
                let _ = style.set_property("width", &format!("{css_w}px"));
                let _ = style.set_property("height", &format!("{css_h}px"));
            }
        }
    });
}

/// Convert host-canvas pixel coords to movie-space coords, inverting the
/// drawRect scaling so Lingo's mouseH/mouseV and script-facing APIs see the
/// authored coordinate system.
///
/// `x`/`y` are CSS pixels — a pointer event's position within the canvas
/// element, which is what every caller gets from the DOM. `draw_rect` is in
/// device pixels, so the ratio between the two has to go in here; this is the
/// only place canvas coordinates enter the player, so it is the only place that
/// needs it.
pub fn canvas_to_movie_coords(player: &DirPlayer, x: f64, y: f64) -> (f64, f64) {
    let dpr = stage_pixel_ratio(player);
    let (x, y) = (x * dpr, y * dpr);
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

            // `(the stage).image` is addressed in MOVIE coordinates — a script
            // that does `theStage.copyPixels(src, rect(0, 0, 320, 240), ...)`
            // means movie pixels, and `the stage.rect` is what it sizes against.
            // `capture_stage_bitmap` answers at the CANVAS size, which a scaled
            // stage has enlarged, so the persistent image came back 1920x1080
            // for a 640x480 movie: every draw landed in the top-left quarter and
            // the compositor (which scales by canvas/image) then had nothing left
            // to scale. Worse, the bitmap is persistent and never resized, so
            // leaving fullscreen left a 1920x1080 image against a 640x480 canvas
            // and the overlay shrank to a third. Render at the movie's own size
            // whenever the two disagree.
            let movie_w = player.movie.rect.width().max(1);
            let movie_h = player.movie.rect.height().max(1);
            let stage_is_scaled = {
                let (cw, ch) = stage_canvas_dims(player);
                cw != movie_w as u32 || ch != movie_h as u32
            };
            let mut snapshot = None;
            if !stage_is_scaled {
                with_renderer_mut(|renderer_opt| {
                    if let Some(renderer) = renderer_opt {
                        snapshot = Some(renderer.capture_stage_bitmap(player));
                    }
                });
            }
            let mut snapshot = snapshot.unwrap_or_else(|| {
                // Movie size, not `stage_rect` — under `swStretchStyle = meet`
                // the stage rect IS the container, which is the same mismatch
                // described above.
                let (w, h) = (movie_w, movie_h);
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
    use super::{compute_stage_layout, StretchStyle};

    #[test]
    fn stretch_meet_letterboxes_inside_stage() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 1000, StretchStyle::Meet, false);
        assert_eq!(layout.canvas_width, 1000);
        assert_eq!(layout.canvas_height, 1000);
        // The MOVIE rect, not the container — scripts work in movie coordinates.
        assert_eq!(layout.stage_rect, [0.0, 0.0, 640.0, 480.0]);
        assert_eq!(layout.draw_rect, [0.0, 125.0, 1000.0, 875.0]);
    }

    #[test]
    fn stretch_meet_snaps_scale_to_a_whole_number() {
        // 640x480 in 1920x1080 fits at 2.25x; snapped it is 2x, centred, with
        // the letterbox now on both axes instead of only the sides.
        let layout = compute_stage_layout(640.0, 480.0, 1920, 1080, StretchStyle::Meet, true);
        assert_eq!(layout.draw_rect, [320.0, 60.0, 1600.0, 1020.0]);
        let exact = compute_stage_layout(640.0, 480.0, 1920, 1080, StretchStyle::Meet, false);
        assert_eq!(exact.draw_rect, [240.0, 0.0, 1680.0, 1080.0]);
    }

    #[test]
    fn stretch_meet_snap_never_minifies_below_one() {
        // Container SMALLER than the movie: flooring would give 0 and crop.
        let layout = compute_stage_layout(640.0, 480.0, 320, 240, StretchStyle::Meet, true);
        assert_eq!(layout.draw_rect, [0.0, 0.0, 320.0, 240.0]);
    }

    #[test]
    fn stretch_fill_matches_container() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 600, StretchStyle::Fill, false);
        assert_eq!(layout.canvas_width, 1000);
        assert_eq!(layout.canvas_height, 600);
        // Movie rect — same reasoning as Meet.
        assert_eq!(layout.stage_rect, [0.0, 0.0, 640.0, 480.0]);
        assert_eq!(layout.draw_rect, [0.0, 0.0, 1000.0, 600.0]);
    }

    #[test]
    fn stretch_stage_resizes_stage_without_scaling_content() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 600, StretchStyle::Stage, false);
        assert_eq!(layout.canvas_width, 1000);
        assert_eq!(layout.canvas_height, 600);
        assert_eq!(layout.stage_rect, [0.0, 0.0, 1000.0, 600.0]);
        assert_eq!(layout.draw_rect, [0.0, 0.0, 640.0, 480.0]);
    }

    #[test]
    fn stretch_none_keeps_authored_movie_size() {
        let layout = compute_stage_layout(640.0, 480.0, 1000, 600, StretchStyle::None, false);
        assert_eq!(layout.canvas_width, 640);
        assert_eq!(layout.canvas_height, 480);
        assert_eq!(layout.stage_rect, [0.0, 0.0, 640.0, 480.0]);
        assert_eq!(layout.draw_rect, [0.0, 0.0, 640.0, 480.0]);
    }
}
