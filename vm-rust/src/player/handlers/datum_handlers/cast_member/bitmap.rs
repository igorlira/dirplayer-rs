use crate::{
    director::lingo::datum::Datum,
    player::{
        bitmap::bitmap::{BuiltInPalette, PaletteRef},
        cast_lib::CastMemberRef,
        handlers::datum_handlers::cast_member_ref::{borrow_member_mut, CastMemberRefHandlers},
        reserve_player_mut, DirPlayer, ScriptError,
    },
};
use num_traits::FromPrimitive;

pub struct BitmapMemberHandlers {}

impl BitmapMemberHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let bitmap_member = member.member_type.as_bitmap().unwrap();
        let bitmap_ref = bitmap_member.image_ref;
        let bitmap = player.bitmap_manager.get_bitmap(bitmap_member.image_ref);
        if !bitmap.is_some() {
            return Err(ScriptError::new(format!(
                "Cannot get prop of invalid bitmap ref"
            )));
        }
        match prop.as_str() {
            "width" => Ok(Datum::Int(bitmap.map(|x| x.width as i32).unwrap_or(0))),
            "height" => Ok(Datum::Int(bitmap.map(|x| x.height as i32).unwrap_or(0))),
            "image" => Ok(Datum::BitmapRef(bitmap_ref)),
            "paletteRef" => Ok(Datum::PaletteRef(
                bitmap
                    .map(|x| x.palette_ref.clone())
                    .unwrap_or(PaletteRef::BuiltIn(BuiltInPalette::GrayScale)),
            )),
            "regPoint" => Ok(Datum::IntPoint((
                bitmap_member.reg_point.0 as i32,
                bitmap_member.reg_point.1 as i32,
            ))),
            "rect" => {
                let width = bitmap.map(|x| x.width as i32).unwrap_or(0);
                let height = bitmap.map(|x| x.height as i32).unwrap_or(0);
                Ok(Datum::IntRect((0, 0, width, height)))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for bitmap",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &String,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop.as_str() {
            "image" => {
                let bitmap_ref = value.to_bitmap_ref()?;
                reserve_player_mut(|player| {
                    let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();
                    let clone = bitmap.clone();

                    let member_image_ref = {
                        let cast_member = player
                            .movie
                            .cast_manager
                            .find_member_by_ref(member_ref)
                            .unwrap();
                        let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                        bitmap_member.image_ref
                    };
                    player
                        .bitmap_manager
                        .replace_bitmap(member_image_ref, clone);
                    Ok(())
                })
            }
            "regPoint" => borrow_member_mut(
                member_ref,
                |_| {},
                |cast_member, _| {
                    let value = value.to_int_point()?;
                    let value: (i16, i16) = (value.0 as i16, value.1 as i16);
                    cast_member.member_type.as_bitmap_mut().unwrap().reg_point = value;
                    Ok(())
                },
            ),
            "paletteRef" => {
                let bitmap_id = borrow_member_mut(
                    member_ref,
                    |_| {},
                    |cast_member, _| {
                        let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                        let bitmap = bitmap_member.image_ref;
                        Ok(bitmap)
                    },
                )?;
                match value {
                    Datum::Symbol(name) => {
                        let palette_ref = BuiltInPalette::from_symbol_string(&name).unwrap();
                        reserve_player_mut(|player| {
                            let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                            bitmap.palette_ref = PaletteRef::BuiltIn(palette_ref);
                            Ok(())
                        })?;
                    }
                    Datum::CastMember(member_ref) => {
                        reserve_player_mut(|player| {
                            let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                            bitmap.palette_ref = PaletteRef::from(
                                member_ref.cast_member as i16,
                                member_ref.cast_lib as u32,
                            );
                            Ok(())
                        })?;
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Cannot set bitmap member paletteRef to type {}",
                            value.type_str()
                        )))
                    }
                }
                Ok(())
            }
            "palette" => {
                let bitmap_id = borrow_member_mut(
                    member_ref,
                    |_| {},
                    |cast_member, _| {
                        let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                        let bitmap = bitmap_member.image_ref;
                        Ok(bitmap)
                    },
                )?;
                match value {
                    Datum::Int(palette_ref) => {
                        let member =
                            CastMemberRefHandlers::member_ref_from_slot_number(palette_ref as u32);
                        reserve_player_mut(|player| {
                            let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                            if palette_ref < 0 {
                                bitmap.palette_ref = PaletteRef::BuiltIn(
                                    BuiltInPalette::from_i16(palette_ref as i16).unwrap(),
                                )
                            } else {
                                bitmap.palette_ref = PaletteRef::from(
                                    member.cast_member as i16,
                                    member.cast_lib as u32,
                                );
                            }
                            Ok(())
                        })?;
                    }
                    Datum::CastMember(member_ref) => {
                        reserve_player_mut(|player| {
                            let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                            bitmap.palette_ref = PaletteRef::from(
                                member_ref.cast_member as i16,
                                member_ref.cast_lib as u32,
                            );
                            Ok(())
                        })?;
                    }
                    _ => {
                        return Err(ScriptError::new(format!(
                            "Cannot set bitmap member palette to type {}",
                            value.type_str()
                        )))
                    }
                }
                Ok(())
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set castMember prop {} for bitmap",
                prop
            ))),
        }
    }
}
