use std::collections::HashMap;

use crate::{
    director::lingo::datum::{Datum, datum_bool},
    player::{
        ColorRef, DatumRef, DirPlayer, ScriptError, bitmap::{
            bitmap::{Bitmap, BuiltInPalette, PaletteRef, resolve_color_ref},
            manager::BitmapRef,
            mask::BitmapMask,
        }, geometry::IntRect, handlers::types::TypeUtils, player_duplicate_datum, reserve_player_mut
    },
};

use super::prop_list::PropListUtils;

pub struct BitmapDatumHandlers {}

impl BitmapDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name {
            "fill" => Self::fill(datum, args),
            "draw" => Self::draw(datum, args),
            "setPixel" => Self::set_pixel(datum, args),
            "extractAlpha" => Self::extract_alpha(datum, args),
            "duplicate" => Self::duplicate(datum, args),
            "copyPixels" => Self::copy_pixels(datum, args),
            "applyFilter" => Self::apply_filter(datum, args),
            "createMatte" | "createMask" => Self::create_matte(datum, args),
            "trimWhiteSpace" => Self::trim_whitespace(datum, args),
            "getPixel" => Self::get_pixel(datum, args),
            "crop" => Self::crop(datum, args),
            "setAlpha" => Self::set_alpha(datum, args),
            "floodFill" => reserve_player_mut(|player| {
                // Args: point, color  OR  x, y, color
                if args.len() != 2 && args.len() != 3 {
                    return Err(ScriptError::new(
                        "floodFill requires 2 or 3 arguments".to_string(),
                    ));
                }

                let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;

                let (x, y, color_arg_idx) = if args.len() == 3 {
                    // floodFill(x, y, color)
                    let x = player.get_datum(&args[0]).int_value()?;
                    let y = player.get_datum(&args[1]).int_value()?;
                    (x, y, 2)
                } else {
                    // floodFill(point, color)
                    let (pt_vals, _flags) = player.get_datum(&args[0]).to_point_inline()?;
                    let x = pt_vals[0] as i32;
                    let y = pt_vals[1] as i32;
                    (x, y, 1)
                };

                let point_tuple = (x, y);

                let color_ref = player.get_datum(&args[color_arg_idx]).to_color_ref()?;

                // Get palettes once
                let palettes = player.movie.cast_manager.palettes();

                // Get bitmap palette and resolve color in one scope
                let (target_rgb, bitmap_palette) = {
                    let bitmap = player
                        .bitmap_manager
                        .get_bitmap(*bitmap_ref)
                        .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;

                    let palette = bitmap.palette_ref.clone();
                    let rgb = resolve_color_ref(
                        &palettes,
                        &color_ref,
                        &palette,
                        bitmap.original_bit_depth,
                    );
                    (rgb, palette)
                }; // bitmap borrow ends here

                // Now mutate the bitmap with the resolved color
                let bitmap = player
                    .bitmap_manager
                    .get_bitmap_mut(*bitmap_ref)
                    .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;

                bitmap.flood_fill(point_tuple, target_rgb, &palettes);

                Ok(player.alloc_datum(Datum::Void))
            }),
            "getProp" => Self::get_prop_handler(datum, args),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for bitmap datum"
            ))),
        }
    }

    pub fn get_pixel(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();
            // Parse args: (point [, #integer]) or (x, y)
            let first_is_point = matches!(player.get_datum(&args[0]), Datum::Point(..));
            let (x, y, return_integer) = if first_is_point {
                let (pt_vals, _flags) = player.get_datum(&args[0]).to_point_inline()?;
                let x = pt_vals[0] as i32;
                let y = pt_vals[1] as i32;
                let return_integer = if args.len() > 1 {
                    let flag = player.get_datum(&args[1]).string_value().unwrap_or_default();
                    flag.eq_ignore_ascii_case("integer")
                } else {
                    false
                };
                (x, y, return_integer)
            } else {
                let x = player.get_datum(&args[0]).int_value()?;
                let y = player.get_datum(&args[1]).int_value()?;
                (x, y, false)
            };
            let color = bitmap.get_pixel_color_ref(x as u16, y as u16);
            if return_integer {
                let palettes = player.movie.cast_manager.palettes();
                let (r, g, b) = crate::player::bitmap::bitmap::resolve_color_ref(
                    &palettes,
                    &color,
                    &bitmap.palette_ref,
                    bitmap.original_bit_depth,
                );
                // Director's getPixel(pt, #integer) returns the pixel's value in
                // the bitmap's native format, not always 24-bit RGB:
                //   - 8-bit: palette index (0..255)
                //   - 16-bit: RGB555 packed word (0..32767)
                //   - 32-bit: 24-bit RGB (0..16_777_215)
                // Many classic Lingo hit-test handlers compare getPixel against
                // 16-bit color constants (e.g. 32767 for white / transparent marker);
                // returning 24-bit RGB here broke pixel-accurate avatar click
                // tests so clicks on transparent pixels registered as hits.
                let int_color = match bitmap.original_bit_depth {
                    1 | 2 | 4 | 8 => {
                        if let crate::player::sprite::ColorRef::PaletteIndex(idx) = color {
                            idx as i32
                        } else {
                            ((r as i32) << 16) | ((g as i32) << 8) | (b as i32)
                        }
                    }
                    16 => {
                        // Pack as RGB555 (Director's 16-bit format). The original
                        // file may have had the high bit set (giving 65535 vs 32767
                        // for white) but that bit is lost during decode; return the
                        // 15-bit value which matches the common 32767 transparent
                        // marker. Scripts that accept either 32767 or 65535 (which
                        // is the standard hit-test idiom) work correctly.
                        let r5 = (r as i32 >> 3) & 0x1F;
                        let g5 = (g as i32 >> 3) & 0x1F;
                        let b5 = (b as i32 >> 3) & 0x1F;
                        (r5 << 10) | (g5 << 5) | b5
                    }
                    _ => ((r as i32) << 16) | ((g as i32) << 8) | (b as i32),
                };
                Ok(player.alloc_datum(Datum::Int(int_color)))
            } else {
                let color_ref = player.alloc_datum(Datum::ColorRef(color));
                Ok(color_ref)
            }
        })
    }

    pub fn trim_whitespace(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap = player.get_datum(datum).to_bitmap_ref()?;
            let bitmap = player.bitmap_manager.get_bitmap_mut(*bitmap).unwrap();
            bitmap.trim_whitespace(&player.movie.cast_manager.palettes());
            Ok(datum.clone())
        })
    }

    pub fn create_matte(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            // Director: imageObject.createMatte({alphaThreshold}). The
            // alphaThreshold (0..255) excludes pixels whose alpha falls
            // below that value from the resulting matte; only meaningful
            // for 32-bit images with an alpha channel. We don't honour
            // the threshold yet (our matte builder uses the bg color key
            // path, see Bitmap::create_matte), but accept and ignore the
            // argument so scripts that pass it don't error out.
            if args.len() > 1 {
                return Err(ScriptError::new(
                    "createMatte takes at most 1 argument (alphaThreshold)".to_string(),
                ));
            }
            let bitmap = player.get_datum(datum).to_bitmap_ref()?;
            let bitmap = player.bitmap_manager.get_bitmap_mut(*bitmap).unwrap();
            bitmap.create_matte(&player.movie.cast_manager.palettes());
            let matte_arc = bitmap.matte.as_ref().unwrap().clone();
            Ok(player.alloc_datum(Datum::Matte(matte_arc)))
        })
    }

    pub fn duplicate(datum: &DatumRef, _: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        Ok(player_duplicate_datum(datum))
    }

    pub fn crop(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() != 1 {
                return Err(ScriptError::new(
                    "crop requires 1 argument (rect)".to_string(),
                ));
            }

            let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;
            let (rect_vals, _flags) = player.get_datum(&args[0]).to_rect_inline()?;

            let left = rect_vals[0] as i32;
            let top = rect_vals[1] as i32;
            let right = rect_vals[2] as i32;
            let bottom = rect_vals[3] as i32;

            // Calculate cropped dimensions
            let crop_width = (right - left).max(0) as u16;
            let crop_height = (bottom - top).max(0) as u16;

            if crop_width == 0 || crop_height == 0 {
                return Err(ScriptError::new(
                    "crop rect must have positive dimensions".to_string(),
                ));
            }

            let src_bitmap = player
                .bitmap_manager
                .get_bitmap(*bitmap_ref)
                .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;

            // Create new bitmap with cropped dimensions, preserving bit depth and palette
            let mut cropped_bitmap = crate::player::bitmap::bitmap::Bitmap::new(
                crop_width,
                crop_height,
                src_bitmap.bit_depth,
                src_bitmap.original_bit_depth,
                if src_bitmap.use_alpha { 8 } else { 0 },
                src_bitmap.palette_ref.clone(),
            );
            cropped_bitmap.use_alpha = src_bitmap.use_alpha;
            cropped_bitmap.trim_white_space = src_bitmap.trim_white_space;

            let palettes = player.movie.cast_manager.palettes();

            // Copy pixels from source rect to destination (0,0 to crop_width, crop_height)
            let src_rect = IntRect::from(left, top, right, bottom);
            let dst_rect = IntRect::from(0, 0, crop_width as i32, crop_height as i32);

            let params = crate::player::bitmap::drawing::CopyPixelsParams::default(&src_bitmap);

            // Need to clone src_bitmap to avoid borrow issues
            let src_bitmap_clone = src_bitmap.clone();

            cropped_bitmap.copy_pixels_with_params(
                &palettes,
                &src_bitmap_clone,
                dst_rect,
                src_rect,
                &params,
            );

            // Ephemeral: `bitmap.duplicate(rect)` produces a fresh bitmap not
            // owned by any cast member. Free when the wrapping DatumRef drops.
            let new_bitmap_ref = player.bitmap_manager.add_ephemeral_bitmap(cropped_bitmap);
            Ok(player.alloc_datum(Datum::BitmapRef(new_bitmap_ref)))
        })
    }

    pub fn extract_alpha(datum: &DatumRef, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;
            let src = player
                .bitmap_manager
                .get_bitmap(*bitmap_ref)
                .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;

            let w = src.width;
            let h = src.height;
            let is_32bit = src.bit_depth == 32;

            // Create an 8-bit grayscale bitmap for the alpha channel
            let mut alpha_bitmap = crate::player::bitmap::bitmap::Bitmap::new(
                w, h, 32, 32, 0,
                src.palette_ref.clone(),
            );

            if is_32bit {
                // Extract alpha from 32-bit RGBA data (4 bytes per pixel, alpha = byte 3)
                let row_bytes = w as usize * 4;
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let src_idx = y * row_bytes + x * 4;
                        let alpha = if src_idx + 3 < src.data.len() {
                            src.data[src_idx + 3]
                        } else {
                            255
                        };
                        // Write grayscale: R=G=B=alpha, A=255
                        let dst_idx = y * row_bytes + x * 4;
                        if dst_idx + 3 < alpha_bitmap.data.len() {
                            alpha_bitmap.data[dst_idx] = alpha;
                            alpha_bitmap.data[dst_idx + 1] = alpha;
                            alpha_bitmap.data[dst_idx + 2] = alpha;
                            alpha_bitmap.data[dst_idx + 3] = 255;
                        }
                    }
                }
            } else {
                // Non-32-bit: no alpha channel, return all-white (fully opaque)
                alpha_bitmap.data.fill(255);
            }

            // Ephemeral: `bitmap.extractAlpha()` returns a fresh derived bitmap
            // not owned by a cast member. Free when the wrapping DatumRef drops.
            let new_ref = player.bitmap_manager.add_ephemeral_bitmap(alpha_bitmap);
            Ok(player.alloc_datum(Datum::BitmapRef(new_ref)))
        })
    }

    pub fn set_alpha(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.len() != 1 {
                return Err(ScriptError::new(
                    "setAlpha requires 1 argument".to_string(),
                ));
            }

            let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;
            let arg = player.get_datum(&args[0]);

            // Check if target bitmap is 32-bit
            let (width, height, bit_depth) = {
                let bitmap = player
                    .bitmap_manager
                    .get_bitmap(*bitmap_ref)
                    .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;
                (bitmap.width, bitmap.height, bitmap.bit_depth)
            };

            if bit_depth != 32 {
                // setAlpha only works on 32-bit images
                log::warn!("setAlpha called on non-32-bit bitmap");
                return Ok(player.alloc_datum(datum_bool(false)));
            }

            match arg {
                Datum::Int(alpha_level) => {
                    // Set all pixels to a flat alpha level (0-255)
                    let alpha = (*alpha_level).clamp(0, 255) as u8;
                    let bitmap = player
                        .bitmap_manager
                        .get_bitmap_mut(*bitmap_ref)
                        .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;

                    // For 32-bit images, data is RGBA, so we modify every 4th byte (alpha channel)
                    for i in (3..bitmap.data.len()).step_by(4) {
                        bitmap.data[i] = alpha;
                    }
                    bitmap.use_alpha = true;
                    // bitmap.version += 1;

                    Ok(player.alloc_datum(datum_bool(true)))
                }
                Datum::BitmapRef(alpha_bitmap_ref) => {
                    // Set alpha from an 8-bit grayscale image
                    let alpha_bitmap = player
                        .bitmap_manager
                        .get_bitmap(*alpha_bitmap_ref)
                        .ok_or_else(|| ScriptError::new("Invalid alpha bitmap reference".to_string()))?;

                    // Alpha image must be 8-bit
                    if alpha_bitmap.bit_depth != 8 {
                        return Ok(player.alloc_datum(datum_bool(false)));
                    }

                    // Both images must have the same dimensions
                    if alpha_bitmap.width != width || alpha_bitmap.height != height {
                        return Ok(player.alloc_datum(datum_bool(false)));
                    }

                    // Clone the alpha data to avoid borrow issues
                    let alpha_data = alpha_bitmap.data.clone();

                    let bitmap = player
                        .bitmap_manager
                        .get_bitmap_mut(*bitmap_ref)
                        .ok_or_else(|| ScriptError::new("Invalid bitmap reference".to_string()))?;

                    // Copy alpha values from the 8-bit image to the alpha channel of the 32-bit image.
                    // Director's setAlpha uses raw palette indices as alpha values directly:
                    // index 0 = transparent, index 255 = opaque.
                    for y in 0..height {
                        for x in 0..width {
                            let alpha_idx = y as usize * width as usize + x as usize;
                            let dst_idx = (y as usize * width as usize + x as usize) * 4 + 3;
                            if alpha_idx < alpha_data.len() && dst_idx < bitmap.data.len() {
                                bitmap.data[dst_idx] = alpha_data[alpha_idx];
                            }
                        }
                    }
                    bitmap.use_alpha = true;

                    Ok(player.alloc_datum(datum_bool(true)))
                }
                _ => {
                    // Invalid argument type
                    Ok(player.alloc_datum(datum_bool(false)))
                }
            }
        })
    }

    pub fn draw(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap = player.get_datum(datum);
            let bitmap_ref = match bitmap {
                Datum::BitmapRef(bitmap) => Ok(bitmap),
                _ => Err(ScriptError::new("Cannot draw non-bitmap".to_string())),
            }?;
            if args.is_empty() {
                return Err(ScriptError::new(
                    "draw requires arguments".to_string(),
                ));
            }
            let first_arg = player.get_datum(&args[0]);
            let mut arg_pos = 1;
            let (x1, y1, x2, y2) = match first_arg {
                Datum::Int(x1) => {
                    if args.len() < 4 {
                        return Err(ScriptError::new(
                            "draw(x1, y1, x2, y2, ...) requires at least 4 arguments"
                                .to_string(),
                        ));
                    }
                    let y1 = player.get_datum(&args[arg_pos]).int_value()?;
                    arg_pos += 1;
                    let x2 = player.get_datum(&args[arg_pos]).int_value()?;
                    arg_pos += 1;
                    let y2 = player.get_datum(&args[arg_pos]).int_value()?;
                    arg_pos += 1;
                    (*x1, y1, x2, y2)
                }
                Datum::Point(point_vals, _flags) => {
                    let x1 = point_vals[0] as i32;
                    let y1 = point_vals[1] as i32;
                    let (point2_vals, _flags) = player.get_datum(&args[arg_pos]).to_point_inline()?;
                    arg_pos += 1;
                    let x2 = point2_vals[0] as i32;
                    let y2 = point2_vals[1] as i32;
                    (x1, y1, x2, y2)
                }
                Datum::Rect(rect_vals, _flags) => {
                    let x1 = rect_vals[0] as i32;
                    let y1 = rect_vals[1] as i32;
                    let x2 = rect_vals[2] as i32;
                    let y2 = rect_vals[3] as i32;
                    (x1, y1, x2, y2)
                }
                _ => {
                    return Err(ScriptError::new(
                        "First argument to draw must be coordinates, a point, or a rect"
                            .to_string(),
                    ))
                }
            };

            // Handle optional color argument before the prop list
            // draw(x1, y1, x2, y2, [color,] propList)
            let explicit_color = if arg_pos + 1 < args.len() {
                let maybe_color = player.get_datum(&args[arg_pos]);
                if matches!(maybe_color, Datum::ColorRef(_)) {
                    let c = maybe_color.to_color_ref().ok();
                    arg_pos += 1;
                    c
                } else {
                    None
                }
            } else {
                None
            };

            let draw_map = player.get_datum(&args[arg_pos]).to_map()?;
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();

            let color_ref = if let Some(c) = explicit_color {
                c
            } else {
                let cr = PropListUtils::get_by_concrete_key(
                    &draw_map,
                    &Datum::Symbol("color".to_owned()),
                    &player.allocator,
                )?;
                player.get_datum(&cr).to_color_ref()?
            };
            let palettes = player.movie.cast_manager.palettes();
            let color = resolve_color_ref(
                &palettes,
                &color_ref,
                &bitmap.palette_ref,
                bitmap.original_bit_depth,
            );

            let shape_type = PropListUtils::get_by_concrete_key(
                &draw_map,
                &Datum::Symbol("shapeType".to_owned()),
                &player.allocator,
            )?;
            let shape_type = player.get_datum(&shape_type).string_value()?;

            let blend = PropListUtils::get_by_concrete_key(
                &draw_map,
                &Datum::Symbol("blend".to_owned()),
                &player.allocator,
            )?;
            let blend = player.get_datum(&blend);
            let blend = if blend.is_void() {
                100
            } else {
                blend.int_value()?
            };

            // Director chapter 15: optional `#lineSize` (default 1) controls
            // stroke thickness for rect/oval/roundRect/line outlines. Some
            // scripts use alias keys (#lineWidth, #width, #strokeWidth) for
            // the same property — accept all four.
            let thickness = ["lineSize", "lineWidth", "width", "strokeWidth"]
                .into_iter()
                .find_map(|key| {
                    let value = PropListUtils::get_by_concrete_key(
                        &draw_map,
                        &Datum::Symbol(key.to_owned()),
                        &player.allocator,
                    )
                    .ok()?;
                    if player.get_datum(&value).is_void() {
                        None
                    } else {
                        player.get_datum(&value).int_value().ok()
                    }
                })
                .unwrap_or(1)
                .max(1);

            // Optional `#radius` for #roundRect — defaults to 8 (Director's
            // visual default for the rounded-rect tool).
            let radius_d = PropListUtils::get_by_concrete_key(
                &draw_map,
                &Datum::Symbol("radius".to_owned()),
                &player.allocator,
            )?;
            let radius_d = player.get_datum(&radius_d);
            let radius = if radius_d.is_void() { 8 } else { radius_d.int_value()?.max(0) };

            let bitmap = player.bitmap_manager.get_bitmap_mut(*bitmap_ref).unwrap();
            let alpha = blend as f32 / 100.0;
            match shape_type.as_str() {
                "rect" => {
                    bitmap.stroke_rect(x1, y1, x2, y2, color, &palettes, alpha);
                }
                "oval" => {
                    bitmap.stroke_ellipse(x1, y1, x2, y2, color, &palettes, alpha, thickness);
                }
                "roundRect" => {
                    bitmap.stroke_round_rect(x1, y1, x2, y2, radius, color, &palettes, alpha, thickness);
                }
                "line" => {
                    // For #line, (x1,y1) and (x2,y2) are the line endpoints
                    // (rather than a bounding rect like the other shapes).
                    bitmap.draw_line_thick(x1, y1, x2, y2, color, &palettes, alpha, thickness);
                }
                "oval" => {
                    bitmap.stroke_ellipse(
                        x1,
                        y1,
                        x2,
                        y2,
                        color,
                        &palettes,
                        blend as f32 / 100.0,
                        thickness,
                    );
                }
                "line" => {
                    bitmap.draw_line_thick(
                        x1,
                        y1,
                        x2,
                        y2,
                        color,
                        &palettes,
                        blend as f32 / 100.0,
                        thickness,
                    );
                }
                _ => {
                    return Err(ScriptError::new(format!(
                        "Invalid shapeType '#{}' for draw (expected #rect, #oval, #roundRect, #line)",
                        shape_type
                    )));
                }
            }
            Ok(datum.clone())
        })
    }

    pub fn set_pixel(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap_datum = player.get_datum(datum);
            let bitmap_ref = match bitmap_datum {
                Datum::BitmapRef(bitmap) => Ok(bitmap),
                _ => Err(ScriptError::new("Cannot draw non-bitmap".to_string())),
            }?;

            // setPixel supports both (x, y, color) and (point, color) forms
            let (x, y, color_obj_or_int, bit_depth, original_bit_depth, palette_ref) = {
                let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();

                let first_arg = player.get_datum(&args[0]);
                let (x, y, color_obj_or_int) = if let Datum::Point(pt_vals, _flags) = first_arg {
                    let px = pt_vals[0] as i32;
                    let py = pt_vals[1] as i32;
                    let color = player.get_datum(&args[1]);
                    (px, py, color)
                } else {
                    let x = first_arg.int_value()?;
                    let y = player.get_datum(&args[1]).int_value()?;
                    let color = player.get_datum(&args[2]);
                    (x, y, color)
                };

                if x < 0 || y < 0 || x >= bitmap.width as i32 || y >= bitmap.height as i32 {
                    return Ok(player.alloc_datum(datum_bool(false)));
                }

                (
                    x,
                    y,
                    color_obj_or_int.to_owned(),
                    bitmap.bit_depth,
                    bitmap.original_bit_depth,
                    bitmap.palette_ref.clone(),
                )
            };

            let palettes = player.movie.cast_manager.palettes();
            let bitmap = player.bitmap_manager.get_bitmap_mut(*bitmap_ref).unwrap();

            if color_obj_or_int.is_int() {
                let int_value = color_obj_or_int.int_value()?;
                if bit_depth == 8 {
                    // 8-bit: treat as palette index → grayscale
                    let idx = int_value as u8;
                    bitmap.set_pixel(x, y, (idx, idx, idx), &palettes);
                } else {
                    // 16/32-bit: treat as packed RGB integer (r*65536 + g*256 + b)
                    let r = ((int_value >> 16) & 0xFF) as u8;
                    let g = ((int_value >> 8) & 0xFF) as u8;
                    let b = (int_value & 0xFF) as u8;
                    bitmap.set_pixel(x, y, (r, g, b), &palettes);
                }
            } else {
                let color = color_obj_or_int.to_color_ref()?;
                let color = resolve_color_ref(&palettes, &color, &palette_ref, original_bit_depth);
                bitmap.set_pixel(x, y, color, &palettes);
            }

            Ok(player.alloc_datum(datum_bool(true)))
        })
    }

    pub fn fill(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap = player.get_datum(datum);
            let (rect_i32, color_ref) = if args.len() == 2 {
                let (rect_vals, _flags) = player.get_datum(&args[0]).to_rect_inline()?;
                let params = player.get_datum(&args[1]);
                let (color, shape) = match params {
                    Datum::ColorRef(color_ref) => (color_ref.clone(), "rect".to_string()),
                    Datum::PropList(prop_list, ..) => {
                        let color_ref = PropListUtils::get_by_concrete_key(
                            &prop_list,
                            &Datum::Symbol("color".to_string()),
                            &player.allocator,
                        )?;
                        let shape_ref = PropListUtils::get_by_concrete_key(
                            &prop_list,
                            &Datum::Symbol("shapeType".to_string()),
                            &player.allocator,
                        )?;
                        let shape_datum = player.get_datum(&shape_ref);
                        let shape = match shape_datum {
                            Datum::Symbol(s) => s.clone(),
                            Datum::Void => "rect".to_string(),
                            _ => {
                                return Err(ScriptError::new(
                                    "Invalid shapeType in fill prop list".to_string(),
                                ))
                            }
                        };
                        let color_ref = player.get_datum(&color_ref).to_color_ref()?;
                        (color_ref.clone(), shape)
                    }
                    _ => {
                        return Err(ScriptError::new(
                            "Invalid parameter for fill".to_string(),
                        ))
                    }
                };
                
                if shape != "rect" {
                    log::warn!("Unsupported shapeType '{}' for bitmap fill handler, skipping", shape);
                    return Ok(datum.clone()); // Silently ignore unsupported shape types for now
                }

                let x1 = rect_vals[0] as i32;
                let y1 = rect_vals[1] as i32;
                let x2 = rect_vals[2] as i32;
                let y2 = rect_vals[3] as i32;

                ((x1, y1, x2, y2), color)
            } else if args.len() == 5 {
                let x = player.get_datum(&args[0]).int_value()?;
                let y = player.get_datum(&args[1]).int_value()?;
                let width = player.get_datum(&args[2]).int_value()?;
                let height = player.get_datum(&args[3]).int_value()?;
                let params = player.get_datum(&args[4]);
                let color = match params {
                    Datum::ColorRef(color_ref) => color_ref.clone(),
                    Datum::PropList(prop_list, ..) => {
                        let color_ref = PropListUtils::get_by_concrete_key(
                            &prop_list,
                            &Datum::Symbol("color".to_string()),
                            &player.allocator,
                        )?;
                        player.get_datum(&color_ref).to_color_ref()?.clone()
                    }
                    Datum::Int(i) => ColorRef::PaletteIndex(*i as u8),
                    _ => {
                        return Err(ScriptError::new(
                            "Invalid color parameter for fill".to_string(),
                        ))
                    }
                };
                ((x, y, width, height), color)
            } else {
                return Err(ScriptError::new(
                    "Invalid number of arguments for fill".to_string(),
                ));
            };
            let bitmap_ref = match bitmap {
                Datum::BitmapRef(bitmap) => Ok(bitmap),
                _ => Err(ScriptError::new("Cannot fill non-bitmap".to_string())),
            }?;
            let (x1, y1, x2, y2) = rect_i32;
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();
            let palettes = player.movie.cast_manager.palettes();
            let color = resolve_color_ref(
                &palettes,
                &color_ref,
                &bitmap.palette_ref,
                bitmap.original_bit_depth,
            );
            let bitmap = player.bitmap_manager.get_bitmap_mut(*bitmap_ref).unwrap();
            bitmap.fill_rect(x1, y1, x2, y2, color, &palettes, 1.0);
            Ok(datum.clone())
        })
    }

    pub fn copy_pixels(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let dst_bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;
            let src_bitmap_ref = player.get_datum(&args[0]);
            let src_bitmap_ref = if src_bitmap_ref.is_void()
                || (src_bitmap_ref.is_number() && src_bitmap_ref.int_value()? == 0)
            {
                return Ok(datum.clone());
            } else {
                src_bitmap_ref.to_bitmap_ref()?
            };
            let dest_rect_or_quad = player.get_datum(&args[1]);
            let (src_rect_vals, _flags) = player.get_datum(&args[2]).to_rect_inline()?;
            let sx1 = src_rect_vals[0] as i32;
            let sy1 = src_rect_vals[1] as i32;
            let sx2 = src_rect_vals[2] as i32;
            let sy2 = src_rect_vals[3] as i32;
            let param_list = args.get(3).map(|x| player.get_datum(x));
            let mut param_list_concrete = HashMap::new();
            if let Some(param_list) = param_list {
                if let Datum::PropList(param_list, ..) = param_list {
                    for (key, value) in param_list {
                        let key = player.get_datum(key).string_value()?;
                        let value = player.get_datum(value).clone();
                        param_list_concrete.insert(key, value);
                    }
                }
            }

            // Pre-convert BitmapRef maskImage to BitmapMask
            // Director's #maskImage param accepts a bitmap where white=transparent, black=opaque
            if let Some(Datum::BitmapRef(mask_ref)) = param_list_concrete.get("maskImage") {
                let mask_ref = *mask_ref;
                if let Some(mask_bitmap) = player.bitmap_manager.get_bitmap(mask_ref) {
                    let palettes = player.movie.cast_manager.palettes();
                    let w = mask_bitmap.width;
                    let h = mask_bitmap.height;
                    let mut mask = BitmapMask::new(w, h, false);
                    for y in 0..h {
                        for x in 0..w {
                            let (r, g, b) = mask_bitmap.get_pixel_color(&palettes, x, y);
                            let luminance = (r as u16 + g as u16 + b as u16) / 3;
                            if luminance <= 128 {
                                mask.set_bit(x, y, true);
                            }
                        }
                    }
                    param_list_concrete.insert("maskImage".to_string(), Datum::Matte(std::sync::Arc::new(mask)));
                }
            }

            // Decode dest as either Rect (axis-aligned blit) or List of 4 Points (quad warp).
            enum DestShape { Rect(IntRect), Quad([(i32, i32); 4]) }
            let dest_shape = match dest_rect_or_quad {
                Datum::Rect(rect_vals, _flags) => {
                    let x1 = rect_vals[0] as i32;
                    let y1 = rect_vals[1] as i32;
                    let x2 = rect_vals[2] as i32;
                    let y2 = rect_vals[3] as i32;
                    DestShape::Rect(IntRect::from_tuple((x1, y1, x2, y2)))
                }
                Datum::List(_, list_val, _) => {
                    let p1 = {
                        let (pv, _f) = player.get_datum(&list_val[0]).to_point_inline()?;
                        (pv[0] as i32, pv[1] as i32)
                    };
                    let p2 = {
                        let (pv, _f) = player.get_datum(&list_val[1]).to_point_inline()?;
                        (pv[0] as i32, pv[1] as i32)
                    };
                    let p3 = {
                        let (pv, _f) = player.get_datum(&list_val[2]).to_point_inline()?;
                        (pv[0] as i32, pv[1] as i32)
                    };
                    let p4 = {
                        let (pv, _f) = player.get_datum(&list_val[3]).to_point_inline()?;
                        (pv[0] as i32, pv[1] as i32)
                    };
                    // Detect axis-aligned quad (top.y==top.y, etc.) — those
                    // map cleanly to a Rect and let the existing fast path
                    // run with ink / blend / matte support. Otherwise route
                    // through the inverse-bilinear quad warp, which
                    // currently supports copy ink only.
                    let axis_aligned = p1.1 == p2.1 && p4.1 == p3.1
                        && p1.0 == p4.0 && p2.0 == p3.0;
                    if axis_aligned {
                        DestShape::Rect(IntRect::from_quad(p1, p2, p3, p4))
                    } else {
                        DestShape::Quad([p1, p2, p3, p4])
                    }
                },
                _ => {
                    return Err(ScriptError::new(
                        "Invalid destRect for copyPixels".to_string(),
                    ))
                }
            };
            let src_bitmap = player
                .bitmap_manager
                .get_bitmap(*src_bitmap_ref)
                .unwrap()
                .clone();
            let palettes = player.movie.cast_manager.palettes();
            let dst_bitmap = player
                .bitmap_manager
                .get_bitmap_mut(*dst_bitmap_ref)
                .unwrap();

            match dest_shape {
                DestShape::Rect(dest_rect) => {
                    dst_bitmap.copy_pixels(
                        &palettes,
                        &src_bitmap,
                        dest_rect,
                        IntRect::from_tuple((sx1, sy1, sx2, sy2)),
                        &param_list_concrete,
                        Some(&player.movie.score),
                    );
                }
                DestShape::Quad(quad) => {
                    dst_bitmap.copy_pixels_quad(
                        &palettes,
                        &src_bitmap,
                        quad,
                        IntRect::from_tuple((sx1, sy1, sx2, sy2)),
                        &param_list_concrete,
                    );
                }
            }
            Ok(datum.clone())
        })
    }

    /// Director chapter 15 `image.applyFilter(filterObj)`. Mutates the bitmap
    /// in place. The filter is the PropList produced by the global `filter()`
    /// constructor — its `#filterType` symbol decides the dispatch.
    ///
    /// Currently implemented:
    ///   - `#adjustcolorfilter` — applies hue / saturation / contrast /
    ///     brightness using Adobe Flash's AdjustColor convention (see source
    ///     reference in `apply_adjust_color_filter`).
    ///
    /// Other filter symbols (#blurfilter / #glowfilter / etc.) are accepted
    /// without crashing but produce a warning and leave the bitmap unchanged
    /// — this matches the AGEIA Xtra's behaviour for unimplemented filters.
    pub fn apply_filter(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            if args.is_empty() {
                return Err(ScriptError::new(
                    "applyFilter requires a filter argument".to_string(),
                ));
            }
            let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;

            // Read the filter PropList. Lookup is case-insensitive on symbol /
            // string keys to match Director's convention.
            let (filter_type, props) = match player.get_datum(&args[0]) {
                Datum::PropList(items, _) => {
                    let mut filter_type: Option<String> = None;
                    let mut props: HashMap<String, f64> = HashMap::new();
                    for (k, v) in items.iter() {
                        let key = match player.get_datum(k) {
                            Datum::Symbol(s) | Datum::String(s) => s.to_lowercase(),
                            _ => continue,
                        };
                        if key == "filtertype" {
                            if let Datum::Symbol(s) | Datum::String(s) = player.get_datum(v) {
                                filter_type = Some(s.to_lowercase());
                            }
                        } else {
                            // Numeric properties for AdjustColor.
                            let val = player.get_datum(v).float_value().unwrap_or(0.0);
                            props.insert(key, val);
                        }
                    }
                    (filter_type, props)
                }
                _ => {
                    return Err(ScriptError::new(
                        "applyFilter argument is not a filter object".to_string(),
                    ));
                }
            };

            let kind = filter_type.unwrap_or_default();
            match kind.as_str() {
                "adjustcolorfilter" => {
                    let bitmap = player.bitmap_manager.get_bitmap_mut(*bitmap_ref).ok_or_else(
                        || ScriptError::new("applyFilter: invalid bitmap".to_string()),
                    )?;
                    let brightness = props.get("brightness").copied().unwrap_or(0.0).clamp(-100.0, 100.0);
                    let contrast = props.get("contrast").copied().unwrap_or(0.0).clamp(-100.0, 100.0);
                    let saturation = props.get("saturation").copied().unwrap_or(0.0).clamp(-100.0, 100.0);
                    let hue = props.get("hue").copied().unwrap_or(0.0).clamp(-180.0, 180.0);
                    apply_adjust_color_filter(bitmap, brightness, contrast, saturation, hue);
                    bitmap.mark_dirty();
                }
                "" => {
                    return Err(ScriptError::new(
                        "applyFilter: filter object has no #filterType".to_string(),
                    ));
                }
                other => {
                    log::warn!(
                        "applyFilter: filter type '#{}' is not implemented \u{2014} bitmap unchanged",
                        other
                    );
                }
            }

            Ok(datum.clone())
        })
    }

    pub fn get_prop_handler(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if args.len() == 0 {
            return Err(ScriptError::new("getProp requires at least 1 argument".to_string()));
        }
        reserve_player_mut(|player| {
            let prop = player.get_datum(&args[0]).string_value()?;
            let prop_value = Self::get_prop(player, datum, &prop)?;
            if args.len() == 1 {
                Ok(prop_value)
            } else if args.len() == 2 {
                let prop_key_ref = args[1].clone();
                TypeUtils::get_sub_prop(&prop_value, &prop_key_ref, player)
            } else {
                Err(ScriptError::new(
                    "getProp with sub-property requires 2 arguments".to_string(),
                ))
            }
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        let bitmap = player.get_datum(datum);
        let bitmap = match bitmap {
            Datum::BitmapRef(bitmap) => Ok(bitmap),
            _ => Err(ScriptError::new(
                "Cannot get prop of non-bitmap".to_string(),
            )),
        }?;
        let bitmap = player.bitmap_manager.get_bitmap(*bitmap).unwrap();
        let width = bitmap.width as i32;
        let height = bitmap.height as i32;
        let result = match prop {
            "width" => Ok(Datum::Int(width)),
            "height" => Ok(Datum::Int(height)),
            "rect" => {
                 Ok(Datum::Rect([0.0, 0.0, width as f64, height as f64], 0))
            }
            "depth" => Ok(Datum::Int(bitmap.bit_depth as i32)),
            "paletteRef" => {
                if let PaletteRef::BuiltIn(palette) = bitmap.palette_ref {
                    Ok(Datum::Symbol(palette.symbol_string()))
                } else {
                    Ok(Datum::PaletteRef(bitmap.palette_ref.to_owned()))
                }
            }
            "ilk" => Ok(Datum::Symbol("image".to_string())),
            "useAlpha" => Ok(Datum::Int(if bitmap.use_alpha { 1 } else { 0 })),
            _ => Err(ScriptError::new(format!(
                "Cannot get bitmap property {}",
                prop
            ))),
        }?;
        Ok(player.alloc_datum(result))
    }

    pub fn set_bitmap_ref_prop(
        player: &mut DirPlayer,
        bitmap_ref: BitmapRef,
        prop: &str,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        let value = player.get_datum(value);
        match prop {
            "paletteRef" => match value {
                Datum::Symbol(symbol) => {
                    let palette = BuiltInPalette::from_symbol_string(&symbol).ok_or_else(|| {
                        ScriptError::new("Invalid built-in palette symbol".to_string())
                    })?;
                    let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_ref).unwrap();
                    bitmap.palette_ref = PaletteRef::BuiltIn(palette);
                    Ok(())
                }
                Datum::CastMember(member_ref) => {
                    let member_ref = member_ref.to_owned();
                    let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_ref).unwrap();
                    bitmap.palette_ref = PaletteRef::Member(member_ref);
                    Ok(())
                }
                Datum::PaletteRef(palette_ref) => {
                    let palette_ref = palette_ref.to_owned();
                    let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_ref).unwrap();
                    bitmap.palette_ref = palette_ref;
                    Ok(())
                }
                _ => Err(ScriptError::new(format!(
                    "Cannot set paletteRef to datum of type {}",
                    value.type_str()
                ))),
            },
            "useAlpha" => {
                let use_alpha = value.to_bool()?;
                let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_ref).unwrap();
                bitmap.use_alpha = use_alpha;
                Ok(())
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set bitmap property {}",
                prop
            ))),
        }
    }
}

/// Adobe Flash AdjustColor convention (Director chapter 15 inherits this from
/// the Flash Bitmap Filters surface):
///   brightness  ∈ [-100, 100]   — additive luminance shift; ±100 maps to
///                                  ±100/255 added per channel in 0..1 space.
///   contrast    ∈ [-100, 100]   — multiplicative around 0.5 grey;
///                                  factor = (100 + contrast) / 100.
///   saturation  ∈ [-100, 100]   — chroma scale; -100 = full grayscale,
///                                  0 = identity, +100 = doubled chroma.
///                                  factor = (100 + saturation) / 100.
///   hue         ∈ [-180, 180]   — hue rotation in degrees around the
///                                  Rec. 601 luma axis.
///
/// Order matches Flash's internal pipeline: hue → saturation → contrast →
/// brightness. (Some implementations swap the last two; Flash applies
/// contrast then brightness, which is what we do here.)
///
/// Operates in-place on RGB channels; alpha is preserved. Supports 32-bit and
/// 16-bit bitmaps (the common cases for textures); 8-bit / palette bitmaps
/// would need a roundtrip through the palette and are out of scope for now —
/// the function emits a debug-log warning and skips paletted images.
fn apply_adjust_color_filter(
    bitmap: &mut Bitmap,
    brightness: f64, contrast: f64, saturation: f64, hue_deg: f64,
) {
    if bitmap.bit_depth < 16 {
        log::warn!(
            "applyFilter(#adjustColorFilter): bitmap is {}-bit / paletted; skipped",
            bitmap.bit_depth
        );
        return;
    }

    // Pre-compute the contributions of each step so we can apply them per pixel.
    let cos_h = (hue_deg.to_radians()).cos();
    let sin_h = (hue_deg.to_radians()).sin();
    // Rec. 601 luma weights — Flash uses these for the AdjustColor filter.
    let lr = 0.213_f64;
    let lg = 0.715_f64;
    let lb = 0.072_f64;
    // Hue-rotation matrix (RGB → RGB) around the luminance axis. Standard
    // formulation: M = L + cos·(I − L) + sin·R, where L is the projection
    // onto the luma axis (constant rows) and R is the rotation matrix in the
    // chroma plane.
    let m00 = lr + cos_h * (1.0 - lr) + sin_h * (-lr);
    let m01 = lg + cos_h * (-lg)      + sin_h * (-lg);
    let m02 = lb + cos_h * (-lb)      + sin_h * (1.0 - lb);
    let m10 = lr + cos_h * (-lr)      + sin_h * (0.143);
    let m11 = lg + cos_h * (1.0 - lg) + sin_h * (0.140);
    let m12 = lb + cos_h * (-lb)      + sin_h * (-0.283);
    let m20 = lr + cos_h * (-lr)      + sin_h * (-(1.0 - lr));
    let m21 = lg + cos_h * (-lg)      + sin_h * (lg);
    let m22 = lb + cos_h * (1.0 - lb) + sin_h * (lb);

    let sat_factor = (100.0 + saturation) / 100.0;
    let contrast_factor = (100.0 + contrast) / 100.0;
    let bright_shift = brightness / 100.0; // 0..1 scale (±100 maps to ±1.0).

    let width = bitmap.width as i32;
    let height = bitmap.height as i32;
    let bytes_per_pixel = (bitmap.bit_depth / 8) as usize;

    // Snapshot the existing matte (if any) so we can restore it after
    // set_pixel — set_pixel clears it because it doesn't know we're not
    // changing the alpha shape, just the RGB values.
    let saved_matte = bitmap.matte.clone();

    // We need a palette map for resolve_color_ref / set_pixel. AdjustColor on
    // 16/32-bit images doesn't actually need the palette but the API requires
    // one — pass an empty map (resolve_color_ref handles None palettes).
    let palettes = crate::player::bitmap::palette_map::PaletteMap::new();

    for y in 0..height {
        for x in 0..width {
            // Read alpha first (preserve through transform).
            let (r0, g0, b0, a) = if bitmap.bit_depth == 32 {
                let idx = (y as usize * width as usize + x as usize) * bytes_per_pixel;
                (
                    bitmap.data[idx],
                    bitmap.data[idx + 1],
                    bitmap.data[idx + 2],
                    bitmap.data[idx + 3],
                )
            } else {
                // 16-bit: get_pixel_color_with_alpha resolves through palette.
                bitmap.get_pixel_color_with_alpha(&palettes, x as u16, y as u16)
            };

            // 0..1 floats.
            let mut r = r0 as f64 / 255.0;
            let mut g = g0 as f64 / 255.0;
            let mut b = b0 as f64 / 255.0;

            // 1) Hue rotation.
            let hr = m00 * r + m01 * g + m02 * b;
            let hg = m10 * r + m11 * g + m12 * b;
            let hb = m20 * r + m21 * g + m22 * b;
            r = hr; g = hg; b = hb;

            // 2) Saturation: blend between luminance grayscale and original.
            let lum = lr * r + lg * g + lb * b;
            r = lum + (r - lum) * sat_factor;
            g = lum + (g - lum) * sat_factor;
            b = lum + (b - lum) * sat_factor;

            // 3) Contrast: scale around 0.5 grey.
            r = (r - 0.5) * contrast_factor + 0.5;
            g = (g - 0.5) * contrast_factor + 0.5;
            b = (b - 0.5) * contrast_factor + 0.5;

            // 4) Brightness: additive shift.
            r += bright_shift;
            g += bright_shift;
            b += bright_shift;

            // Clamp to 0..1 and back to u8.
            let r_u = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
            let g_u = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
            let b_u = (b.clamp(0.0, 1.0) * 255.0).round() as u8;

            // Write RGB. For 32-bit we write directly to preserve the existing
            // alpha byte (set_pixel doesn't take alpha). For 16-bit we use
            // set_pixel which roundtrips through the 5550 packing.
            if bitmap.bit_depth == 32 {
                let idx = (y as usize * width as usize + x as usize) * bytes_per_pixel;
                bitmap.data[idx] = r_u;
                bitmap.data[idx + 1] = g_u;
                bitmap.data[idx + 2] = b_u;
                // Alpha untouched (a is the original).
                let _ = a;
            } else {
                bitmap.set_pixel(x, y, (r_u, g_u, b_u), &palettes);
            }
        }
    }

    bitmap.matte = saved_matte;
}

