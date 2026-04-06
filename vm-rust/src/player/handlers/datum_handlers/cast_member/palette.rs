use crate::{director::lingo::datum::Datum, player::{DirPlayer, ScriptError, cast_lib::CastMemberRef, cast_member::{CastMemberType, Media}}};


pub struct PaletteMemberHandlers;

impl PaletteMemberHandlers {
    pub fn get_prop(player: &mut DirPlayer, member_ref: &CastMemberRef, prop_name: &str) -> Result<Datum, ScriptError> {
        match prop_name {
            "media" => {
                let palette_member = player.movie.cast_manager.find_member_by_ref(member_ref).unwrap();
                let palette = match &palette_member.member_type {
                    CastMemberType::Palette(palette) => palette.clone(),
                    _ => return Err(ScriptError::new(format!("Member with ref {:?} is not a palette", member_ref))),
                };
                Ok(Datum::Media(Media::Palette(palette)))
            }
            _ => Err(ScriptError::new(format!("Cannot get property '{}' for palette member", prop_name))),
        }
    }

    pub fn set_prop(_player: &mut DirPlayer, member_ref: &CastMemberRef, prop_name: &str, value: Datum) -> Result<(), ScriptError> {
        match prop_name {
            "media" => {
                let palette_member = _player.movie.cast_manager.find_mut_member_by_ref(member_ref).unwrap();
                match &mut palette_member.member_type {
                    CastMemberType::Palette(palette) => {
                        if let Datum::Media(Media::Palette(new_palette)) = value {
                            *palette = new_palette;
                        } else {
                            return Err(ScriptError::new("Value for 'media' property of a palette member must be a palette media".to_string()));
                        }
                    }
                    _ => return Err(ScriptError::new(format!("Member with ref {:?} is not a palette", member_ref))),
                };
                Ok(())
            }
            _ => Err(ScriptError::new(format!("Cannot set property '{}' for palette member", prop_name))),
        }
    }
}