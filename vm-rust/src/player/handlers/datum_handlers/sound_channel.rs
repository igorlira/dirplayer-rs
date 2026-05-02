use std::collections::VecDeque;

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{DatumRef, DirPlayer, ScriptError},
};

use std::sync::Arc;
use wasm_bindgen::JsCast;
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioScheduledSourceNode, Blob, GainNode,
    HtmlAudioElement, OfflineAudioContext, StereoPannerNode, Url,
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
use std::rc::Rc;

use js_sys::Uint8Array;
use log::{debug, error, warn};
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
        if let Datum::PropList(props, _) = prop_list {
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
            if let Datum::CastMember(cast_member_ref) = member_datum {
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
        handler_name: &str,
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
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        // Get the Rc<RefCell<SoundChannel>>
        let channel_rc = Self::get_sound_channel(player, datum)?;

        // Borrow the inner SoundChannel
        let channel = channel_rc.borrow();

        match prop {
            "volume" => Ok(Datum::Float(channel.volume as f64)),
            "duration" => Ok(Datum::Float(channel.get_duration() as f64)),
            "pan" => Ok(Datum::Float(channel.pan as f64)),
            "loopCount" => Ok(Datum::Int(channel.loop_count)),
            "loopsRemaining" => Ok(Datum::Int(channel.loops_remaining)),
            "startTime" => Ok(Datum::Float(channel.start_time as f64)),
            "endTime" => Ok(Datum::Float(channel.end_time as f64)),
            "loopStartTime" => Ok(Datum::Float(channel.loop_start_time as f64)),
            "loopEndTime" => Ok(Datum::Float(channel.loop_end_time as f64)),
            "elapsedTime" => Ok(Datum::Float(channel.elapsed_time as f64)),
            "sampleRate" => Ok(Datum::Int(channel.sample_rate.try_into().unwrap())),
            "sampleCount" => Ok(Datum::Int(channel.sample_count.try_into().unwrap())),
            "channelCount" => Ok(Datum::Int(channel.channel_count.into())),
            "status" => Ok(Datum::Int(channel.status.clone() as i32)),
            "member" => {
                match &channel.member {
                    Some(member_ref) => Ok(player.get_datum(member_ref).clone()),
                    None => Ok(Datum::Void),
                }
            }
            "currentTime" => {
                let ct = if channel.status == SoundStatus::Playing {
                    let elapsed = channel.audio_context.as_ref().map_or(0.0, |ctx| ctx.current_time()) - channel.playback_start_context_time;
                    (channel.start_time + elapsed * 1000.0).min(channel.get_duration() as f64)
                } else {
                    channel.elapsed_time
                };
                Ok(Datum::Float(ct))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get property {} for sound channel",
                prop
            ))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: &str,
        value_ref: &DatumRef,
    ) -> Result<(), ScriptError> {
        match prop {
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
        debug!("🎵 handle_play_member() - Playing member directly");

        let channel_rc = Self::get_sound_channel_mut(player, datum)?;

        // Clear any playlist and play this member directly
        {
            let mut ch = channel_rc.borrow_mut();
            ch.playlist_segments.clear();
            ch.playlist.clear();
            ch.current_segment_index = None;
            ch.stop_playback_nodes();
            
            ch.loop_count = 1;
            ch.loops_remaining = 1;
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
            ch.playback_start_context_time = ch.context_time();

            debug!("▶️ Starting async playlist playback");

            // Drop the borrow before spawning
            let channel_rc = channel.clone();
            drop(ch);

            // Spawn async task that doesn't block
            spawn_local(async move {
                SoundChannel::play_current_segment_async(channel_rc).await;
            });
        } else {
            warn!("⚠️ No playlist queued");
            ch.status = SoundStatus::Idle;
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

        // Reset loop_count to play-once (1) so stale loopCount settings don't leak
        {
            let mut ch = channel_rc.borrow_mut();
            ch.loop_count = 1;
            ch.loops_remaining = 1;
        }

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
        to_volume: f64,
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
        to_volume: f64,
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

        // Convert Lingo list or proplist
        let list_datum = player.get_datum(list_ref);
        let lingo_list = match list_datum {
            Datum::List(_, items, _) => items.clone(),
            Datum::PropList(items, _) => {
                // Empty proplist [:] means clear the playlist
                if items.is_empty() {
                    VecDeque::new()
                } else {
                    // Non-empty proplist is not valid for setPlayList
                    return Err(ScriptError::new(
                        "setPlayList expects a list, not a non-empty proplist".to_string()
                    ));
                }
            }
            _ => {
                return Err(ScriptError::new(format!(
                    "setPlayList expects a list or empty proplist, got {}",
                    list_datum.type_str()
                )));
            }
        };

        // --- 🧹 If the playlist is empty → clear both and exit early
        if lingo_list.is_empty() {
            let channel_rc = Self::get_sound_channel_mut(player, datum)?;
            let mut channel = channel_rc.borrow_mut();
            channel.playlist_segments.clear();
            channel.playlist.clear();
            channel.current_segment_index = None;
            channel.queued_members.clear();
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
                // Default loopCount to 1 when not provided (consistent with queue() and set_playlist())
                let loop_count = loopcount_value.unwrap_or(1);

                match (member_value, loop_count) {
                    // ✅ valid member and positive loopCount
                    (Some(_member_val), loop_count) if loop_count > 0 => {
                        // Store the original proplist ref (not the extracted member datum)
                        // to match queue() behavior - play_segment_for_member extracts #member from proplist
                        segments.push(SoundSegment {
                            member_ref: segment_ref.clone(),
                            loop_count,
                            loops_remaining: loop_count,
                        });
                        playlist.push(segment_ref.clone());
                    }

                    // ⚠️ loopCount == 0
                    (Some(_), 0) => {
                        warn!("  ⚠️ Skipped: loopCount is 0 (nothing to play)");
                    }

                    // ⚠️ loopCount negative (invalid)
                    (Some(_), loop_count) if loop_count < 0 => {
                        console::log_1(
                            &format!("  ⚠️ Skipped: invalid negative loopCount ({})", loop_count)
                                .into(),
                        );
                    }

                    // ⚠️ missing member entirely
                    (None, _) => {
                        warn!("  ⚠️ Skipped: missing member property");
                    }

                    // 🧩 fallback (compiler exhaustiveness guard)
                    _ => {
                        console::log_1(
                            &"  ⚠️ Unexpected combination of properties — skipped".into(),
                        );
                    }
                }
            } else {
                warn!("  ⚠️ Skipped: item is not a PropList");
            }
        }

        // --- ✅ Save results to the channel
        let channel_rc = Self::get_sound_channel_mut(player, datum)?;
        let mut channel = channel_rc.borrow_mut();
        channel.playlist_segments = segments;
        channel.playlist = playlist;

        if channel.status == SoundStatus::Playing {
            // Sound is currently playing — don't set current_segment_index.
            // When the current sound finishes, start_next_segment will discover
            // the new playlist entries via the "direct playback" fallback path.
            channel.current_segment_index = None;
        } else {
            channel.current_segment_index = None;
            if !channel.playlist_segments.is_empty() {
                channel.current_segment_index = Some(0);
            }
        }

        debug!("✅ Built {} valid playlist entries (playing={})", channel.playlist.len(), channel.status == SoundStatus::Playing);
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
            VecDeque::from(playlist),
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
        vol: f64,
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
        pan: f64,
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
        time: f64,
    ) -> Result<(), ScriptError> {
        let channel_rc = Self::get_sound_channel(player, datum)?;
        let mut channel = channel_rc.borrow_mut();
        channel.start_time = time.max(0.0);
        Ok(())
    }

    fn set_end_time(
        player: &mut DirPlayer,
        datum: &DatumRef,
        time: f64,
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
        time: f64,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().loop_start_time = time.max(0.0);
        Ok(())
    }

    fn set_loop_end_time(
        player: &mut DirPlayer,
        datum: &DatumRef,
        time: f64,
    ) -> Result<(), ScriptError> {
        let channel = Self::get_sound_channel_mut(player, datum)?;
        channel.borrow_mut().loop_end_time = time;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SoundStatus {
    Idle = 0,      // No sounds are queued or playing
    Loading = 1,   // A queued sound is being preloaded but not yet playing
    Queued = 2,    // The sound channel has finished preloading but is not yet playing
    Playing = 3,   // A sound is playing
    Paused = 4,    // A sound is paused
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
    // Add a getter that returns a JavaScript `Float32Array` from the Rust `Vec<f64>`
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
    audio_context: Option<Arc<AudioContext>>,
}

impl SoundManager {
    pub fn new(num_channels: usize) -> Result<Self, ScriptError> {
        #[cfg(target_arch = "wasm32")]
        let context = Some(Arc::new(getAudioContext()));
        #[cfg(not(target_arch = "wasm32"))]
        let context: Option<Arc<AudioContext>> = None;

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

    pub fn update(&mut self, delta_time: f64, player: &mut DirPlayer) -> Result<(), ScriptError> {
        for channel in &self.channels {
            // debug: log that update tick is processing this channel
            debug!("[CH{}] update tick", channel.borrow().channel_num);

            channel.borrow_mut().update(delta_time, player)?;
        }
        Ok(())
    }

    pub fn stop_all(&mut self) {
        for channel in &self.channels {
            channel.borrow_mut().stop();
        }
    }

    pub fn audio_context(&self) -> Option<Arc<AudioContext>> {
        self.audio_context.clone()
    }
}

#[derive(Clone)]
pub struct SoundChannel {
    pub channel_num: i32,
    pub member: Option<DatumRef>,
    pub sound_member: Option<SoundMember>,

    // Playback state
    pub volume: f64,
    pub pan: f64,
    pub loop_count: i32,
    pub loops_remaining: i32,
    pub start_time: f64,
    pub end_time: f64,
    pub loop_start_time: f64,
    pub loop_end_time: f64,
    pub status: SoundStatus,

    // Audio properties
    pub sample_rate: u32,
    pub sample_count: u32,
    pub channel_count: u16,
    pub elapsed_time: f64,

    // Fade state
    pub is_fading: bool,
    pub fade_start_volume: f64,
    pub fade_target_volume: f64,
    pub fade_duration: f64,
    pub fade_elapsed: f64,

    // Playlist and playback queue
    pub playlist_segments: Vec<SoundSegment>,
    pub playlist: Vec<DatumRef>,
    pub current_segment_index: Option<usize>,

    // 2. Wrap audio nodes in Rc to make them cloneable
    pub source_node: Option<Rc<AudioBufferSourceNode>>,
    pub gain_node: Option<Rc<GainNode>>,
    pub pan_node: Option<Rc<StereoPannerNode>>,

    pub queued_members: Vec<DatumRef>,

    // Web Audio backend
    pub audio_context: Option<Arc<AudioContext>>,
    pub current_audio_buffer: Option<Rc<AudioBuffer>>,

    pub expected_sample_rate: Option<u32>,
    pub is_decoding: Rc<RefCell<bool>>,
    pub decode_generation: Rc<RefCell<u32>>, // Incremented each time a new decode starts
    pub playback_start_context_time: f64, // AudioContext.currentTime when playback started

}

impl SoundChannel {
    fn audio_context(&self) -> &AudioContext {
        self.audio_context.as_ref().expect("AudioContext not available (non-wasm target?)")
    }

    pub fn context_time(&self) -> f64 {
        self.audio_context.as_ref().map_or(0.0, |ctx| ctx.current_time())
    }

    /// Find MP3 start with ROBUST validation (checks 3+ consecutive frames)
    /// Also verifies minimum data size to avoid false positives
    fn find_mp3_start(data: &[u8]) -> Option<usize> {
        const MIN_FRAMES_TO_VALIDATE: usize = 3;
        const MIN_MP3_SIZE: usize = 512; // Reduced for small Director sound effects (was 4096)
        
        // Skip if data is suspiciously small
        if data.len() < MIN_MP3_SIZE {
            console::log_1(&format!(
                "⚠️ Data too small for MP3 ({} bytes < {} min)",
                data.len(), MIN_MP3_SIZE
            ).into());
            return None;
        }
        
        for i in 0..data.len().saturating_sub(4) {
            if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 {
                // Calculate expected remaining data size
                let remaining = data.len() - i;
                
                // If MP3 start is found too late in the data, it's likely a false positive
                if remaining < MIN_MP3_SIZE {
                    continue;
                }
                
                if let Some(valid) = Self::validate_mp3_sequence(&data[i..], MIN_FRAMES_TO_VALIDATE) {
                    if valid {
                        console::log_1(&format!(
                            "✅ Valid MP3 sequence found at offset {} ({} bytes remaining, validated {} frames)",
                            i, remaining, MIN_FRAMES_TO_VALIDATE
                        ).into());
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    /// Validate multiple consecutive MP3 frames
    fn validate_mp3_sequence(data: &[u8], min_frames: usize) -> Option<bool> {
        let mut offset = 0;
        let mut frames_found = 0;
        
        while frames_found < min_frames && offset < data.len().saturating_sub(4) {
            if data[offset] != 0xFF || (data[offset + 1] & 0xE0) != 0xE0 {
                return Some(false);
            }
            
            let header = u32::from_be_bytes([
                data[offset],
                data[offset + 1],
                if offset + 2 < data.len() { data[offset + 2] } else { 0 },
                if offset + 3 < data.len() { data[offset + 3] } else { 0 },
            ]);
            
            let version = (header >> 19) & 0x3;
            let layer = (header >> 17) & 0x3;
            let bitrate_index = (header >> 12) & 0xF;
            let sample_rate_index = (header >> 10) & 0x3;
            
            if version == 1 || layer == 0 || bitrate_index == 0xF || 
            bitrate_index == 0 || sample_rate_index == 3 {
                return Some(false);
            }
            
            let frame_size = Self::calculate_mp3_frame_size(header);
            if frame_size == 0 || frame_size > 4096 {
                return Some(false);
            }
            
            frames_found += 1;
            offset += frame_size;
            
            if offset + 4 > data.len() {
                break;
            }
        }
        
        Some(frames_found >= min_frames || (frames_found > 0 && offset >= data.len() - 4))
    }

    /// Calculate MP3 frame size from header
    fn calculate_mp3_frame_size(header: u32) -> usize {
        const BITRATES: [[[u32; 16]; 4]; 4] = [
            // MPEG 2.5
            [
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],
                [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],
                [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0],
            ],
            // Reserved
            [
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ],
            // MPEG 2
            [
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],
                [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],
                [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0],
            ],
            // MPEG 1
            [
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0],
                [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0],
                [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0],
            ],
        ];
        
        const SAMPLE_RATES: [[u32; 4]; 4] = [
            [11025, 12000, 8000, 0],
            [0, 0, 0, 0],
            [22050, 24000, 16000, 0],
            [44100, 48000, 32000, 0],
        ];
        
        let version = ((header >> 19) & 0x3) as usize;
        let layer = ((header >> 17) & 0x3) as usize;
        let bitrate_index = ((header >> 12) & 0xF) as usize;
        let sample_rate_index = ((header >> 10) & 0x3) as usize;
        let padding = (header >> 9) & 0x1;
        
        if version >= 4 || layer >= 4 || bitrate_index >= 16 || sample_rate_index >= 4 {
            return 0;
        }
        
        let bitrate = BITRATES[version][layer][bitrate_index];
        let sample_rate = SAMPLE_RATES[version][sample_rate_index];
        
        if bitrate == 0 || sample_rate == 0 {
            return 0;
        }
        
        let frame_size = if layer == 3 {
            ((12 * bitrate * 1000 / sample_rate) + padding) * 4
        } else {
            (144 * bitrate * 1000 / sample_rate) + padding
        };
        
        frame_size as usize
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
        self.playback_start_context_time = self.context_time();

        Ok(())
    }

    pub fn new(channel: i32, audio_context: Option<Arc<AudioContext>>) -> Self {
        Self {
            channel_num: channel,
            member: None,
            sound_member: None,
            volume: 255.0,
            pan: 0.0,
            loop_count: 1,
            loops_remaining: 1,
            start_time: 0.0,
            end_time: 0.0,
            loop_start_time: 0.0,
            loop_end_time: 0.0,
            status: SoundStatus::Idle,
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
            queued_members: Vec::new(),
            audio_context,
            current_audio_buffer: None,
            expected_sample_rate: None,
            is_decoding: Rc::new(RefCell::new(false)),
            decode_generation: Rc::new(RefCell::new(0)),
            playback_start_context_time: 0.0,
        }
    }

    fn get_proplist_prop(player: &DirPlayer, prop_list: &Datum, key_name: &str) -> Option<Datum> {
        if let Datum::PropList(props, _) = prop_list {
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
        debug!("SND header bytes: {:02X?}", &header_bytes);

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
                    warn!("⚠️ No current segment to play");
                    return;
                }
            };
            let seg = match ch.playlist_segments.get(idx) {
                Some(s) => s.clone(),
                None => {
                    warn!("⚠️ Invalid segment index");
                    return;
                }
            };
            (
                seg.member_ref.clone(),
                ch.audio_context.clone().expect("Audio context not initialized"),
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
        debug!("🚀 About to spawn MP3 decode task (play_current_segment_async)");
        wasm_bindgen_futures::spawn_local(async move {
            debug!("mp3 task started");
            // call into existing play_file entry which handles MP3 and PCM paths
            SoundChannel::play_file(rc_clone, member_ref);
        });
        debug!("🚀 Spawned MP3 decode task (play_current_segment_async)");
    }

    /// Primary entry point for playing a sound member.
    pub fn play(&mut self, member_ref: DatumRef, loop_count: i32) -> Result<(), ScriptError> {
        // Store the member and loop count
        self.member = Some(member_ref.clone());
        self.loop_count = loop_count;
        self.loops_remaining = loop_count;

        // Set playing status
        self.status = SoundStatus::Playing;
        self.playback_start_context_time = self.context_time();
        self.elapsed_time = 0.0;

        Ok(())
    }

    // NOT USED
    async fn play_async(
        channel_rc: Rc<RefCell<Self>>,
        member_ref: DatumRef,
        loop_count: i32,
    ) -> Result<(), JsValue> {
        let player_opt = unsafe { crate::PLAYER_OPT.as_ref() };
        let player = player_opt.ok_or_else(|| JsValue::from_str("Player not initialized"))?;

        // FIX: audio_context is an Arc, so we dereference it to get a reference (&AudioContext)
        let audio_context = player.sound_manager.audio_context().expect("Audio context not initialized");

        // Load and create the buffer
        // FIX: Dereferencing Arc<AudioContext> to pass &AudioContext
        let (audio_buffer, _channels, _sample_rate) = channel_rc
            .borrow_mut()
            .load_audio_data(&*audio_context, &member_ref)
            .await?;

        let mut channel = channel_rc.borrow_mut();

        // Disconnect and stop any existing nodes
        if let Some(source) = channel.source_node.take() {
            let scheduled: &AudioScheduledSourceNode = source.as_ref();
            let _ = scheduled.stop();
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
        if let Datum::CastMember(member_ref) = datum {
            let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref)?;
            match &cast_member.member_type {
                CastMemberType::Sound(sound_member) => return Some(sound_member.clone()),
                _ => {
                    warn!("⚠️ CastMember is not a sound");
                    return None;
                }
            }
        }

        // Case 2: String member name lookup
        if let Ok(member_name) = datum.string_value() {
            // Find member by name
            if let Some(member_ref) = player.movie.cast_manager.find_member_ref_by_name(&member_name) {
                let cast_member = player.movie.cast_manager.find_member_by_ref(&member_ref)?;
                match &cast_member.member_type {
                    CastMemberType::Sound(sound_member) => return Some(sound_member.clone()),
                    _ => {
                        warn!("⚠️ Member '{}' is not a sound", member_name);
                        return None;
                    }
                }
            } else {
                warn!("⚠️ Member '{}' not found", member_name);
                return None;
            }
        }

        // Case 3: PropList with #member property (for playlist items)
        let member_datum = Self::get_proplist_prop(player, datum, "member")?;

        match member_datum {
            Datum::CastMember(ref member_ref) => {
                let cast_member = player.movie.cast_manager.find_member_by_ref(member_ref)?;
                match &cast_member.member_type {
                    CastMemberType::Sound(sound_member) => Some(sound_member.clone()),
                    _ => {
                        warn!("⚠️ CastMember is not a sound");
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
        debug!("▶️ SoundChannel::play_file() called with {:?}", member_ref);

        // Director behavior: playing a sound on a channel immediately stops any current sound
        // and starts the new one - no queueing
        {
            let mut channel = self_rc.borrow_mut();

            web_sys::console::log_1(
                &format!(
                    "play_file(): channel={} status={:?} queued={} current_idx={:?}",
                    channel.channel_num,
                    channel.status,
                    channel.queued_members.len(),
                    channel.current_segment_index
                )
                .into(),
            );

            // Stop any currently playing sound
            if channel.status == SoundStatus::Playing || channel.status == SoundStatus::Loading {
                debug!("🛑 Stopping current sound to play new one");
                channel.stop_playback_nodes();
            }

            debug!("🔄 Ready to play immediately");
            channel.status = SoundStatus::Idle;
            channel.queued_members.clear();

            if channel.loop_count == 0 || channel.loop_count > 1 {
                channel.member = Some(member_ref.clone());
                debug!("📦 Stored member_ref for looping (loop_count={})", channel.loop_count);
            }

            // Only set current_segment_index if it's not already set (first time playing from playlist)
            // or if we can't find the member in the playlist (external play call)
            if channel.current_segment_index.is_none() {
                for (idx, segment) in channel.playlist_segments.iter().enumerate() {
                    if segment.member_ref == member_ref {
                        channel.current_segment_index = Some(idx);
                        debug!("📍 Set current_segment_index to {}", idx);
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
        let (channel_num, audio_context, loop_count) = {
            let mut this = self_rc.borrow_mut();
            
            if let Some(ref ctx) = this.audio_context {
                let state = ctx.state();
                debug!("🎵 AudioContext state: {:?}", state);

                if state == web_sys::AudioContextState::Suspended {
                    let resume_result = ctx.resume();
                    debug!("🎵 AudioContext resume result: {:?}", resume_result);
                }
            }

            console::log_1(
                &format!(
                    "▶️ SoundChannel::start_sound() called with {:?}",
                    member_ref
                )
                .into(),
            );

            // Reset status
            if this.status != SoundStatus::Idle {
                this.status = SoundStatus::Idle;
            }

            // Stop current sound if playing
            if this.status == SoundStatus::Playing {
                this.queued_members.push(member_ref.clone());
                return;
            }
            
            (this.channel_num, this.audio_context.clone().unwrap(), this.loop_count)
        };

        let player_opt = unsafe { crate::PLAYER_OPT.as_mut() };
        let player = match player_opt {
            Some(p) => p,
            None => {
                error!("❌ No global player found");
                return;
            }
        };

        // Retrieve datum
        let datum = player.get_datum(&member_ref);

        if let Some(sound_member) = Self::resolve_sound_member(player, &datum) {
            // Update expected sample rate
            {
                let mut this = self_rc.borrow_mut();
                this.expected_sample_rate = Some(sound_member.info.sample_rate);
                this.sample_rate = sound_member.info.sample_rate;
                this.sample_count = sound_member.info.sample_count;
                this.channel_count = sound_member.info.channels;
            }

            // Load audio data
            let audio_data = match Self::load_director_audio_data(
                &sound_member.sound.data(),
                sound_member.info.channels,
                sound_member.info.sample_rate,
                sound_member.info.sample_size,
                &sound_member.sound.codec(),
                Some(sound_member.info.sample_count),
                sound_member.sound.big_endian_data(),
            ) {
                Ok(data) => data,
                Err(e) => {
                    error!("❌ Failed to load sound: {}", e);
                    let mut this = self_rc.borrow_mut();
                    this.status = SoundStatus::Idle;
                    return;
                }
            };

            // Check if this is MP3 data
            if let Some(mp3_bytes) = &audio_data.compressed_data {
                debug!("🎵 MP3 detected! {} bytes", mp3_bytes.len());

                let self_rc_clone = self_rc.clone();
                let mp3_data = mp3_bytes.clone();

                debug!("🚀 About to spawn MP3 decode task (start_sound)");
                wasm_bindgen_futures::spawn_local(async move {
                    {
                        let mut ch = self_rc_clone.borrow_mut();
                        ch.status = SoundStatus::Loading;
                    }

                    if let Err(e) = Self::start_sound_mp3_async(self_rc_clone.clone(), mp3_data.clone()).await {
                        error!("❌ MP3 playback failed: {:?}", e);
                        debug!("📊 MP3 data size: {} bytes, first bytes: {:02X?}", 
                            mp3_data.len(), &mp3_data[0..32.min(mp3_data.len())]);
                        {
                            let mut ch = self_rc_clone.borrow_mut();
                            ch.status = SoundStatus::Idle;
                        }
                        Self::start_sound_pcm_fallback(self_rc_clone, member_ref);
                    }
                });
                debug!("🚀 Spawned MP3 decode task (start_sound)");
                return;
            }

            // PCM/ADPCM path - USE BROWSER RESAMPLING
            if audio_data.samples.is_empty() {
                error!("❌ Audio data has no samples");
                let mut this = self_rc.borrow_mut();
                this.status = SoundStatus::Idle;
                return;
            }

            let source_sample_rate = audio_data.sample_rate as f64;
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
                source_sample_rate as f32,
            ) {
                Ok(buf) => buf,
                Err(_) => {
                    error!("❌ Failed to create source AudioBuffer");
                    let mut this = self_rc.borrow_mut();
                    this.status = SoundStatus::Idle;
                    return;
                }
            };

            // Copy samples to source buffer
            for ch in 0..num_channels {
                let mut channel_data = vec![0.0f32; num_frames];
                for frame in 0..num_frames {
                    let idx = frame * num_channels as usize + ch as usize;
                    channel_data[frame] = audio_data.samples[idx] as f32;
                }
                let _ = source_buffer.copy_to_channel(&channel_data, ch as i32);
            }

            debug!("🔄 Starting async resampling...");

            let (channel_num, volume, pan_value) = {
                let this = self_rc.borrow();
                (this.channel_num, this.volume, this.pan)
            };

            // Spawn async resampling task
            let self_rc_clone = self_rc.clone();
            debug!("🚀 About to spawn MP3 decode task (start_sound #2)");
            
            let my_generation = {
                let mut ch = self_rc.borrow_mut();
                *ch.decode_generation.borrow_mut() += 1;
                let gen_ = *ch.decode_generation.borrow();
                gen_  // Return the copied value
            };
            
            wasm_bindgen_futures::spawn_local(async move {

                {
                    let mut ch = self_rc_clone.borrow_mut();
                    ch.status = SoundStatus::Loading;
                    *ch.is_decoding.borrow_mut() = true;
                }

                // Resample using OfflineAudioContext
                let resampled_buffer =
                    match Self::resample_audio_buffer(&source_buffer, target_sample_rate).await {
                        Ok(buf) => {
                            debug!("✅ Resampled to {} Hz", buf.sample_rate());
                            buf
                        }
                        Err(e) => {
                            warn!("⚠️ Resampling failed: {:?}, using original", e);
                            source_buffer
                        }
                    };

                // Check if decoding was cancelled
                {
                    let ch = self_rc_clone.borrow();
                    let current_gen = *ch.decode_generation.borrow();
                    if current_gen != my_generation {
                        warn!("⚠️ Channel {} PCM decode was cancelled (gen {} != {}), not starting playback", channel_num, my_generation, current_gen);
                        return;
                    }
                }

                // Now play the resampled buffer
                let mut ch = self_rc_clone.borrow_mut();

                // Create source node
                let source = match ch.audio_context().create_buffer_source() {
                    Ok(s) => s,
                    Err(_) => {
                        web_sys::console::log_1(
                            &"❌ Failed to create AudioBufferSourceNode".into(),
                        );
                        return;
                    }
                };
                source.set_buffer(Some(&resampled_buffer));
                source.set_loop(loop_count == 0); 

                // Create gain node
                let gain = match ch.audio_context().create_gain() {
                    Ok(g) => g,
                    Err(_) => {
                        error!("❌ Failed to create GainNode");
                        return;
                    }
                };
                gain.gain().set_value((volume / 255.0) as f32);

                // Create pan node
                let pan = match ch.audio_context().create_stereo_panner() {
                    Ok(p) => p,
                    Err(_) => {
                        error!("❌ Failed to create StereoPannerNode");
                        return;
                    }
                };
                pan.pan().set_value((pan_value / 100.0) as f32);

                // Connect audio graph
                let _ = source.connect_with_audio_node(&pan);
                let _ = pan.connect_with_audio_node(&gain);
                let _ = gain.connect_with_audio_node(&ch.audio_context().destination());

                // Store state BEFORE setting up callback (we need ch dropped)
                ch.source_node = Some(Rc::new(source.clone()));
                ch.gain_node = Some(Rc::new(gain));
                ch.pan_node = Some(Rc::new(pan));
                ch.status = SoundStatus::Playing;
                ch.playback_start_context_time = ch.context_time();
                ch.elapsed_time = 0.0;
                *ch.is_decoding.borrow_mut() = false;

                drop(ch);

                // Set up ended callback
                let self_rc_clone2 = self_rc_clone.clone();
                let closure = Closure::<dyn FnMut()>::new(move || {
                    debug!("🔚 Channel {} sound ended", channel_num);

                    let mut ch = self_rc_clone2.borrow_mut();
                    
                    // Only proceed if we were actually playing (not stopped early)
                    if ch.status != SoundStatus::Playing {
                        warn!("⚠️ Channel {} was already stopped, ignoring ended event", channel_num);
                        return;
                    }
                    
                    ch.status = SoundStatus::Idle;
                    ch.source_node = None;
                    ch.start_next_segment();
                });

                let _ = source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref());
                closure.forget();

                // Start playback (ONLY ONCE)
                let _ = source.start();

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
            debug!("🚀 Spawned MP3 decode task (start_sound #2)");
        } else {
            let mut this = self_rc.borrow_mut();
            this.status = SoundStatus::Idle;
            error!("❌ start_sound failed - couldn't get sound member");
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
            debug!("✅ Sample rates match, no resampling needed");
            return Ok(buffer.clone());
        }

        debug!("🔄 Resampling {} Hz → {} Hz", current_rate, target_rate);

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
            error!("❌ Failed to start rendering: {:?}", e);
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
                warn!("⚠️ Channel {} already decoding – skipping", ch.channel_num);
                return Ok(());
            }
        }

        // Set decoding flag and increment generation
        let my_generation = {
            let mut ch = self_rc.borrow_mut();
            *ch.is_decoding.borrow_mut() = true;
            *ch.decode_generation.borrow_mut() += 1;
            let gen_ = *ch.decode_generation.borrow();
            gen_  // Return the copied value
        };
        
        debug!("🔢 Starting decode generation {}", my_generation);

        // Ensure flag is cleared on exit
        let clear_flag = || {
            let mut ch = self_rc.borrow_mut();
            *ch.is_decoding.borrow_mut() = false;
        };

        // Extract valid MP3 frames
        let clean_mp3 = match Self::extract_valid_mp3_frames(&mp3_bytes) {
            Some(data) => data,
            None => {
                error!("❌ No valid MP3 frames found – treating as PCM");
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
                warn!("⚠️ MP3 data suspiciously small: {} bytes", clean_mp3.len());
            }
        }

        // Create ArrayBuffer from cleaned MP3 data
        let arr = Uint8Array::from(&clean_mp3[..]);
        let buf = arr.buffer();

        // Get AudioContext
        let (ctx, channel_num) = {
            let ch = self_rc.borrow();
            (ch.audio_context.clone().unwrap(), ch.channel_num)
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
                error!("❌ MP3 decoding failed: {:?}", e);
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

        let final_buffer = audio_buffer;

        // // Handle sample rate if needed
        // let expected_rate_opt = {
        //     let ch = self_rc.borrow();
        //     ch.expected_sample_rate
        // };

        // console::log_1(
        //     &format!(
        //         "✅ Using browser-decoded MP3 at {} Hz (no resampling needed)",
        //         audio_buffer.sample_rate()
        //     )
        //     .into(),
        // );

        // let final_buffer = if let Some(expected_rate) = expected_rate_opt {
        //     let expected_rate_f = expected_rate as f32;
        //     if (audio_buffer.sample_rate() as f32 - expected_rate_f).abs() > 1.0 {
        //         console::log_1(
        //             &format!(
        //                 "🔄 Resampling {} -> {} Hz",
        //                 audio_buffer.sample_rate(),
        //                 expected_rate_f
        //             )
        //             .into(),
        //         );

        //         match Self::resample_audio_buffer(&audio_buffer, expected_rate_f).await {
        //             Ok(rbuf) => {
        //                 console::log_1(
        //                     &format!("✅ Resampled buffer: {} samples", rbuf.length()).into(),
        //                 );
        //                 rbuf
        //             }
        //             Err(e) => {
        //                 console::log_1(
        //                     &format!("⚠️ Resample failed: {:?}, using original", e).into(),
        //                 );
        //                 audio_buffer
        //             }
        //         }
        //     } else {
        //         audio_buffer
        //     }
        // } else {
        //     audio_buffer
        // };

        // Create source node
        let source = ctx.create_buffer_source()?;

        // Create gain node
        let gain = ctx.create_gain()?;
        let volume = {
            let ch = self_rc.borrow();
            ch.volume
        };
        gain.gain().set_value((volume / 255.0) as f32);
        debug!("🔊 Setting gain to {} (volume: {})", volume / 255.0, volume);

        // Create pan node if available
        if let Ok(pan_node) = ctx.create_stereo_panner() {
            let pan_value = {
                let ch = self_rc.borrow();
                ch.pan
            };
            pan_node.pan().set_value((pan_value / 100.0) as f32);
            let _ = source.connect_with_audio_node(&pan_node);
            let _ = pan_node.connect_with_audio_node(&gain);
        } else {
            let _ = source.connect_with_audio_node(&gain);
        }

        let _ = gain.connect_with_audio_node(&ctx.destination());

        source.set_buffer(Some(&final_buffer));

        // CRITICAL: Check if decoding was cancelled before we start playback
        {
            let ch = self_rc.borrow();
            let current_gen = *ch.decode_generation.borrow();
            if current_gen != my_generation {
                warn!("⚠️ Channel {} decode was cancelled (gen {} != {}), not starting playback", channel_num, my_generation, current_gen);
                return Ok(());
            }
        }

        // Set up ended callback with proper lifetime management
        let self_rc_clone = self_rc.clone();

        let loop_count = {
            let ch = self_rc_clone.borrow();
            ch.loop_count
        };

        if loop_count == 0 {
            source.set_loop(true);
            debug!("🔁 Enabled native WebAudio looping (loop_count=0)");
        } else {
            source.set_loop(false);
        }

        let closure = Closure::<dyn FnMut()>::new(move || {
            debug!("🔚 Channel {} MP3 ended", channel_num);
            
            let mut ch = self_rc_clone.borrow_mut();
            debug!("Setting status from {:?} to Idle", ch.status);
            
            // Only proceed if we were actually playing (not stopped early)
            if ch.status != SoundStatus::Playing {
                warn!("⚠️ Channel {} was already stopped, ignoring ended event", channel_num);
                return;
            }
            
            ch.status = SoundStatus::Idle;
            ch.source_node = None;  // Clear the source node
            ch.start_next_segment();
        });

        let _ = source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref());
        closure.forget();

        // Start playback
        source.start()?;
        debug!("▶️ MP3 source.start() called");

        // Store nodes in channel state AFTER starting (only once!)
        {
            let mut ch = self_rc.borrow_mut();
            ch.source_node = Some(Rc::new(source));
            ch.gain_node = Some(Rc::new(gain));
            ch.status = SoundStatus::Playing;
            ch.playback_start_context_time = ch.context_time();
            ch.elapsed_time = 0.0;
            *ch.is_decoding.borrow_mut() = false;
        }

        debug!("✅ Channel {} MP3 playback started", channel_num);

        Ok(())
    }

    /// PCM fallback when MP3 decoding fails
    fn start_sound_pcm_fallback(self_rc: Rc<RefCell<Self>>, member_ref: DatumRef) {
        debug!("🔄 Starting PCM fallback playback");

        let mut this = self_rc.borrow_mut();

        // Get global player
        let player_opt = unsafe { crate::PLAYER_OPT.as_mut() };
        let player = match player_opt {
            Some(p) => p,
            None => {
                error!("❌ No global player found");
                return;
            }
        };

        // Retrieve datum
        let datum = player.get_datum(&member_ref);

        if let Some(sound_member) = Self::resolve_sound_member(player, &datum) {
            let audio_context = this.audio_context.clone().unwrap();

            // CRITICAL FIX: Don't check for MP3 patterns here!
            // If MP3 decoding failed and we're in fallback, just try PCM.
            // The false positive MP3 detection was preventing any sound playback.

            debug!("🔧 Forcing PCM decoding (ignoring any MP3-like patterns)");

            // Force PCM decoding by treating as raw PCM (NOT MP3)
            let pcm_wav = match Self::load_director_sound_from_bytes(
                &sound_member.sound.data(),
                sound_member.info.channels,
                sound_member.info.sample_rate,
                sound_member.info.sample_size,
                "raw_pcm", // ← Force raw_pcm codec to avoid MP3 detection
                Some(sound_member.info.sample_count),
                sound_member.sound.big_endian_data(),
            ) {
                Ok(wav) => wav,
                Err(e) => {
                    error!("❌ PCM fallback failed: {}", e);
                    this.status = SoundStatus::Idle;
                    return;
                }
            };

            // Decode WAV to AudioData
            let audio_data = match AudioData::from_wav_bytes(&pcm_wav) {
                Ok(data) => data,
                Err(e) => {
                    error!("❌ WAV decode failed: {}", e);
                    this.status = SoundStatus::Idle;
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
                error!("❌ Audio data has no samples");
                this.status = SoundStatus::Idle;
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
                target_sample_rate as f32,
            ) {
                Ok(buf) => buf,
                Err(_) => {
                    error!("❌ Failed to create AudioBuffer");
                    this.status = SoundStatus::Idle;
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

                    let sample1 = audio_data.samples[idx1] as f32;
                    let sample2 = audio_data.samples[idx2] as f32;
                    channel_data[frame] = sample1 + (sample2 - sample1) * frac;
                }

                let _ = buffer.copy_to_channel(&channel_data, ch as i32);
            }

            // CRITICAL FIX: Wrap in Rc::new() for Rc<AudioBuffer>
            this.current_audio_buffer = Some(Rc::new(buffer.clone()));

            // Create and connect source node
            let source = match audio_context.create_buffer_source() {
                Ok(s) => s,
                Err(_) => {
                    error!("❌ Failed to create buffer source");
                    this.status = SoundStatus::Idle;
                    return;
                }
            };

            source.set_buffer(Some(&buffer));
            
            // Set up looping if needed
            let loop_count = this.loop_count;
            if loop_count == 0 {
                source.set_loop(true);
            } else {
                source.set_loop(false);
            }

            // Create gain node
            let gain = match audio_context.create_gain() {
                Ok(g) => g,
                Err(_) => {
                    error!("❌ Failed to create gain node");
                    this.status = SoundStatus::Idle;
                    return;
                }
            };

            let volume = this.volume;
            gain.gain().set_value((volume / 255.0) as f32);

            // Create pan node
            let pan = match audio_context.create_stereo_panner() {
                Ok(p) => p,
                Err(_) => {
                    error!("❌ Failed to create pan node");
                    this.status = SoundStatus::Idle;
                    return;
                }
            };

            let pan_value = this.pan;
            pan.pan().set_value(pan_value as f32);

            // Connect the audio graph
            let _ = source.connect_with_audio_node(&gain);
            let _ = gain.connect_with_audio_node(&pan);
            let _ = pan.connect_with_audio_node(&audio_context.destination());

            // Set up the onended callback before starting
            let channel_index = this.channel_num;
            let closure = Closure::<dyn FnMut()>::new(move || {
                SoundChannel::handle_end_of_sound(channel_index);
            });
            source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref())
                .unwrap_or_else(|e| {
                    warn!("⚠️ Failed to add ended listener: {:?}", e);
                });
            closure.forget();

            // Start playback
            let _ = source.start();

            debug!("✅ PCM fallback playback started successfully");

            // CRITICAL FIX: Wrap all nodes in Rc::new()
            this.source_node = Some(Rc::new(source));
            this.gain_node = Some(Rc::new(gain));
            this.pan_node = Some(Rc::new(pan));
            this.status = SoundStatus::Playing;
            this.playback_start_context_time = this.context_time();
        } else {
            error!("❌ Could not resolve sound member");
            this.status = SoundStatus::Idle;
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

        debug!("✅ Extracted {} bytes of MP3 data", mp3_data.len());

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

        error!("❌ MP3 validation failed");
        None
    }

    pub fn play_next(&mut self) {
        debug!("SoundChannel({:?}) -> playNext", self.channel_num);

        self.start_next_segment();
    }

    pub fn stop(&mut self) {
        self.status = SoundStatus::Idle;
        self.elapsed_time = 0.0;
        self.loops_remaining = 0;
        self.is_fading = false;

        if let Some(ref source) = self.source_node {
            let scheduled: &AudioScheduledSourceNode = source.as_ref();
            let _ = scheduled.stop_with_when(0.0);
            let _ = source.disconnect();
            debug!("🛑 Stopped previous sound");
        }

        self.source_node = None;
    }

    pub fn pause(&mut self) {
        if self.status == SoundStatus::Playing {
            self.status = SoundStatus::Paused;

            if let Some(ref source) = self.source_node {
                // Suspend the audio context — stops all nodes temporarily
                if let Some(ref ctx) = self.audio_context {
                    let _ = ctx.suspend();
                }
                debug!("⏸️ Paused playback");
            }
        }
    }

    pub fn resume(&mut self) {
        if self.status == SoundStatus::Paused {
            self.status = SoundStatus::Playing;
            // Don't reset playback_start_context_time on resume - original start is still valid

            if let Some(ref source) = self.source_node {
                // Resume the AudioContext
                if let Some(ref ctx) = self.audio_context {
                    let _ = ctx.resume();
                }
                debug!("▶️ Resumed playback");
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
    //         debug!("MP3 detected → storing raw compressed bytes");
    //         data.to_vec()
    //     } else if is_probably_adpcm {
    //         debug!("IMA ADPCM detected → decoding...");
    //         Self::decode_ima_adpcm(data, expected_samples.unwrap_or(0))?
    //     } else if is_probably_8bit {
    //         debug!("8-bit PCM detected → converting to 16-bit");
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
    //         debug!("Assuming PCM16 → normalizing endianness");
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
    //         sample_count as f64 / sample_rate as f64
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
        big_endian: bool,
    ) -> Result<Vec<u8>, String> {
        use web_sys::console;
        if sound_bytes.is_empty() {
            return Err("sound_bytes empty".into());
        }

        // --- STEP 1: Check for residual headers ---
        // SoundChunk and MediaChunk now store only audio data (headers stripped at parse time).
        // Check for Mac snd resource signature in case unstripped data arrives.
        let header_size = if sound_bytes.len() >= 4
            && sound_bytes[0] == 0x00
            && (sound_bytes[1] == 0x01 || sound_bytes[1] == 0x02)
        {
            // Looks like a Mac snd resource (type 1 or 2) that wasn't stripped.
            // Try to skip past it. Most type 1 with 1 modifier + 1 command + extended header = 84 bytes.
            // This is a best-effort fallback; proper stripping happens in from_snd_chunk.
            debug!("Detected unstripped snd resource header in audio data");
            if sound_bytes.len() >= 84 { 84 } else { 0 }
        } else {
            0
        };
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
        // Check for MP3, but skip when data size matches expected raw PCM size.
        let bytes_per_sample_est = if bits_per_sample > 0 { bits_per_sample as usize / 8 } else { 2 };
        let expected_pcm_size = expected_samples
            .filter(|&s| s > 0)
            .map(|s| s as usize * channels as usize * bytes_per_sample_est);
        let data_likely_pcm = expected_pcm_size
            .map_or(false, |expected| data.len() >= expected * 4 / 5);
        let is_mp3 = if data_likely_pcm { false } else { Self::find_mp3_start(data).is_some() };
        let is_probably_adpcm = !is_mp3 && codec.contains("ima");
        // Only use byte-distribution heuristic when bits_per_sample is unknown (0).
        // When metadata explicitly says 16-bit, trust it — the heuristic can false-positive
        // on 16-bit big-endian audio where byte values cluster in certain ranges.
        let is_probably_8bit = !is_mp3
            && (bits_per_sample == 8
                || (bits_per_sample == 0
                    && data.len() >= 100
                    && data[0..100]
                        .iter()
                        .filter(|&&b| b >= 0x60 && b <= 0xA0)
                        .count()
                        > 50));

        let (is_mp3, is_probably_adpcm, is_probably_8bit) = (is_mp3, is_probably_adpcm, is_probably_8bit);

        console::log_1(
            &format!(
                "Detected: MP3={}, ADPCM={}, 8-bit={}, bits={}, rate={}, ch={}",
                is_mp3, is_probably_adpcm, is_probably_8bit, bits_per_sample, sample_rate, channels
            )
            .into(),
        );

        // --- STEP 3: Decode/normalize ---
        let pcm_data = if is_mp3 {
            debug!("MP3 detected → storing raw compressed bytes");
            data.to_vec()
        } else if is_probably_adpcm {
            debug!("IMA ADPCM detected → decoding...");

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

            debug!("✅ ADPCM decoded: {} samples", decoded_pcm_samples.len());

            let mut converted = Vec::with_capacity(decoded_pcm_samples.len() * 2);
            for &sample in &decoded_pcm_samples {
                converted.extend_from_slice(&sample.to_le_bytes());
            }
            converted
        } else if is_probably_8bit {
            debug!("8-bit PCM detected → converting to 16-bit");
            let mut converted = Vec::with_capacity(data.len() * 2);
            for &byte in data {
                let sample_16 = ((byte as i32 - 128) * 257) as i16;
                converted.extend_from_slice(&sample_16.to_le_bytes());
            }
            converted
        } else {
            debug!("Assuming PCM16 → big_endian={}", big_endian);
            if bits_per_sample == 16 && big_endian {
                let mut converted = Vec::with_capacity(data.len());

                debug!("🔄 Big-endian 16-bit → swapping bytes to little-endian");
                for chunk in data.chunks_exact(2) {
                    converted.push(chunk[1]);
                    converted.push(chunk[0]);
                }
                if data.len() % 2 == 1 {
                    converted.push(*data.last().unwrap());
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
        big_endian: bool,
    ) -> Result<AudioData, String> {
        console::log_1(&format!(
            "=== load_director_audio_data ===\nTotal file size: {} bytes\nChannels: {}, Sample Rate: {} Hz, Bits: {}, Codec: '{}', Expected samples: {:?}",
            sound_bytes.len(), channels, sample_rate, bits_per_sample, codec, expected_samples
        ).into());
        
        // Check for MP3, but skip when data size matches expected raw PCM.
        // sndH/sndS headers sometimes claim "raw_pcm" when data is actually MP3-compressed.
        // We detect this by comparing data size to what raw PCM would need.
        // If the data is close to expected PCM size, it IS PCM and MP3 patterns are false positives.
        let bytes_per_sample_est = if bits_per_sample > 0 { bits_per_sample as usize / 8 } else { 2 };
        let expected_pcm_size = expected_samples
            .filter(|&s| s > 0)
            .map(|s| s as usize * channels as usize * bytes_per_sample_est);
        let data_likely_pcm = expected_pcm_size
            .map_or(false, |expected| sound_bytes.len() >= expected * 4 / 5);
        let mp3_start = if data_likely_pcm {
            None
        } else {
            Self::find_mp3_start(sound_bytes)
        };

        if let Some(mp3_start) = mp3_start {
            console::log_1(
                &format!(
                    "✅ Valid MP3 sequence found at offset {} (0x{:04X}) - using MP3 decoder (codec was '{}')",
                    mp3_start, mp3_start, codec
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
        
        // No MP3 found - treat as PCM/ADPCM
        debug!("📝 No MP3 detected - processing as PCM/ADPCM (codec='{}')", codec);
        let wav_bytes = Self::load_director_sound_from_bytes(
            sound_bytes,
            channels,
            sample_rate,
            bits_per_sample,
            codec,
            expected_samples,
            big_endian,
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

        let blob_opts = web_sys::BlobPropertyBag::new();
        blob_opts.set_type("audio/mpeg");
        let blob = Blob::new_with_u8_array_sequence_and_options(
            &blob_parts,
            &blob_opts,
        )?;

        debug!("✅ Blob created: {} bytes", blob.size());

        // Create an object URL for the Blob
        let url = Url::create_object_url_with_blob(&blob).map_err(|e| {
            console::error_1(&format!("Failed to create object URL: {:?}", e).into());
            e
        })?;

        debug!("🔗 Object URL created: {}", url);

        // Create an audio element and play it
        let audio = HtmlAudioElement::new().map_err(|e| {
            console::error_1(&"Failed to create audio element".into());
            e
        })?;

        audio.set_src(&url);

        // Play the audio
        audio.play().map_err(|e| {
            console::error_1(&format!("Failed to play audio: {:?}", e).into());
            e
        })?;

        debug!("▶️ MP3 playback started");

        Ok(())
    }

    /// Alternative: Decode MP3 to WAV using Web Audio API (for audio context integration)
    pub async fn decode_mp3_to_wav(
        audio_context: &AudioContext,
        mp3_bytes: &[u8],
    ) -> Result<AudioBuffer, JsValue> {
        debug!("🔄 Decoding MP3 data via Web Audio API");

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
        debug!("🎵 Starting MP3 playback via Web Audio");

        // Step 1: Decode MP3 to AudioBuffer
        let audio_buffer = Self::decode_mp3_to_wav(self.audio_context(), mp3_bytes).await?;

        // Step 3: Stop any existing playback
        if let Some(ref source) = self.source_node {
            let scheduled: &AudioScheduledSourceNode = source.as_ref();
            let _ = scheduled.stop_with_when(0.0);
            let _ = source.disconnect();
        }
        self.source_node = None;

        // Step 4: Create new source node
        let source = self.audio_context().create_buffer_source()?;
        source.set_buffer(Some(&audio_buffer));
 
        // Create FRESH gain and pan nodes
        let gain = match self.audio_context().create_gain() {
            Ok(g) => g,
            Err(_) => {
                error!("❌ Failed to create GainNode");
                return Ok(());
            }
        };

        let volume = self.volume;
        gain.gain().set_value((volume / 255.0) as f32);
        debug!("🔊 Setting gain to {} (volume: {})", volume / 255.0, volume);

        let pan = match self.audio_context().create_stereo_panner() {
            Ok(p) => p,
            Err(_) => {
                error!("❌ Failed to create StereoPannerNode");
                return Ok(());
            }
        };

        let pan_value = self.pan;
        pan.pan().set_value((pan_value / 100.0) as f32);

        let _ = source.connect_with_audio_node(&pan);
        let _ = pan.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(&self.audio_context().destination());

        // Step 6: Set up on-ended callback
        let channel_index = self.channel_num;
        let closure = Closure::<dyn FnMut()>::new(move || {
            SoundChannel::handle_end_of_sound(channel_index);
        });

        let _ = source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref());
        closure.forget();

        // Step 7: Start playback
        source.start()?;
        debug!("called source.start()");
        self.source_node = Some(Rc::new(source));
        self.status = SoundStatus::Playing;
        self.playback_start_context_time = self.context_time();

        debug!("✅ MP3 playback started on channel {}", self.channel_num);

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
        debug!("ℹ️ Wrapped WAV length: {} bytes", wrapped.len());

        if Self::is_wav_format(data) && data.len() >= 44 {
            // Try normal WAV parsing first
            match Self::load_wav(data) {
                Ok(audio) if audio.sample_rate > 0 && !audio.samples.is_empty() => {
                    return Ok(audio)
                }
                _ => {
                    warn!("⚠️ Invalid WAV header, wrapping as proper WAV");
                    let wrapped_wav = Self::wrap_director_wav(data);
                    return Self::load_wav(&wrapped_wav);
                }
            }
        } else if Self::is_aiff_format(data) {
            Self::load_aiff(data)
        } else {
            // raw PCM or headerless WAV
            debug!("ℹ️ Raw PCM detected, wrapping into WAV");
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
            warn!("⚠️ WAV file too small");
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
            warn!("⚠️ Unsupported bit depth: {}", bits_per_sample);
            return Err(format!("Unsupported bit depth: {}", bits_per_sample));
        };

        debug!("🎧 WAV loaded: {} samples", samples.len());

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
            samples: Vec::new(), // keep as Vec<f64>, not Float32Array
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

        if self.status != SoundStatus::Idle {
            self.status = SoundStatus::Idle;
        }
    }

    pub fn queue(&mut self, datum_ref: DatumRef, player: &DirPlayer) {
        use web_sys::console;

        let datum = player.get_datum(&datum_ref);

        let props = match datum {
            Datum::PropList(p, _) if !p.is_empty() => p,
            _ => {
                console::log_1(
                    &"⚠️ queue(): called with non-propList or empty list — ignored".into(),
                );
                return;
            }
        };

        let member_opt = SoundChannel::get_proplist_prop(player, &datum, "member");
        if member_opt.is_none() {
            warn!("⚠️ queue(): missing #member — ignored");
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
            debug!("🧹 Cleared playlist for channel {}", self.channel_num);
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
        }
    }

    pub fn get_playlist(&self) -> Vec<DatumRef> {
        if self.current_segment_index.is_some() && !self.playlist.is_empty() {
            // Director's getPlayList() returns entries that haven't started playing yet,
            // excluding the currently playing segment (always at index 0).
            // This allows scripts to detect when the playlist is about to run out
            // and refill it while the last segment is still playing.
            self.playlist[1..].to_vec()
        } else {
            self.playlist.clone()
        }
    }

    pub fn fade_in(&mut self, ticks: i32, to_volume: f64) {
        let duration = ticks as f64 / 60.0;
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

    pub fn fade_to(&mut self, ticks: i32, to_volume: f64) {
        let duration = ticks as f64 / 60.0;
        self.is_fading = true;
        self.fade_start_volume = self.volume;
        self.fade_target_volume = to_volume;
        self.fade_duration = duration;
        self.fade_elapsed = 0.0;
    }

    pub fn is_busy(&self) -> bool {
        self.status != SoundStatus::Idle
    }

    pub fn set_loop_count(&mut self, count: i32) {
        self.loop_count = count;
        self.loops_remaining = count;
    }

    pub fn get_duration(&self) -> f64 {
        if self.member.is_none() || self.sample_rate == 0 {
            return 0.0;
        }

        self.sample_count as f64 / self.sample_rate as f64
    }

    pub fn update(&mut self, delta_time: f64, player: &mut DirPlayer) -> Result<(), ScriptError> {
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

        // Safety net: if elapsed time exceeds the sound duration, force status
        // to Idle. The onended callback should handle this, but it can fire late
        // due to async decode/resampling overhead.
        if self.sample_rate > 0 && self.sample_count > 0 && self.loop_count != 0 {
            let duration_secs = self.sample_count as f64 / self.sample_rate as f64;
            if self.elapsed_time > duration_secs + 0.1 {
                self.status = SoundStatus::Idle;
                self.source_node = None;
            }
        }

        Ok(())
    }

    /// Stops the currently playing WebAudio source node.
    pub fn stop_playback_nodes(&mut self) {
        // Set status to Idle FIRST, before stopping the source
        // This prevents the onended callback from calling start_next_segment
        self.status = SoundStatus::Idle;
        
        if let Some(source) = self.source_node.take() {
            debug!("🛑 Stopping channel {}", self.channel_num);
            let scheduled: &AudioScheduledSourceNode = source.as_ref();
            let _ = scheduled.stop_with_when(0.0);
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
        
        // Cancel any in-progress async decode by clearing the flag
        *self.is_decoding.borrow_mut() = false;
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

        let _ = self.audio_context().resume();

        let sound_data = &sound_member.sound.data();
        let channels = sound_member.info.channels;
        let sample_rate = sound_member.info.sample_rate;
        let sample_size = sound_member.info.sample_size;
        let codec = sound_member.sound.codec();
        let expected_samples = Some(sound_member.info.sample_count);
        let big_endian = sound_member.sound.big_endian_data();

        let audio_data = Self::load_director_audio_data(
            sound_data,
            channels,
            sample_rate,
            sample_size,
            &codec,
            expected_samples,
            big_endian,
        )
        .map_err(|e| ScriptError::new(format!("Failed to load sound: {}", e)))?;

        let num_frames = audio_data.samples.len() / audio_data.num_channels as usize;

        let buffer = self
            .audio_context.as_ref().unwrap()
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
            .audio_context.as_ref().unwrap()
            .create_buffer_source()
            .map_err(|e| ScriptError::new(format!("Failed to create source: {:?}", e)))?;
        source.set_buffer(Some(&buffer));

        let gain = self
            .audio_context.as_ref().unwrap()
            .create_gain()
            .map_err(|e| ScriptError::new(format!("Failed to create gain: {:?}", e)))?;
        gain.gain().set_value((self.volume / 255.0) as f32);

        let pan = self.audio_context().create_stereo_panner().ok();

        if let Some(ref pan_node) = pan {
            source.connect_with_audio_node(pan_node).map_err(|e| {
                ScriptError::new(format!("Failed to connect source to pan: {:?}", e))
            })?;
            pan_node
                .connect_with_audio_node(&gain)
                .map_err(|e| ScriptError::new(format!("Failed to connect pan to gain: {:?}", e)))?;
            pan_node.pan().set_value((self.pan / 100.0) as f32);
        } else {
            source.connect_with_audio_node(&gain).map_err(|e| {
                ScriptError::new(format!("Failed to connect source to gain: {:?}", e))
            })?;
        }

        gain.connect_with_audio_node(&self.audio_context().destination())
            .map_err(|e| {
                ScriptError::new(format!("Failed to connect gain to destination: {:?}", e))
            })?;

        // Set up the onended callback to handle sound completion
        let channel_index = self.channel_num;
        let closure = Closure::<dyn FnMut()>::new(move || {
            SoundChannel::handle_end_of_sound(channel_index);
        });
        source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref())
            .map_err(|e| ScriptError::new(format!("Failed to add ended listener: {:?}", e)))?;
        closure.forget();

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
        self.playback_start_context_time = self.context_time();

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
        if let Datum::CastMember(cast_member_ref) = member_datum {
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
                    self.playback_start_context_time = self.context_time();
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
                let duration = self.sample_count as f64 / self.sample_rate as f64;
                return self.elapsed_time >= duration;
            }
        }
        false
    }

    pub async fn play_castmember(
        &mut self,
        samples: &[f32],
        num_channels: u32,
        sample_rate: f64,
    ) -> Result<(), JsValue> {
        console::log_1(
            &format!(
                "🎵 play_castmember: {} frames @ {}Hz",
                samples.len() / num_channels as usize,
                sample_rate
            )
            .into(),
        );

        let context = self.audio_context.clone().expect("AudioContext not available (non-wasm target?)");

        // Resume context if needed
        let state_val = Reflect::get(context.as_ref(), &"state".into())?;
        let state_str = state_val.as_string().unwrap_or_default();
        if state_str == "suspended" {
            debug!("🌀 Resuming AudioContext");
            let promise = context.resume()?;
            JsFuture::from(promise).await?;
        }

        if let Some(ref source) = self.source_node {
            let scheduled: &AudioScheduledSourceNode = source.as_ref();
            let _ = scheduled.stop_with_when(0.0);
        }
        self.source_node = None;

        let num_frames = samples.len() / num_channels as usize;
        if num_frames == 0 {
            return Ok(());
        }

        let buffer = context.create_buffer(num_channels, num_frames as u32, sample_rate as f32)?;
        for channel in 0..num_channels {
            let mut channel_data = vec![0.0f32; num_frames];
            for frame in 0..num_frames {
                let idx = frame * num_channels as usize + channel as usize;
                channel_data[frame] = samples[idx] as f32;
            }
            buffer.copy_to_channel(&channel_data, channel as i32)?;
        }

        let source = context.create_buffer_source()?;
        source.set_buffer(Some(&buffer));

        let pan = match context.create_stereo_panner() {
            Ok(p) => p,
            Err(_) => {
                error!("❌ Failed to create StereoPannerNode");
                return Ok(());
            }
        };

        let pan_value = self.pan;
        pan.pan().set_value((pan_value / 100.0) as f32);

        // Create FRESH gain and pan nodes
        let gain = match context.create_gain() {
            Ok(g) => g,
            Err(_) => {
                error!("❌ Failed to create GainNode");
                return Ok(());
            }
        };

        let volume = self.volume;
        gain.gain().set_value((volume / 255.0) as f32);
        debug!("🔊 Setting gain to {} (volume: {})", volume / 255.0, volume);

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

        // Set status to Playing BEFORE starting the source
        self.status = SoundStatus::Playing;
        self.playback_start_context_time = self.context_time();

        source.start()?;
        debug!("called source.start()");
        self.source_node = Some(Rc::new(source));
        debug!("✅ Audio scheduled for channel {}", self.channel_num);

        Ok(())
    }

    pub async fn play_sound(&mut self, audio_data: &AudioData) -> Result<(), JsValue> {
        use web_sys::{Blob, HtmlAudioElement, Url};

        debug!("🎵 play_sound() called");

        let context = self.audio_context();

        // 🔊 If MP3 → decode & play via <audio>
        if let Some(ref compressed) = audio_data.compressed_data {
            debug!("MP3 stream detected → playing via Blob URL");

            let array = js_sys::Uint8Array::from(&compressed[..]);
            let blob = Blob::new_with_u8_array_sequence(&js_sys::Array::of1(&array))
                .map_err(|e| JsValue::from_str(&format!("Failed to create blob: {:?}", e)))?;
            let url = Url::create_object_url_with_blob(&blob)
                .map_err(|e| JsValue::from_str(&format!("Failed to create object URL: {:?}", e)))?;

            let audio = HtmlAudioElement::new_with_src(&url)?;
            audio.play()?;
            return Ok(());
        }

        // 🎧 Otherwise normal PCM playback through WebAudio
        self.play_castmember(
            &audio_data.samples,
            audio_data.num_channels as u32,
            audio_data.sample_rate as f64,
        )
        .await
    }

    pub fn stop_sound(&mut self) {
        if let Some(ref source) = self.source_node {
            let scheduled: &AudioScheduledSourceNode = source.as_ref();
            let _ = scheduled.stop_with_when(0.0);
            let _ = source.disconnect();
            debug!("🛑 Stopped previous sound");
        } else {
            debug!("ℹ️ No active sound source to stop");
        }

        self.source_node = None;
        self.status = SoundStatus::Idle;
    }

    pub fn pause_sound(&mut self) {
        if let Some(ref source) = self.source_node {
            debug!("⏸️ Pausing current sound...");
            let scheduled: &AudioScheduledSourceNode = source.as_ref();
            let _ = scheduled.stop_with_when(0.0);
            self.source_node = None;
        } else {
            debug!("ℹ️ No active sound source to pause");
        }
    }

    pub fn set_volume(&mut self, volume: f64) -> Result<(), JsValue> {
        self.volume = volume.clamp(0.0, 255.0);
        if let Some(ref gain) = self.gain_node {
            gain.gain().set_value((self.volume / 255.0) as f32);
        }
        Ok(())
    }

    pub fn set_pan(&mut self, pan: f64) -> Result<(), JsValue> {
        let clamped = pan.clamp(-1.0, 1.0);

        if let Some(ref pan_node) = self.pan_node {
            let _ = pan_node.pan().set_value(clamped as f32);
            console::log_1(&JsValue::from_str(&format!("🎚️ Pan set to {:.2}", clamped)));
        }

        Ok(())
    }

    // This is the static entry point called by the AudioBufferSourceNode's 'onended' event.
    // It needs to safely retrieve the DirPlayer and the specific SoundChannel.
    pub fn handle_end_of_sound(channel_index: i32) {
        

        let player_opt = unsafe { crate::PLAYER_OPT.as_mut() };
        if player_opt.is_none() {
            error!("❌ PLAYER_OPT is None in handle_end_of_sound");
            return;
        }
        let player = player_opt.unwrap();

        let channel_opt = player.sound_manager.get_channel_mut(channel_index as usize);

        if let Some(channel_rc) = channel_opt {
            let mut ch = channel_rc.borrow_mut();
            
            // Only proceed if we were actually playing (not stopped early)
            if ch.status != SoundStatus::Playing {
                warn!("⚠️ Channel {} was already stopped, ignoring ended event", channel_index);
                return;
            }
            
            ch.status = SoundStatus::Idle;
            ch.source_node = None;
            ch.start_next_segment();
        } else {
            error!("❌ SoundChannel {} not found.", channel_index);
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

        // Check for queued members first
        let queued_ref = self.queued_members.first().cloned();
        if let Some(queued_ref) = queued_ref {
            self.queued_members.remove(0); // Remove from queue
            console::log_1(
                &format!(
                    "▶️ Playing queued member from start_next_segment ({} remaining in queue)",
                    self.queued_members.len()
                )
                .into(),
            );
            let channel_num = self.channel_num;
            
            spawn_local(async move {
                if let Some(player) = unsafe { crate::PLAYER_OPT.as_mut() } {
                    if let Some(channel_rc) = player.sound_manager.get_channel(channel_num as usize) {
                        SoundChannel::play_file(channel_rc, queued_ref);
                    }
                }
            });
            return;
        }

        // Handle direct playback looping (non-playlist sounds)
        if self.current_segment_index.is_none() {
            console::log_1(
                &format!(
                    "🔁 Direct playback: loop_count={}, loops_remaining={}",
                    self.loop_count, self.loops_remaining
                )
                .into(),
            );

            // Check if we should loop
            if self.loop_count == 0 {
                // Loop forever
                debug!("♾️ Looping forever (loop_count=0)");
                
                if let Some(ref member_ref) = self.member {
                    let member_ref_clone = member_ref.clone();
                    let channel_num = self.channel_num;
                    
                    spawn_local(async move {
                        if let Some(player) = unsafe { crate::PLAYER_OPT.as_mut() } {
                            if let Some(channel_rc) = player.sound_manager.get_channel(channel_num as usize) {
                                SoundChannel::play_file(channel_rc, member_ref_clone);
                            }
                        }
                    });
                    return;
                }
            } else if self.loops_remaining > 1 {
                // Still have loops remaining
                self.loops_remaining -= 1;
                console::log_1(
                    &format!(
                        "🔁 Looping: {} loops remaining",
                        self.loops_remaining
                    )
                    .into(),
                );
                
                if let Some(ref member_ref) = self.member {
                    let member_ref_clone = member_ref.clone();
                    let channel_num = self.channel_num;
                    
                    spawn_local(async move {
                        if let Some(player) = unsafe { crate::PLAYER_OPT.as_mut() } {
                            if let Some(channel_rc) = player.sound_manager.get_channel(channel_num as usize) {
                                SoundChannel::play_file(channel_rc, member_ref_clone);
                            }
                        }
                    });
                    return;
                }
            }
            
            // No more loops - check if there's a playlist to start
            if !self.playlist_segments.is_empty() {
                debug!("🎵 Found {} queued sounds", self.playlist_segments.len());
                self.current_segment_index = Some(0);

                // Try gapless replay from cached buffer first
                if !self.replay_cached_buffer() {
                    let member_ref = self.playlist_segments[0].member_ref.clone();
                    let channel_num = self.channel_num;
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
            
            // No loops left and no playlist - stop
            debug!("⏸️ Playback complete, no loops left");
            self.status = SoundStatus::Idle;
            return;
        }

        // Handle playlist-based looping (existing logic)
        let index = self.current_segment_index.unwrap();

        if index >= self.playlist_segments.len() {
            warn!("⚠️ Current index out of bounds");
            self.current_segment_index = None;
            self.status = SoundStatus::Idle;
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
            // Clone member_ref before releasing the borrow on segment
            let member_ref = segment.member_ref.clone();
            debug!("🔁 Looping segment {}", index);

            // Try gapless replay from cached buffer first
            if self.replay_cached_buffer() {
                return;
            }

            // Fall back to full decode path
            let channel_num = self.channel_num;
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
        debug!("✅ Segment {} completed, REMOVING", index);
        self.playlist_segments.remove(index);
        self.playlist.remove(index);

        debug!("📊 {} items remaining", self.playlist_segments.len());

        // Play next segment if available
        if index < self.playlist_segments.len() {
            self.current_segment_index = Some(index);
            self.playlist_segments[index].loops_remaining = self.playlist_segments[index].loop_count;
            let member_ref = self.playlist_segments[index].member_ref.clone();

            debug!("⏭️ Playing next segment at index {}", index);

            // Try gapless replay from cached buffer first
            if !self.replay_cached_buffer() {
                // Fall back to full decode path
                let channel_num = self.channel_num;
                spawn_local(async move {
                    if let Some(player) = unsafe { crate::PLAYER_OPT.as_mut() } {
                        if let Some(channel_rc) = player.sound_manager.get_channel(channel_num as usize)
                        {
                            SoundChannel::play_file(channel_rc, member_ref);
                        }
                    }
                });
            }
        } else {
            debug!("⏸️ Playlist empty, stopped");
            self.current_segment_index = None;
            self.status = SoundStatus::Idle;
        }
    }

    /// Fast path for gapless playlist playback: reuse the already-decoded AudioBuffer
    /// to create a new source node immediately, without going through the full
    /// decode/resample pipeline. Returns true if successful.
    fn replay_cached_buffer(&mut self) -> bool {
        

        let ctx = match self.audio_context.as_ref() {
            Some(ctx) => ctx,
            None => return false,
        };

        let buffer = match self.current_audio_buffer {
            Some(ref buf) => buf.clone(),
            None => return false,
        };

        let source = match ctx.create_buffer_source() {
            Ok(s) => s,
            Err(_) => return false,
        };
        source.set_buffer(Some(&*buffer));

        let gain = match ctx.create_gain() {
            Ok(g) => g,
            Err(_) => return false,
        };
        gain.gain().set_value((self.volume / 255.0) as f32);

        // Connect: source -> gain -> destination
        let _ = source.connect_with_audio_node(&gain);
        let _ = gain.connect_with_audio_node(&ctx.destination());

        // Set up ended callback
        let channel_num = self.channel_num;
        let closure = Closure::<dyn FnMut()>::new(move || {
            SoundChannel::handle_end_of_sound(channel_num);
        });
        let _ = source.add_event_listener_with_callback("ended", closure.as_ref().unchecked_ref());
        closure.forget();

        let _ = source.start();

        self.source_node = Some(Rc::new(source));
        self.gain_node = Some(Rc::new(gain));
        self.status = SoundStatus::Playing;
        self.playback_start_context_time = ctx.current_time();

        debug!("⚡ Channel {} gapless replay from cached buffer", self.channel_num);
        true
    }

    fn spawn_playback_async(&self) {
        
        debug!("🚀 Spawning async playback task");

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
            Datum::CastMember(r) => r,
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
            sound_member.sound.big_endian_data(),
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
