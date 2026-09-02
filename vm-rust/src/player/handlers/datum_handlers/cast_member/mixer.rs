use crate::director::lingo::datum::Datum;
use crate::player::cast_lib::CastMemberRef;
use crate::player::cast_member::{MixerSoundObject, MixerStatus};
use crate::player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError};
use crate::player::symbols::symbol::Symbol;
use std::collections::VecDeque;

/// Director 11's Sound Mixer member and the sound objects it owns.
///
/// The dictionary calls these "Audio methods" and hangs them off a member
/// created with `new(#Mixer)`:
///
///   * `createSoundObject(name, castMem {, propList})` → a SoundObject
///   * `getSoundObjectList()` → "a list of all the sound objects in the mixer
///     that have not been deleted"
///   * `deleteSoundObject(soundObjRef | soundObjName)`
///   * `play()` / `stop()` / `pause()` / `mute` / `unmute`
///   * `reset()` — "reverts the mixer to the state before the last save… If the
///     mixer was created after the last save, this method empties the mixer
///     (along with the sound objects and filters in it). Call reset only when
///     the mixer is in the stopped state… returns 1 on success and 0 on failure."
///
/// Properties: `volume` (0-255), `bufferSize` (ms, a multiple of 10, default
/// 100, settable only while `#stopped`), `status`, `name`.
///
/// Burnin' Rubber 3 is the movie this was built for: `[M] Sound Manager`'s
/// `CreateMixer` makes one per car and `[M] Cars` hangs the engine, tyre-ground
/// and skid loops on it as sound objects, riding each object's `volume` and
/// `playRate` from the car's speed and slip every frame. Without the member type
/// the whole car build died on its first line.
pub struct MixerMemberHandlers {}

/// Resolve the mixer member behind a datum, or a clear error.
fn mixer_ref(player: &DirPlayer, datum: &DatumRef) -> Result<CastMemberRef, ScriptError> {
    match player.get_datum(datum) {
        Datum::CastMember(r) => Ok(r.to_owned()),
        _ => Err(ScriptError::new("Not a mixer member".to_string())),
    }
}

fn sound_object_datum(player: &mut DirPlayer, member: &CastMemberRef, name: &str) -> DatumRef {
    player.alloc_datum(Datum::MixerSoundObjectRef(member.clone(), name.to_string()))
}

impl MixerMemberHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let member_ref = mixer_ref(player, datum)?;
            match_ci!(handler_name, {
                "createSoundObject" => Self::create_sound_object(player, &member_ref, args),
                "deleteSoundObject" => Self::delete_sound_object(player, &member_ref, args),
                "getSoundObjectList" => {
                    let names: Vec<String> = player
                        .movie
                        .cast_manager
                        .find_member_by_ref(&member_ref)
                        .and_then(|m| m.member_type.as_mixer())
                        .map(|mx| mx.objects.iter().map(|o| o.name.clone()).collect())
                        .unwrap_or_default();
                    let items: VecDeque<DatumRef> = names
                        .iter()
                        .map(|n| sound_object_datum(player, &member_ref, n))
                        .collect();
                    Ok(player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List,
                        items,
                        false,
                    )))
                },
                "getSoundObject" => {
                    let name = args
                        .first()
                        .map(|a| player.get_datum(a).string_value())
                        .transpose()?
                        .unwrap_or_default();
                    let exists = player
                        .movie
                        .cast_manager
                        .find_member_by_ref(&member_ref)
                        .and_then(|m| m.member_type.as_mixer())
                        .map_or(false, |mx| mx.object(&name).is_some());
                    if exists {
                        Ok(sound_object_datum(player, &member_ref, &name))
                    } else {
                        Ok(DatumRef::Void)
                    }
                },
                // play / stop / pause drive every object in the mixer as a unit.
                "play" => Self::set_status(player, &member_ref, MixerStatus::Playing),
                "stop" => Self::set_status(player, &member_ref, MixerStatus::Stopped),
                "pause" => Self::set_status(player, &member_ref, MixerStatus::Paused),
                "mute" => Self::set_muted(player, &member_ref, true),
                "unmute" => Self::set_muted(player, &member_ref, false),
                // "Call reset only when the mixer is in the stopped state. This
                // method returns 1 on success and 0 on failure. For example,
                // when this method is called while the mixer is playing, it
                // returns 0." Nothing here has ever been saved, so reset is the
                // documented "empties the mixer" case.
                "reset" => {
                    let ok = {
                        let Some(mx) = player
                            .movie
                            .cast_manager
                            .find_mut_member_by_ref(&member_ref)
                            .and_then(|m| m.member_type.as_mixer_mut())
                        else {
                            return Ok(player.alloc_datum(Datum::Int(0)));
                        };
                        if mx.status == MixerStatus::Playing {
                            false
                        } else {
                            mx.objects.clear();
                            true
                        }
                    };
                    Ok(player.alloc_datum(Datum::Int(if ok { 1 } else { 0 })))
                },
                // Saving a mixer to disk has no meaning in the browser.
                "save" | "startSave" | "stopSave" => Ok(player.alloc_datum(Datum::Int(0))),
                _ => Err(ScriptError::new(format!(
                    "No handler {} for mixer member",
                    handler_name
                ))),
            })
        })
    }

    fn set_status(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        status: MixerStatus,
    ) -> Result<DatumRef, ScriptError> {
        if let Some(mx) = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_mixer_mut())
        {
            mx.status = status;
            for obj in mx.objects.iter_mut() {
                obj.status = status;
            }
        }
        Ok(DatumRef::Void)
    }

    fn set_muted(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        muted: bool,
    ) -> Result<DatumRef, ScriptError> {
        if let Some(mx) = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_mixer_mut())
        {
            mx.muted = muted;
        }
        Ok(DatumRef::Void)
    }

    /// `createSoundObject(name, castMem {, startTime, endTime, loopCount,
    /// loopStartTime, loopEndTime, preLoadTime})`.
    ///
    /// The optional arguments are documented positionally AND as the `proplist`
    /// parameter; Burnin' Rubber 3 always passes the property-list form, e.g.
    /// `createSoundObject("CarSkid", tSoundSet[2], [#loopCount: 0,
    /// #loopStartTime: 135, #loopEndTime: tSoundSet[2].duration - 135])`.
    /// Both are accepted here.
    fn create_sound_object(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let name = args
            .first()
            .map(|a| player.get_datum(a).string_value())
            .transpose()?
            .unwrap_or_default();
        if name.is_empty() {
            return Err(ScriptError::new(
                "createSoundObject: a sound object name is required".to_string(),
            ));
        }
        let mut obj = MixerSoundObject::new(name.clone());
        // Second arg: a sound cast member, or a file path.
        if let Some(src) = args.get(1) {
            match player.get_datum(src) {
                Datum::CastMember(r) => obj.member = Some(r.to_owned()),
                other => obj.file = other.string_value().unwrap_or_default(),
            }
        }
        // Remaining args: either one property list, or the positional tail.
        let mut positional: Vec<i32> = Vec::new();
        let mut props: Vec<(String, i32)> = Vec::new();
        for a in args.iter().skip(2) {
            match player.get_datum(a) {
                Datum::PropList(pairs, _) => {
                    let pairs = pairs.clone();
                    for (k, v) in pairs.iter() {
                        let key = player.get_datum(k).string_value().unwrap_or_default();
                        let val = player.get_datum(v).int_value().unwrap_or(0);
                        props.push((key.trim_start_matches('#').to_string(), val));
                    }
                }
                other => positional.push(other.int_value().unwrap_or(0)),
            }
        }
        for (i, v) in positional.iter().enumerate() {
            match i {
                0 => obj.start_time = *v,
                1 => obj.end_time = *v,
                2 => obj.loop_count = *v,
                3 => obj.loop_start_time = *v,
                4 => obj.loop_end_time = *v,
                _ => {}
            }
        }
        for (k, v) in props {
            match_ci!(k.as_str(), {
                "startTime" => obj.start_time = v,
                "endTime" => obj.end_time = v,
                "loopCount" => obj.loop_count = v,
                "loopStartTime" => obj.loop_start_time = v,
                "loopEndTime" => obj.loop_end_time = v,
                "volume" => obj.volume = v,
                _ => {}
            });
        }
        let Some(mx) = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_mixer_mut())
        else {
            return Err(ScriptError::new(
                "createSoundObject: not a mixer member".to_string(),
            ));
        };
        // "Sound objects with duplicate names are not allowed" — replace, so a
        // rebuilt car doesn't accumulate stale entries under the same name.
        if let Some(slot) = mx.objects.iter().position(|o| o.name.eq_ignore_ascii_case(&name)) {
            mx.objects[slot] = obj;
        } else {
            mx.objects.push(obj);
        }
        Ok(sound_object_datum(player, member_ref, &name))
    }

    fn delete_sound_object(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // "deletes the sound object using the reference OR the name".
        let name = match args.first().map(|a| player.get_datum(a)) {
            Some(Datum::MixerSoundObjectRef(_, n)) => n.clone(),
            Some(other) => other.string_value().unwrap_or_default(),
            None => String::new(),
        };
        if let Some(mx) = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_mixer_mut())
        {
            mx.objects.retain(|o| !o.name.eq_ignore_ascii_case(&name));
        }
        Ok(DatumRef::Void)
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
    ) -> Result<Datum, ScriptError> {
        let Some(mx) = player
            .movie
            .cast_manager
            .find_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_mixer())
        else {
            return Err(ScriptError::new("Not a mixer member".to_string()));
        };
        match_ci!(prop, {
            "volume" => Ok(Datum::Int(mx.volume)),
            "bufferSize" => Ok(Datum::Int(mx.buffer_size)),
            "status" => Ok(Datum::Symbol(Symbol::from_str(mx.status.symbol()))),
            "mute" => Ok(Datum::Int(if mx.muted { 1 } else { 0 })),
            // Not modelled, but scripts read them for logging; answer the
            // documented defaults rather than raising.
            "channelCount" => Ok(Datum::Int(2)),
            "sampleRate" => Ok(Datum::Int(44100)),
            "bitDepth" => Ok(Datum::Int(16)),
            "isSaving" => Ok(Datum::Int(0)),
            "elapsedTime" => Ok(Datum::Int(0)),
            _ => Err(ScriptError::new(format!("Cannot get mixer property {}", prop))),
        })
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        prop: &str,
        value: &Datum,
    ) -> Result<(), ScriptError> {
        let is_stopped = player
            .movie
            .cast_manager
            .find_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_mixer())
            .map_or(false, |mx| mx.status == MixerStatus::Stopped);
        let Some(mx) = player
            .movie
            .cast_manager
            .find_mut_member_by_ref(member_ref)
            .and_then(|m| m.member_type.as_mixer_mut())
        else {
            return Err(ScriptError::new("Not a mixer member".to_string()));
        };
        match_ci!(prop, {
            "volume" => { mx.volume = value.int_value()?.clamp(0, 255); Ok(()) },
            "bufferSize" => {
                // "bufferSize can be set only when the mixer is in the #stopped
                // state" and "is a multiple of 10".
                if is_stopped {
                    let v = value.int_value()?.max(10);
                    mx.buffer_size = (v / 10) * 10;
                }
                Ok(())
            },
            "mute" => { mx.muted = value.int_value()? != 0; Ok(()) },
            _ => Err(ScriptError::new(format!("Cannot set mixer property {}", prop))),
        })
    }
}

/// The sound objects themselves: `so.volume`, `so.playRate`, `so.filterList`,
/// `so.play()` / `so.stop()`.
pub struct MixerSoundObjectHandlers {}

impl MixerSoundObjectHandlers {
    fn parts(player: &DirPlayer, datum: &DatumRef) -> Result<(CastMemberRef, String), ScriptError> {
        match player.get_datum(datum) {
            Datum::MixerSoundObjectRef(m, n) => Ok((m.clone(), n.clone())),
            _ => Err(ScriptError::new("Not a sound object".to_string())),
        }
    }

    pub fn call(
        datum: &DatumRef,
        handler_name: &str,
        _args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (member_ref, name) = Self::parts(player, datum)?;
            let status = match_ci!(handler_name, {
                "play" => Some(MixerStatus::Playing),
                "stop" => Some(MixerStatus::Stopped),
                "pause" => Some(MixerStatus::Paused),
                _ => None,
            });
            match status {
                Some(s) => {
                    if let Some(obj) = player
                        .movie
                        .cast_manager
                        .find_mut_member_by_ref(&member_ref)
                        .and_then(|m| m.member_type.as_mixer_mut())
                        .and_then(|mx| mx.object_mut(&name))
                    {
                        obj.status = s;
                    }
                    Ok(DatumRef::Void)
                }
                None => Err(ScriptError::new(format!(
                    "No handler {} for sound object",
                    handler_name
                ))),
            }
        })
    }

    pub fn get_prop(datum: &DatumRef, prop: &str) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let (member_ref, name) = Self::parts(player, datum)?;
            // `filterList` must answer the SAME list across reads so
            // `so.filterList.append(audioFilter(...))` sticks — the same live
            // reference rule `userData` follows on a 3D node.
            if prop.eq_ignore_ascii_case("filterList") {
                let existing = player
                    .movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .and_then(|m| m.member_type.as_mixer())
                    .and_then(|mx| mx.object(&name))
                    .and_then(|o| o.filter_list.clone());
                if let Some(r) = existing {
                    return Ok(r);
                }
                let list = player.alloc_datum(Datum::List(
                    crate::director::lingo::datum::DatumType::List,
                    VecDeque::new(),
                    false,
                ));
                if let Some(obj) = player
                    .movie
                    .cast_manager
                    .find_mut_member_by_ref(&member_ref)
                    .and_then(|m| m.member_type.as_mixer_mut())
                    .and_then(|mx| mx.object_mut(&name))
                {
                    obj.filter_list = Some(list.clone());
                }
                return Ok(list);
            }
            let Some(obj) = player
                .movie
                .cast_manager
                .find_member_by_ref(&member_ref)
                .and_then(|m| m.member_type.as_mixer())
                .and_then(|mx| mx.object(&name))
            else {
                return Ok(DatumRef::Void);
            };
            let d = match_ci!(prop, {
                "name" => Datum::String(obj.name.clone()),
                "volume" => Datum::Int(obj.volume),
                "playRate" => Datum::Float(obj.play_rate as f64),
                "status" => Datum::Symbol(Symbol::from_str(obj.status.symbol())),
                "loopCount" => Datum::Int(obj.loop_count),
                "startTime" => Datum::Int(obj.start_time),
                "endTime" => Datum::Int(obj.end_time),
                "loopStartTime" => Datum::Int(obj.loop_start_time),
                "loopEndTime" => Datum::Int(obj.loop_end_time),
                "member" => match &obj.member {
                    Some(m) => Datum::CastMember(m.clone()),
                    None => Datum::Void,
                },
                "elapsedTime" => Datum::Int(0),
                _ => return Err(ScriptError::new(format!(
                    "Cannot get sound object property {}", prop
                ))),
            });
            Ok(player.alloc_datum(d))
        })
    }

    pub fn set_prop(datum: &DatumRef, prop: &str, value: &Datum) -> Result<(), ScriptError> {
        reserve_player_mut(|player| {
            let (member_ref, name) = Self::parts(player, datum)?;
            if prop.eq_ignore_ascii_case("filterList") {
                // Whole-list assignment replaces the live list.
                let new_ref = player.alloc_datum(value.clone());
                if let Some(obj) = player
                    .movie
                    .cast_manager
                    .find_mut_member_by_ref(&member_ref)
                    .and_then(|m| m.member_type.as_mixer_mut())
                    .and_then(|mx| mx.object_mut(&name))
                {
                    obj.filter_list = Some(new_ref);
                }
                return Ok(());
            }
            let v_int = value.int_value().unwrap_or(0);
            let v_float = value.to_float().unwrap_or(0.0) as f32;
            let Some(obj) = player
                .movie
                .cast_manager
                .find_mut_member_by_ref(&member_ref)
                .and_then(|m| m.member_type.as_mixer_mut())
                .and_then(|mx| mx.object_mut(&name))
            else {
                return Ok(());
            };
            match_ci!(prop, {
                "volume" => { obj.volume = v_int.clamp(0, 255); Ok(()) },
                "playRate" => { obj.play_rate = v_float; Ok(()) },
                "loopCount" => { obj.loop_count = v_int; Ok(()) },
                "startTime" => { obj.start_time = v_int; Ok(()) },
                "endTime" => { obj.end_time = v_int; Ok(()) },
                "loopStartTime" => { obj.loop_start_time = v_int; Ok(()) },
                "loopEndTime" => { obj.loop_end_time = v_int; Ok(()) },
                _ => Err(ScriptError::new(format!(
                    "Cannot set sound object property {}", prop
                ))),
            })
        })
    }
}
