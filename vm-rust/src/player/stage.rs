use log::warn;

use crate::{director::lingo::datum::Datum, player::bitmap::bitmap::PaletteRef};

use super::{
    bitmap::bitmap::{get_system_default_palette, Bitmap},
    DatumRef, DirPlayer, ScriptError,
};

pub fn get_stage_prop(player: &mut DirPlayer, prop: &str) -> Result<Datum, ScriptError> {
    match prop {
        "rect" => Ok(Datum::IntRect((
            0,
            0,
            player.movie.rect.width(),
            player.movie.rect.height(),
        ))),
        "sourceRect" => {
            // TODO where does this come from?
            Ok(Datum::IntRect((
                0,
                0,
                player.movie.rect.width(),
                player.movie.rect.height(),
            )))
        }
        "bgColor" => Ok(Datum::ColorRef(player.bg_color.clone())),
        "image" => {
            warn!("TODO get stage image");
            let new_bitmap = Bitmap::new(
                player.movie.rect.width() as u16,
                player.movie.rect.height() as u16,
                32,
                32,
                0,
                PaletteRef::BuiltIn(get_system_default_palette()),
            );
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
