use crate::{
    director::lingo::datum::Datum,
    player::{
        bitmap::bitmap::{get_system_default_palette, resolve_color_ref, PaletteRef},
        reserve_player_mut,
        sprite::ColorRef,
        DatumRef, DirPlayer, ScriptError,
    },
};

pub struct ColorDatumHandlers {}

impl ColorDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &String,
        _args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "hexString" => reserve_player_mut(|player| {
                let color_ref = player.get_datum(datum).to_color_ref()?;
                let (r, g, b) = resolve_color_ref(
                    &player.movie.cast_manager.palettes(),
                    color_ref,
                    &PaletteRef::BuiltIn(get_system_default_palette()),
                    8,
                );
                let hex_string = format!("#{:02x}{:02x}{:02x}", r, g, b);
                Ok(player.alloc_datum(Datum::String(hex_string)))
            }),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for color"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        let color_ref = player.get_datum(datum).to_color_ref()?;
        match prop.as_str() {
            "red" => match color_ref {
                ColorRef::Rgb(r, _, _timeout_name) => Ok(player.alloc_datum(Datum::Int(*r as i32))),
                ColorRef::PaletteIndex(i) => match i {
                    0 => Ok(player.alloc_datum(Datum::Int(255))),
                    255 => Ok(player.alloc_datum(Datum::Int(0))),
                    _ => Ok(player.alloc_datum(Datum::Int(255))),
                },
            },
            "green" => match color_ref {
                ColorRef::Rgb(_, g, _timeout_name) => Ok(player.alloc_datum(Datum::Int(*g as i32))),
                ColorRef::PaletteIndex(i) => match i {
                    0 => Ok(player.alloc_datum(Datum::Int(255))),
                    255 => Ok(player.alloc_datum(Datum::Int(0))),
                    _ => Ok(player.alloc_datum(Datum::Int(0))),
                },
            },
            "blue" => match color_ref {
                ColorRef::Rgb(_, _, b) => Ok(player.alloc_datum(Datum::Int(*b as i32))),
                ColorRef::PaletteIndex(i) => match i {
                    0 => Ok(player.alloc_datum(Datum::Int(255))),
                    255 => Ok(player.alloc_datum(Datum::Int(0))),
                    _ => Ok(player.alloc_datum(Datum::Int(255))),
                },
            },
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("color".to_owned()))),
            _ => Err(ScriptError::new(format!(
                "Cannot get color property {}",
                prop
            ))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        match prop.as_str() {
            "red" => {
                let r = player.get_datum(value).int_value()?;
                let color_ref = player.get_datum_mut(datum).to_color_ref_mut()?;
                match color_ref {
                    ColorRef::Rgb(_, g, b) => {
                        *color_ref = ColorRef::Rgb(r as u8, *g, *b);
                        Ok(())
                    }
                    ColorRef::PaletteIndex(_) => {
                        *color_ref = ColorRef::Rgb(r as u8, 0, 0);
                        Ok(())
                    }
                }
            }
            "green" => {
                let g = player.get_datum(value).int_value()?;
                let color_ref = player.get_datum_mut(datum).to_color_ref_mut()?;
                match color_ref {
                    ColorRef::Rgb(r, _, b) => {
                        *color_ref = ColorRef::Rgb(*r, g as u8, *b);
                        Ok(())
                    }
                    ColorRef::PaletteIndex(_) => {
                        *color_ref = ColorRef::Rgb(0, g as u8, 0);
                        Ok(())
                    }
                }
            }
            "blue" => {
                let b = player.get_datum(value).int_value()?;
                let color_ref = player.get_datum_mut(datum).to_color_ref_mut()?;
                match color_ref {
                    ColorRef::Rgb(r, g, _) => {
                        *color_ref = ColorRef::Rgb(*r, *g, b as u8);
                        Ok(())
                    }
                    ColorRef::PaletteIndex(_) => {
                        *color_ref = ColorRef::Rgb(0, 0, b as u8);
                        Ok(())
                    }
                }
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set color property {}",
                prop
            ))),
        }
    }
}
