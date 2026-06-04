use crate::{
    director::lingo::datum::Datum,
    player::{
        bitmap::bitmap::{get_system_default_palette, nearest_palette_index, resolve_color_ref, PaletteRef},
        reserve_player_mut,
        sprite::ColorRef,
        symbols::symbol::Symbol,
        DatumRef, DirPlayer, ScriptError,
    },
};

pub struct ColorDatumHandlers {}

impl ColorDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: Symbol,
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
                let hex_string = format!("#{:02X}{:02X}{:02X}", r, g, b);
                Ok(player.alloc_datum(Datum::String(hex_string)))
            }),
            "duplicate" => Ok(datum.clone()),
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for color"
            ))),
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        let color_ref = player.get_datum(datum).to_color_ref()?;
        match prop.as_str() {
            "red" => match color_ref {
                ColorRef::Rgb(r, _, _) => Ok(player.alloc_datum(Datum::Int(*r as i32))),
                ColorRef::PaletteIndex(i) => match i {
                    0 => Ok(player.alloc_datum(Datum::Int(255))),
                    255 => Ok(player.alloc_datum(Datum::Int(0))),
                    _ => Ok(player.alloc_datum(Datum::Int(255))),
                },
            },
            "green" => match color_ref {
                ColorRef::Rgb(_, g, _) => Ok(player.alloc_datum(Datum::Int(*g as i32))),
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
            "ilk" => Ok(player.alloc_datum(Datum::Symbol(Symbol::from_str("color")))),
            "colorType" => match color_ref {
                ColorRef::Rgb(..) => Ok(player.alloc_datum(Datum::Symbol(Symbol::from_str("rgb")))),
                ColorRef::PaletteIndex(_) => Ok(player.alloc_datum(Datum::Symbol(Symbol::from_str("paletteIndex")))),
            },
            "paletteIndex" => match color_ref {
                ColorRef::PaletteIndex(i) => Ok(player.alloc_datum(Datum::Int(*i as i32))),
                // Director 11.5 Scripting Dictionary p.832: `.paletteIndex` on
                // an RGB color returns the nearest match in the current palette.
                // E.g. `rgb(0,0,0).paletteIndex` → 255 on SystemWin.
                ColorRef::Rgb(r, g, b) => {
                    let idx = nearest_palette_index(*r, *g, *b, &get_system_default_palette());
                    Ok(player.alloc_datum(Datum::Int(idx as i32)))
                }
            },
            _ => Err(ScriptError::new(format!(
                "Cannot get color property {}",
                prop
            ))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: Symbol,
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
            "colorType" => {
                let symbol = player.get_datum(value).string_value()?;
                let color_ref = player.get_datum(datum).to_color_ref()?.clone();
                match symbol.as_str() {
                    "rgb" => {
                        if let ColorRef::PaletteIndex(_) = color_ref {
                            let (r, g, b) = resolve_color_ref(
                                &player.movie.cast_manager.palettes(),
                                &color_ref,
                                &PaletteRef::BuiltIn(get_system_default_palette()),
                                8,
                            );
                            let color_mut = player.get_datum_mut(datum).to_color_ref_mut()?;
                            *color_mut = ColorRef::Rgb(r, g, b);
                        }
                        Ok(())
                    }
                    "paletteIndex" => {
                        if let ColorRef::Rgb(r, g, b) = color_ref {
                            let luminance = (r as u16 * 30 + g as u16 * 59 + b as u16 * 11) / 100;
                            let index = if luminance > 128 { 0u8 } else { 255u8 };
                            let color_mut = player.get_datum_mut(datum).to_color_ref_mut()?;
                            *color_mut = ColorRef::PaletteIndex(index);
                        }
                        Ok(())
                    }
                    _ => Err(ScriptError::new(format!(
                        "Invalid colorType: {}. Expected #rgb or #paletteIndex", symbol
                    ))),
                }
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set color property {}",
                prop
            ))),
        }
    }
}
