use crate::{
    director::lingo::datum::{datum_bool, Datum, DatumType},
    player::{
        cast_lib::CastMemberRef,
        cast_member::CastMemberType,
        reserve_player_mut, DirPlayer, ScriptError,
    },
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

        // Properties shared by both #sound and #swa members
        match prop {
            "type" if sound.info.is_swa => Ok(Datum::Symbol("swa".to_string())),
            "duration" => Ok(Datum::Int(sound.info.duration as i32)),
            "sampleRate" => Ok(Datum::Int(sound.info.sample_rate as i32)),
            "sampleSize" | "bitsPerSample" => Ok(Datum::Int(sound.info.sample_size as i32)),
            "channelCount" | "numChannels" => Ok(Datum::Int(sound.info.channels as i32)),
            "sampleCount" => Ok(Datum::Int(sound.info.sample_count as i32)),
            "loop" => Ok(datum_bool(sound.info.loop_enabled)),
            "fileName" => Ok(Datum::String(
                sound.info.linked_path.clone().unwrap_or_default()
            )),
            "cuePointNames" => {
                let names: Vec<Datum> = sound.info.cue_point_names.iter()
                    .map(|n| Datum::String(n.clone()))
                    .collect();
                Ok(Datum::List(DatumType::List, names.into_iter()
                    .map(|d| player.alloc_datum(d))
                    .collect(), false))
            }
            "cuePointTimes" => {
                let times: Vec<Datum> = sound.info.cue_point_times.iter()
                    .map(|&t| Datum::Int(t as i32))
                    .collect();
                Ok(Datum::List(DatumType::List, times.into_iter()
                    .map(|d| player.alloc_datum(d))
                    .collect(), false))
            }
            // SWA-only properties
            "bitRate" if sound.info.is_swa => Ok(Datum::Int(sound.info.bit_rate.unwrap_or(0) as i32)),
            "soundChannel" if sound.info.is_swa => Ok(Datum::Int(sound.info.swa_sound_channel)),
            "volume" if sound.info.is_swa => Ok(Datum::Int(sound.info.swa_volume as i32)),
            "preLoadTime" if sound.info.is_swa => Ok(Datum::Int(sound.info.preload_time as i32)),
            "state" if sound.info.is_swa => Ok(Datum::Int(sound.info.swa_state as i32)),
            "percentPlayed" if sound.info.is_swa => {
                let duration = sound.info.duration as f64;
                if duration > 0.0 {
                    for i in 0..player.sound_manager.num_channels() {
                        if let Some(ch_rc) = player.sound_manager.get_channel(i) {
                            let ch = ch_rc.borrow();
                            if ch.status == crate::player::handlers::datum_handlers::sound_channel::SoundStatus::Playing {
                                if let Some(ref ch_member) = ch.sound_member {
                                    if ch_member.info.is_swa == sound.info.is_swa
                                        && ch_member.info.sample_rate == sound.info.sample_rate
                                        && ch_member.info.linked_path == sound.info.linked_path
                                    {
                                        let percent = ((ch.elapsed_time * 1000.0 / duration) * 100.0)
                                            .clamp(0.0, 100.0) as i32;
                                        return Ok(Datum::Int(percent));
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Datum::Int(0))
            }
            "percentStreamed" if sound.info.is_swa => Ok(Datum::Int(sound.info.percent_streamed as i32)),
            "URL" | "streamName" if sound.info.is_swa => Ok(Datum::String(
                sound.info.linked_path.clone().unwrap_or_default()
            )),
            "copyrightInfo" if sound.info.is_swa => Ok(Datum::String(
                sound.info.copyright_info.clone().unwrap_or_default()
            )),
            _ => Err(ScriptError::new(format!(
                "Cannot get castMember property {} for {}",
                prop, if sound.info.is_swa { "SWA" } else { "sound" }
            ))),
        }
    }

    pub fn set_prop(
        member_ref: &CastMemberRef,
        prop: &str,
        value: Datum,
    ) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            let member = player
                .movie
                .cast_manager
                .find_mut_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;

            if let CastMemberType::Sound(ref mut sound) = member.member_type {
                match prop {
                    "loop" => {
                        sound.info.loop_enabled = value.to_bool()?;
                        Ok(())
                    }
                    "volume" if sound.info.is_swa => {
                        sound.info.swa_volume = value.int_value()?.clamp(0, 255) as u8;
                        Ok(())
                    }
                    "preLoadTime" if sound.info.is_swa => {
                        sound.info.preload_time = value.int_value()? as u32;
                        Ok(())
                    }
                    "soundChannel" if sound.info.is_swa => {
                        let ch = value.int_value()?;
                        if ch != 0 {
                            log::warn!("SWA soundChannel auto-assignment not yet implemented (set to {})", ch);
                        }
                        sound.info.swa_sound_channel = ch;
                        Ok(())
                    }
                    "URL" | "streamName" if sound.info.is_swa => {
                        sound.info.linked_path = Some(value.string_value()?);
                        Ok(())
                    }
                    _ => Err(ScriptError::new(format!(
                        "Cannot set castMember property {} for {}",
                        prop,
                        if sound.info.is_swa { "SWA" } else { "sound" }
                    ))),
                }
            } else {
                Err(ScriptError::new("Cast member is not a sound".to_string()))
            }
        })
    }
}
