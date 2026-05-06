use crate::{
    director::lingo::datum::Datum,
    player::{
        DirPlayer, ScriptError, bitmap::bitmap::{BuiltInPalette, PaletteRef}, cast_lib::CastMemberRef, cast_member::Media, handlers::datum_handlers::cast_member_ref::{CastMemberRefHandlers, borrow_member_mut}, reserve_player_mut
    },
};
use num_traits::FromPrimitive;

pub struct BitmapMemberHandlers {}

impl BitmapMemberHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &str,
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
        match prop {
            "width" => Ok(Datum::Int(bitmap.map(|x| x.width as i32).unwrap_or(0))),
            "height" => Ok(Datum::Int(bitmap.map(|x| x.height as i32).unwrap_or(0))),
            "image" | "picture" => Ok(Datum::BitmapRef(bitmap_ref)),
            "media" => Ok(Datum::Media(Media::Bitmap {
                bitmap: bitmap.unwrap().clone(),
                reg_point: bitmap_member.reg_point,
            })),
            // `palette` and `paletteRef` are the same thing in our model —
            // both return the bitmap's current PaletteRef. CS catalog
            // scripts read both interchangeably (`member.palette` for
            // applying a swap, `member.paletteRef` for inspecting). Without
            // the alias, `put member("studiofloor_1_preview").palette`
            // errored even though the matching setter at line 200 accepts
            // the same name.
            "palette" | "paletteRef" => {
                let palette = bitmap
                    .map(|x| x.palette_ref.clone())
                    .unwrap_or(PaletteRef::BuiltIn(BuiltInPalette::GrayScale));
                match palette {
                    PaletteRef::BuiltIn(builtin) => Ok(Datum::Symbol(builtin.symbol_string())),
                    PaletteRef::Member(member_ref) => {
                        let member_ref = CastMemberRef {
                            cast_member: member_ref.cast_member,
                            cast_lib: member_ref.cast_lib,
                        };
                        Ok(Datum::CastMember(member_ref))
                    }
                    PaletteRef::Default => Ok(Datum::PaletteRef(PaletteRef::Default)),
                }
            },
            "rect" => {
                let width = bitmap.map(|x| x.width as i32).unwrap_or(0);
                let height = bitmap.map(|x| x.height as i32).unwrap_or(0);
                Ok(Datum::Rect([0.0, 0.0, width as f64, height as f64], 0))
            }
            "depth" => Ok(Datum::Int(
                bitmap
                    .map(|x| x.bit_depth as i32)
                    .unwrap_or(0),
            )),
            "useAlpha" => Ok(Datum::Int(if bitmap_member.info.use_alpha { 1 } else { 0 })),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for bitmap",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop {
            "image" | "picture" => {
                if value.is_void() {
                    return Ok(());
                }
                reserve_player_mut(|player| {
                    let bitmap_ref = player.resolve_bitmap_ref(&value)?;
                    let bitmap = player.bitmap_manager.get_bitmap(bitmap_ref).unwrap();
                    let new_width = bitmap.width;
                    let new_height = bitmap.height;
                    let mut clone = bitmap.clone();

                    let (member_image_ref, old_palette) = {
                        let cast_member = player
                            .movie
                            .cast_manager
                            .find_member_by_ref(member_ref)
                            .unwrap();
                        let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                        let old_palette = player.bitmap_manager.get_bitmap(bitmap_member.image_ref)
                            .map(|bm| bm.palette_ref.clone());
                        (bitmap_member.image_ref, old_palette)
                    };

                    // Inherit the member's existing palette when the new bitmap has
                    // the default system palette (e.g. from image(w,h,24) with no
                    // palette arg). This preserves the cast member's original palette.
                    if let Some(old_pal) = old_palette {
                        if matches!(clone.palette_ref, PaletteRef::BuiltIn(BuiltInPalette::SystemWin)) {
                            clone.palette_ref = old_pal;
                        }
                    }

                    player
                        .bitmap_manager
                        .replace_bitmap(member_image_ref, clone);

                    // Update the member's info.width and info.height to match the new bitmap
                    let cast_member = player
                        .movie
                        .cast_manager
                        .find_mut_member_by_ref(member_ref)
                        .unwrap();
                    let bitmap_member = cast_member.member_type.as_bitmap_mut().unwrap();
                    bitmap_member.info.width = new_width as u16;
                    bitmap_member.info.height = new_height as u16;
                    // Director auto-centers regPoint when `member.image = ...` is
                    // assigned. Without this, dynamically composed bitmaps (e.g.
                    // Coke Studios' AvatarEngine v-ego preview, where the script
                    // does `member("...").image = oPreviewImage` and expects the
                    // sprite to render with the figure centered on the sprite's
                    // loc) end up at regPoint (0, 0) and render off-center.
                    let reg_x = (new_width as i32) / 2;
                    let reg_y = (new_height as i32) / 2;
                    bitmap_member.reg_point = (reg_x as i16, reg_y as i16);
                    cast_member.reg_point = (reg_x, reg_y);

                    Ok(())
                })
            }
            "media" => {
                let media = value.media_value()?;
                let (media_bitmap, media_reg_point) = match media {
                    Media::Bitmap { bitmap, reg_point } => (bitmap, reg_point),
                    _ => return Err(ScriptError::new("Expected a bitmap media".to_string())),
                };
                reserve_player_mut(|player| {
                    let member_image_ref = {
                        let cast_member = player
                            .movie
                            .cast_manager
                            .find_member_by_ref(member_ref)
                            .unwrap();
                        let bitmap_member = cast_member.member_type.as_bitmap().unwrap();
                        bitmap_member.image_ref
                    };
                    player.bitmap_manager.replace_bitmap(member_image_ref, media_bitmap.clone());

                    let cast_member = player
                        .movie
                        .cast_manager
                        .find_mut_member_by_ref(member_ref)
                        .unwrap();
                    let bitmap_member = cast_member.member_type.as_bitmap_mut().unwrap();
                    bitmap_member.reg_point = media_reg_point;
                    cast_member.reg_point = (media_reg_point.0 as i32, media_reg_point.1 as i32);
                    Ok(())
                })
            }
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
                            bitmap.palette_ref = PaletteRef::Member(member_ref.clone());
                            Ok(())
                        })?;
                    }
                    Datum::PaletteRef(palette_ref) => {
                        reserve_player_mut(|player| {
                            let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                            bitmap.palette_ref = palette_ref;
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
                                bitmap.palette_ref = PaletteRef::Member(member.clone());
                            }
                            Ok(())
                        })?;
                    }
                    Datum::CastMember(member_ref) => {
                        reserve_player_mut(|player| {
                            let bitmap = player.bitmap_manager.get_bitmap_mut(bitmap_id).unwrap();
                            bitmap.palette_ref = PaletteRef::Member(member_ref.clone());
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
            "useAlpha" => {
                let use_alpha = value.to_bool()?;
                borrow_member_mut(
                    member_ref,
                    |_| {},
                    |cast_member, _| {
                        let bitmap_member = cast_member.member_type.as_bitmap_mut().unwrap();
                        bitmap_member.info.use_alpha = use_alpha;
                        Ok(())
                    },
                )?;
                // Also update the actual bitmap's use_alpha flag
                reserve_player_mut(|player| {
                    let member = player
                        .movie
                        .cast_manager
                        .find_member_by_ref(member_ref)
                        .unwrap();
                    let bitmap_member = member.member_type.as_bitmap().unwrap();
                    let bitmap_ref = bitmap_member.image_ref;
                    if let Some(bitmap) = player.bitmap_manager.get_bitmap_mut(bitmap_ref) {
                        bitmap.use_alpha = use_alpha;
                    }
                    Ok(())
                })
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set castMember prop {} for bitmap",
                prop
            ))),
        }
    }
}
