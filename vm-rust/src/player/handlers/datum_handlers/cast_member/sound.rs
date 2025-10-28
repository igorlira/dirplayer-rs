use crate::{
    director::lingo::datum::Datum,
    player::{cast_lib::CastMemberRef, DirPlayer, ScriptError},
};

pub struct SoundMemberHandlers {}

impl SoundMemberHandlers {
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
        let sound = member.member_type.as_sound().unwrap();

        match prop.as_str() {
            "duration" => Ok(Datum::Int(sound.info.duration as i32)),
            "sampleRate" => Ok(Datum::Int(sound.info.sample_rate as i32)),
            "sampleSize" => Ok(Datum::Int(sound.info.sample_size as i32)),
            "channelCount" => Ok(Datum::Int(sound.info.channels as i32)),
            "sampleCount" => Ok(Datum::Int(sound.info.sample_count as i32)),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for sound",
                prop
            ))),
        }
    }
}
