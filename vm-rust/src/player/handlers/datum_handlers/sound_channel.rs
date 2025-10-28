use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{DatumRef, DirPlayer, ScriptError},
};

use std::sync::Arc;
use wasm_bindgen::JsCast;
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, Blob, GainNode, HtmlAudioElement,
    OfflineAudioContext, StereoPannerNode, Url,
};

use crate::player::cast_member::CastMemberType;
use std::convert::TryInto;

use wasm_bindgen::prelude::*;

use wasm_bindgen::JsValue;
use web_sys::console;

use crate::player::cast_member::SoundMember;
use binary_reader::BinaryReader;
use binary_reader::Endian;

use js_sys::Reflect;

use wasm_bindgen_futures::JsFuture;

use std::cell::RefCell;
use std::cell::RefMut;
use std::rc::Rc;

use js_sys::Uint8Array;

use wasm_bindgen_futures::spawn_local;

// Standard IMA ADPCM tables
const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32773,
];

const INDEX_TABLE: [i32; 16] = [
    -1, -1, -1, -1, 2, 4, 6, 8, // For the lower nibble
    -1, -1, -1, -1, 2, 4, 6,
    8, // (The full nibble 0-15 is used to map, but the first 8 and last 8 are often symmetrical)
];

#[derive(Debug, Clone)]
pub struct SoundSegment {
    pub member_ref: DatumRef,
    pub loop_count: i32,
    pub loops_remaining: i32,
    // Note: The Lingo VM likely handles converting the Score's "#beat" value
    // into the sequence of members, so we only need to track the current member and its loop info.
}

pub struct SoundChannelDatumHandlers {}

impl SoundChannelDatumHandlers {
    pub fn get_proplist_prop(
        player: &DirPlayer,
        prop_list: &Datum,
        key_name: &str,
    ) -> Option<Datum> {
        if let Datum::PropList(ref props, _) = prop_list {
            for (key_ref, value_ref) in props {
                let key = player.get_datum(key_ref);
                if let Ok(sym) = key.symbol_value() {
                    if sym == key_name {
                        return Some(player.get_datum(value_ref).clone());
                    }
                }
            }
        }
        None
    }

    pub fn play_segment_for_member(
        player: &mut DirPlayer,
        datum: &DatumRef,
        member_ref: DatumRef,
        loop_count: i32,
    ) -> Result<(), ScriptError> {
        let channel_idx = Self::get_channel_index(datum, player)?;

        let sound_member = {
            let member_datum = player.get_datum(&member_ref);
            if let Datum::CastMember(ref cast_member_ref) = member_datum {
                if let Some(cast_member) = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(cast_member_ref)
                {
                    if let CastMemberType::Sound(sound_member) = &cast_member.member_type {
                        Some(sound_member.clone())
                    } else {
                        return Err(ScriptError::new("Member is not a sound".to_string()));
                    }
                } else {
                    return Err(ScriptError::new("Cast member not found".to_string()));
                }
            } else {
                return Err(ScriptError::new("Expected CastMember datum".to_string()));
            }
        };

        let sound_member =
            sound_member.ok_or_else(|| ScriptError::new("Invalid sound member".to_string()))?;

        let channel_rc = player
            .sound_manager
            .get_channel_mut(channel_idx)
            .ok_or_else(|| {
                ScriptError::new(format!("Invalid sound channel {}", channel_idx + 1))
            })?;

        // Mutably borrow the channel
        {
            let mut channel = channel_rc.borrow_mut();
            channel.playlist.clear();
            channel.current_segment_index = None;
            channel.stop_playback_nodes();
            channel.sound_member = Some(sound_member.clone());
            channel.loop_count = loop_count;
            channel.loops_remaining = loop_count;

            // Now call the method that requires &mut self
            channel.start_segment_playback(&sound_member, loop_count)?;
        }

        Ok(())
    }

    pub fn call(
        player: &mut DirPlayer,
        datum: &DatumRef,
        handler_name: &String,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let handler_name_lower = handler_name.to_lowercase();
        match handler_name_lower.as_str() {
            "play" => {
                if args.is_empty() {
                    // play() with no args - play current playlist
                    Self::handle_play(player, datum)?;
                } else {
                    // play(member) - play a specific member directly
                    Self::handle_play_member(player, datum, &args[0])?;
                }
                Ok(datum.clone())
            }
            "playfile" => {
                if args.is_empty() {
                    return Err(ScriptError::new(
                        "playFile requires a member argument".to_string(),
                    ));
                }
                Self::handle_play_file(player, datum, &args[0])?;
                Ok(datum.clone())
            }
            "playnext" => {
                Self::handle_play_next(player, datum)?;
                Ok(datum.clone())
            }
            "stop" => {
                Self::handle_stop(player, datum)?;
                Ok(datum.clone())
            }
            "pause" => {
                Self::handle_pause(player, datum)?;
                Ok(datum.clone())
            }
            "rewind" => {
                Self::handle_rewind(player, datum)?;
                Ok(datum.clone())
            }
            "queue" => {
                if args.is_empty() {
                    return Err(ScriptError::new(
                        "queue requires a member argument".to_string(),
                    ));
                }
                Self::handle_queue(player, datum, &args[0])?;
                Ok(datum.clone())
            }
            "breakloop" => {
                Self::handle_break_loop(player, datum)?;
                Ok(datum.clone())
            }
            "fadein" => {
                let ticks = if args.is_empty() {
                    60
                } else {
                    player.get_datum(&args[0]).int_value()?
                };
                let to_volume = if args.len() > 1 {
                    player.get_datum(&args[1]).float_value()?
                } else {
                    255.0
                };
                Self::handle_fade_in(player, datum, ticks, to_volume)?;
                Ok(datum.clone())
            }
            "fadeout" => {
                let ticks = if args.is_empty() {
                    60
                } else {
                    player.get_datum(&args[0]).int_value()?
                };
                Self::handle_fade_out(player, datum, ticks)?;
                Ok(datum.clone())
            }
            "fadeto" => {
                if args.len() < 2 {
                    return Err(ScriptError::new(
                        "fadeTo requires ticks and volume arguments".to_string(),
                    ));
                }
                let ticks = player.get_datum(&args[0]).int_value()?;
                let to_volume = player.get_datum(&args[1]).float_value()?;
                Self::handle_fade_to(player, datum, ticks, to_volume)?;
                Ok(datum.clone())
            }
            "setplaylist" => {
                if args.is_empty() {
                    return Err(ScriptError::new(
                        "setPlayList requires a list argument".to_string(),
                    ));
                }
                Self::handle_set_playlist(player, datum, &args[0])?;
                Ok(datum.clone())
            }
            "getplaylist" => Self::handle_get_playlist(player, datum),
            "isbusy" => {
                let is_busy = Self::handle_is_busy(player, datum)?;
                Ok(player.alloc_datum(Datum::Int(if is_busy { 1 } else { 0 })))
            }
            _ => Err(ScriptError::new(format!(
                "No handler {handler_name} for sound channel"
            ))),
        }
    }

    pub fn get_prop(
        player: &DirPlayer,
        datum: &DatumRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        // Get the Rc<RefCell<SoundChannel>>
        let channel_rc = Self::get_sound_channel(player, datum)?;

        // Borrow the inner SoundChannel
        let channel = channel_rc.borrow();

        match prop.as_str() {
            "volume" => Ok(Datum::Float(channel.volume as f32)),
            "duration" => Ok(Datum::Float(channel.get_duration() as f32)),
            "pan" => Ok(Datum::Float(channel.pan as f32)),
            "loopCount" => Ok(Datum::Int(channel.loop_count)),
            "loopsRemaining" => Ok(Datum::Int(channel.loops_remaining)),
            "startTime" => Ok(Datum::Float(channel.start_time as f32)),
            "endTime" => Ok(Datum::Float(channel.end_time as f32)),
            "loopStartTime" => Ok(Datum::Float(channel.loop_start_time as f32)),
            "loopEndTime" => Ok(Datum::Float(channel.loop_end_time as f32)),
            "elapsedTime" => Ok(Datum::Float(channel.elapsed_time as f32)),
            "sampleRate" => Ok(Datum::Int(channel.sample_rate.try_into().unwrap())),
            "sampleCount" => Ok(Datum::Int(channel.sample_count.try_into().unwrap())),
            "channelCount" => Ok(Datum::Int(channel.channel_count.into())),
            "status" => Ok(Datum::Int(channel.status.clone() as i32)),
            _ => Err(ScriptError::new(format!(
                "Cannot get property {} for sound channel",
                prop
            ))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &String,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        match prop.as_str() {
            "volume" => {
                let vol = player.get_datum(value_ref).float_value()?;
                Self::set_sound_volume(player, datum, vol)?;
                Ok(())
            }
            "pan" => {
                let pan = player.get_datum(value_ref).float_value()?;
                Self::set_sound_pan(player, datum, pan)?;
                Ok(())
            }
            "loopCount" => {
                let count = player.get_datum(value_ref).int_value()?;
                Self::set_loop_count(player, datum, count)?;
                Ok(())
            }
            "startTime" => {
                let time = player.get_datum(value_ref).float_value()?;
                Self::set_start_time(player, datum, time)?;
                Ok(())
            }
            "endTime" => {
                let time = player.get_datum(value_ref).float_value()?;
                Self::set_end_time(player, datum, time)?;
                Ok(())
            }
            "loopStartTime" => {
                let time = player.get_datum(value_ref).float_value()?;
                Self::set_loop_start_time(player, datum, time)?;
                Ok(())
            }
            "loopEndTime" => {
                let time = player.get_datum(value_ref).float_value()?;
                Self::set_loop_end_time(player, datum, time)?;
                Ok(())
            }
            _ => Err(ScriptError::new(format!(
                "Cannot set property {} for sound channel",
                prop
            ))),
        }
    }

    fn get_channel_index(datum: &DatumRef, player: &DirPlayer) -> Result<usize, ScriptError> {
        let datum_val = player.get_datum(datum);
        match datum_val {
            Datum::SoundChannel(channel_num) => {
                // Director uses 1-based indexing, convert to 0-based
                if *channel_num == 0 {
                    return Err(ScriptError::new(
                        "Sound channel index must be >= 1".to_string(),
                    ));
                }
                Ok((*channel_num - 1) as usize)
            }
            _ => Err(ScriptError::new(
                "Expected sound channel reference".to_string(),
            )),
        }
    }

    fn get_sound_channel(
        player: &DirPlayer,
        datum: &DatumRef,
    ) -> Result<Rc<RefCell<SoundChannel>>, ScriptError> {
        let channel_idx = Self::get_channel_index(datum, player)?;
        player
            .sound_manager
            .get_channel(channel_idx)
            .ok_or_else(|| ScriptError::new(format!("Invalid sound channel {}", channel_idx + 1)))
    }

    fn get_sound_channel_mut(
        player: &mut DirPlayer,
        datum: &DatumRef,
    ) -> Result<Rc<RefCell<SoundChannel>>, ScriptError> {
        let channel_idx = Self::get_channel_index(datum, player)?;
        player
            .sound_manager
            .get_channel_mut(channel_idx)
            .ok_or_else(|| ScriptError::new(format!("Invalid sound channel {}", channel_idx + 1)))
    }

    fn handle_play_member(
        player: &mut DirPlayer,
        datum: &DatumRef,
        member_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        console::log_1(&format!("🎵 handle_play_member() - Playing member directly").into());

        let channel_rc = Self::get_sound_channel_mut(player, datum)?;

        // Clear any playlist and play this member directly
        {
            let mut ch = channel_rc.borrow_mut();
            ch.playlist_segments.clear();
            ch.playlist.clear();
            ch.current_segment_index = None;
            ch.stop_playback_nodes();
        }

        // Use play_file to start playback
        SoundChannel::play_file(channel_rc, member_ref.clone());

        Ok(())
    }

    fn handle_play(player: &mut DirPlayer, datum: &DatumRef) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        let mut ch = channel.borrow_mut();

        console::log_1(
            &format!(
                "🎬 handle_play() - Channel {} has {} items in playlist",
                ch.channel_num,
                ch.playlist_segments.len()
            )
            .into(),
        );

        ch.stop_playback_nodes();

        if !ch.playlist_segments.is_empty() {
            ch.current_segment_index = Some(0);
            ch.status = SoundStatus::Playing;

            console::log_1(&"▶️ Starting async playlist playback".into());

            // Drop the borrow before spawning
            let channel_rc = channel.clone();
            drop(ch);

            // Spawn async task that doesn't block
            spawn_local(async move {
                SoundChannel::play_current_segment_async(channel_rc).await;
            });
        } else {
            console::log_1(&"⚠️ No playlist queued".into());
            ch.status = SoundStatus::Stopped;
        }

        Ok(())
    }

    pub fn handle_play_file(
        player: &mut DirPlayer,
        datum: &DatumRef,
        member: &DatumRef,
    ) -> Result<(), ScriptError> {
        // Get the channel as Rc<RefCell<SoundChannel>> (do NOT borrow)
        let channel_rc = Self::get_sound_channel_mut(player, datum)?;

        // Call the associated function with the Rc
        SoundChannel::play_file(Rc::clone(&channel_rc), member.clone());

        Ok(())
    }

    fn handle_playfile(
        player: &mut DirPlayer,
        datum: &DatumRef,
        member_ref: &DatumRef,
    ) -> Result<DatumRef, ScriptError> {
        // Don't get the channel at all - just call play_member directly
        // which will get the channel internally
        Self::handle_play_file(player, datum, member_ref)?;
        Ok(DatumRef::Void)
    }

    fn handle_play_next(player: &mut DirPlayer, datum: &DatumRef) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().play_next();
        Ok(())
    }

    pub fn handle_stop(player: &mut DirPlayer, datum: &DatumRef) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().stop();
        Ok(())
    }

    fn handle_pause(player: &mut DirPlayer, datum: &DatumRef) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().pause();
        Ok(())
    }

    fn handle_rewind(player: &mut DirPlayer, datum: &DatumRef) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().rewind();
        Ok(())
    }

    fn handle_queue(
        player: &mut DirPlayer,
        datum: &DatumRef,
        member: &DatumRef,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().queue(member.clone(), player); // <-- pass player here
        Ok(())
    }

    fn handle_break_loop(player: &mut DirPlayer, datum: &DatumRef) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().break_loop();
        Ok(())
    }

    fn handle_fade_in(
        player: &mut DirPlayer,
        datum: &DatumRef,
        ticks: i32,
        to_volume: f32,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().fade_in(ticks, to_volume);
        Ok(())
    }

    fn handle_fade_out(
        player: &mut DirPlayer,
        datum: &DatumRef,
        ticks: i32,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().fade_out(ticks);
        Ok(())
    }

    fn handle_fade_to(
        player: &mut DirPlayer,
        datum: &DatumRef,
        ticks: i32,
        to_volume: f32,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().fade_to(ticks, to_volume);
        Ok(())
    }

    fn handle_set_playlist(
        player: &mut DirPlayer,
        datum: &DatumRef,
        list_ref: &DatumRef,
    ) -> Result<DatumRef, ScriptError> {
        use web_sys::console;

        // Convert Lingo list
        let lingo_list = player.get_datum(list_ref).to_list()?.clone();

        // --- 🧹 If the playlist is empty → clear both and exit early
        if lingo_list.is_empty() {
            let channel_rc = Self::get_sound_channel_mut(player, datum)?;
            let mut channel = channel_rc.borrow_mut();
            channel.playlist_segments.clear();
            channel.playlist.clear();
            channel.current_segment_index = None;
            channel.queued_member = None;
            return Ok(DatumRef::Void);
        }

        // --- Otherwise, build the playlist normally ---
        let mut segments: Vec<SoundSegment> = Vec::new();
        let mut playlist: Vec<DatumRef> = Vec::new();

        for (idx, segment_ref) in lingo_list.iter().enumerate() {
            let segment_datum = player.get_datum(segment_ref).clone();

            if let Datum::PropList(props, _) = segment_datum {
                let mut member_value: Option<Datum> = None;
                let mut loopcount_value: Option<i32> = None;

                for (key_ref, value_ref) in props {
                    let key = player.get_datum(&key_ref);
                    if let Ok(sym) = key.symbol_value() {
                        let value = player.get_datum(&value_ref).clone();

                        match sym.to_lowercase().as_str() {
                            "member" | "#member" => {
                                member_value = Some(value.clone());
                            }
                            "loopcount" | "#loopcount" => {
                                if let Datum::Int(n) = value {
                                    loopcount_value = Some(n);
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Validate and push
                match (member_value, loopcount_value) {
                    // ✅ valid member and positive loopCount
                    (Some(member_val), Some(loop_count)) if loop_count > 0 => {
                        let member_ref = player.alloc_datum(member_val);
                        segments.push(SoundSegment {
                            member_ref: member_ref.clone(),
                            loop_count,
                            loops_remaining: loop_count,
                        });
                        playlist.push(segment_ref.clone());
                    }

                    // ⚠️ loopCount == 0
                    (Some(_), Some(0)) => {
                        console::log_1(&"  ⚠️ Skipped: loopCount is 0 (nothing to play)".into());
                    }

                    // ⚠️ loopCount negative (invalid)
                    (Some(_), Some(loop_count)) if loop_count < 0 => {
                        console::log_1(
                            &format!("  ⚠️ Skipped: invalid negative loopCount ({})", loop_count)
                                .into(),
                        );
                    }

                    // ⚠️ missing loopCount
                    (Some(_), None) => {
                        console::log_1(&"  ⚠️ Skipped: missing loopCount property".into());
                    }

                    // ⚠️ missing member entirely
                    (None, _) => {
                        console::log_1(&"  ⚠️ Skipped: missing member property".into());
                    }

                    // 🧩 fallback (compiler exhaustiveness guard)
                    _ => {
                        console::log_1(
                            &"  ⚠️ Unexpected combination of properties — skipped".into(),
                        );
                    }
                }
            } else {
                console::log_1(&format!("  ⚠️ Skipped: item is not a PropList").into());
            }
        }

        // --- ✅ Save results to the channel
        let channel_rc = Self::get_sound_channel_mut(player, datum)?;
        let mut channel = channel_rc.borrow_mut();
        channel.playlist_segments = segments;
        channel.playlist = playlist;

        channel.current_segment_index = None;
        if !channel.playlist_segments.is_empty() {
            channel.current_segment_index = Some(0);
            channel.queued_member = Some(channel.playlist_segments[0].member_ref.clone());
        }

        console::log_1(
            &format!("✅ Built {} valid playlist entries", channel.playlist.len()).into(),
        );
        Ok(DatumRef::Void)
    }

    fn handle_get_playlist(
        player: &mut DirPlayer,
        datum: &DatumRef,
    ) -> Result<DatumRef, ScriptError> {
        // 1️⃣ Get the channel instance
        let channel_rc = Self::get_sound_channel(player, datum)?;
        let channel = channel_rc.borrow();

        // 2️⃣ Get this channel's playlist
        let playlist = channel.get_playlist();

        // 3️⃣ Convert the Vec<DatumRef> into a Datum::List
        Ok(player.alloc_datum(Datum::List(
            DatumType::List, // or appropriate type
            playlist,
            false, // sorted = false
        )))
    }

    fn handle_is_busy(player: &DirPlayer, datum: &DatumRef) -> Result<bool, ScriptError> {
        let channel_rc = Self::get_sound_channel(player, datum)?;
        let channel = channel_rc.borrow(); // immutable borrow
        Ok(channel.is_busy())
    }

    fn set_sound_volume(
        player: &mut DirPlayer,
        datum: &DatumRef,
        vol: f32,
    ) -> Result<(), ScriptError> {
        // get the Rc<RefCell<SoundChannel>>
        let channel_rc = Self::get_sound_channel(player, datum)?;
        // borrow mutably to access fields and methods
        let mut channel = channel_rc.borrow_mut();
        channel.set_volume(vol);
        Ok(())
    }

    fn set_sound_pan(
        player: &mut DirPlayer,
        datum: &DatumRef,
        pan: f32,
    ) -> Result<(), ScriptError> {
        let channel_rc = Self::get_sound_channel(player, datum)?;
        let mut channel = channel_rc.borrow_mut();
        channel.set_pan(pan);
        Ok(())
    }

    fn set_loop_count(
        player: &mut DirPlayer,
        datum: &DatumRef,
        count: i32,
    ) -> Result<(), ScriptError> {
        let channel_rc = Self::get_sound_channel(player, datum)?;
        let mut channel = channel_rc.borrow_mut();
        channel.set_loop_count(count);
        Ok(())
    }

    fn set_start_time(
        player: &mut DirPlayer,
        datum: &DatumRef,
        time: f32,
    ) -> Result<(), ScriptError> {
        let channel_rc = Self::get_sound_channel(player, datum)?;
        let mut channel = channel_rc.borrow_mut();
        channel.start_time = time.max(0.0);
        Ok(())
    }

    fn set_end_time(
        player: &mut DirPlayer,
        datum: &DatumRef,
        time: f32,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        let mut ch = channel.borrow_mut();
        ch.end_time = time;
        if ch.end_time == 0.0 {
            ch.end_time = ch.get_duration();
        }
        Ok(())
    }

    fn set_loop_start_time(
        player: &mut DirPlayer,
        datum: &DatumRef,
        time: f32,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().loop_start_time = time.max(0.0);
        Ok(())
    }

    fn set_loop_end_time(
        player: &mut DirPlayer,
        datum: &DatumRef,
        time: f32,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().loop_end_time = time;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SoundStatus {
    Stopped = 0,
    Playing = 1,
    Paused = 2,
}

#[derive(Clone, Debug)]
pub struct AudioData {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub num_channels: u16,
    pub compressed_data: Option<Vec<u8>>, // for MP3 etc.
}

//#[wasm_bindgen]
impl AudioData {
    // Add a getter that returns a JavaScript `Float32Array` from the Rust `Vec<f32>`
    //#[wasm_bindgen(getter)]
    pub fn samples(&self) -> js_sys::Float32Array {
        js_sys::Float32Array::from(&self.samples[..])
    }

    pub fn from_wav_bytes(bytes: &[u8]) -> Result<AudioData, String> {
        fn read_u16_le(b: &[u8], i: usize) -> Result<u16, String> {
            if i + 2 <= b.len() {
                Ok(u16::from_le_bytes([b[i], b[i + 1]]))
            } else {
                Err("Unexpected EOF reading u16".into())
            }
        }

        fn read_u32_le(b: &[u8], i: usize) -> Result<u32, String> {
            if i + 4 <= b.len() {
                Ok(u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]))
            } else {
                Err("Unexpected EOF reading u32".into())
            }
        }

        if bytes.len() < 12 {
            return Err("WAV data too small".into());
        }
        if &bytes[0..4] != b"RIFF" {
            return Err("Missing 'RIFF'".into());
        }
        if &bytes[8..12] != b"WAVE" {
            return Err("Missing 'WAVE'".into());
        }

        let mut pos = 12usize;
        let mut audio_format: Option<u16> = None;
        let mut num_channels: Option<u16> = None;
        let mut sample_rate: Option<u32> = None;
        let mut bits_per_sample: Option<u16> = None;
        let mut data_start: Option<usize> = None;
        let mut data_len: Option<usize> = None;

        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = read_u32_le(bytes, pos + 4)? as usize;
            pos += 8;

            if pos + size > bytes.len() {
                return Err(format!(
                    "Chunk size out of bounds for {:?} (pos={}, size={}, total={})",
                    std::str::from_utf8(id).unwrap_or("<non-utf8>"),
                    pos,
                    size,
                    bytes.len()
                ));
            }

            match id {
                b"fmt " => {
                    if size < 16 {
                        return Err("fmt chunk too small".into());
                    }
                    audio_format = Some(read_u16_le(bytes, pos)?);
                    num_channels = Some(read_u16_le(bytes, pos + 2)?);
                    sample_rate = Some(read_u32_le(bytes, pos + 4)?);
                    bits_per_sample = Some(read_u16_le(bytes, pos + 14)?);
                }
                b"data" => {
                    data_start = Some(pos);
                    data_len = Some(size);
                    break;
                }
                _ => {}
            }

            pos += size;
            if size % 2 == 1 {
                pos += 1; // pad byte
            }
        }

        let audio_format = audio_format.ok_or("fmt chunk not found")?;
        let num_channels = num_channels.ok_or("fmt chunk not found")? as usize;
        let sample_rate = sample_rate.ok_or("fmt chunk not found")?;
        let bits_per_sample = bits_per_sample.ok_or("fmt chunk not found")?;
        let data_start = data_start.ok_or("data chunk not found")?;
        let data_len = data_len.ok_or("data chunk not found")?;

        let bytes_per_sample = (bits_per_sample / 8) as usize;
        let num_frames = data_len / (bytes_per_sample * num_channels);

        let mut samples: Vec<f32> = Vec::with_capacity(num_frames * num_channels);

        match (audio_format, bits_per_sample) {
            (1, 16) => {
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        let idx = data_start + (frame * num_channels + ch) * 2;
                        if idx + 1 >= bytes.len() {
                            break;
                        }
                        let raw = i16::from_le_bytes([bytes[idx], bytes[idx + 1]]);
                        let normalized = raw as f32 / 32768.0; // -1.0 .. 1.0
                        samples.push(normalized);
                    }
                }
            }
            (1, 8) => {
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        let idx = data_start + frame * num_channels + ch;
                        if idx >= bytes.len() {
                            break;
                        }
                        let raw = bytes[idx] as f32;
                        let normalized = (raw - 128.0) / 128.0; // -1.0 .. 1.0
                        samples.push(normalized);
                    }
                }
            }
            (3, 32) => {
                for frame in 0..num_frames {
                    for ch in 0..num_channels {
                        let idx = data_start + (frame * num_channels + ch) * 4;
                        if idx + 3 >= bytes.len() {
                            break;
                        }
                        let bits = u32::from_le_bytes([
                            bytes[idx],
                            bytes[idx + 1],
                            bytes[idx + 2],
                            bytes[idx + 3],
                        ]);
                        samples.push(f32::from_bits(bits));
                    }
                }
            }
            _ => {
                return Err(format!(
                    "Unsupported WAV format: audio_format={}, bits_per_sample={}",
                    audio_format, bits_per_sample
                ));
            }
        }

        Ok(AudioData {
            samples,
            sample_rate,
            num_channels: num_channels as u16,
            compressed_data: None, // WAV = uncompressed
        })
    }
}

#[derive(Clone)]
#[wasm_bindgen]
pub struct WebAudioBackend {
    context: AudioContext,
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window)]
    fn getAudioContext() -> AudioContext;
}

#[wasm_bindgen]
impl WebAudioBackend {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<Self, String> {
        let context = getAudioContext();

        console::log_1(&JsValue::from_str("🎵 AudioContext created"));

        Ok(Self { context })
    }

    pub fn resume_context(&self) -> Result<(), String> {
        // Resume context (required for autoplay policy)
        // The resume() call will handle suspended state internally
        console::log_1(&JsValue::from_str("▶️ Resuming AudioContext..."));
        self.context
            .resume()
            .map_err(|e| format!("Failed to resume context: {:?}", e))?;
        Ok(())
    }

    pub fn resume_sound(&mut self) {
        console::log_1(&JsValue::from_str(
            "▶️ resume_sound Resuming AudioContext...",
        ));
        let _ = self.context.resume();
    }
}

// Sound Manager - manages all sound channels
pub struct SoundManager {
    channels: Vec<Rc<RefCell<SoundChannel>>>,
    audio_context: Arc<AudioContext>,
}

impl SoundManager {
    pub fn new(num_channels: usize) -> Result<Self, ScriptError> {
        let context = Arc::new(getAudioContext());

        let mut channels = Vec::with_capacity(num_channels);
        for i in 0..num_channels {
            channels.push(Rc::new(RefCell::new(SoundChannel::new(
                i as i32,
                context.clone(),
            ))));
        }

        Ok(Self {
            channels,
            audio_context: context,
        })
    }

    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn get_channel(&self, channel: usize) -> Option<Rc<RefCell<SoundChannel>>> {
        self.channels.get(channel).cloned()
    }

    pub fn get_channel_mut(&mut self, channel: usize) -> Option<Rc<RefCell<SoundChannel>>> {
        self.channels.get(channel).cloned()
    }

    pub fn update(&mut self, delta_time: f32, player: &mut DirPlayer) -> Result<(), ScriptError> {
        for channel in &self.channels {
            // debug: log that update tick is processing this channel
            web_sys::console::log_1(
                &format!("[CH{}] update tick", channel.borrow().channel_num).into(),
            );

            channel.borrow_mut().update(delta_time, player)?;
        }
        Ok(())
    }

    pub fn stop_all(&mut self) {
        for channel in &self.channels {
            channel.borrow_mut().stop();
        }
    }

    pub fn audio_context(&self) -> Arc<AudioContext> {
        self.audio_context.clone()
    }
}

#[derive(Clone)]
pub struct SoundChannel {
    pub channel_num: i32,
    pub member: Option<DatumRef>,
    pub sound_member: Option<SoundMember>,

    // Playback state
    pub volume: f32,
    pub pan: f32,
    pub loop_count: i32,
    pub loops_remaining: i32,
    pub start_time: f32,
    pub end_time: f32,
    pub loop_start_time: f32,
    pub loop_end_time: f32,
    pub status: SoundStatus,

    // Audio properties
    pub sample_rate: u32,
    pub sample_count: u32,
    pub channel_count: u16,
    pub elapsed_time: f32,

    // Fade state
    pub is_fading: bool,
    pub fade_start_volume: f32,
    pub fade_target_volume: f32,
    pub fade_duration: f32,
    pub fade_elapsed: f32,

    // Playlist and playback queue
    pub playlist_segments: Vec<SoundSegment>,
    pub playlist: Vec<DatumRef>,
    pub current_segment_index: Option<usize>,

    // 2. Wrap audio nodes in Rc to make them cloneable
    pub source_node: Option<Rc<AudioBufferSourceNode>>,
    pub gain_node: Option<Rc<GainNode>>,
    pub pan_node: Option<Rc<StereoPannerNode>>,

    pub queued_member: Option<DatumRef>,

    // Web Audio backend
    pub audio_context: Arc<AudioContext>,
    pub current_audio_buffer: Option<Rc<AudioBuffer>>,

    pub expected_sample_rate: Option<u32>,
    pub is_decoding: Rc<RefCell<bool>>,
}

impl SoundChannel {
    /// Find the first valid MP3 frame sync and validate it's actually MP3
    fn find_mp3_start(data: &[u8]) -> Option<usize> {
        for i in 0..data.len().saturating_sub(4) {
            // Check for frame sync (11 bits set)
            if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 {
                // Validate this is a real MP3 frame header
                let header = u32::from_be_bytes([
                    data[i],
                    data[i + 1],
                    if i + 2 < data.len() { data[i + 2] } else { 0 },
                    if i + 3 < data.len() { data[i + 3] } else { 0 },
                ]);

                // Extract MPEG version (bits 19-20)
                let version = (header >> 19) & 0x3;
                // Extract layer (bits 17-18)
                let layer = (header >> 17) & 0x3;
                // Extract bitrate index (bits 12-15)
                let bitrate_index = (header >> 12) & 0xF;
                // Extract sample rate index (bits 10-11)
                let sample_rate_index = (header >> 10) & 0x3;

                // Validate: version != 1 (reserved), layer != 0 (reserved),
                // bitrate != 0xF (invalid), bitrate != 0 (free format),
                // sample_rate != 3 (reserved)
                if version != 1
                    && layer != 0
                    && bitrate_index != 0xF
                    && bitrate_index != 0
                    && sample_rate_index != 3
                {
                    console::log_1(&format!(
                        "✅ Valid MP3 frame found at offset {}: version={}, layer={}, bitrate_idx={}, sr_idx={}",
                        i, version, layer, bitrate_index, sample_rate_index
                    ).into());
                    return Some(i);
                }
            }
        }
        None
    }

    pub fn play_member_direct(
        &mut self,
        sound_member: SoundMember,
        loop_count: i32,
    ) -> Result<(), ScriptError> {
        // Clear playlist
        self.playlist.clear();
        self.current_segment_index = None;

        // Stop previous sound
        self.stop_playback_nodes();

        self.sound_member = Some(sound_member.clone());
        self.loop_count = loop_count;
        self.loops_remaining = loop_count;

        self.start_segment_playback(&sound_member, loop_count)?;
        self.status = SoundStatus::Playing;

        Ok(())
    }

    pub fn new(channel: i32, audio_context: Arc<AudioContext>) -> Self {
        Self {
            channel_num: channel,
            member: None,
            sound_member: None,
            volume: 255.0,
            pan: 0.0,
            loop_count: 0,
            loops_remaining: 0,
            start_time: 0.0,
            end_time: 0.0,
            loop_start_time: 0.0,
            loop_end_time: 0.0,
            status: SoundStatus::Stopped,
            sample_rate: 0,
            sample_count: 0,
            channel_count: 0,
            elapsed_time: 0.0,
            is_fading: false,
            fade_start_volume: 0.0,
            fade_target_volume: 0.0,
            fade_duration: 0.0,
            fade_elapsed: 0.0,
            playlist_segments: Vec::new(),
            playlist: Vec::new(),
            current_segment_index: None,
            source_node: None,
            gain_node: None,
            pan_node: None,
            queued_member: None,
            audio_context,
            current_audio_buffer: None,
            expected_sample_rate: None,
            is_decoding: Rc::new(RefCell::new(false)),
        }
    }

    fn get_proplist_prop(player: &DirPlayer, prop_list: &Datum, key_name: &str) -> Option<Datum> {
        if let Datum::PropList(ref props, _) = prop_list {
            for (key_ref, value_ref) in props {
                let key = player.get_datum(key_ref);
                if let Ok(sym) = key.symbol_value() {
                    if sym == key_name {
                        return Some(player.get_datum(value_ref).clone());
                    }
                }
            }
        }
        None
    }

    pub fn snd_to_wav(
        reader: &mut BinaryReader,
        channels: u16,
        sample_rate: u32,
        bits_per_sample: u16,
    ) -> Result<Vec<u8>, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        // Read 64-byte header (optional, keep for debugging)
        let mut header_bytes = vec![0u8; 64];
        for i in 0..64 {
            header_bytes[i] = reader.read_u8().map_err(|e| e.to_string())?;
        }
        console::log_1(&format!("SND header bytes: {:02X?}", &header_bytes).into());

        // Read audio bytes
        let mut data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            data.push(byte);
        }

        reader.endian = original_endian;

        let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
        let block_align = channels * bits_per_sample / 8;

        // Construct WAV header
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        let chunk_size = 36 + data.len() as u32;
        wav.extend_from_slice(&chunk_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt length
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());

        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);

        Ok(wav)
    }

    fn load_director_sound_as_wav(
        &mut self,
        sound_member: &SoundMember,
    ) -> Result<AudioData, String> {
        // Create a BinaryReader from the raw SND data
        let mut reader = BinaryReader::from_vec(&sound_member.sound.data());

        self.expected_sample_rate = Some(sound_member.info.sample_rate);

        // Convert SND chunk to WAV bytes
        let wav_bytes = Self::snd_to_wav(
            &mut reader,
            sound_member.info.channels,
            sound_member.info.sample_rate,
            sound_member.info.sample_size,
        )?;

        // Now decode WAV bytes into AudioData
        AudioData::from_wav_bytes(&wav_bytes)
    }

    /// Updated play_current_segment_async that uses per-channel decoding guard and
    /// avoids double-spawning overlapping decode/play tasks.
    async fn play_current_segment_async(channel_rc: Rc<RefCell<Self>>) {
        use web_sys::console;

        // quick checks and gather data while holding short borrow
        let (member_ref, audio_context, channel_num, is_decoding) = {
            let ch = channel_rc.borrow();
            let idx = match ch.current_segment_index {
                Some(i) => i,
                None => {
                    console::log_1(&"⚠️ No current segment to play".into());
                    return;
                }
            };
            let seg = match ch.playlist_segments.get(idx) {
                Some(s) => s.clone(),
                None => {
                    console::log_1(&"⚠️ Invalid segment index".into());
                    return;
                }
            };
            (
                seg.member_ref.clone(),
                ch.audio_context.clone(),
                ch.channel_num,
                ch.is_decoding.clone(),
            )
        };

        // If decode in-progress, skip
        if *is_decoding.borrow() {
            console::log_1(
                &format!(
                    "⏳ Channel {} is currently decoding — skipping play_current_segment_async",
                    channel_num
                )
                .into(),
            );
            return;
        }

        // Resolve the sound member and then call play_file which will use the guarded MP3 path if needed
        // We call play_file with Rc to ensure no borrow conflicts
        let rc_clone = channel_rc.clone();
        wasm_bindgen_futures::spawn_local(async move {
            web_sys::console::log_1(&"mp3 task started".into());
            // call into existing play_file entry which handles MP3 and PCM paths
            SoundChannel::play_file(rc_clone, member_ref);
        });
    }

    /// Primary entry point for playing a sound member.
    pub fn play(&mut self, member_ref: DatumRef, loop_count: i32) -> Result<(), ScriptError> {
        // Store the member and loop count
        self.member = Some(member_ref.clone());
        self.loop_count = loop_count;
        self.loops_remaining = loop_count;

        // Set playing status
        self.status = SoundStatus::Playing;
        self.elapsed_time = 0.0;

        Ok(())
    }

    async fn play_async(
        channel_rc: Rc<RefCell<Self>>,
        member_ref: DatumRef,
        loop_count: i32,
    ) -> Result<(), JsValue> {
        let player_opt = unsafe { crate::PLAYER_OPT.as_ref() };
        let player = player_opt.ok_or_else(|| JsValue::from_str("Player not initialized"))?;

        // FIX: audio_context is an Arc, so we dereference it to get a reference (&AudioContext)
        let audio_context = player.sound_manager.audio_context();

        // Load and create the buffer
        // FIX: Dereferencing Arc<AudioContext> to pass &AudioContext
        let (audio_buffer, _channels, _sample_rate) = channel_rc
            .borrow_mut()
            .load_audio_data(&*audio_context, &member_ref)
            .await?;

        let mut channel = channel_rc.borrow_mut();

        // Disconnect and stop any existing nodes
        if let Some(source) = channel.source_node.take() {
            let _ = source.stop();
            let _ = source.disconnect();
        }

        // 1. Create Source Node
        // FIX: Dereferencing Arc<AudioContext> to pass &BaseAudioContext
        let source_node = AudioBufferSourceNode::new(&*audio_context)
            .map_err(|e| JsValue::from_str(&format!("Failed to create source node: {:?}", e)))?;
        source_node.set_buffer(Some(&audio_buffer));
        source_node.set_loop(loop_count > 0);

        // 2. Create Gain Node (for volume)
        // FIX: Dereferencing Arc<AudioContext> to pass &BaseAudioContext
        let gain_node = GainNode::new(&*audio_context)
            .map_err(|e| JsValue::from_str(&format!("Failed to create gain node: {:?}", e)))?;

        // 3. Create Panner Node (for stereo effects)
        // FIX: Dereferencing Arc<AudioContext> to pass &BaseAudioContext
        let panner_node = StereoPannerNode::new(&*audio_context)
            .map_err(|e| JsValue::from_str(&format!("Failed to create panner node: {:?}", e)))?;

        // 4. Connect the chain: Source -> Gain -> Panner -> Destination
        source_node
            .connect_with_audio_node(&gain_node)
            .map_err(|e| {
                JsValue::from_str(&format!("Failed to connect source to gain: {:?}", e))
            })?;

        gain_node
            .connect_with_audio_node(&panner_node)
            .map_err(|e| {
                JsValue::from_str(&format!("Failed to connect gain to panner: {:?}", e))
            })?;

        // FIX: Dereferencing Arc<AudioContext> to get destination
        panner_node
            .connect_with_audio_node(&audio_context.destination())
            .map_err(|e| {
                JsValue::from_str(&format!("Failed to connect panner to destination: {:?}", e))
            })?;

        // 5. Store nodes and buffer
        channel.source_node = Some(Rc::new(source_node.clone()));
        channel.gain_node = Some(Rc::new(gain_node));
        channel.pan_node = Some(Rc::new(panner_node));
        channel.current_audio_buffer = Some(Rc::new(audio_buffer));

        // 6. Start Playback
        source_node
            .start()
            .map_err(|e| JsValue::from_str(&format!("Failed to start source node: {:?}", e)))?;

        Ok(())
    }

    fn resolve_sound_member(player: &DirPlayer, datum: &Datum) -> Option<SoundMember> {
        // Case 1: Direct CastMember reference
        if let Datum::CastMember(ref member_ref) = datum {
            let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref)?;
            match &cast_member.member_type {
                CastMemberType::Sound(sound_member) => return Some(sound_member.clone()),
                _ => {
                    console::log_1(&"⚠️ CastMember is not a sound".into());
                    return None;
                }
            }
        }

        // Case 2: PropList with #member property (for playlist items)
        let member_datum = Self::get_proplist_prop(player, datum, "member")?;

        match member_datum {
            Datum::CastMember(ref member_ref) => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref)?;
                match &cast_member.member_type {
                    CastMemberType::Sound(sound_member) => Some(sound_member.clone()),
                    _ => {
                        console::log_1(&"⚠️ CastMember is not a sound".into());
                        None
                    }
                }
            }
            other => {
                console::log_1(
                    &format!(
                        "⚠️ #member is not a CastMember, it's {:?}",
                        other.type_str()
                    )
                    .into(),
                );
                None
            }
        }
    }

    pub fn play_file(self_rc: Rc<RefCell<Self>>, member_ref: DatumRef) {
        console::log_1(
            &format!("▶️ SoundChannel::play_file() called with {:?}", member_ref).into(),
        );

        {
            let mut channel = self_rc.borrow_mut();
            web_sys::console::log_1(
                &format!(
                    "play_file(): channel={} status={:?} queued={:?} current_idx={:?}",
                    channel.channel_num,
                    channel.status,
                    channel.queued_member,
                    channel.current_segment_index
                )
                .into(),
            );

            // Only queue if we have an active source node that's actually playing
            if channel.status == SoundStatus::Playing && channel.source_node.is_some() {
                console::log_1(&"⏳ Sound busy, queuing next file...".into());
                channel.queued_member = Some(member_ref);
                return;
            }

            // Reset status and clear any stale queue
            console::log_1(&"🔄 Ready to play immediately".into());
            channel.status = SoundStatus::Stopped;
            channel.queued_member = None;

            // Only set current_segment_index if it's not already set (first time playing from playlist)
            // or if we can't find the member in the playlist (external play call)
            if channel.current_segment_index.is_none() {
                for (idx, segment) in channel.playlist_segments.iter().enumerate() {
                    if segment.member_ref == member_ref {
                        channel.current_segment_index = Some(idx);
                        web_sys::console::log_1(
                            &format!("📍 Set current_segment_index to {}", idx).into(),
                        );
                        break;
                    }
                }
            } else {
                web_sys::console::log_1(
                    &format!(
                        "🔒 Keeping existing current_segment_index={:?}",
                        channel.current_segment_index
                    )
                    .into(),
                );
            }
        }

        SoundChannel::start_sound(Rc::clone(&self_rc), member_ref);
    }

    pub fn start_sound(self_rc: Rc<RefCell<Self>>, member_ref: DatumRef) {
        let mut this = self_rc.borrow_mut();
        console::log_1(
            &format!(
                "▶️ SoundChannel::start_sound() called with {:?}",
                member_ref
            )
            .into(),
        );

        // Ensure AudioContext is active
        let state = (*this.audio_context).state();
        if state == web_sys::AudioContextState::Suspended {
            let _ = (*this.audio_context).resume();
        }

        // Reset status
        if this.status != SoundStatus::Stopped {
            this.status = SoundStatus::Stopped;
        }

        // Stop current sound if playing
        if this.status == SoundStatus::Playing {
            this.queued_member = Some(member_ref);
            return;
        }

        // Get global player
        let player_opt = unsafe { crate::PLAYER_OPT.as_mut() };
        let player = match player_opt {
            Some(p) => p,
            None => {
                web_sys::console::log_1(&"❌ No global player found".into());
                return;
            }
        };

        // Retrieve datum
        let datum = player.get_datum(&member_ref);

        if let Some(sound_member) = Self::resolve_sound_member(player, &datum) {
            this.expected_sample_rate = Some(sound_member.info.sample_rate);

            // Load audio data
            let audio_data = match Self::load_director_audio_data(
                &sound_member.sound.data(),
                sound_member.info.channels,
                sound_member.info.sample_rate,
                sound_member.info.sample_size,
                &sound_member.sound.codec(),
                Some(sound_member.info.sample_count),
            ) {
                Ok(data) => data,
                Err(e) => {
                    web_sys::console::log_1(&format!("❌ Failed to load sound: {}", e).into());
                    return;
                }
            };

            // Check if this is MP3 data
            if let Some(mp3_bytes) = &audio_data.compressed_data {
                console::log_1(&format!("🎵 MP3 detected! {} bytes", mp3_bytes.len()).into());

                let self_rc_clone = self_rc.clone();
                let mp3_data = mp3_bytes.clone();

                drop(this);

                wasm_bindgen_futures::spawn_local(async move {
                    {
                        let mut ch = self_rc_clone.borrow_mut();
                        ch.status = SoundStatus::Playing;
                    }

                    if let Err(e) =
                        Self::start_sound_mp3_async(self_rc_clone.clone(), mp3_data).await
                    {
                        web_sys::console::log_1(&format!("❌ MP3 playback failed: {:?}", e).into());
                        {
                            let mut ch = self_rc_clone.borrow_mut();
                            ch.status = SoundStatus::Stopped;
                        }
                        Self::start_sound_pcm_fallback(self_rc_clone, member_ref);
                    }
                });
                return;
            }

            // PCM/ADPCM path - USE BROWSER RESAMPLING
            let audio_context = this.audio_context.clone();

            if audio_data.samples.is_empty() {
                web_sys::console::log_1(&"❌ Audio data has no samples".into());
                return;
            }

            let source_sample_rate = audio_data.sample_rate as f32;
            let target_sample_rate = audio_context.sample_rate();
            let num_channels = audio_data.num_channels;

            console::log_1(
                &format!(
                    "📊 Source: {} Hz, Target: {} Hz, Channels: {}",
                    source_sample_rate, target_sample_rate, num_channels
                )
                .into(),
            );

            // Create buffer at SOURCE sample rate first
            let num_frames = audio_data.samples.len() / num_channels as usize;

            let source_buffer = match audio_context.create_buffer(
                num_channels as u32,
                num_frames as u32,
                source_sample_rate,
            ) {
                Ok(buf) => buf,
                Err(_) => {
                    web_sys::console::log_1(&"❌ Failed to create source AudioBuffer".into());
                    return;
                }
            };

            // Copy samples to source buffer
            for ch in 0..num_channels {
                let mut channel_data = vec![0.0f32; num_frames];
                for frame in 0..num_frames {
                    let idx = frame * num_channels as usize + ch as usize;
                    channel_data[frame] = audio_data.samples[idx];
                }
                let _ = source_buffer.copy_to_channel(&channel_data, ch as i32);
            }

            console::log_1(&"🔄 Starting async resampling...".into());

            // Drop the borrow before spawning async task
            let channel_num = this.channel_num;
            let volume = this.volume;
            let pan_value = this.pan;
            drop(this);

            // Spawn async resampling task
            let self_rc_clone = self_rc.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // Resample using OfflineAudioContext
                let resampled_buffer =
                    match Self::resample_audio_buffer(&source_buffer, target_sample_rate).await {
                        Ok(buf) => {
                            console::log_1(
                                &format!("✅ Resampled to {} Hz", buf.sample_rate()).into(),
                            );
                            buf
                        }
                        Err(e) => {
                            console::log_1(
                                &format!("⚠️ Resampling failed: {:?}, using original", e).into(),
                            );
                            source_buffer
                        }
                    };

                // Now play the resampled buffer
                let mut ch = self_rc_clone.borrow_mut();

                // Create source node
                let source = match ch.audio_context.create_buffer_source() {
                    Ok(s) => s,
                    Err(_) => {
                        web_sys::console::log_1(
                            &"❌ Failed to create AudioBufferSourceNode".into(),
                        );
                        return;
                    }
                };
                source.set_buffer(Some(&resampled_buffer));

                // Create gain node
                let gain = match ch.audio_context.create_gain() {
                    Ok(g) => g,
                    Err(_) => {
                        web_sys::console::log_1(&"❌ Failed to create GainNode".into());
                        return;
                    }
                };
                gain.gain().set_value(volume / 255.0);

                // Create pan node
                let pan = match ch.audio_context.create_stereo_panner() {
                    Ok(p) => p,
                    Err(_) => {
                        web_sys::console::log_1(&"❌ Failed to create StereoPannerNode".into());
                        return;
                    }
                };
                pan.pan().set_value(pan_value / 100.0);

                // Connect audio graph
                let _ = source.connect_with_audio_node(&pan);
                let _ = pan.connect_with_audio_node(&gain);
                let _ = gain.connect_with_audio_node(&ch.audio_context.destination());

                // Start playback
                let _ = source.start();

                // Store state
                ch.status = SoundStatus::Playing;
                ch.source_node = Some(Rc::new(source.clone()));
                ch.gain_node = Some(Rc::new(gain));
                ch.pan_node = Some(Rc::new(pan));

                drop(ch);

                // Set up ended callback
                let self_rc_clone2 = self_rc_clone.clone();
                let closure = Closure::<dyn FnMut()>::new(move || {
                    web_sys::console::log_1(
                        &format!("🔚 Channel {} sound ended", channel_num).into(),
                    );

                    let mut ch = self_rc_clone2.borrow_mut();
                    ch.status = SoundStatus::Stopped;
                    ch.source_node = None;
                    ch.start_next_segment();
                });

                source.set_onended(Some(closure.as_ref().unchecked_ref()));
                closure.forget();

                web_sys::console::log_1(
                    &format!(
                        "✅ Channel {} started playback: {} samples @ {} Hz",
                        channel_num,
                        audio_data.samples.len(),
                        resampled_buffer.sample_rate()
                    )
                    .into(),
                );
            });
        } else {
            this.status = SoundStatus::Stopped;
            web_sys::console::log_1(&"❌ start_sound failed - couldn't get sound member".into());
        }
    }

    /// Helper: resample an AudioBuffer to `target_rate` using OfflineAudioContext.
    /// Returns the rendered AudioBuffer at the new sample rate.
    async fn resample_audio_buffer(
        buffer: &AudioBuffer,
        target_rate: f32,
    ) -> Result<AudioBuffer, JsValue> {
        let current_rate = buffer.sample_rate();

        // If rates are already close enough, skip resampling
        if (current_rate - target_rate).abs() < 1.0 {
            console::log_1(&"✅ Sample rates match, no resampling needed".into());
            return Ok(buffer.clone());
        }

        console::log_1(&format!("🔄 Resampling {} Hz → {} Hz", current_rate, target_rate).into());

        let num_channels = buffer.number_of_channels();

        // Calculate new length proportionally
        let original_length = buffer.length();
        let new_length = ((original_length as f32) * (target_rate / current_rate)).ceil() as u32;

        console::log_1(
            &format!(
                "📐 Resampling: {} samples → {} samples ({} channels)",
                original_length, new_length, num_channels
            )
            .into(),
        );

        // Create offline context at target rate
        let offline = OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
            num_channels,
            new_length,
            target_rate,
        )?;

        // Create source and connect to offline context destination
        let src = offline.create_buffer_source()?;
        src.set_buffer(Some(buffer));
        src.connect_with_audio_node(&offline.destination())?;
        src.start()?;

        // Render the audio
        let render_promise = offline.start_rendering().map_err(|e| {
            console::log_1(&format!("❌ Failed to start rendering: {:?}", e).into());
            e
        })?;

        let rendered = wasm_bindgen_futures::JsFuture::from(render_promise).await?;
        let resampled = AudioBuffer::from(rendered);

        console::log_1(
            &format!(
                "✅ Resampling complete: {} Hz, {} samples",
                resampled.sample_rate(),
                resampled.length()
            )
            .into(),
        );

        Ok(resampled)
    }

    /// Updated MP3 async playback with better validation
    async fn start_sound_mp3_async(
        self_rc: Rc<RefCell<SoundChannel>>,
        mp3_bytes: Vec<u8>,
    ) -> Result<(), JsValue> {
        use web_sys::console;

        // Guard: prevent re-entrant decode/play
        {
            let ch = self_rc.borrow();
            if *ch.is_decoding.borrow() {
                console::log_1(
                    &format!("⚠️ Channel {} already decoding – skipping", ch.channel_num).into(),
                );
                return Ok(());
            }
        }

        // Set decoding flag
        {
            let mut ch = self_rc.borrow_mut();
            *ch.is_decoding.borrow_mut() = true;
        }

        // Ensure flag is cleared on exit
        let clear_flag = || {
            let mut ch = self_rc.borrow_mut();
            *ch.is_decoding.borrow_mut() = false;
        };

        // Extract valid MP3 frames
        let clean_mp3 = match Self::extract_valid_mp3_frames(&mp3_bytes) {
            Some(data) => data,
            None => {
                console::log_1(&"❌ No valid MP3 frames found – treating as PCM".into());
                clear_flag();
                return Err(JsValue::from_str("No valid MP3 data"));
            }
        };

        // NEW: Log detailed MP3 info
        if clean_mp3.len() >= 4 {
            let header =
                u32::from_be_bytes([clean_mp3[0], clean_mp3[1], clean_mp3[2], clean_mp3[3]]);
            let version = (header >> 19) & 0x3;
            let layer = (header >> 17) & 0x3;
            let bitrate_index = (header >> 12) & 0xF;
            let sample_rate_index = (header >> 10) & 0x3;

            console::log_1(
                &format!(
                    "🎵 MP3 Header Analysis: version={}, layer={}, bitrate_idx={}, sr_idx={}",
                    version, layer, bitrate_index, sample_rate_index
                )
                .into(),
            );

            // Check for issues
            if clean_mp3.len() < 100 {
                console::log_1(
                    &format!("⚠️ MP3 data suspiciously small: {} bytes", clean_mp3.len()).into(),
                );
            }
        }

        // Create ArrayBuffer from cleaned MP3 data
        let arr = Uint8Array::from(&clean_mp3[..]);
        let buf = arr.buffer();

        // Get AudioContext
        let (ctx, channel_num) = {
            let ch = self_rc.borrow();
            (ch.audio_context.clone(), ch.channel_num)
        };

        console::log_1(
            &format!(
                "🔄 Starting MP3 decode for channel {} ({} bytes)",
                channel_num,
                clean_mp3.len()
            )
            .into(),
        );

        // Decode audio data
        let decode_promise = ctx.decode_audio_data(&buf)?;
        let decoded_js = match wasm_bindgen_futures::JsFuture::from(decode_promise).await {
            Ok(v) => v,
            Err(e) => {
                // NEW: Better error reporting
                console::log_1(&format!("❌ MP3 decoding failed: {:?}", e).into());
                console::log_1(
                    &format!(
                        "📊 Data size: {} bytes, First 16 bytes: {:02X?}",
                        clean_mp3.len(),
                        &clean_mp3[0..16.min(clean_mp3.len())]
                    )
                    .into(),
                );
                console::log_1(
                    &"ℹ️ This might be Director-specific encoding. Falling back to PCM.".into(),
                );
                clear_flag();
                return Err(e);
            }
        };

        let audio_buffer: AudioBuffer = AudioBuffer::from(decoded_js);
        console::log_1(
            &format!(
                "✅ MP3 decoded: {} channels, {} samples, {} Hz",
                audio_buffer.number_of_channels(),
                audio_buffer.length(),
                audio_buffer.sample_rate()
            )
            .into(),
        );

        // Handle sample rate if needed
        let expected_rate_opt = {
            let ch = self_rc.borrow();
            ch.expected_sample_rate
        };

        let final_buffer = if let Some(expected_rate) = expected_rate_opt {
            let expected_rate_f = expected_rate as f32;
            if (audio_buffer.sample_rate() - expected_rate_f).abs() > 1.0 {
                console::log_1(
                    &format!(
                        "🔄 Resampling {} -> {} Hz",
                        audio_buffer.sample_rate(),
                        expected_rate_f
                    )
                    .into(),
                );

                match Self::resample_audio_buffer(&audio_buffer, expected_rate_f).await {
                    Ok(rbuf) => {
                        console::log_1(
                            &format!("✅ Resampled buffer: {} samples", rbuf.length()).into(),
                        );
                        rbuf
                    }
                    Err(e) => {
                        console::log_1(
                            &format!("⚠️ Resample failed: {:?}, using original", e).into(),
                        );
                        audio_buffer
                    }
                }
            } else {
                audio_buffer
            }
        } else {
            audio_buffer
        };

        // Create source node
        let source = ctx.create_buffer_source()?;
        source.set_buffer(Some(&final_buffer));

        // Create gain node
        let gain = ctx.create_gain()?;
        let volume = {
            let ch = self_rc.borrow();
            ch.volume
        };
        gain.gain().set_value(volume / 255.0);
        console::log_1(
            &format!("🔊 Setting gain to {} (volume: {})", volume / 255.0, volume).into(),
        );

        // Create pan node if available
        if let Ok(pan_node) = ctx.create_stereo_panner() {
            let pan_value = {
                let ch = self_rc.borrow();
                ch.pan
            };
            pan_node.pan().set_value(pan_value / 100.0);
            let _ = source.connect_with_audio_node(&pan_node);
            let _ = pan_node.connect_with_audio_node(&gain);
        } else {
            let _ = source.connect_with_audio_node(&gain);
        }

        let _ = gain.connect_with_audio_node(&ctx.destination());

        // Set up ended callback
        {
            let self_rc_clone = self_rc.clone();
            let closure = Closure::<dyn FnMut()>::new(move || {
                console::log_1(
                    &format!(
                        "🔚 Channel {} MP3 ended",
                        self_rc_clone.borrow().channel_num
                    )
                    .into(),
                );

                let mut ch = self_rc_clone.borrow_mut();
                ch.source_node = None;
                ch.status = SoundStatus::Stopped;
                *ch.is_decoding.borrow_mut() = false;

                drop(ch);
                self_rc_clone.borrow_mut().start_next_segment();
            });

            let _ =
                source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref());
            closure.forget();
        }

        // Start playback
        source.start()?;
        console::log_1(&"▶️ MP3 source.start() called".into());

        // Store nodes in channel state
        {
            let mut ch = self_rc.borrow_mut();
            ch.source_node = Some(Rc::new(source));
            ch.gain_node = Some(Rc::new(gain));
            ch.status = SoundStatus::Playing;
            *ch.is_decoding.borrow_mut() = false;
        }

        console::log_1(&format!("✅ Channel {} MP3 playback started", channel_num).into());

        Ok(())
    }

    /// PCM fallback when MP3 decoding fails
    fn start_sound_pcm_fallback(self_rc: Rc<RefCell<Self>>, member_ref: DatumRef) {
        console::log_1(&"🔄 Starting PCM fallback playback".into());

        let mut this = self_rc.borrow_mut();

        // Get global player
        let player_opt = unsafe { crate::PLAYER_OPT.as_mut() };
        let player = match player_opt {
            Some(p) => p,
            None => {
                web_sys::console::log_1(&"❌ No global player found".into());
                return;
            }
        };

        // Retrieve datum
        let datum = player.get_datum(&member_ref);

        if let Some(sound_member) = Self::resolve_sound_member(player, &datum) {
            let audio_context = this.audio_context.clone();

            // Check if this is actually MP3 data
            let sound_data = &sound_member.sound.data();
            if sound_data.len() > 64 {
                let data_after_header = &sound_data[64..];
                if let Some(_) = Self::find_mp3_start(data_after_header) {
                    console::log_1(&"❌ PCM fallback skipped - data is MP3, cannot decode".into());
                    this.status = SoundStatus::Stopped;
                    return;
                }
            }

            // Force PCM decoding by treating as raw PCM (NOT MP3)
            let pcm_wav = match Self::load_director_sound_from_bytes(
                &sound_member.sound.data(),
                sound_member.info.channels,
                sound_member.info.sample_rate,
                sound_member.info.sample_size,
                "raw_pcm", // ← Force raw_pcm codec to avoid MP3 detection
                Some(sound_member.info.sample_count),
            ) {
                Ok(wav) => wav,
                Err(e) => {
                    web_sys::console::log_1(&format!("❌ PCM fallback failed: {}", e).into());
                    this.status = SoundStatus::Stopped;
                    return;
                }
            };

            // Decode WAV to AudioData
            let audio_data = match AudioData::from_wav_bytes(&pcm_wav) {
                Ok(data) => data,
                Err(e) => {
                    web_sys::console::log_1(&format!("❌ WAV decode failed: {}", e).into());
                    this.status = SoundStatus::Stopped;
                    return;
                }
            };

            console::log_1(
                &format!(
                    "✅ PCM fallback: {} samples, {} Hz, {} channels",
                    audio_data.samples.len(),
                    audio_data.sample_rate,
                    audio_data.num_channels
                )
                .into(),
            );

            // Handle empty samples
            if audio_data.samples.is_empty() {
                web_sys::console::log_1(&"❌ Audio data has no samples".into());
                this.status = SoundStatus::Stopped;
                return;
            }

            let num_frames = audio_data.samples.len() / audio_data.num_channels as usize;
            let target_sample_rate = audio_context.sample_rate();
            let source_sample_rate = audio_data.sample_rate as f32;

            // Calculate resampling
            let resample_ratio = target_sample_rate / source_sample_rate;
            let resampled_frames = (num_frames as f32 * resample_ratio).round() as usize;

            console::log_1(
                &format!(
                    "🔄 Resampling {} frames -> {} frames (ratio: {:.3})",
                    num_frames, resampled_frames, resample_ratio
                )
                .into(),
            );

            // Create buffer at target sample rate
            let buffer = match audio_context.create_buffer(
                audio_data.num_channels as u32,
                resampled_frames as u32,
                target_sample_rate,
            ) {
                Ok(buf) => buf,
                Err(_) => {
                    web_sys::console::log_1(&"❌ Failed to create AudioBuffer".into());
                    this.status = SoundStatus::Stopped;
                    return;
                }
            };

            // Resample and copy data
            for ch in 0..audio_data.num_channels {
                let mut channel_data = vec![0.0f32; resampled_frames];

                for frame in 0..resampled_frames {
                    let source_pos = frame as f32 / resample_ratio;
                    let source_frame = source_pos.floor() as usize;
                    let frac = source_pos - source_frame as f32;

                    let idx1 = (source_frame * audio_data.num_channels as usize + ch as usize)
                        .min(audio_data.samples.len() - 1);
                    let idx2 = ((source_frame + 1) * audio_data.num_channels as usize
                        + ch as usize)
                        .min(audio_data.samples.len() - 1);

                    let sample1 = audio_data.samples[idx1];
                    let sample2 = audio_data.samples[idx2];
                    channel_data[frame] = sample1 + (sample2 - sample1) * frac;
                }

                let _ = buffer.copy_to_channel(&channel_data, ch as i32);
            }

            // Create source node
            let source = match audio_context.create_buffer_source() {
                Ok(s) => s,
                Err(_) => {
                    web_sys::console::log_1(&"❌ Failed to create AudioBufferSourceNode".into());
                    this.status = SoundStatus::Stopped;
                    return;
                }
            };
            source.set_buffer(Some(&buffer));

            // Create gain and pan nodes
            let gain = match this.audio_context.create_gain() {
                Ok(g) => g,
                Err(_) => {
                    web_sys::console::log_1(&"❌ Failed to create GainNode".into());
                    this.status = SoundStatus::Stopped;
                    return;
                }
            };

            let volume = this.volume;
            gain.gain().set_value(volume / 255.0);

            let pan = match this.audio_context.create_stereo_panner() {
                Ok(p) => p,
                Err(_) => {
                    web_sys::console::log_1(&"❌ Failed to create StereoPannerNode".into());
                    this.status = SoundStatus::Stopped;
                    return;
                }
            };

            let pan_value = this.pan;
            pan.pan().set_value(pan_value / 100.0);

            // Connect audio graph
            let _ = source.connect_with_audio_node(&pan);
            let _ = pan.connect_with_audio_node(&gain);
            let _ = gain.connect_with_audio_node(&audio_context.destination());

            // Start playback
            let _ = source.start();

            // Store state
            this.status = SoundStatus::Playing;
            this.sound_member = Some(sound_member.clone());
            this.source_node = Some(Rc::new(source.clone()));
            this.gain_node = Some(Rc::new(gain));
            this.pan_node = Some(Rc::new(pan));

            let channel_num = this.channel_num;
            drop(this);

            // Set up ended callback
            let self_rc_clone = self_rc.clone();
            let closure = Closure::<dyn FnMut()>::new(move || {
                web_sys::console::log_1(
                    &format!("🔚 Channel {} PCM sound ended", channel_num).into(),
                );

                let mut ch = self_rc_clone.borrow_mut();
                ch.status = SoundStatus::Stopped;
                ch.start_next_segment();
            });

            source.set_onended(Some(closure.as_ref().unchecked_ref()));
            closure.forget();

            web_sys::console::log_1(
                &format!(
                    "✅ Channel {} started PCM playback: {} samples @ {} Hz",
                    channel_num,
                    audio_data.samples.len(),
                    audio_data.sample_rate
                )
                .into(),
            );
        } else {
            this.status = SoundStatus::Stopped;
            web_sys::console::log_1(&"❌ PCM fallback failed - couldn't get sound member".into());
        }
    }

    /// Validates MP3 frame headers and calculates frame size
    fn get_mp3_frame_info(header: &[u8; 4]) -> Option<(usize, u32)> {
        if header[0] != 0xFF || (header[1] & 0xE0) != 0xE0 {
            return None;
        }

        let version = (header[1] >> 3) & 0x03;
        let layer = (header[1] >> 1) & 0x03;
        let bitrate_index = (header[2] >> 4) & 0x0F;
        let sample_rate_index = (header[2] >> 2) & 0x03;
        let padding = (header[2] >> 1) & 0x01;

        // Bitrate table for MPEG1 Layer III
        const BITRATES: [[u32; 16]; 2] = [
            // MPEG1 Layer III
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
            ],
            // MPEG2/2.5 Layer III
            [
                0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
            ],
        ];

        const SAMPLE_RATES: [[u32; 4]; 2] = [
            [44100, 48000, 32000, 0], // MPEG1
            [22050, 24000, 16000, 0], // MPEG2/2.5
        ];

        let version_index = if version == 3 { 0 } else { 1 };
        let bitrate = BITRATES[version_index][bitrate_index as usize];
        let sample_rate = SAMPLE_RATES[version_index][sample_rate_index as usize];

        if bitrate == 0 || sample_rate == 0 {
            return None;
        }

        // Calculate frame size
        let samples_per_frame = if version == 3 { 1152 } else { 576 };
        let frame_size =
            ((samples_per_frame / 8 * bitrate * 1000 / sample_rate) + padding as u32) as usize;

        Some((frame_size, sample_rate))
    }

    /// Extract only valid MP3 frames, skipping any garbage
    fn extract_valid_mp3_frames(data: &[u8]) -> Option<Vec<u8>> {
        // First, find where MP3 data starts
        let mp3_start = Self::find_mp3_start(data)?;

        console::log_1(
            &format!(
                "📍 MP3 data starts at offset {} (0x{:04X})",
                mp3_start, mp3_start
            )
            .into(),
        );

        // From the MP3 start position, extract all remaining data
        // Most Director MP3s are complete streams after the header
        let mp3_data = &data[mp3_start..];

        console::log_1(&format!("✅ Extracted {} bytes of MP3 data", mp3_data.len()).into());

        // Validate first frame
        if mp3_data.len() >= 4 {
            let header = [mp3_data[0], mp3_data[1], mp3_data[2], mp3_data[3]];
            if let Some((frame_size, sample_rate)) = Self::get_mp3_frame_info(&header) {
                console::log_1(
                    &format!(
                        "✅ First MP3 frame validated: size={}, rate={}Hz",
                        frame_size, sample_rate
                    )
                    .into(),
                );
                return Some(mp3_data.to_vec());
            }
        }

        console::log_1(&"❌ MP3 validation failed".into());
        None
    }

    pub fn play_next(&mut self) {
        console::log_1(&format!("SoundChannel({:?}) -> playNext", self.channel_num).into());

        self.start_next_segment();
    }

    pub fn stop(&mut self) {
        self.status = SoundStatus::Stopped;
        self.elapsed_time = 0.0;
        self.loops_remaining = 0;
        self.is_fading = false;

        if let Some(ref source) = self.source_node {
            let _ = source.stop_with_when(0.0);
            let _ = source.disconnect();
            console::log_1(&"🛑 Stopped previous sound".into());
        }

        self.source_node = None;
    }

    pub fn pause(&mut self) {
        if self.status == SoundStatus::Playing {
            self.status = SoundStatus::Paused;

            if let Some(ref source) = self.source_node {
                // Suspend the audio context — stops all nodes temporarily
                let _ = self.audio_context.suspend();
                console::log_1(&"⏸️ Paused playback".into());
            }
        }
    }

    pub fn resume(&mut self) {
        if self.status == SoundStatus::Paused {
            self.status = SoundStatus::Playing;

            if let Some(ref source) = self.source_node {
                // Resume the AudioContext
                let _ = self.audio_context.resume();
                console::log_1(&"▶️ Resumed playback".into());
            }
        }
    }

    // Helper method to extract audio data from cast member
    fn get_member_audio_data(&self) -> Option<AudioData> {
        // TODO: Implement based on your cast member structure
        None
    }

    //LAST /// Converts raw sound bytes into proper WAV byte stream
    // pub fn load_director_sound_from_bytes(
    //     sound_bytes: &[u8],
    //     channels: u16,
    //     sample_rate: u32,
    //     bits_per_sample: u16,
    //     codec: &str,
    //     expected_samples: Option<u32>,
    // ) -> Result<Vec<u8>, String> {
    //     use web_sys::console;

    //     if sound_bytes.is_empty() {
    //         return Err("sound_bytes empty".into());
    //     }

    //     // --- STEP 1: Skip Director/Media headers ---
    //     let mut header_size = 0;

    //     // Check for various header patterns
    //     if sound_bytes.len() >= 128 {
    //         // Look for audio data patterns - values around 0x80 (128) are typical for 8-bit audio center
    //         let potential_audio_start = sound_bytes[64..128].iter()
    //             .any(|&b| b >= 0x70 && b <= 0x90);

    //         if potential_audio_start {
    //             header_size = 64;
    //         } else if sound_bytes[96..128].iter().any(|&b| b >= 0x70 && b <= 0x90) {
    //             header_size = 96;
    //         } else {
    //             header_size = 128;
    //         }
    //     } else if sound_bytes.len() >= 64 {
    //         header_size = 64;
    //     }

    //     if sound_bytes.len() < header_size {
    //         return Err(format!(
    //             "Sound data too short ({}) for header ({})",
    //             sound_bytes.len(),
    //             header_size
    //         ));
    //     }

    //     let data = &sound_bytes[header_size..];

    //     console::log_1(&format!(
    //         "load_director_sound_from_bytes → codec='{}', header_skip={}, total_len={}, bits_per_sample={}",
    //         codec, header_size, sound_bytes.len(), bits_per_sample
    //     ).into());

    //     // --- STEP 2: Detect actual format ---
    //     let is_mp3 = data.len() > 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0;
    //     let is_probably_adpcm = codec.contains("ima") || (data.len() > 8 && data[4] == 0x00 && data[5] == 0x00);

    //     // Check if data looks like 8-bit PCM (values clustered around 128)
    //     let is_probably_8bit = bits_per_sample == 8 || (
    //         data.len() >= 100 &&
    //         data[0..100].iter().filter(|&&b| b >= 0x60 && b <= 0xA0).count() > 50
    //     );

    //     console::log_1(&format!(
    //         "Detected: MP3={}, ADPCM={}, 8-bit={}, bits={}, rate={}, ch={}",
    //         is_mp3, is_probably_adpcm, is_probably_8bit, bits_per_sample, sample_rate, channels
    //     ).into());

    //     // --- STEP 3: Decode/normalize ---
    //     let pcm_data = if is_mp3 {
    //         console::log_1(&"MP3 detected → storing raw compressed bytes".into());
    //         data.to_vec()
    //     } else if is_probably_adpcm {
    //         console::log_1(&"IMA ADPCM detected → decoding...".into());
    //         Self::decode_ima_adpcm(data, expected_samples.unwrap_or(0))?
    //     } else if is_probably_8bit {
    //         console::log_1(&"8-bit PCM detected → converting to 16-bit".into());
    //         // Convert 8-bit unsigned to 16-bit signed
    //         let mut converted = Vec::with_capacity(data.len() * 2);
    //         for &byte in data {
    //             // Convert unsigned 8-bit (0-255) centered at 128
    //             // to signed 16-bit (-32768 to 32767)
    //             // Multiply by 257 instead of 256 for better amplitude
    //             let sample_16 = ((byte as i32 - 128) * 257) as i16;
    //             converted.extend_from_slice(&sample_16.to_le_bytes());
    //         }
    //         converted
    //     } else {
    //         console::log_1(&"Assuming PCM16 → normalizing endianness".into());
    //         if bits_per_sample == 16 {
    //             let mut converted = Vec::with_capacity(data.len());
    //             let needs_swap = codec.contains("raw_pcm") || codec.contains("sound");
    //             for chunk in data.chunks_exact(2) {
    //                 if needs_swap {
    //                     converted.push(chunk[1]);
    //                     converted.push(chunk[0]);
    //                 } else {
    //                     converted.extend_from_slice(chunk);
    //                 }
    //             }
    //             converted
    //         } else {
    //             data.to_vec()
    //         }
    //     };

    //     // --- STEP 4: Build valid RIFF/WAV ---
    //     let bits = if is_probably_8bit && bits_per_sample != 16 { 16 } else { bits_per_sample }; // Always output as 16-bit
    //     let sample_count = (pcm_data.len() / (channels as usize * (bits as usize / 8))) as u32;
    //     let byte_rate = sample_rate * channels as u32 * bits as u32 / 8;
    //     let block_align = (channels * bits / 8) as u16;
    //     let data_len = pcm_data.len() as u32;

    //     let mut wav = Vec::with_capacity(58 + pcm_data.len());
    //     wav.extend_from_slice(b"RIFF");
    //     wav.extend_from_slice(&(50u32 + data_len).to_le_bytes());
    //     wav.extend_from_slice(b"WAVE");

    //     wav.extend_from_slice(b"fmt ");
    //     wav.extend_from_slice(&16u32.to_le_bytes());
    //     wav.extend_from_slice(&1u16.to_le_bytes());
    //     wav.extend_from_slice(&channels.to_le_bytes());
    //     wav.extend_from_slice(&sample_rate.to_le_bytes());
    //     wav.extend_from_slice(&byte_rate.to_le_bytes());
    //     wav.extend_from_slice(&block_align.to_le_bytes());
    //     wav.extend_from_slice(&bits.to_le_bytes());

    //     wav.extend_from_slice(b"data");
    //     wav.extend_from_slice(&data_len.to_le_bytes());
    //     wav.extend_from_slice(&pcm_data);

    //     console::log_1(&format!(
    //         "✅ Final WAV: {} bytes, {} Hz, {}-bit, {} ch, {:.2}s",
    //         wav.len(),
    //         sample_rate,
    //         bits,
    //         channels,
    //         sample_count as f32 / sample_rate as f32
    //     ).into());

    //     Ok(wav)
    // }

    /// Converts raw sound bytes into proper WAV byte stream
    pub fn load_director_sound_from_bytes(
        sound_bytes: &[u8],
        channels: u16,
        sample_rate: u32,
        bits_per_sample: u16,
        codec: &str,
        expected_samples: Option<u32>,
    ) -> Result<Vec<u8>, String> {
        use web_sys::console;
        if sound_bytes.is_empty() {
            return Err("sound_bytes empty".into());
        }

        // --- STEP 1: Skip Director/Media headers ---
        let mut header_size = 0;
        if sound_bytes.len() >= 128 {
            let potential_audio_start =
                sound_bytes[64..128].iter().any(|&b| b >= 0x70 && b <= 0x90);
            if potential_audio_start {
                header_size = 64;
            } else if sound_bytes[96..128].iter().any(|&b| b >= 0x70 && b <= 0x90) {
                header_size = 96;
            } else {
                header_size = 128;
            }
        } else if sound_bytes.len() >= 64 {
            header_size = 64;
        }
        if sound_bytes.len() < header_size {
            return Err(format!(
                "Sound data too short ({}) for header ({})",
                sound_bytes.len(),
                header_size
            ));
        }
        let data = &sound_bytes[header_size..];

        console::log_1(
            &format!(
                "🔍 First 32 audio bytes: {:02X?}",
                &data[0..data.len().min(32)]
            )
            .into(),
        );

        console::log_1(&format!(
            "load_director_sound_from_bytes → codec='{}', header_skip={}, total_len={}, bits_per_sample={}",
            codec, header_size, sound_bytes.len(), bits_per_sample
        ).into());

        // --- STEP 2: Detect actual format ---
        // CRITICAL: If codec is explicitly raw_pcm, skip all format detection
        let (is_mp3, is_probably_adpcm, is_probably_8bit) = if codec == "raw_pcm" {
            console::log_1(
                &"🔒 Codec is 'raw_pcm' - forcing PCM processing, no format detection".into(),
            );

            // For raw_pcm, trust the bits_per_sample from metadata
            // Don't try to detect 8-bit from content
            (false, false, bits_per_sample == 8)
        } else {
            // Only do format detection for non-raw_pcm codecs
            let is_mp3 = if let Some(_) = Self::find_mp3_start(data) {
                true
            } else {
                false
            };

            let is_probably_adpcm = !is_mp3 && codec.contains("ima");
            let is_probably_8bit = !is_mp3
                && (bits_per_sample == 8
                    || (data.len() >= 100
                        && data[0..100]
                            .iter()
                            .filter(|&&b| b >= 0x60 && b <= 0xA0)
                            .count()
                            > 50));

            (is_mp3, is_probably_adpcm, is_probably_8bit)
        };

        console::log_1(
            &format!(
                "Detected: MP3={}, ADPCM={}, 8-bit={}, bits={}, rate={}, ch={}",
                is_mp3, is_probably_adpcm, is_probably_8bit, bits_per_sample, sample_rate, channels
            )
            .into(),
        );

        // --- STEP 3: Decode/normalize ---
        let pcm_data = if is_mp3 {
            console::log_1(&"MP3 detected → storing raw compressed bytes".into());
            data.to_vec()
        } else if is_probably_adpcm {
            console::log_1(&"IMA ADPCM detected → decoding...".into());

            if data.len() < 4 {
                return Err("IMA ADPCM data too short to read initial state.".to_string());
            }

            let initial_predictor = i16::from_le_bytes([data[0], data[1]]) as i32;
            let initial_index = data[2] as i32;

            console::log_1(
                &format!(
                    "🎵 ADPCM state: predictor={}, index={}",
                    initial_predictor, initial_index
                )
                .into(),
            );

            let adpcm_samples = &data[4..];
            let decoded_pcm_samples =
                Self::decode_ima_adpcm_to_pcm(adpcm_samples, initial_predictor, initial_index)?;

            console::log_1(
                &format!("✅ ADPCM decoded: {} samples", decoded_pcm_samples.len()).into(),
            );

            let mut converted = Vec::with_capacity(decoded_pcm_samples.len() * 2);
            for &sample in &decoded_pcm_samples {
                converted.extend_from_slice(&sample.to_le_bytes());
            }
            converted
        } else if is_probably_8bit {
            console::log_1(&"8-bit PCM detected → converting to 16-bit".into());
            let mut converted = Vec::with_capacity(data.len() * 2);
            for &byte in data {
                let sample_16 = ((byte as i32 - 128) * 257) as i16;
                converted.extend_from_slice(&sample_16.to_le_bytes());
            }
            converted
        } else {
            console::log_1(&"Assuming PCM16 → normalizing endianness".into());
            if bits_per_sample == 16 {
                let mut converted = Vec::with_capacity(data.len());

                // Check if this looks like 8-bit unsigned samples stored as individual bytes
                // Pattern: values clustered around 0x7F-0x80 (127-128), which is 8-bit silence
                let looks_like_8bit_unsigned = data.len() >= 100
                    && data[0..100]
                        .iter()
                        .filter(|&&b| b >= 0x40 && b <= 0xBF)
                        .count()
                        > 80;

                if looks_like_8bit_unsigned {
                    console::log_1(
                        &"🔍 Detected 8-bit unsigned samples - converting to 16-bit signed".into(),
                    );

                    // Convert each 8-bit unsigned sample (0-255, centered at 128)
                    // to 16-bit signed (-32768 to 32767, centered at 0)
                    for &byte in data {
                        // Convert: subtract 128 to center at 0, then scale to 16-bit range
                        let sample_16 = ((byte as i32 - 128) * 256) as i16;
                        converted.extend_from_slice(&sample_16.to_le_bytes());
                    }
                } else {
                    console::log_1(&"🔄 Standard big-endian 16-bit - swapping bytes".into());
                    // Director stores 16-bit PCM as BIG-ENDIAN
                    for chunk in data.chunks_exact(2) {
                        converted.push(chunk[1]);
                        converted.push(chunk[0]);
                    }
                    if data.len() % 2 == 1 {
                        converted.push(*data.last().unwrap());
                    }
                }

                converted
            } else {
                data.to_vec()
            }
        };

        // --- STEP 4: Build valid RIFF/WAV ---
        let bits = if (is_probably_8bit || is_probably_adpcm) && bits_per_sample != 16 {
            16
        } else {
            bits_per_sample
        };
        let sample_count = (pcm_data.len() / (channels as usize * (bits as usize / 8))) as u32;
        let byte_rate = sample_rate * channels as u32 * bits as u32 / 8;
        let block_align = (channels * bits / 8) as u16;
        let data_len = pcm_data.len() as u32;

        let mut wav = Vec::with_capacity(58 + pcm_data.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(50u32 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&pcm_data);

        console::log_1(
            &format!(
                "✅ Final WAV: {} bytes, {} Hz, {}-bit, {} ch",
                wav.len(),
                sample_rate,
                bits,
                channels
            )
            .into(),
        );

        Ok(wav)
    }

    /// Decodes raw IMA ADPCM data (4-bit samples) into 16-bit PCM samples (i16).
    ///
    /// Args:
    ///   - adpcm_data: The raw ADPCM compressed bytes.
    ///   - initial_predictor: The starting 16-bit PCM value for the first block.
    ///   - initial_index: The starting step table index (0-88) for the first block.
    ///
    /// Returns:
    ///   - Vec<i16> of decoded 16-bit PCM audio samples.
    pub fn decode_ima_adpcm_to_pcm(
        adpcm_data: &[u8],
        mut predictor: i32, // Use mutable copies of the initial state
        mut index: i32,
    ) -> Result<Vec<i16>, String> {
        let mut pcm_samples: Vec<i16> = Vec::with_capacity(adpcm_data.len() * 2);

        for &byte in adpcm_data.iter() {
            // Each byte contains two 4-bit ADPCM samples (nibbles).

            // --- 1. Decode Lower Nibble (bits 0-3) ---
            let lower_nibble = (byte & 0x0F) as i32;

            let step = STEP_TABLE[index as usize];

            // Calculate the difference (delta) based on the nibble
            // This formula is equivalent to: diff = step / 8 + (step * (nibble & 7) * 2 + ...)
            let mut diff: i32 = step >> 3; // Start with step/8
            if (lower_nibble & 0x1) != 0 {
                diff += step;
            }
            if (lower_nibble & 0x2) != 0 {
                diff += step >> 1;
            }
            if (lower_nibble & 0x4) != 0 {
                diff += step >> 2;
            }

            // Apply the sign (MSB of the nibble) and update the predictor
            if (lower_nibble & 0x8) != 0 {
                predictor -= diff;
            } else {
                predictor += diff;
            }

            // CRITICAL FIX: Clamp the predictor to prevent runaway values and noise/distortion.
            predictor = predictor.clamp(-32768, 32767);
            pcm_samples.push(predictor as i16);

            // Update the index and clamp it between 0 and 88
            index += INDEX_TABLE[lower_nibble as usize];
            index = index.clamp(0, 88);

            // --- 2. Decode Upper Nibble (bits 4-7) ---
            let upper_nibble = (byte >> 4) as i32;

            let step = STEP_TABLE[index as usize];

            // Calculate the difference (delta)
            let mut diff: i32 = step >> 3; // Start with step/8
            if (upper_nibble & 0x1) != 0 {
                diff += step;
            }
            if (upper_nibble & 0x2) != 0 {
                diff += step >> 1;
            }
            if (upper_nibble & 0x4) != 0 {
                diff += step >> 2;
            }

            // Apply the sign and update the predictor
            if (upper_nibble & 0x8) != 0 {
                predictor -= diff;
            } else {
                predictor += diff;
            }

            // CRITICAL FIX: Clamp the predictor
            predictor = predictor.clamp(-32768, 32767);
            pcm_samples.push(predictor as i16);

            // Update the index and clamp it
            index += INDEX_TABLE[upper_nibble as usize];
            index = index.clamp(0, 88);
        }

        Ok(pcm_samples)
    }

    pub fn load_director_audio_data(
        sound_bytes: &[u8],
        channels: u16,
        sample_rate: u32,
        bits_per_sample: u16,
        codec: &str,
        expected_samples: Option<u32>,
    ) -> Result<AudioData, String> {
        console::log_1(&format!(
            "=== load_director_audio_data ===\nTotal file size: {} bytes\nChannels: {}, Sample Rate: {} Hz, Bits: {}, Codec: '{}', Expected samples: {:?}",
            sound_bytes.len(), channels, sample_rate, bits_per_sample, codec, expected_samples
        ).into());

        // NEW: If codec explicitly says raw_pcm, trust it and skip MP3 detection
        if codec == "raw_pcm" {
            console::log_1(
                &"📝 Codec is 'raw_pcm' - treating as PCM, skipping MP3 detection".into(),
            );
            let wav_bytes = Self::load_director_sound_from_bytes(
                sound_bytes,
                channels,
                sample_rate,
                bits_per_sample,
                codec,
                expected_samples,
            )?;
            return AudioData::from_wav_bytes(&wav_bytes);
        }

        // Only do format analysis if codec suggests it might be compressed
        if codec.contains("mp3") || codec.contains("mpeg") {
            console::log_1(&"🔍 Checking for MP3 data...".into());

            if let Some(mp3_start) = Self::find_mp3_start(sound_bytes) {
                console::log_1(
                    &format!(
                        "✅ Valid MP3 data found at offset {} (0x{:04X})",
                        mp3_start, mp3_start
                    )
                    .into(),
                );

                let mp3_data = sound_bytes[mp3_start..].to_vec();

                return Ok(AudioData {
                    samples: vec![],
                    num_channels: channels as u16,
                    sample_rate,
                    compressed_data: Some(mp3_data),
                });
            }
        }

        // Otherwise treat as PCM/ADPCM
        console::log_1(&"Processing as PCM/ADPCM".into());

        let wav_bytes = Self::load_director_sound_from_bytes(
            sound_bytes,
            channels,
            sample_rate,
            bits_per_sample,
            codec,
            expected_samples,
        )?;

        AudioData::from_wav_bytes(&wav_bytes)
    }

    /// Play MP3 data using HtmlAudioElement (browser handles decoding)
    pub async fn play_mp3_data(mp3_bytes: &[u8]) -> Result<(), JsValue> {
        console::log_1(
            &format!(
                "🎵 Creating Blob from {} bytes of MP3 data",
                mp3_bytes.len()
            )
            .into(),
        );

        // Create a Uint8Array from the MP3 bytes
        let uint8_array = js_sys::Uint8Array::from(mp3_bytes);

        // Create a Blob with the MP3 data
        let mut blob_parts = js_sys::Array::new();
        blob_parts.push(&uint8_array);

        let blob = Blob::new_with_u8_array_sequence_and_options(
            &blob_parts,
            web_sys::BlobPropertyBag::new().type_("audio/mpeg"),
        )?;

        console::log_1(&format!("✅ Blob created: {} bytes", blob.size()).into());

        // Create an object URL for the Blob
        let url = Url::create_object_url_with_blob(&blob).map_err(|e| {
            console::error_1(&format!("Failed to create object URL: {:?}", e).into());
            e
        })?;

        console::log_1(&format!("🔗 Object URL created: {}", url).into());

        // Create an audio element and play it
        let audio = HtmlAudioElement::new().map_err(|e| {
            console::error_1(&"Failed to create audio element".into());
            e
        })?;

        audio.set_src(&url);
        audio.set_loop(false);

        // Play the audio
        audio.play().map_err(|e| {
            console::error_1(&format!("Failed to play audio: {:?}", e).into());
            e
        })?;

        console::log_1(&"▶️ MP3 playback started".into());

        Ok(())
    }

    /// Alternative: Decode MP3 to WAV using Web Audio API (for audio context integration)
    pub async fn decode_mp3_to_wav(
        audio_context: &AudioContext,
        mp3_bytes: &[u8],
    ) -> Result<AudioBuffer, JsValue> {
        console::log_1(&"🔄 Decoding MP3 data via Web Audio API".into());

        // Create a Uint8Array from MP3 bytes
        let uint8_array = js_sys::Uint8Array::from(mp3_bytes);

        // Create an ArrayBuffer from the Uint8Array
        let array_buffer = uint8_array.buffer();

        // Use Web Audio's decodeAudioData to decode the MP3
        // This returns a Promise, so we need to handle it asynchronously
        let decode_promise = audio_context
            .decode_audio_data(&array_buffer)
            .map_err(|e| {
                console::error_1(&format!("Failed to start decoding: {:?}", e).into());
                e
            })?;

        // Await the promise
        let audio_buffer = wasm_bindgen_futures::JsFuture::from(decode_promise)
            .await
            .map_err(|e| {
                console::error_1(&format!("MP3 decode failed: {:?}", e).into());
                e
            })?;

        // The result should be an AudioBuffer
        let buffer = audio_buffer.dyn_into::<AudioBuffer>().map_err(|e| {
            console::error_1(&"Decode result is not an AudioBuffer".into());
            e
        })?;

        console::log_1(
            &format!(
                "✅ MP3 decoded successfully: {} channels, {} samples, {} Hz",
                buffer.number_of_channels(),
                buffer.length(),
                buffer.sample_rate()
            )
            .into(),
        );

        Ok(buffer)
    }

    /// Integrated MP3 playback in your sound channel
    pub async fn play_mp3_via_web_audio(&mut self, mp3_bytes: &[u8]) -> Result<(), JsValue> {
        console::log_1(&"🎵 Starting MP3 playback via Web Audio".into());

        // Step 1: Decode MP3 to AudioBuffer
        let audio_buffer = Self::decode_mp3_to_wav(&self.audio_context, mp3_bytes).await?;

        // Step 3: Stop any existing playback
        if let Some(ref source) = self.source_node {
            let _ = source.stop_with_when(0.0);
            let _ = source.disconnect();
        }
        self.source_node = None;

        // Step 4: Create new source node
        let source = self.audio_context.create_buffer_source()?;
        source.set_buffer(Some(&audio_buffer));

        // Create FRESH gain and pan nodes
        let gain = match self.audio_context.create_gain() {
            Ok(g) => g,
            Err(_) => {
                web_sys::console::log_1(&"❌ Failed to create GainNode".into());
                return Ok(());
            }
        };

        let volume = self.volume;
        gain.gain().set_value(volume / 255.0);
        console::log_1(
            &format!("🔊 Setting gain to {} (volume: {})", volume / 255.0, volume).into(),
        );

        let pan = match self.audio_context.create_stereo_panner() {
            Ok(p) => p,
            Err(_) => {
                web_sys::console::log_1(&"❌ Failed to create StereoPannerNode".into());
                return Ok(());
            }
        };

        let pan_value = self.pan;
        pan.pan().set_value(pan_value / 100.0);

        let _ = source.connect_with_audio_node(&pan);
        let _ = pan.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(&self.audio_context.destination());

        // Step 6: Set up on-ended callback
        let channel_index = self.channel_num;
        let closure = Closure::<dyn FnMut()>::new(move || {
            SoundChannel::handle_end_of_sound(channel_index);
        });

        let _ = source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref());
        closure.forget();

        // Step 7: Start playback
        source.start()?;
        web_sys::console::log_1(&"called source.start()".into());
        self.source_node = Some(Rc::new(source));
        self.status = SoundStatus::Playing;

        console::log_1(&format!("✅ MP3 playback started on channel {}", self.channel_num).into());

        Ok(())
    }

    pub fn load_director_sound(data: &[u8]) -> Result<AudioData, String> {
        console::log_1(
            &format!(
                "ℹ️ load_director_sound: data_len={}, is_wav={}, is_aiff={}",
                data.len(),
                Self::is_wav_format(data),
                Self::is_aiff_format(data)
            )
            .into(),
        );

        let wrapped = Self::wrap_director_wav(data);
        console::log_1(&format!("ℹ️ Wrapped WAV length: {} bytes", wrapped.len()).into());

        if Self::is_wav_format(data) && data.len() >= 44 {
            // Try normal WAV parsing first
            match Self::load_wav(data) {
                Ok(audio) if audio.sample_rate > 0 && !audio.samples.is_empty() => {
                    return Ok(audio)
                }
                _ => {
                    console::log_1(&"⚠️ Invalid WAV header, wrapping as proper WAV".into());
                    let wrapped_wav = Self::wrap_director_wav(data);
                    return Self::load_wav(&wrapped_wav);
                }
            }
        } else if Self::is_aiff_format(data) {
            Self::load_aiff(data)
        } else {
            // raw PCM or headerless WAV
            console::log_1(&"ℹ️ Raw PCM detected, wrapping into WAV".into());
            let wrapped_wav = Self::wrap_director_wav(data);
            Self::load_wav(&wrapped_wav)
        }
    }

    /// Wrap raw Director PCM data as proper WAV bytes
    fn wrap_director_wav(data: &[u8]) -> Vec<u8> {
        let sample_rate = 16000; // default for Director sounds
        let channels: u16 = 1;
        let bits_per_sample: u16 = 16;
        let sample_rate: u32 = 16000;

        let byte_rate: u32 = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
        let block_align: u16 = channels * bits_per_sample / 8; // cast to u16

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // fmt chunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&(channels as u16).to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&(bits_per_sample as u16).to_le_bytes());

        // data chunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());

        console::log_1(
            &format!(
                "🧪 First 8 bytes of PCM: {:02X?}",
                &data[0..8.min(data.len())]
            )
            .into(),
        );

        // Convert 16-bit big-endian to little-endian
        for chunk in data.chunks_exact(2) {
            wav.push(chunk[1]);
            wav.push(chunk[0]);
        }
        if data.len() % 2 == 1 {
            wav.push(*data.last().unwrap());
        }

        wav
    }

    fn is_wav_format(data: &[u8]) -> bool {
        data.len() >= 4 && &data[0..4] == b"RIFF"
    }

    fn is_aiff_format(data: &[u8]) -> bool {
        data.len() >= 4 && &data[0..4] == b"FORM"
    }

    fn load_wav(data: &[u8]) -> Result<AudioData, String> {
        if data.len() < 44 {
            console::log_1(&"⚠️ WAV file too small".into());
            return Err("WAV file too small".to_string());
        }

        let sample_rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
        let num_channels = u16::from_le_bytes([data[22], data[23]]);
        let bits_per_sample = u16::from_le_bytes([data[34], data[35]]);

        console::log_1(
            &format!(
                "🎵 load_wav: sample_rate={}, num_channels={}, bits_per_sample={}, data_len={}",
                sample_rate,
                num_channels,
                bits_per_sample,
                data.len()
            )
            .into(),
        );

        let audio_data = &data[44..];
        let samples: Vec<f32> = if bits_per_sample == 16 {
            audio_data
                .chunks_exact(2)
                .map(|chunk| {
                    let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                    sample as f32 / 32768.0
                })
                .collect()
        } else if bits_per_sample == 8 {
            audio_data
                .iter()
                .map(|&byte| (byte as f32 - 128.0) / 128.0)
                .collect()
        } else {
            console::log_1(&format!("⚠️ Unsupported bit depth: {}", bits_per_sample).into());
            return Err(format!("Unsupported bit depth: {}", bits_per_sample));
        };

        console::log_1(&format!("🎧 WAV loaded: {} samples", samples.len()).into());

        Ok(AudioData {
            samples,
            sample_rate,
            num_channels: num_channels.into(),
            compressed_data: None, // WAV = uncompressed
        })
    }

    fn load_aiff(data: &[u8]) -> Result<AudioData, String> {
        // Simplified AIFF parser stub
        Ok(AudioData {
            samples: Vec::new(), // keep as Vec<f32>, not Float32Array
            sample_rate: 44100,
            num_channels: 2,
            compressed_data: None, // WAV = uncompressed
        })
    }

    fn load_raw_pcm(data: &[u8]) -> Result<AudioData, String> {
        // Assume 16-bit PCM
        let samples: Vec<f32> = data
            .chunks_exact(2)
            .map(|chunk| {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                sample as f32 / 32768.0
            })
            .collect();

        Ok(AudioData {
            samples,
            sample_rate: 22050,
            num_channels: 1,
            compressed_data: None, // WAV = uncompressed
        })
    }

    pub fn rewind(&mut self) {
        self.elapsed_time = self.start_time;
        self.loops_remaining = self.loop_count;

        if self.status != SoundStatus::Stopped {
            self.status = SoundStatus::Stopped;
        }
    }

    pub fn queue(&mut self, datum_ref: DatumRef, player: &DirPlayer) {
        use web_sys::console;

        let datum = player.get_datum(&datum_ref);

        let props = match datum {
            Datum::PropList(ref p, _) if !p.is_empty() => p,
            _ => {
                console::log_1(
                    &"⚠️ queue(): called with non-propList or empty list — ignored".into(),
                );
                return;
            }
        };

        let member_opt = SoundChannel::get_proplist_prop(player, &datum, "member");
        if member_opt.is_none() {
            console::log_1(&"⚠️ queue(): missing #member — ignored".into());
            return;
        }

        let loop_count = if let Some(Datum::Int(count)) =
            SoundChannel::get_proplist_prop(player, &datum, "loopCount")
        {
            count
        } else {
            1
        };

        if loop_count <= 0 {
            console::log_1(
                &format!(
                    "⚠️ queue(): invalid loopCount={} — skipping entry",
                    loop_count
                )
                .into(),
            );
            return;
        }

        console::log_1(
            &format!(
                "➕ queue() - Adding to channel {} | loop_count={} | current status: {:?}",
                self.channel_num, loop_count, self.status
            )
            .into(),
        );

        let segment = SoundSegment {
            member_ref: datum_ref.clone(),
            loop_count,
            loops_remaining: loop_count,
        };

        self.playlist_segments.push(segment);
        self.playlist.push(datum_ref.clone());

        console::log_1(
            &format!(
                "✅ Channel {} playlist now has {} items",
                self.channel_num,
                self.playlist_segments.len()
            )
            .into(),
        );

        // DON'T auto-start or change current_segment_index here
        // That's the job of play() or playNext()
    }

    pub fn break_loop(&mut self) {
        self.loops_remaining = 0;
    }

    pub fn set_playlist(&mut self, list: Vec<DatumRef>, player: &DirPlayer) {
        // Clear current state for this channel
        self.playlist_segments.clear();
        self.playlist.clear();

        if list.is_empty() {
            web_sys::console::log_1(
                &format!("🧹 Cleared playlist for channel {}", self.channel_num).into(),
            );
            return;
        }

        for datum_ref in list {
            let datum = player.get_datum(&datum_ref);

            if let Some(member_datum) = SoundChannel::get_proplist_prop(player, datum, "member") {
                let loop_count = if let Some(Datum::Int(count)) =
                    SoundChannel::get_proplist_prop(player, datum, "loopCount")
                {
                    count
                } else {
                    1
                };

                self.playlist_segments.push(SoundSegment {
                    member_ref: datum_ref.clone(),
                    loop_count,
                    loops_remaining: loop_count,
                });
                self.playlist.push(datum_ref.clone());
            }
        }

        self.current_segment_index = None;

        if !self.playlist_segments.is_empty() {
            self.current_segment_index = Some(0);
            self.queued_member = Some(self.playlist_segments[0].member_ref.clone());
        }
    }

    pub fn get_playlist(&self) -> Vec<DatumRef> {
        self.playlist.clone()
    }

    pub fn fade_in(&mut self, ticks: i32, to_volume: f32) {
        let duration = ticks as f32 / 60.0;
        self.is_fading = true;
        self.fade_start_volume = 0.0;
        self.fade_target_volume = to_volume;
        self.fade_duration = duration;
        self.fade_elapsed = 0.0;
        self.volume = 0.0;
    }

    pub fn fade_out(&mut self, ticks: i32) {
        self.fade_to(ticks, 0.0);
    }

    pub fn fade_to(&mut self, ticks: i32, to_volume: f32) {
        let duration = ticks as f32 / 60.0;
        self.is_fading = true;
        self.fade_start_volume = self.volume;
        self.fade_target_volume = to_volume;
        self.fade_duration = duration;
        self.fade_elapsed = 0.0;
    }

    pub fn is_busy(&self) -> bool {
        self.status != SoundStatus::Stopped
    }

    pub fn set_loop_count(&mut self, count: i32) {
        self.loop_count = count;
        self.loops_remaining = count;
    }

    pub fn get_duration(&self) -> f32 {
        if self.member.is_none() || self.sample_rate == 0 {
            return 0.0;
        }

        self.sample_count as f32 / self.sample_rate as f32
    }

    pub fn update(&mut self, delta_time: f32, player: &mut DirPlayer) -> Result<(), ScriptError> {
        // Handle fading
        if self.is_fading {
            self.fade_elapsed += delta_time;
            if self.fade_elapsed >= self.fade_duration {
                self.set_volume(self.fade_target_volume);
                self.is_fading = false;
            } else {
                let t = self.fade_elapsed / self.fade_duration;
                let new_volume =
                    self.fade_start_volume + (self.fade_target_volume - self.fade_start_volume) * t;
                self.set_volume(new_volume);
            }
        }

        // ⭐ Remove the playback advancement logic - it's handled by onended callback now
        // Audio plays asynchronously in Web Audio thread
        // When it finishes, onended → handle_end_of_sound → start_next_segment

        if self.status != SoundStatus::Playing {
            return Ok(());
        }

        self.elapsed_time += delta_time;

        // That's it! The onended callback handles everything else
        Ok(())
    }

    /// Stops the currently playing WebAudio source node.
    pub fn stop_playback_nodes(&mut self) {
        if let Some(source) = self.source_node.take() {
            console::log_1(&format!("🛑 Stopping channel {}", self.channel_num).into());
            let _ = source.stop_with_when(0.0);
            let _ = source.disconnect();
        }
        if let Some(gain) = self.gain_node.take() {
            let _ = gain.disconnect();
        }
        if let Some(pan) = self.pan_node.take() {
            let _ = pan.disconnect();
        }
        self.source_node = None;
        self.gain_node = None;
        self.pan_node = None;
    }

    pub fn start_segment_playback(
        &mut self,
        sound_member: &SoundMember,
        _loop_count: i32,
    ) -> Result<(), ScriptError> {
        self.sound_member = Some(sound_member.clone());
        self.sample_rate = sound_member.info.sample_rate;
        self.sample_count = sound_member.info.sample_count;
        self.channel_count = sound_member.info.channels;

        self.expected_sample_rate = Some(sound_member.info.sample_rate);

        let _ = self.audio_context.resume();

        let sound_data = &sound_member.sound.data();
        let channels = sound_member.info.channels;
        let sample_rate = sound_member.info.sample_rate;
        let sample_size = sound_member.info.sample_size;
        let codec = sound_member.sound.codec();
        let expected_samples = Some(sound_member.info.sample_count);

        let audio_data = Self::load_director_audio_data(
            sound_data,
            channels,
            sample_rate,
            sample_size,
            &codec,
            expected_samples,
        )
        .map_err(|e| ScriptError::new(format!("Failed to load sound: {}", e)))?;

        let num_frames = audio_data.samples.len() / audio_data.num_channels as usize;

        let buffer = self
            .audio_context
            .create_buffer(
                audio_data.num_channels as u32,
                num_frames as u32,
                audio_data.sample_rate as f32,
            )
            .map_err(|e| ScriptError::new(format!("Failed to create buffer: {:?}", e)))?;

        for channel in 0..audio_data.num_channels {
            let mut channel_data = buffer
                .get_channel_data(channel as u32)
                .map_err(|e| ScriptError::new(format!("Failed to get channel data: {:?}", e)))?;
            for frame in 0..num_frames {
                let idx = frame * audio_data.num_channels as usize + channel as usize;
                channel_data[frame] = audio_data.samples[idx];
            }
        }

        let source = self
            .audio_context
            .create_buffer_source()
            .map_err(|e| ScriptError::new(format!("Failed to create source: {:?}", e)))?;
        source.set_buffer(Some(&buffer));

        let gain = self
            .audio_context
            .create_gain()
            .map_err(|e| ScriptError::new(format!("Failed to create gain: {:?}", e)))?;
        gain.gain().set_value(self.volume / 255.0);

        let pan = self.audio_context.create_stereo_panner().ok();

        if let Some(ref pan_node) = pan {
            source.connect_with_audio_node(pan_node).map_err(|e| {
                ScriptError::new(format!("Failed to connect source to pan: {:?}", e))
            })?;
            pan_node
                .connect_with_audio_node(&gain)
                .map_err(|e| ScriptError::new(format!("Failed to connect pan to gain: {:?}", e)))?;
            pan_node.pan().set_value(self.pan / 100.0);
        } else {
            source.connect_with_audio_node(&gain).map_err(|e| {
                ScriptError::new(format!("Failed to connect source to gain: {:?}", e))
            })?;
        }

        gain.connect_with_audio_node(&self.audio_context.destination())
            .map_err(|e| {
                ScriptError::new(format!("Failed to connect gain to destination: {:?}", e))
            })?;

        source
            .start()
            .map_err(|e| ScriptError::new(format!("Failed to start source: {:?}", e)))?;

        console::log_1(
            &format!(
                "✅ Channel {} playing: {} samples @ {} Hz",
                self.channel_num,
                audio_data.samples.len(),
                audio_data.sample_rate
            )
            .into(),
        );

        // Wrap nodes in Rc
        self.source_node = Some(Rc::new(source));
        self.gain_node = Some(Rc::new(gain));
        self.pan_node = pan.map(Rc::new);
        self.status = SoundStatus::Playing;

        Ok(())
    }

    /// Plays a single sound member, clearing any existing playlist (Score-based) data.
    /// This method is called by Lingo commands like `sound(x).playFile(...)` or `sound(x).member = ...`.
    pub fn play_member(
        &mut self,
        player: &mut DirPlayer,
        member_ref: &DatumRef,
        loop_count: i32,
    ) -> Result<(), ScriptError> {
        // Clear playlist
        self.playlist.clear();
        self.current_segment_index = None;

        // Stop previous sound
        self.stop_playback_nodes();

        // Get the datum
        let member_datum = player.get_datum(member_ref);

        // Check if it's a CastMember
        if let Datum::CastMember(ref cast_member_ref) = member_datum {
            // Find the actual cast member
            if let Some(cast_member) = player
                .movie
                .cast_manager
                .find_member_by_ref(cast_member_ref)
            {
                if let CastMemberType::Sound(sound_member) = &cast_member.member_type {
                    self.sound_member = Some(sound_member.clone());
                    self.loop_count = loop_count;
                    self.loops_remaining = loop_count;

                    self.start_segment_playback(sound_member, loop_count)?;
                    self.status = SoundStatus::Playing;
                } else {
                    return Err(ScriptError::new("Member is not a sound".to_string()));
                }
            } else {
                return Err(ScriptError::new("Cast member not found".to_string()));
            }
        } else {
            return Err(ScriptError::new("Expected CastMember datum".to_string()));
        }

        Ok(())
    }

    pub fn is_playback_finished(&self) -> bool {
        // Check if the source node exists and has finished playing
        if let Some(ref source) = self.source_node {
            // In web_sys, we can't directly check if playback finished
            // Instead, rely on elapsed_time vs duration
            if self.sample_rate > 0 && self.sample_count > 0 {
                let duration = self.sample_count as f32 / self.sample_rate as f32;
                return self.elapsed_time >= duration;
            }
        }
        false
    }

    pub async fn play_castmember(
        &mut self,
        samples: &[f32],
        num_channels: u32,
        sample_rate: f32,
    ) -> Result<(), JsValue> {
        console::log_1(
            &format!(
                "🎵 play_castmember: {} frames @ {}Hz",
                samples.len() / num_channels as usize,
                sample_rate
            )
            .into(),
        );

        let context: &AudioContext = &*self.audio_context;

        // Resume context if needed
        let state_val = Reflect::get(context.as_ref(), &"state".into())?;
        let state_str = state_val.as_string().unwrap_or_default();
        if state_str == "suspended" {
            console::log_1(&"🌀 Resuming AudioContext".into());
            let promise = context.resume()?;
            JsFuture::from(promise).await?;
        }

        if let Some(ref source) = self.source_node {
            let _ = source.stop_with_when(0.0);
        }
        self.source_node = None;

        let num_frames = samples.len() / num_channels as usize;
        if num_frames == 0 {
            return Ok(());
        }

        let buffer = context.create_buffer(num_channels, num_frames as u32, sample_rate)?;
        for channel in 0..num_channels {
            let mut channel_data = vec![0.0f32; num_frames];
            for frame in 0..num_frames {
                let idx = frame * num_channels as usize + channel as usize;
                channel_data[frame] = samples[idx];
            }
            buffer.copy_to_channel(&channel_data, channel as i32)?;
        }

        let source = context.create_buffer_source()?;
        source.set_buffer(Some(&buffer));

        let pan = match context.create_stereo_panner() {
            Ok(p) => p,
            Err(_) => {
                web_sys::console::log_1(&"❌ Failed to create StereoPannerNode".into());
                return Ok(());
            }
        };

        let pan_value = self.pan;
        pan.pan().set_value(pan_value / 100.0);

        // Create FRESH gain and pan nodes
        let gain = match context.create_gain() {
            Ok(g) => g,
            Err(_) => {
                web_sys::console::log_1(&"❌ Failed to create GainNode".into());
                return Ok(());
            }
        };

        let volume = self.volume;
        gain.gain().set_value(volume / 255.0);
        console::log_1(
            &format!("🔊 Setting gain to {} (volume: {})", volume / 255.0, volume).into(),
        );

        // Connect the chain: Source -> Pan -> Gain -> Destination
        let _ = source.connect_with_audio_node(&pan);
        let _ = pan.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(&context.destination());

        // Use add_event_listener instead of deprecated set_onended
        let channel_index = self.channel_num;
        let closure = Closure::<dyn FnMut()>::new(move || {
            SoundChannel::handle_end_of_sound(channel_index);
        });

        let _ = source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref());
        closure.forget();

        source.start()?;
        web_sys::console::log_1(&"called source.start()".into());
        self.source_node = Some(Rc::new(source)); // Wrap in Rc

        console::log_1(&format!("✅ Audio scheduled for channel {}", self.channel_num).into());

        Ok(())
    }

    pub async fn play_sound(&mut self, audio_data: &AudioData) -> Result<(), JsValue> {
        use web_sys::{Blob, HtmlAudioElement, Url};

        console::log_1(&"🎵 play_sound() called".into());

        let context = &*self.audio_context;

        // 🔊 If MP3 → decode & play via <audio>
        if let Some(ref compressed) = audio_data.compressed_data {
            console::log_1(&"MP3 stream detected → playing via Blob URL".into());

            let array = js_sys::Uint8Array::from(&compressed[..]);
            let blob = Blob::new_with_u8_array_sequence(&js_sys::Array::of1(&array))
                .map_err(|e| JsValue::from_str(&format!("Failed to create blob: {:?}", e)))?;
            let url = Url::create_object_url_with_blob(&blob)
                .map_err(|e| JsValue::from_str(&format!("Failed to create object URL: {:?}", e)))?;

            let audio = HtmlAudioElement::new_with_src(&url)?;
            audio.set_loop(false);
            audio.play()?;
            return Ok(());
        }

        // 🎧 Otherwise normal PCM playback through WebAudio
        self.play_castmember(
            &audio_data.samples,
            audio_data.num_channels as u32,
            audio_data.sample_rate as f32,
        )
        .await
    }

    pub fn stop_sound(&mut self) {
        if let Some(ref source) = self.source_node {
            let _ = source.stop_with_when(0.0);
            let _ = source.disconnect();
            console::log_1(&"🛑 Stopped previous sound".into());
        } else {
            console::log_1(&"ℹ️ No active sound source to stop".into());
        }

        self.source_node = None;
        self.status = SoundStatus::Stopped;
    }

    pub fn pause_sound(&mut self) {
        if let Some(ref source) = self.source_node {
            console::log_1(&"⏸️ Pausing current sound...".into());
            let _ = source.stop_with_when(0.0);
            self.source_node = None;
        } else {
            console::log_1(&"ℹ️ No active sound source to pause".into());
        }
    }

    pub fn set_volume(&mut self, volume: f32) -> Result<(), JsValue> {
        self.volume = volume.clamp(0.0, 255.0);
        if let Some(ref gain) = self.gain_node {
            gain.gain().set_value(self.volume / 255.0);
        }
        Ok(())
    }

    pub fn set_pan(&mut self, pan: f32) -> Result<(), JsValue> {
        let clamped = pan.clamp(-1.0, 1.0);

        if let Some(ref pan_node) = self.pan_node {
            let _ = pan_node.pan().set_value(clamped);
            console::log_1(&JsValue::from_str(&format!("🎚️ Pan set to {:.2}", clamped)));
        }

        Ok(())
    }

    // This is the static entry point called by the AudioBufferSourceNode's 'onended' event.
    // It needs to safely retrieve the DirPlayer and the specific SoundChannel.
    pub fn handle_end_of_sound(channel_index: i32) {
        use web_sys::console;

        let player_opt = unsafe { crate::PLAYER_OPT.as_mut() };
        if player_opt.is_none() {
            console::log_1(&"❌ PLAYER_OPT is None in handle_end_of_sound".into());
            return;
        }
        let player = player_opt.unwrap();

        let channel_opt = player.sound_manager.get_channel_mut(channel_index as usize);

        if let Some(channel_rc) = channel_opt {
            channel_rc.borrow_mut().start_next_segment();
        } else {
            console::log_1(&format!("❌ SoundChannel {} not found.", channel_index).into());
        }
    }

    /// Called by onended callback to handle loops and playlist
    pub fn start_next_segment(&mut self) {
        use web_sys::console;

        console::log_1(
            &format!(
                "🔄 start_next_segment called for channel {}",
                self.channel_num
            )
            .into(),
        );

        self.queued_member = None;

        let index = match self.current_segment_index {
            Some(i) => i,
            None => {
                console::log_1(&"⚠️ No current index, checking for queued sounds".into());
                if !self.playlist_segments.is_empty() {
                    console::log_1(
                        &format!("🎵 Found {} queued sounds", self.playlist_segments.len()).into(),
                    );
                    self.current_segment_index = Some(0);

                    // Get the member_ref to play
                    let member_ref = self.playlist_segments[0].member_ref.clone();
                    let channel_num = self.channel_num;

                    // Spawn async playback
                    spawn_local(async move {
                        if let Some(player) = unsafe { crate::PLAYER_OPT.as_mut() } {
                            if let Some(channel_rc) =
                                player.sound_manager.get_channel(channel_num as usize)
                            {
                                SoundChannel::play_file(channel_rc, member_ref);
                            }
                        }
                    });
                }
                return;
            }
        };

        if index >= self.playlist_segments.len() {
            console::log_1(&"⚠️ Current index out of bounds".into());
            self.current_segment_index = None;
            self.status = SoundStatus::Stopped;
            return;
        }

        let segment = &mut self.playlist_segments[index];

        console::log_1(
            &format!(
                "🔄 start_next_segment: index={}, loops_remaining={}/{}",
                index, segment.loops_remaining, segment.loop_count
            )
            .into(),
        );

        // Loop logic - if loop_count is 0, loop forever
        if segment.loop_count == 0 || segment.loops_remaining > 1 {
            if segment.loop_count != 0 {
                segment.loops_remaining -= 1;
            }
            console::log_1(&format!("🔁 Looping segment {}", index).into());

            // Get the member_ref to play
            let member_ref = segment.member_ref.clone();
            let channel_num = self.channel_num;

            // Spawn async playback for the loop
            spawn_local(async move {
                if let Some(player) = unsafe { crate::PLAYER_OPT.as_mut() } {
                    if let Some(channel_rc) = player.sound_manager.get_channel(channel_num as usize)
                    {
                        SoundChannel::play_file(channel_rc, member_ref);
                    }
                }
            });
            return;
        }

        // Segment finished - REMOVE IT
        console::log_1(&format!("✅ Segment {} completed, REMOVING", index).into());
        self.playlist_segments.remove(index);
        self.playlist.remove(index);

        console::log_1(&format!("📊 {} items remaining", self.playlist_segments.len()).into());

        // Play next segment if available
        if index < self.playlist_segments.len() {
            self.current_segment_index = Some(index);
            let next_segment = &mut self.playlist_segments[index];
            next_segment.loops_remaining = next_segment.loop_count;

            console::log_1(&format!("⏭️ Playing next segment at index {}", index).into());

            // Get the member_ref to play
            let member_ref = next_segment.member_ref.clone();
            let channel_num = self.channel_num;

            // Spawn async playback for next segment
            spawn_local(async move {
                if let Some(player) = unsafe { crate::PLAYER_OPT.as_mut() } {
                    if let Some(channel_rc) = player.sound_manager.get_channel(channel_num as usize)
                    {
                        SoundChannel::play_file(channel_rc, member_ref);
                    }
                }
            });
        } else {
            console::log_1(&"⏸️ Playlist empty, stopped".into());
            self.current_segment_index = None;
            self.status = SoundStatus::Stopped;
        }
    }

    fn spawn_playback_async(&self) {
        use web_sys::console;
        console::log_1(&"🚀 Spawning async playback task".into());

        // We need to get an Rc to self somehow
        // This requires changing how SoundChannel is stored in SoundManager
        // For now, use a workaround with global player
        let channel_num = self.channel_num;

        spawn_local(async move {
            let player_opt = unsafe { crate::PLAYER_OPT.as_mut() };
            if let Some(player) = player_opt {
                if let Some(channel_rc) = player.sound_manager.get_channel(channel_num as usize) {
                    SoundChannel::play_current_segment_async(channel_rc).await;
                }
            }
        });
    }

    fn trigger_playback_for_current_segment(&mut self) {
        use web_sys::console;

        let index = match self.current_segment_index {
            Some(i) => i,
            None => return,
        };

        if index >= self.playlist_segments.len() {
            return;
        }

        let member_ref = self.playlist_segments[index].member_ref.clone();
        console::log_1(&format!("🎬 trigger_playback_for_current_segment({})", index).into());

        // Store member ref for play() to use
        self.member = Some(member_ref.clone()); // Clone member_ref for play
        self.loop_count = self.playlist_segments[index].loop_count;
        self.loops_remaining = self.playlist_segments[index].loops_remaining;

        // Call play() which loads audio and schedules it in...
        let loop_count_val = self.loop_count;
        // FIX: Providing required DatumRef and i32 arguments to self.play()
        if let Err(e) = self.play(member_ref, loop_count_val) {
            console::error_1(&JsValue::from_str(&format!(
                "Failed to trigger playback: {:?}",
                e
            )));
        }
    }

    /// Loads the sound member data, decodes the generated WAV bytes, and creates the AudioBuffer.
    async fn load_audio_data(
        &mut self,
        audio_context: &AudioContext,
        member_ref: &DatumRef,
    ) -> Result<(AudioBuffer, u32, u32), JsValue> {
        let player_opt = unsafe { crate::PLAYER_OPT.as_ref() };
        let player = player_opt.ok_or_else(|| JsValue::from_str("Player not initialized"))?;

        // First, get the datum and extract CastMemberRef
        let datum = player.get_datum(member_ref);
        let cast_member_ref = match datum {
            Datum::CastMember(ref r) => r,
            _ => return Err(JsValue::from_str("Expected CastMember datum")),
        };

        // Now find the member using the CastMemberRef
        let cast_member = player
            .movie
            .cast_manager
            .find_member_by_ref(cast_member_ref)
            .ok_or_else(|| JsValue::from_str("Sound member not found"))?;

        // Extract the sound member - no need to borrow, it's already a reference
        let sound_member = match &cast_member.member_type {
            CastMemberType::Sound(s) => s,
            _ => return Err(JsValue::from_str("Member is not a sound type")),
        };

        self.expected_sample_rate = Some(sound_member.info.sample_rate);

        // Get WAV Bytes from raw Director data
        let wav_bytes = Self::load_director_sound_from_bytes(
            &sound_member.sound.data(), // Get the actual sound data
            sound_member.info.channels,
            sound_member.info.sample_rate,
            sound_member.info.sample_size,
            &sound_member.sound.codec(), // Get codec from sound chunk
            Some(sound_member.info.sample_count),
        )
        .map_err(|e| JsValue::from_str(&format!("WAV creation failed: {}", e)))?;

        // Decode the WAV bytes into raw f32 samples
        let audio_data = AudioData::from_wav_bytes(&wav_bytes)
            .map_err(|e| JsValue::from_str(&format!("WAV decoding failed: {}", e)))?;

        let num_samples = audio_data.samples.len() as u32;
        let num_channels: u32 = audio_data.num_channels as u32;
        let buffer_sample_rate = audio_data.sample_rate;

        console::log_1(
            &format!(
                "Audio buffer created at {} Hz. Context is {} Hz.",
                buffer_sample_rate,
                audio_context.sample_rate()
            )
            .into(),
        );

        // Create AudioBuffer using the CORRECT sample rate
        let buffer = audio_context
            .create_buffer(num_channels, num_samples, buffer_sample_rate as f32)
            .map_err(|e| JsValue::from_str(&format!("Failed to create audio buffer: {:?}", e)))?;

        // Copy samples into the buffer
        if num_channels == 1 {
            buffer
                .copy_to_channel(&audio_data.samples, 0)
                .map_err(|e| JsValue::from_str(&format!("Failed to copy samples: {:?}", e)))?;
        } else {
            return Err(JsValue::from_str(
                "Multi-channel sound not yet implemented for decoding",
            ));
        }

        Ok((buffer, num_channels, buffer_sample_rate))
    }
}
