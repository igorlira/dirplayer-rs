use crate::{director::lingo::datum::Datum, player::bitmap::bitmap::PaletteRef, rendering::render_stage_to_bitmap};

use super::{
    bitmap::bitmap::{get_system_default_palette, Bitmap},
    DatumRef, DirPlayer, ScriptError,
};

pub fn get_stage_prop(player: &mut DirPlayer, prop: &str) -> Result<Datum, ScriptError> {
    match prop {
        "rect" => Ok(
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
        _ => return Err(ScriptError::new(format!("Invalid stage property {}", prop))),
    }
}

pub fn set_stage_prop(
    player: &mut DirPlayer,
    prop: &str,
    _value: &DatumRef,
) -> Result<(), ScriptError> {
    match prop {
        "title" => {
            player.title = "title".to_string();
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
