/// PFR1 Rasterizer - Outline-to-bitmap rendering
/// Implements: Bezier flattening, non-zero winding scanline fill, grid bitmap assembly

use super::types::*;
use super::glyph::fixed_point_multiply16;
use super::log;

/// Rasterize a single glyph using the browser's Canvas2D path fill.
/// This leverages the browser's native rasterizer which handles edge cases
/// (self-intersecting contours, sub-pixel alignment) more accurately than
/// our custom scanline fill.
/// Returns an alpha mask (one byte per pixel) or None if Canvas2D is unavailable.
#[cfg(target_arch = "wasm32")]
fn rasterize_glyph_canvas2d(
    ctx: &web_sys::CanvasRenderingContext2d,
    glyph: &OutlineGlyph,
    cell_width: usize,
    cell_height: usize,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Option<Vec<u8>> {
    ctx.clear_rect(0.0, 0.0, cell_width as f64, cell_height as f64);

    // Use f64 for all coordinate arithmetic to avoid f32 precision loss
    let sx = scale_x as f64;
    let sy = scale_y as f64;
    let ox = offset_x as f64;
    let oy = offset_y as f64;

    // Build path from all contours (Canvas2D uses non-zero winding by default)
    ctx.begin_path();
    for contour in &glyph.contours {
        for cmd in &contour.commands {
            let x = cmd.x as f64 * sx + ox;
            let y = cmd.y as f64 * sy + oy;
            match cmd.cmd_type {
                PfrCmdType::MoveTo => ctx.move_to(x, y),
                PfrCmdType::LineTo => ctx.line_to(x, y),
                PfrCmdType::CurveTo => {
                    let x1 = cmd.x1 as f64 * sx + ox;
                    let y1 = cmd.y1 as f64 * sy + oy;
                    let x2 = cmd.x2 as f64 * sx + ox;
                    let y2 = cmd.y2 as f64 * sy + oy;
                    ctx.bezier_curve_to(x1, y1, x2, y2, x, y);
                }
                PfrCmdType::Close => ctx.close_path(),
            }
        }
        ctx.close_path(); // Ensure sub-path is closed for proper winding
    }

    ctx.set_fill_style_str("black");
    // Use non-zero winding fill rule.
    // PFR1 fonts use opposite winding for inner/outer contours, so non-zero winding
    // correctly creates holes in characters like 'o', 'e', 'q', etc.
    // Canvas2D's default fill() already uses non-zero winding, but we specify explicitly.
    {
        
        let fill_rule = wasm_bindgen::JsValue::from_str("nonzero");
        let _ = js_sys::Reflect::apply(
            &js_sys::Function::from(
                js_sys::Reflect::get(ctx.as_ref(), &"fill".into()).unwrap_or(wasm_bindgen::JsValue::UNDEFINED)
            ),
            ctx.as_ref(),
            &js_sys::Array::of1(&fill_rule),
        );
    }

    // Read back pixel data and extract alpha channel.
    // Keep raw anti-aliased coverage values from Canvas2D — no binary threshold.
    // The PFR zone table transformation already maps coordinates to pixel space,
    // so Canvas2D's native rasterizer produces correct coverage naturally.
    let image_data = ctx
        .get_image_data(0.0, 0.0, cell_width as f64, cell_height as f64)
        .ok()?;
    let data = image_data.data();
    let mut alpha = vec![0u8; cell_width * cell_height];

    for i in 0..cell_width * cell_height {
        alpha[i] = data[i * 4 + 3];
    }

    Some(alpha)
}

/// Flatten a cubic bezier curve into line segments using de Casteljau subdivision
fn flatten_cubic_bezier(
    x0: f32, y0: f32,
    x1: f32, y1: f32,
    x2: f32, y2: f32,
    x3: f32, y3: f32,
    tolerance: f32,
    output: &mut Vec<(f32, f32)>,
) {
    // Check if the curve is flat enough
    let dx = x3 - x0;
    let dy = y3 - y0;
    let d2 = ((x1 - x3) * dy - (y1 - y3) * dx).abs();
    let d3 = ((x2 - x3) * dy - (y2 - y3) * dx).abs();

    let flatness = (d2 + d3) * (d2 + d3);
    let tolerance_sq = tolerance * tolerance * (dx * dx + dy * dy);

    if flatness <= tolerance_sq {
        output.push((x3, y3));
        return;
    }

    // Subdivide at t=0.5
    let x01 = (x0 + x1) * 0.5;
    let y01 = (y0 + y1) * 0.5;
    let x12 = (x1 + x2) * 0.5;
    let y12 = (y1 + y2) * 0.5;
    let x23 = (x2 + x3) * 0.5;
    let y23 = (y2 + y3) * 0.5;

    let x012 = (x01 + x12) * 0.5;
    let y012 = (y01 + y12) * 0.5;
    let x123 = (x12 + x23) * 0.5;
    let y123 = (y12 + y23) * 0.5;

    let x0123 = (x012 + x123) * 0.5;
    let y0123 = (y012 + y123) * 0.5;

    flatten_cubic_bezier(x0, y0, x01, y01, x012, y012, x0123, y0123, tolerance, output);
    flatten_cubic_bezier(x0123, y0123, x123, y123, x23, y23, x3, y3, tolerance, output);
}

/// Convert contour commands to line segments (flattening curves)
fn contour_to_edges(contour: &PfrContour, tolerance: f32) -> Vec<(f32, f32)> {
    let mut points: Vec<(f32, f32)> = Vec::new();
    let mut cur_x: f32 = 0.0;
    let mut cur_y: f32 = 0.0;

    for cmd in &contour.commands {
        match cmd.cmd_type {
            PfrCmdType::MoveTo => {
                cur_x = cmd.x;
                cur_y = cmd.y;
                points.push((cur_x, cur_y));
            }
            PfrCmdType::LineTo => {
                cur_x = cmd.x;
                cur_y = cmd.y;
                points.push((cur_x, cur_y));
            }
            PfrCmdType::CurveTo => {
                flatten_cubic_bezier(
                    cur_x, cur_y,
                    cmd.x1, cmd.y1,
                    cmd.x2, cmd.y2,
                    cmd.x, cmd.y,
                    tolerance,
                    &mut points,
                );
                cur_x = cmd.x;
                cur_y = cmd.y;
            }
            PfrCmdType::Close => {
                // Close is handled implicitly by the polygon
            }
        }
    }

    points
}

/// Rasterize a single glyph outline to a bitmap using winding number fill rule
fn rasterize_glyph_to_bitmap(
    glyph: &OutlineGlyph,
    width: usize,
    height: usize,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Vec<u8> {
    rasterize_glyph_to_bitmap_oversampled(glyph, width, height, scale_x, scale_y, offset_x, offset_y, 1)
}

fn rasterize_glyph_to_alpha_mask(
    glyph: &OutlineGlyph,
    width: usize,
    height: usize,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
    oversample: usize,
) -> Vec<u8> {
    if oversample <= 1 {
        let bitmap = rasterize_glyph_to_bitmap_raw(glyph, width, height, scale_x, scale_y, offset_x, offset_y);
        let bytes_per_row = (width + 7) / 8;
        let mut alpha = vec![0u8; width * height];
        for y in 0..height {
            for x in 0..width {
                let byte_idx = y * bytes_per_row + x / 8;
                let bit_idx = 7 - (x % 8);
                if byte_idx < bitmap.len() && (bitmap[byte_idx] & (1 << bit_idx)) != 0 {
                    alpha[y * width + x] = 255;
                }
            }
        }
        return alpha;
    }

    let hi_w = width * oversample;
    let hi_h = height * oversample;
    let hi_scale_x = scale_x * oversample as f32;
    let hi_scale_y = scale_y * oversample as f32;
    let hi_offset_x = offset_x * oversample as f32;
    let hi_offset_y = offset_y * oversample as f32;

    let hi_bitmap = rasterize_glyph_to_bitmap_raw(glyph, hi_w, hi_h, hi_scale_x, hi_scale_y, hi_offset_x, hi_offset_y);
    let hi_bpr = (hi_w + 7) / 8;

    let mut alpha = vec![0u8; width * height];
    let block = (oversample * oversample) as u32;

    for y in 0..height {
        for x in 0..width {
            let mut count = 0u32;
            let base_x = x * oversample;
            let base_y = y * oversample;
            for oy in 0..oversample {
                let hy = base_y + oy;
                let row_base = hy * hi_bpr;
                for ox in 0..oversample {
                    let hx = base_x + ox;
                    let byte_idx = row_base + hx / 8;
                    let bit_idx = 7 - (hx % 8);
                    if byte_idx < hi_bitmap.len() && (hi_bitmap[byte_idx] & (1 << bit_idx)) != 0 {
                        count += 1;
                    }
                }
            }
            let raw_coverage = (count * 255 / block) as u8;
            // Apply gamma boost (gamma=0.5) to make anti-aliased edges more prominent.
            // This makes text bolder and more readable at small sizes.
            let coverage = if raw_coverage == 0 || raw_coverage == 255 {
                raw_coverage
            } else {
                let norm = raw_coverage as f32 / 255.0;
                (norm.sqrt() * 255.0).round().min(255.0) as u8
            };
            alpha[y * width + x] = coverage;
        }
    }

    alpha
}

fn rasterize_glyph_to_bitmap_oversampled(
    glyph: &OutlineGlyph,
    width: usize,
    height: usize,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
    oversample: usize,
) -> Vec<u8> {
    if oversample <= 1 {
        return rasterize_glyph_to_bitmap_raw(glyph, width, height, scale_x, scale_y, offset_x, offset_y);
    }

    let hi_w = width * oversample;
    let hi_h = height * oversample;
    let hi_scale_x = scale_x * oversample as f32;
    let hi_scale_y = scale_y * oversample as f32;
    let hi_offset_x = offset_x * oversample as f32;
    let hi_offset_y = offset_y * oversample as f32;

    let hi_bitmap = rasterize_glyph_to_bitmap_raw(glyph, hi_w, hi_h, hi_scale_x, hi_scale_y, hi_offset_x, hi_offset_y);
    let hi_bpr = (hi_w + 7) / 8;

    let bytes_per_row = (width + 7) / 8;
    let mut bitmap = vec![0u8; bytes_per_row * height];

    let block = oversample * oversample;
    let threshold = 1; // any sub-pixel filled → pixel filled (captures thin strokes)

    for y in 0..height {
        for x in 0..width {
            let mut count = 0usize;
            let base_x = x * oversample;
            let base_y = y * oversample;
            for oy in 0..oversample {
                let hy = base_y + oy;
                let row_base = hy * hi_bpr;
                for ox in 0..oversample {
                    let hx = base_x + ox;
                    let byte_idx = row_base + hx / 8;
                    let bit_idx = 7 - (hx % 8);
                    if byte_idx < hi_bitmap.len() && (hi_bitmap[byte_idx] & (1 << bit_idx)) != 0 {
                        count += 1;
                    }
                }
            }
            if count >= threshold {
                let byte_idx = y * bytes_per_row + x / 8;
                let bit_idx = 7 - (x % 8);
                if byte_idx < bitmap.len() {
                    bitmap[byte_idx] |= 1 << bit_idx;
                }
            }
        }
    }

    bitmap
}

fn rasterize_glyph_to_bitmap_raw(
    glyph: &OutlineGlyph,
    width: usize,
    height: usize,
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
) -> Vec<u8> {
    let bytes_per_row = (width + 7) / 8;
    let mut bitmap = vec![0u8; bytes_per_row * height];

    // Flatten all contours to polygon edges
    let mut all_polygons: Vec<Vec<(f32, f32)>> = Vec::new();
    for contour in &glyph.contours {
        let points = contour_to_edges(contour, 0.5);
        if points.len() >= 3 {
            all_polygons.push(points);
        }
    }

    if all_polygons.is_empty() {
        return bitmap;
    }

    // Scanline rasterization with non-zero winding fill rule.
    // Matches SKPathFillType.Winding — PFR1 fonts use opposite
    // winding for inner/outer contours, so non-zero winding correctly creates
    // holes (counters) in characters like 'o', 'e', 'q', etc.
    for y in 0..height {
        let scan_y = y as f32 + 0.5;

        // Collect edge crossings with winding direction:
        // (x_crossing, direction) where direction is +1 (upward) or -1 (downward)
        let mut crossings: Vec<(f32, i32)> = Vec::new();

        for polygon in &all_polygons {
            let n = polygon.len();
            for i in 0..n {
                let (mut x0, mut y0) = polygon[i];
                let (mut x1, mut y1) = polygon[(i + 1) % n];

                // Transform to bitmap coordinates
                x0 = x0 * scale_x + offset_x;
                y0 = y0 * scale_y + offset_y;
                x1 = x1 * scale_x + offset_x;
                y1 = y1 * scale_y + offset_y;

                // Skip horizontal edges
                if (y0 - y1).abs() < 0.001 {
                    continue;
                }

                // Check if scanline crosses this edge
                if (y0 <= scan_y && y1 > scan_y) || (y1 <= scan_y && y0 > scan_y) {
                    let t = (scan_y - y0) / (y1 - y0);
                    let x_cross = x0 + t * (x1 - x0);
                    // Direction: +1 if edge goes upward (y1 > y0), -1 if downward
                    let dir = if y1 > y0 { 1 } else { -1 };
                    crossings.push((x_cross, dir));
                }
            }
        }

        // Sort crossings by x
        crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // Fill using non-zero winding rule: track winding count across crossings,
        // fill spans where winding != 0
        if crossings.len() >= 2 {
            let mut winding: i32 = 0;
            for ci in 0..crossings.len() - 1 {
                winding += crossings[ci].1;
                if winding != 0 {
                    let x0 = crossings[ci].0;
                    let x1 = crossings[ci + 1].0;
                    // Use pixel-center sampling: fill pixel bx if center (bx+0.5) ∈ [x0, x1)
                    let x_start = (x0 - 0.5).ceil().max(0.0).min(width as f32) as usize;
                    let x_end = (x1 - 0.5).ceil().max(0.0).min(width as f32) as usize;

                    for bx in x_start..x_end {
                        let byte_idx = y * bytes_per_row + bx / 8;
                        let bit_idx = 7 - (bx % 8);
                        if byte_idx < bitmap.len() {
                            bitmap[byte_idx] |= 1 << bit_idx;
                        }
                    }
                }
            }
        }
    }

    bitmap
}

/// Result of rasterizing a PFR1 font
pub struct RasterizedFont {
    /// RGBA bitmap data for the entire glyph grid
    pub bitmap_data: Vec<u8>,
    /// Width of the bitmap in pixels
    pub bitmap_width: usize,
    /// Height of the bitmap in pixels
    pub bitmap_height: usize,
    /// Width of each grid cell
    pub cell_width: usize,
    /// Height of each grid cell
    pub cell_height: usize,
    /// Number of grid columns
    pub grid_columns: usize,
    /// Number of grid rows
    pub grid_rows: usize,
    /// Per-character advance widths (in pixels)
    pub char_widths: Vec<u16>,
    /// First char code in the grid
    pub first_char: u8,
    /// Number of chars
    pub num_chars: usize,
}

/// Steepen alpha ramp for crisper glyph edges.
/// Alpha below `lo` -> 0, above `hi` -> 255, between -> linear remap to 0..255.
fn steepen_alpha_ramp(alpha: &mut [u8], lo: u8, hi: u8) {
    if lo >= hi { return; }
    let range = (hi - lo) as f32;
    for a in alpha.iter_mut() {
        let v = *a;
        if v <= lo {
            *a = 0;
        } else if v >= hi {
            *a = 255;
        } else {
            *a = (((v - lo) as f32 / range) * 255.0).round() as u8;
        }
    }
}

/// Rasterize a parsed PFR1 font into a grid bitmap
/// Returns RGBA bitmap data + per-character advance widths
///
/// `design_size` is the font's native display size (e.g. 16 for Volter fonts).
/// When target_height differs from design_size, a two-step advance calculation is used:
///   1. native_advance = floor(set_width * design_size / outline_res)
///   2. target_advance = floor(native_advance * target_height / design_size)
/// This matches Shockwave's behavior and avoids off-by-one rounding at scaled sizes.
/// Pass 0 to fall back to single-step calculation.
pub fn rasterize_pfr1_font(
    parsed_font: &Pfr1ParsedFont,
    target_height: usize,
    design_size: usize,
) -> RasterizedFont {
    let phys = &parsed_font.physical_font;

    let outline_res = phys.outline_resolution as f32;
    let target_em_px = parsed_font.target_em_px as f32;
    let coords_scaled = target_em_px > 0.0 && outline_res > 0.0;

    let scale = if coords_scaled {
        // Coordinates are already in target pixel space (parsed at actual target size).
        1.0
    } else if outline_res > 0.0 {
        // Scale from ORU space to target pixel size.
        // Must use outline_res (not metric_height = ascender - descender) to match the
        // cell layout which also uses target_height / outline_res for cell dimensions.
        target_height as f32 / outline_res
    } else {
        1.0
    };

    // Apply font matrix (mA, mB, mC, mD at 1/256 scale)
    let font_matrix = if !parsed_font.logical_fonts.is_empty() {
        parsed_font.logical_fonts[0].font_matrix
    } else {
        [256, 0, 0, 256]
    };

    let matrix_scale_x = font_matrix[0] as f32 / 256.0;
    let matrix_scale_y = font_matrix[3] as f32 / 256.0;

    // Use magnitude for sizing; we apply a single Y flip below.
    let scale_x = scale * matrix_scale_x.abs();
    let scale_y = scale * matrix_scale_y.abs();

    // Determine cell dimensions
    // Find max glyph width from set_widths and from actual glyph bbox widths.
    let max_set_width = parsed_font.glyphs.values()
        .map(|g| g.set_width)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(phys.standard_set_width as f32);

    let base_set_width_scale = if coords_scaled {
        // set_width is in orus; scale directly to actual target pixel size
        target_height as f32 / outline_res
    } else {
        scale_x.abs()
    };
    let set_width_scale = base_set_width_scale;

    let mut max_bbox_width = 0.0f32;
    // For coords_scaled fonts, glyph coordinates are in pixel space after zone table
    // transformation. Skip glyphs with unreasonably large bbox — they indicate
    // zone table errors and would otherwise blow up the atlas cell width.
    let bbox_limit = if coords_scaled { target_height as f32 * 4.0 } else { f32::MAX };
    for glyph in parsed_font.glyphs.values() {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        for contour in &glyph.contours {
            for cmd in &contour.commands {
                min_x = min_x.min(cmd.x);
                max_x = max_x.max(cmd.x);
                if cmd.cmd_type == PfrCmdType::CurveTo {
                    min_x = min_x.min(cmd.x1).min(cmd.x2);
                    max_x = max_x.max(cmd.x1).max(cmd.x2);
                }
            }
        }
        if min_x < max_x {
            let w = (max_x - min_x).abs();
            if w <= bbox_limit && w > max_bbox_width {
                max_bbox_width = w;
            }
        }
    }
    let max_bbox_width_px = max_bbox_width * scale_x.abs();

    let set_width_px = max_set_width * base_set_width_scale;
    let cell_width = (set_width_px.ceil() as usize)
        .max(max_bbox_width_px.ceil() as usize)
        .max(1);
    let cell_height = {
        let pixel_scale_metric = if outline_res > 0.0 {
            target_height as f32 / outline_res
        } else {
            1.0
        };
        let descender_px = if phys.metrics.descender < 0 && outline_res > 0.0 {
            (phys.metrics.descender.abs() as f32 * pixel_scale_metric).ceil() as usize
        } else {
            0
        };
        // Baseline row = floor(ascender * pixel_scale). Cell must contain baseline + descender.
        // When ascender > outline_res, baseline exceeds target_height and the old formula
        // (target_height + descender_px) would clip the bottom of every character.
        let baseline_row = if phys.metrics.ascender > 0 && outline_res > 0.0 {
            (phys.metrics.ascender as f32 * pixel_scale_metric).floor() as usize
        } else {
            target_height
        };
        let standard_h = (baseline_row + descender_px + 1).max(target_height);
        // Use Type 5 cell height when available (matches Director line spacing)
        if phys.has_extra_item_type5 {
            let type5_cell = (phys.extra_type5_word37 as i32) - (phys.extra_type5_word36 as i32);
            let type5_h = (type5_cell * target_height as i32) / 256;
            standard_h.max(type5_h.max(0) as usize)
        } else {
            standard_h
        }
    };

    // Shared baseline for both outline and bitmap glyph rendering
    let pixel_scale = if coords_scaled {
        target_height as f32 / outline_res
    } else {
        scale_y.abs()
    };
    let baseline_row_px = if phys.metrics.ascender != 0 {
        let mut bl = phys.metrics.ascender as f32 * pixel_scale;
        bl = bl.round(); // snap to integer for crisp glyph placement
        if !bl.is_finite() { bl = 0.0; }
        bl
    } else {
        target_height as f32
    };

    // Determine character range.
    // 256 covers the full Windows-1252 byte range: ASCII (0x00-0x7F) plus
    // Western European letters in 0x80-0xFF -- German umlauts (ä ö ü ß),
    // Scandinavian (å ø æ), Spanish (á é í ó ú ñ ¿ ¡), smart quotes, Euro.
    // The previous 128-glyph cap silently dropped every non-ASCII glyph
    // present in the PFR font, leaving visible gaps where umlauts should
    // render. Atlas grows from 16x8 to 16x16 cells (one extra row of
    // rasterization work per font instance).
    let first_char: u8 = 0;
    let num_chars: usize = 256;
    let grid_columns: usize = 16;
    let grid_rows: usize = (num_chars + grid_columns - 1) / grid_columns;

    let bitmap_width = cell_width * grid_columns;
    let bitmap_height = cell_height * grid_rows;

    log(&format!("Rasterizing PFR1 font: {}x{} cells, {}x{} grid, {}x{} bitmap, scale={:.4}",
        cell_width, cell_height, grid_columns, grid_rows, bitmap_width, bitmap_height, scale));
    log(&format!(
        "Rasterize metrics: asc={} desc={} cell_height={} target_height={} scale_y={} coords_scaled={} px_scale={:.6}",
        phys.metrics.ascender,
        phys.metrics.descender,
        cell_height,
        target_height,
        scale_y,
        coords_scaled,
        if outline_res > 0.0 { target_em_px / outline_res } else { 0.0 }
    ));

    log(&format!(
        "[PFR1 rasterize] '{}': target={}px cell={}x{} scale={:.4} outline={} bitmap={} coords_scaled={} fill=nonzero",
        parsed_font.font_name,
        target_height,
        cell_width, cell_height,
        scale,
        parsed_font.glyphs.len(),
        parsed_font.bitmap_glyphs.len(),
        coords_scaled
    ));

    // Create RGBA bitmap (transparent white background)
    let mut rgba = vec![0u8; bitmap_width * bitmap_height * 4];
    for i in 0..(bitmap_width * bitmap_height) {
        let idx = i * 4;
        rgba[idx] = 255;
        rgba[idx + 1] = 255;
        rgba[idx + 2] = 255;
        rgba[idx + 3] = 0;
    }

    // Per-character advance widths
    let mut char_widths = vec![cell_width as u16; num_chars];
    let trace_bitmap_debug = parsed_font
        .font_name
        .to_ascii_lowercase()
        .contains("tiki magic");
    let mut bitmap_overlap_outline = 0usize;
    let mut bitmap_only = 0usize;
    let mut bitmap_pixels_drawn = 0usize;

    // Render each glyph
    let font_min_x = phys.metrics.x_min as f32;
    let font_asc = phys.metrics.ascender as f32;

    // Director advance width precomputation:
    // matrix2136_A = FixedPointMultiply16(targetSize << 16, matrixA << 8)
    let out_res_i = phys.outline_resolution as i32;
    let matrix2136_a = fixed_point_multiply16(
        (target_height as i32) << 16,
        font_matrix[0] << 8,
    );

    // For coords_scaled fonts, glyph coordinates are already in pixel space (post-zone-table).
    // right within their atlas cells, breaking text rendering alignment.

    // Create a reusable Canvas2D context for glyph rasterization (WASM only).
    // The browser's native path fill handles edge cases (self-intersecting contours,
    // sub-pixel alignment) more accurately than our custom scanline rasterizer.
    #[cfg(target_arch = "wasm32")]
    let canvas_ctx: Option<web_sys::CanvasRenderingContext2d> = {
        use wasm_bindgen::JsCast;
        (|| -> Option<web_sys::CanvasRenderingContext2d> {
            let document = web_sys::window()?.document()?;
            let canvas: web_sys::HtmlCanvasElement = document
                .create_element("canvas").ok()?
                .dyn_into().ok()?;
            canvas.set_width(cell_width as u32);
            canvas.set_height(cell_height as u32);
            // Use willReadFrequently to optimize repeated getImageData calls
            let context_attrs = js_sys::Object::new();
            js_sys::Reflect::set(
                &context_attrs,
                &wasm_bindgen::JsValue::from_str("willReadFrequently"),
                &wasm_bindgen::JsValue::TRUE,
            ).ok()?;
            let ctx: web_sys::CanvasRenderingContext2d = canvas
                .get_context_with_context_options("2d", &context_attrs).ok()??
                .dyn_into().ok()?;
            Some(ctx)
        })()
    };

    for (&char_code, glyph) in &parsed_font.glyphs {
        let idx = char_code as usize;
        if idx >= num_chars { continue; }

        // Director advance width (sub_6A11CC67): 16.16 fixed-point formula.
        // v2 = ((outlineRes/2 + (csw << 16)) / outlineRes
        // advance_16_16 = FixedPointMultiply16(v2, matrix2136_A)
        // Rounded to pixel: (advance + 0x8000) >> 16
        let glyph_pixel_width = if out_res_i > 0 {
            let csw = glyph.set_width as i16;
            let v2 = ((out_res_i >> 1) + ((csw as i32) << 16)) / out_res_i;
            let advance_16_16 = fixed_point_multiply16(v2, matrix2136_a);
            let advance_px = ((advance_16_16 + 0x8000) & !0xFFFF) >> 16;
            (advance_px.max(if glyph.set_width > 0.0 { 1 } else { 0 })) as usize
        } else {
            0
        };

        // Always set char_widths from outline metrics (correct spacing)
        if glyph_pixel_width > 0 {
            char_widths[idx] = glyph_pixel_width as u16;
        }

        // Skip rasterization if a bitmap glyph exists — bitmap glyphs are pixel-perfect
        // pre-rendered by the font designer. The bitmap loop will handle rendering.
        // UNLESS "outline" preference is set, in which case always rasterize from outlines.
        let prefer_outline = {
            use crate::player::font::{get_glyph_preference, GlyphPreference};
            get_glyph_preference() == GlyphPreference::Outline
        };
        if !prefer_outline && parsed_font.bitmap_glyphs.contains_key(&char_code) {
            continue;
        }

        // Find glyph bounding box
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for contour in &glyph.contours {
            for cmd in &contour.commands {
                min_x = min_x.min(cmd.x);
                min_y = min_y.min(cmd.y);
                max_x = max_x.max(cmd.x);
                max_y = max_y.max(cmd.y);
                if cmd.cmd_type == PfrCmdType::CurveTo {
                    min_x = min_x.min(cmd.x1).min(cmd.x2);
                    min_y = min_y.min(cmd.y1).min(cmd.y2);
                    max_x = max_x.max(cmd.x1).max(cmd.x2);
                    max_y = max_y.max(cmd.y1).max(cmd.y2);
                }
            }
        }

        if min_x >= max_x || min_y >= max_y {
            continue;
        }

        // Rasterize this outline glyph (no bitmap glyph available)
        let col = idx % grid_columns;
        let row = idx / grid_columns;
        let cell_x = col * cell_width;
        let cell_y = row * cell_height;

        // The glyph coordinates need to be mapped into the cell
        // PFR uses Y-up, bitmap uses Y-down, so flip Y
        let glyph_scale_x = scale_x;
        // y' = baseline - ty. If the font matrix already flipped Y (D<0),
        // then ty is already inverted, so we should not flip again.
        let glyph_scale_y = if matrix_scale_y < 0.0 { scale_y } else { -scale_y };
        // Use font metrics for a stable baseline across glyphs.
        // For coords_scaled fonts, use zero x-offset: the zone table maps coordinates
        // A global offset would shift ALL glyphs (even those with x>=0) and break
        // text rendering where copy functions read from x=0 in the cell.
        let glyph_offset_x = if coords_scaled {
            0.0
        } else {
            (-font_min_x * glyph_scale_x).round()
        };
        let glyph_offset_y = if font_asc != 0.0 {
            let mut baseline = font_asc * pixel_scale;
            baseline = baseline.round();
            if !baseline.is_finite() {
                baseline = 0.0;
            }
            baseline
        } else if glyph_scale_y < 0.0 {
            (max_y * (-glyph_scale_y)).round()
        } else {
            (-min_y * glyph_scale_y).round()
        };

        // Use Canvas2D (Skia-backed) for all font rendering. For coords_scaled
        // pixel fonts, threshold the anti-aliased output to binary. This matches
        // Skia's pixel boundary rules exactly, producing output identical to
        // SkiaSharp. Our custom scanline rasterizer had
        // subtly different boundary pixel rules causing ~1px extra width on
        // some characters (98 pixel differences across 34 chars).
        //
        // For non-pixel fonts, keep the anti-aliased Canvas2D output as-is
        // for higher quality rendering with oversampled fallback.
        let alpha_mask = if coords_scaled {
            // Canvas2D with even-odd fill → binary threshold
            let canvas_result = {
                #[cfg(target_arch = "wasm32")]
                {
                    canvas_ctx.as_ref().and_then(|ctx| {
                        let mut alpha = rasterize_glyph_canvas2d(
                            ctx, glyph, cell_width, cell_height,
                            glyph_scale_x, glyph_scale_y,
                            glyph_offset_x, glyph_offset_y,
                        )?;
                        // Threshold to binary: Canvas2D anti-aliases at edges,
                        // but for integer-coordinate pixel fonts the coverage
                        // at boundaries is deterministic. Threshold at 128
                        // matches Skia's non-anti-aliased behavior.
                        for a in alpha.iter_mut() {
                            *a = if *a >= 128 { 255 } else { 0 };
                        }
                        Some(alpha)
                    })
                }
                #[cfg(not(target_arch = "wasm32"))]
                { None::<Vec<u8>> }
            };
            canvas_result.unwrap_or_else(|| {
                // Fallback to software scanline if Canvas2D unavailable
                rasterize_glyph_to_alpha_mask(
                    glyph,
                    cell_width,
                    cell_height,
                    glyph_scale_x,
                    glyph_scale_y,
                    glyph_offset_x,
                    glyph_offset_y,
                    1,
                )
            })
        } else {
            let canvas_result = {
                #[cfg(target_arch = "wasm32")]
                {
                    canvas_ctx.as_ref().and_then(|ctx| {
                        let mut alpha = rasterize_glyph_canvas2d(
                            ctx, glyph, cell_width, cell_height,
                            glyph_scale_x, glyph_scale_y,
                            glyph_offset_x, glyph_offset_y,
                        )?;
                        // Steepen alpha ramp for crisper edges at small sizes.
                        // lo=48 removes faint fringe, hi=208 saturates near-solid.
                        steepen_alpha_ramp(&mut alpha, 48, 208);
                        Some(alpha)
                    })
                }
                #[cfg(not(target_arch = "wasm32"))]
                { None::<Vec<u8>> }
            };
            canvas_result.unwrap_or_else(|| {
                let mut alpha = rasterize_glyph_to_alpha_mask(
                    glyph,
                    cell_width,
                    cell_height,
                    glyph_scale_x,
                    glyph_scale_y,
                    glyph_offset_x,
                    glyph_offset_y,
                    4, // 4x oversampling for anti-aliased output
                );
                steepen_alpha_ramp(&mut alpha, 48, 208);
                alpha
            })
        };

        // Copy alpha mask to RGBA grid (anti-aliased text)
        for gy in 0..cell_height {
            for gx in 0..cell_width {
                let coverage = alpha_mask[gy * cell_width + gx];
                if coverage > 0 {
                    let px = cell_x + gx;
                    let py = cell_y + gy;
                    if px < bitmap_width && py < bitmap_height {
                        let rgba_idx = (py * bitmap_width + px) * 4;
                        if rgba_idx + 3 < rgba.len() {
                            rgba[rgba_idx] = 0;         // R (black text)
                            rgba[rgba_idx + 1] = 0;     // G
                            rgba[rgba_idx + 2] = 0;     // B
                            rgba[rgba_idx + 3] = coverage; // A (anti-aliased)
                        }
                    }
                }
            }
        }
    }

    // Render bitmap glyphs for pixel-perfect shapes.
    // When an outline glyph also exists, its metrics (char_widths) are kept but
    // the bitmap glyph is used for rendering (cleaner pixel patterns).
    // Skip entirely when "outline" preference is set.
    let skip_bitmap_glyphs = {
        use crate::player::font::{get_glyph_preference, GlyphPreference};
        get_glyph_preference() == GlyphPreference::Outline
    };
    for (&char_code, bmp_glyph) in &parsed_font.bitmap_glyphs {
        if skip_bitmap_glyphs { continue; }
        let has_outline = parsed_font.glyphs.contains_key(&char_code);
        if has_outline {
            bitmap_overlap_outline += 1;
        } else {
            bitmap_only += 1;
        }

        if trace_bitmap_debug {
            log(&format!(
                "[pfr1.bitmap] char={} x_size={} y_size={} x_pos={} y_pos={} set_width={} overlap_outline={}",
                char_code,
                bmp_glyph.x_size,
                bmp_glyph.y_size,
                bmp_glyph.x_pos,
                bmp_glyph.y_pos,
                bmp_glyph.set_width,
                parsed_font.glyphs.contains_key(&char_code)
            ));
        }

        let idx = char_code as usize;
        if idx >= num_chars { continue; }

        let col = idx % grid_columns;
        let row = idx / grid_columns;
        let cell_x = col * cell_width;
        let cell_y = row * cell_height;

        // Guard against malformed/unsupported bitmap glyph metrics.
        // Oversized bitmap glyphs can spill far outside their cell and create block artifacts.
        let max_reasonable_w = cell_width.saturating_mul(4);
        let max_reasonable_h = cell_height.saturating_mul(4);
        if bmp_glyph.x_size == 0
            || bmp_glyph.y_size == 0
            || (bmp_glyph.x_size as usize) > max_reasonable_w
            || (bmp_glyph.y_size as usize) > max_reasonable_h
        {
            if trace_bitmap_debug {
                log(&format!(
                    "[pfr1.bitmap] skip char={} unreasonable size {}x{} for cell {}x{}",
                    char_code,
                    bmp_glyph.x_size,
                    bmp_glyph.y_size,
                    cell_width,
                    cell_height
                ));
            }
            continue;
        }

        // Only set char_widths from bitmap if outline didn't already provide them
        if !has_outline {
            let bmp_adv = if outline_res > 0.0 {
                let advance_f = bmp_glyph.set_width as f32 * target_height as f32 / outline_res;
                advance_f.round().max(if bmp_glyph.set_width > 0 { 1.0 } else { 0.0 }) as u16
            } else {
                ((bmp_glyph.set_width as f32) * set_width_scale).max(1.0) as u16
            };
            char_widths[idx] = bmp_adv;
        }

        // Copy bitmap data to RGBA grid
        // Convert PFR Y-up y_pos to bitmap Y-down using baseline
        let py_base = baseline_row_px as i32 - bmp_glyph.y_pos as i32;
        let px_base = bmp_glyph.x_pos as i32;
        let glyph_bits_per_row = bmp_glyph.x_size as usize;
        for gy in 0..bmp_glyph.y_size as usize {
            let py_signed = cell_y as i32 + py_base + gy as i32;
            if py_signed < 0 || py_signed as usize >= bitmap_height { continue; }
            let py = py_signed as usize;
            if py >= cell_y + cell_height { continue; }
            for gx in 0..bmp_glyph.x_size as usize {
                let bit_index = gy * glyph_bits_per_row + gx;
                let byte_idx = bit_index / 8;
                let bit_idx = 7 - (bit_index % 8);
                if byte_idx < bmp_glyph.image_data.len() {
                    let mut bit = (bmp_glyph.image_data[byte_idx] & (1 << bit_idx)) != 0;
                    if !parsed_font.pfr_black_pixel {
                        bit = !bit;
                    }
                    if !bit {
                        continue;
                    }
                    bitmap_pixels_drawn += 1;
                    let px_signed = cell_x as i32 + px_base + gx as i32;
                    if px_signed < 0 || px_signed as usize >= bitmap_width { continue; }
                    let px = px_signed as usize;
                    if px >= cell_x + cell_width { continue; }
                    let rgba_idx = (py * bitmap_width + px) * 4;
                    if rgba_idx + 3 < rgba.len() {
                        rgba[rgba_idx] = 0;
                        rgba[rgba_idx + 1] = 0;
                        rgba[rgba_idx + 2] = 0;
                        rgba[rgba_idx + 3] = 255;
                    }
                }
            }
        }
    }

    if trace_bitmap_debug {
        log(&format!(
            "[pfr1.bitmap] summary overlap_outline={} bitmap_only={} pixels_drawn={} pfr_black_pixel={}",
            bitmap_overlap_outline,
            bitmap_only,
            bitmap_pixels_drawn,
            parsed_font.pfr_black_pixel
        ));

        // Alpha histogram diagnostic — count pixels by alpha value to detect anti-aliasing
        let mut alpha_hist = [0u32; 256];
        for i in (0..rgba.len()).step_by(4) {
            alpha_hist[rgba[i + 3] as usize] += 1;
        }
        let non_binary: u32 = alpha_hist[1..255].iter().sum();

        log(&format!(
            "PFR1-V2 '{}': target_h={} design_size={} cell={}x{} scale={:.4} coords_scaled={} outline_res={} target_em_px={} asc={} | outlines={} bitmaps={} (overlap={} bmp_only={} px_drawn={}) | alpha: 0={} 255={} other={}",
            parsed_font.font_name,
            target_height, design_size,
            cell_width, cell_height,
            scale, coords_scaled, outline_res, target_em_px,
            phys.metrics.ascender,
            parsed_font.glyphs.len(),
            parsed_font.bitmap_glyphs.len(),
            bitmap_overlap_outline, bitmap_only, bitmap_pixels_drawn,
            alpha_hist[0], alpha_hist[255], non_binary
        ));
    }

    // Fallback for caps-only PFR fonts:
    // if a lowercase cell rendered empty, copy from a non-empty letter glyph.
    let cell_has_ink = |rgba: &[u8], cx: usize, cy: usize| -> bool {
        for gy in 0..cell_height {
            for gx in 0..cell_width {
                let p = ((cy + gy) * bitmap_width + (cx + gx)) * 4;
                if p + 3 < rgba.len() {
                    let r = rgba[p];
                    let g = rgba[p + 1];
                    let b = rgba[p + 2];
                    let a = rgba[p + 3];
                    if a > 0 && !(r >= 250 && g >= 250 && b >= 250) {
                        return true;
                    }
                }
            }
        }
        false
    };


    let cell_bbox_h = |rgba: &[u8], cx: usize, cy: usize| -> usize {
        let mut min_y = cell_height as i32;
        let mut max_y = -1i32;
        for gy in 0..cell_height {
            for gx in 0..cell_width {
                let p = ((cy + gy) * bitmap_width + (cx + gx)) * 4;
                if p + 3 < rgba.len() {
                    let r = rgba[p];
                    let g = rgba[p + 1];
                    let b = rgba[p + 2];
                    let a = rgba[p + 3];
                    if a > 0 && !(r >= 250 && g >= 250 && b >= 250) {
                        min_y = min_y.min(gy as i32);
                        max_y = max_y.max(gy as i32);
                    }
                }
            }
        }
        if max_y >= min_y {
            (max_y - min_y + 1) as usize
        } else {
            0
        }
    };
    let cell_origin = |idx: usize| -> (usize, usize) {
        let col = idx % grid_columns;
        let row = idx / grid_columns;
        (col * cell_width, row * cell_height)
    };

    let copy_cell = |rgba: &mut Vec<u8>, src_idx: usize, dst_idx: usize| {
        let (src_x, src_y) = cell_origin(src_idx);
        let (dst_x, dst_y) = cell_origin(dst_idx);
        for gy in 0..cell_height {
            for gx in 0..cell_width {
                let s = ((src_y + gy) * bitmap_width + (src_x + gx)) * 4;
                let d = ((dst_y + gy) * bitmap_width + (dst_x + gx)) * 4;
                let px = [rgba[s], rgba[s + 1], rgba[s + 2], rgba[s + 3]];
                rgba[d..d + 4].copy_from_slice(&px);
            }
        }
    };

    for lc in b'a'..=b'z' {
        let li = lc as usize;
        if li >= num_chars {
            continue;
        }
        let (lcx, lcy) = cell_origin(li);
        if cell_has_ink(&rgba, lcx, lcy) {
            let lc_h = cell_bbox_h(&rgba, lcx, lcy);
            if lc_h > 2 {
                continue;
            }
        }

        let mut src_idx_opt: Option<usize> = None;
        let ui = (lc - 32) as usize;
        if ui < num_chars {
            let (ucx, ucy) = cell_origin(ui);
            if cell_has_ink(&rgba, ucx, ucy) {
                src_idx_opt = Some(ui);
            }
        }


        if let Some(src_idx) = src_idx_opt {
            copy_cell(&mut rgba, src_idx, li);
            char_widths[li] = char_widths[src_idx];
        }
    }

    RasterizedFont {
        bitmap_data: rgba,
        bitmap_width,
        bitmap_height,
        cell_width,
        cell_height,
        grid_columns,
        grid_rows,
        char_widths,
        first_char,
        num_chars,
    }
}
