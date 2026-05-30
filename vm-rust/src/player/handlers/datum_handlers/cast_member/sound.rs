use crate::{
    director::lingo::datum::Datum,
    player::{DirPlayer, ScriptError, cast_lib::CastMemberRef, cast_member::Media, reserve_player_mut, symbols::{builtin::BuiltInSymbol, symbol::Symbol}},
};

pub struct SoundMemberHandlers {}

impl SoundMemberHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: Symbol,
    ) -> Result<Datum, ScriptError> {
        let member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .unwrap();
        let sound = member.member_type.as_sound().unwrap();

        match prop.into_builtin_or_error()? {
            // `the media of member` is a read/write opaque media blob
            // (Director 11.5 Scripting Dictionary, `media`). The documented
            // idiom is `member(dst).media = member(src).media` to copy content
            // between members — Habbo's Dynamic Downloader uses it to clone a
            // downloaded sound into a bin member.
            BuiltInSymbol::Media => Ok(Datum::Media(Media::Sound(sound.clone()))),
            BuiltInSymbol::Duration => Ok(Datum::Int(sound.info.duration as i32)),
            BuiltInSymbol::SampleRate => Ok(Datum::Int(sound.info.sample_rate as i32)),
            BuiltInSymbol::SampleSize => Ok(Datum::Int(sound.info.sample_size as i32)),
            BuiltInSymbol::ChannelCount => Ok(Datum::Int(sound.info.channels as i32)),
            BuiltInSymbol::SampleCount => Ok(Datum::Int(sound.info.sample_count as i32)),
            BuiltInSymbol::Loop => Ok(Datum::Int(if sound.info.loop_enabled { 1 } else { 0 })),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for sound",
                prop
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: Symbol,
        value: Datum,
    ) -> Result<(), ScriptError> {
        match prop.into_builtin_or_error()? {
            BuiltInSymbol::Media => {
                let media = value.media_value()?;
                let new_sound = match media {
                    Media::Sound(s) => s,
                    _ => return Err(ScriptError::new("Expected a sound media".to_string())),
                };
                reserve_player_mut(|player| {
                    let member = player
                        .movie
                        .cast_manager
                        .find_mut_member_by_ref(member_ref)
                        .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                    let sound = member.member_type.as_sound_mut()
                        .ok_or_else(|| ScriptError::new("Cast member is not a sound".to_string()))?;
                    *sound = new_sound;
                    Ok(())
                })
            }
            BuiltInSymbol::Loop => {
                let loop_enabled = value.bool_value()?;
                reserve_player_mut(|player| {
                    let member = player
                        .movie
                        .cast_manager
                        .find_mut_member_by_ref(member_ref)
                        .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                    let sound = member.member_type.as_sound_mut()
                        .ok_or_else(|| ScriptError::new("Cast member is not a sound".to_string()))?;
                    sound.info.loop_enabled = loop_enabled;
                    Ok(())
                })
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set castMember prop {} for sound",
                prop
            ))),
        }
    }
}
