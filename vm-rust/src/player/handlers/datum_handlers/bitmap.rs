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
            "floodFill" => reserve_player_mut(|player| {
                // Args: point, color
                if args.len() != 2 {
                    return Err(ScriptError::new(
                        "floodFill requires 2 arguments".to_string(),
                    ));
                }

                let bitmap_ref = player.get_datum(datum).to_bitmap_ref()?;
                let point = player.get_datum(&args[0]).to_int_point()?;
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

                bitmap.flood_fill(point, target_rgb, &palettes);

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

    pub fn draw(datum: &DatumRef, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let bitmap = player.get_datum(datum);
            let bitmap_ref = match bitmap {
                Datum::BitmapRef(bitmap) => Ok(bitmap),
                _ => Err(ScriptError::new("Cannot draw non-bitmap".to_string())),
            }?;
            let rect = player.get_datum(&args[0]).to_int_rect()?;
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
                    let (x1, y1, x2, y2) = rect;
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
            let (rect, color_ref) = if args.len() == 2 {
                let rect = player.get_datum(&args[0]).to_int_rect()?;
                let color = player.get_datum(&args[1]).to_color_ref()?;
                (rect, color)
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
            let (x1, y1, x2, y2) = rect;
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
            let src_rect = player.get_datum(&args[2]).to_int_rect()?;
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
                Datum::IntRect(rect) => IntRect::from_tuple(*rect),
                Datum::List(_, list_val, _) => IntRect::from_quad(
                    player.get_datum(&list_val[0]).to_int_point()?,
                    player.get_datum(&list_val[1]).to_int_point()?,
                    player.get_datum(&list_val[2]).to_int_point()?,
                    player.get_datum(&list_val[3]).to_int_point()?,
                ),
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
                IntRect::from_tuple(src_rect),
                &param_list_concrete,
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
        let result = match prop.as_str() {
            "width" => Ok(Datum::Int(bitmap.width as i32)),
            "height" => Ok(Datum::Int(bitmap.height as i32)),
            "rect" => Ok(Datum::IntRect((
                0,
                0,
                bitmap.width as i32,
                bitmap.height as i32,
            ))),
            "depth" => Ok(Datum::Int(bitmap.bit_depth as i32)),
            "paletteRef" => {
                if let PaletteRef::BuiltIn(palette) = bitmap.palette_ref {
                    Ok(Datum::Symbol(palette.symbol_string()))
                } else {
                    Ok(Datum::PaletteRef(bitmap.palette_ref.to_owned()))
                }
            }
            "ilk" => Ok(Datum::Symbol("image".to_string())),
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
            _ => Err(ScriptError::new(format!(
                "Cannot set bitmap property {}",
                prop
            ))),
        }
    }
}
