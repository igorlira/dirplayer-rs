use std::collections::HashMap;

use crate::{
    director::lingo::datum::{datum_bool, Datum},
    player::{
        bitmap::{
            bitmap::{resolve_color_ref, BuiltInPalette, PaletteRef},
            manager::BitmapRef,
        },
        geometry::IntRect,
        player_duplicate_datum, reserve_player_mut, DatumRef, DirPlayer, ScriptError,
    },
};

use super::prop_list::PropListUtils;

pub struct BitmapDatumHandlers {}

impl BitmapDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "fill" => Self::fill(datum, args),
            "draw" => Self::draw(datum, args),
            "setPixel" => Self::set_pixel(datum, args),
            "duplicate" => Self::duplicate(datum, args),
            "copyPixels" => Self::copy_pixels(datum, args),
            "createMatte" => Self::create_matte(datum, args),
            "trimWhiteSpace" => Self::trim_whitespace(datum, args),
            "getPixel" => Self::get_pixel(datum, args),
            "crop" => Self::crop(datum, args),
            "setAlpha" => Self::set_alpha(datum, args),
            "floodFill" => reserve_player_mut(|player| {
                // Args: point, color
                if args.len() != 2 {
                    return Err(ScriptError::new(
                        "floodFill requires 2 arguments".to_string(),
                    ));
                }

                let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;

                let point_ref = player.get_datum(&args[0]).to_point()?;

                let x = player.get_datum(&point_ref[0]).int_value()?;
                let y = player.get_datum(&point_ref[1]).int_value()?;

                let point_tuple = (x, y);

                let color_ref = player.get_datum(&args[1]).to_color_ref()?;

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
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for bitmap datum"
            ))),
        }
    }

    pub fn get_pixel(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap = player.get_datum(datum).to_bitmap_ref()?;
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap).unwrap();
            let x = player.get_datum(&args[0]).int_value()?;
            let y = player.get_datum(&args[1]).int_value()?;
            let color = bitmap.get_pixel_color_ref(x as u16, y as u16);
            let color_ref = player.alloc_datum(Datum::ColorRef(color));
            Ok(color_ref)
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
            // TODO alpha threshold
            if args.len() != 0 {
                return Err(ScriptError::new(
                    "Invalid number of arguments for createMatte".to_string(),
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
            let rect_refs = player.get_datum(&args[0]).to_rect()?;

            let left = player.get_datum(&rect_refs[0]).int_value()?;
            let top = player.get_datum(&rect_refs[1]).int_value()?;
            let right = player.get_datum(&rect_refs[2]).int_value()?;
            let bottom = player.get_datum(&rect_refs[3]).int_value()?;

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

            // Add the new bitmap to the manager and return a reference
            let new_bitmap_ref = player.bitmap_manager.add_bitmap(cropped_bitmap);
            Ok(player.alloc_datum(Datum::BitmapRef(new_bitmap_ref)))
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

                    // Copy alpha values from the 8-bit image to the alpha channel of the 32-bit image
                    // For 8-bit images, each byte is a grayscale value that we use as alpha
                    for y in 0..height {
                        for x in 0..width {
                            let alpha_idx = (y as usize * width as usize + x as usize) * 4;
                            let dst_idx = (y as usize * width as usize + x as usize) * 4 + 3;

                            if alpha_idx < alpha_data.len() && dst_idx < bitmap.data.len() {
                                // Use the red channel of the alpha image as the alpha value
                                // (since it's grayscale, R=G=B)
                                bitmap.data[dst_idx] = alpha_data[alpha_idx];
                            }
                        }
                    }
                    // bitmap.version += 1;

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
            let rect_refs = player.get_datum(&args[0]).to_rect()?;

            let x1 = player.get_datum(&rect_refs[0]).int_value()?;
            let y1 = player.get_datum(&rect_refs[1]).int_value()?;
            let x2 = player.get_datum(&rect_refs[2]).int_value()?;
            let y2 = player.get_datum(&rect_refs[3]).int_value()?;

            let draw_map = player.get_datum(&args[1]).to_map()?;
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();

            let color_ref = PropListUtils::get_by_concrete_key(
                &draw_map,
                &Datum::Symbol("color".to_owned()),
                &player.allocator,
            )?;
            let color_ref = player.get_datum(&color_ref).to_color_ref()?;
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

            let bitmap = player.bitmap_manager.get_bitmap_mut(*bitmap_ref).unwrap();
            match shape_type.as_str() {
                "rect" => {
                    bitmap.stroke_rect(x1, y1, x2, y2, color, &palettes, blend as f32 / 100.0);
                }
                _ => {
                    return Err(ScriptError::new("Invalid shapeType for draw".to_string()));
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

            let (x, y, color_obj_or_int, bit_depth, original_bit_depth, palette_ref) = {
                let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();

                let x = player.get_datum(&args[0]).int_value()?;
                let y = player.get_datum(&args[1]).int_value()?;
                let color_obj_or_int = player.get_datum(&args[2]);

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
                if bit_depth != 8 {
                    return Err(ScriptError::new(
                        "Cannot set pixel with int color on non-8-bit bitmap".to_string(),
                    ));
                }

                let int_value = color_obj_or_int.int_value()? as u8;
                bitmap.set_pixel(x, y, (int_value, int_value, int_value), &palettes);
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
                let rect_refs = player.get_datum(&args[0]).to_rect()?;
                let color = player.get_datum(&args[1]).to_color_ref()?;

                let x1 = player.get_datum(&rect_refs[0]).int_value()?;
                let y1 = player.get_datum(&rect_refs[1]).int_value()?;
                let x2 = player.get_datum(&rect_refs[2]).int_value()?;
                let y2 = player.get_datum(&rect_refs[3]).int_value()?;

                ((x1, y1, x2, y2), color)
            } else if args.len() == 5 {
                let x = player.get_datum(&args[0]).int_value()?;
                let y = player.get_datum(&args[1]).int_value()?;
                let width = player.get_datum(&args[2]).int_value()?;
                let height = player.get_datum(&args[3]).int_value()?;
                let color = player.get_datum(&args[4]).to_color_ref()?;
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
            let src_rect = player.get_datum(&args[2]).to_rect()?;
            let sx1 = player.get_datum(&src_rect[0]).int_value()?;
            let sy1 = player.get_datum(&src_rect[1]).int_value()?;
            let sx2 = player.get_datum(&src_rect[2]).int_value()?;
            let sy2 = player.get_datum(&src_rect[3]).int_value()?;
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

            let dest_rect = match dest_rect_or_quad {
                Datum::Rect(rect_refs) => {
                    let x1 = player.get_datum(&rect_refs[0]).int_value()?;
                    let y1 = player.get_datum(&rect_refs[1]).int_value()?;
                    let x2 = player.get_datum(&rect_refs[2]).int_value()?;
                    let y2 = player.get_datum(&rect_refs[3]).int_value()?;
                    IntRect::from_tuple((x1, y1, x2, y2))
                }
                Datum::List(_, list_val, _) => {
                    let p1 = {
                        let p = player.get_datum(&list_val[0]).to_point()?;
                        let x = player.get_datum(&p[0]).int_value()?;
                        let y = player.get_datum(&p[1]).int_value()?;
                        (x, y)
                    };
                    let p2 = {
                        let p = player.get_datum(&list_val[1]).to_point()?;
                        let x = player.get_datum(&p[0]).int_value()?;
                        let y = player.get_datum(&p[1]).int_value()?;
                        (x, y)
                    };
                    let p3 = {
                        let p = player.get_datum(&list_val[2]).to_point()?;
                        let x = player.get_datum(&p[0]).int_value()?;
                        let y = player.get_datum(&p[1]).int_value()?;
                        (x, y)
                    };
                    let p4 = {
                        let p = player.get_datum(&list_val[3]).to_point()?;
                        let x = player.get_datum(&p[0]).int_value()?;
                        let y = player.get_datum(&p[1]).int_value()?;
                        (x, y)
                    };

                    let dest_rect = IntRect::from_quad(p1, p2, p3, p4);
                    dest_rect
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

            dst_bitmap.copy_pixels(
                &palettes,
                &src_bitmap,
                dest_rect,
                IntRect::from_tuple((sx1, sy1, sx2, sy2)),
                &param_list_concrete,
                Some(&player.movie.score),
            );
            Ok(datum.clone())
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
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
        let result = match prop.as_str() {
            "width" => Ok(Datum::Int(width)),
            "height" => Ok(Datum::Int(height)),
            "rect" => {
                let x0 = player.alloc_datum(Datum::Int(0));
                let y0 = player.alloc_datum(Datum::Int(0));
                let w  = player.alloc_datum(Datum::Int(width));
                let h  = player.alloc_datum(Datum::Int(height));
                 Ok(Datum::Rect([x0, y0, w, h]))
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
        prop: &String,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        let value = player.get_datum(value);
        match prop.as_str() {
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
