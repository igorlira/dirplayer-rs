use crate::{director::lingo::datum::Datum, player::bitmap::bitmap::PaletteRef, rendering::render_stage_to_bitmap};

use super::{
    bitmap::bitmap::{get_system_default_palette, Bitmap},
    DatumRef, DirPlayer, ScriptError,
};

pub fn get_stage_prop(player: &mut DirPlayer, prop: &str) -> Result<Datum, ScriptError> {
    match prop {
        "rect" | "drawRect" => Ok(
            Datum::Rect([
                player.alloc_datum(Datum::Int(0)),
                player.alloc_datum(Datum::Int(0)),
                player.alloc_datum(Datum::Int(player.movie.rect.width())),
                player.alloc_datum(Datum::Int(player.movie.rect.height())),
            ])),
        "sourceRect" => {
            // TODO where does this come from?
            Ok(Datum::Rect([
                player.alloc_datum(Datum::Int(0)),
                player.alloc_datum(Datum::Int(0)),
                player.alloc_datum(Datum::Int(player.movie.rect.width())),
                player.alloc_datum(Datum::Int(player.movie.rect.height())),
            ]))
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
            let bitmap_id = player.bitmap_manager.add_bitmap(new_bitmap);
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
        _ => {
            return Err(ScriptError::new(format!(
                "Cannot set stage property {}",
                prop
            )))
        }
    }
}
