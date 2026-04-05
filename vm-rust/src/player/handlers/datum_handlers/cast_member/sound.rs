use crate::{
    director::lingo::datum::Datum,
    player::{cast_lib::CastMemberRef, reserve_player_mut, DirPlayer, ScriptError},
};

pub struct SoundMemberHandlers {}

impl SoundMemberHandlers {
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
        let sound = member.member_type.as_sound().unwrap();

        match prop {
            "duration" => Ok(Datum::Int(sound.info.duration as i32)),
            "sampleRate" => Ok(Datum::Int(sound.info.sample_rate as i32)),
            "sampleSize" => Ok(Datum::Int(sound.info.sample_size as i32)),
            "channelCount" => Ok(Datum::Int(sound.info.channels as i32)),
            "sampleCount" => Ok(Datum::Int(sound.info.sample_count as i32)),
            "loop" => Ok(Datum::Int(if sound.info.loop_enabled { 1 } else { 0 })),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for sound",
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
            "loop" => {
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
