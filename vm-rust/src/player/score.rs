use std::cmp::max;

use itertools::Itertools;
use log::debug;
use wasm_bindgen::JsValue;
use std::collections::{HashMap, HashSet};

use crate::{
    director::{
        chunks::score::{FrameLabel, ScoreFrameChannelData, SoundChannelData, TempoChannelData},
        file::DirectorFile,
        lingo::datum::{datum_bool, Datum, DatumType},
    },
    js_api::JsApi,
    player::bitmap::palette::SYSTEM_WIN_PALETTE,
    player::events::dispatch_event_endsprite,
    player::font::measure_text,
    player::score_keyframes::{
        ChannelKeyframes,
        build_all_keyframes_cache,
        convert_blend_to_percentage,
        KeyframeTrack,
    },
    player::handlers::datum_handlers::player_call_datum_handler,
    utils::log_i,
};

use super::{
    allocator::ScriptInstanceAllocatorTrait,
    cast_lib::{cast_member_ref, CastMemberRef, NULL_CAST_MEMBER_REF},
    cast_member::CastMemberType,
    datum_ref::DatumRef,
    geometry::{IntRect, IntRectTuple},
    handlers::datum_handlers::{
        cast_member_ref::CastMemberRefHandlers,
        color::ColorDatumHandlers,
        script::{self, ScriptDatumHandlers},
        sound_channel::SoundStatus,
    },
    movie::Movie,
    reserve_player_mut, reserve_player_ref,
    script::{script_get_prop_opt, script_set_prop},
    script_ref::ScriptInstanceRef,
    sprite::{ColorRef, CursorRef, Sprite},
    DirPlayer, ScriptError,
};

pub enum ScoreRef {
    Stage,
    FilmLoop(CastMemberRef),
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct SpriteChannel {
    pub number: usize,
    pub name: String,
    pub scripted: bool,
    pub sprite: Sprite,
}

impl SpriteChannel {
    pub fn new(number: usize) -> SpriteChannel {
        SpriteChannel {
            number,
            name: "".to_owned(),
            scripted: false,
            sprite: Sprite::new(number),
        }
    }
}

#[derive(Clone)]
pub struct ScoreBehaviorReference {
    pub cast_lib: u16,
    pub cast_member: u16,
    pub parameter: Vec<DatumRef>,
}

#[derive(Clone)]
pub struct ScoreSpriteSpan {
    pub channel_number: u32,
    pub start_frame: u32,
    pub end_frame: u32,
    pub scripts: Vec<ScoreBehaviorReference>,
}

#[derive(Clone)]
pub struct Score {
    pub channels: Vec<SpriteChannel>,
    pub sprite_spans: Vec<ScoreSpriteSpan>,
    pub channel_initialization_data: Vec<(u32, u16, ScoreFrameChannelData)>,
    pub sound_channel_data: Vec<(u32, u16, SoundChannelData)>,
    pub tempo_channel_data: Vec<(u32, TempoChannelData)>,
    pub frame_labels: Vec<FrameLabel>,
    pub sound_channel_triggered: HashMap<u16, u32>,
    pub keyframes_cache: HashMap<u16, ChannelKeyframes>,
}

fn get_sprite_rect(player: &DirPlayer, sprite_id: i16) -> IntRectTuple {
    let sprite = player.movie.score.get_sprite(sprite_id);
    let sprite = match sprite {
        Some(sprite) => sprite,
        None => return (0, 0, 0, 0),
    };
    let rect = get_concrete_sprite_rect(player, sprite);
    return (rect.left, rect.top, rect.right, rect.bottom);
}

pub fn get_channel_number_from_index(index: u32) -> u32 {
    match index {
        0 => 0,
        index => index - 5,
    }
}

impl Score {
    pub fn empty() -> Score {
        Score {
            channels: vec![],
            frame_labels: vec![],
            channel_initialization_data: vec![],
            sound_channel_data: vec![],
            tempo_channel_data: vec![],
            sprite_spans: vec![],
            sound_channel_triggered: HashMap::new(),
            keyframes_cache: HashMap::new(),
        }
    }

    pub fn get_script_in_frame(&self, frame: u32) -> Option<ScoreBehaviorReference> {
        return self
            .sprite_spans
            .iter()
            .find(|span| {
                span.channel_number == 0 && frame >= span.start_frame && frame <= span.end_frame
            })
            .and_then(|span| span.scripts.first().cloned());
    }

    fn create_behavior(cast_lib: i32, cast_member: i32) -> (ScriptInstanceRef, DatumRef) {
        let script_ref = CastMemberRef {
            cast_lib,
            cast_member,
        };
        reserve_player_mut(|player| {
            let _ = player
                .movie
                .cast_manager
                .get_script_by_ref(&script_ref)
                .ok_or(ScriptError::new(format!("Script not found")));
        });

        let (script_instance_ref, datum_ref) =
            match ScriptDatumHandlers::create_script_instance(&script_ref) {
                Ok(result) => result,

                Err(e) => {
                    web_sys::console::error_1(
                        &format!("Failed to create script instance: {}", e.message).into(),
                    );

                    panic!("Cannot continue without script instance");
                }
            };

        (script_instance_ref.clone(), datum_ref.clone())
    }

    pub fn is_span_in_frame(span: &ScoreSpriteSpan, frame_num: u32) -> bool {
        span.start_frame <= frame_num && span.end_frame >= frame_num
    }

    pub async fn initialize_behavior_defaults_async(
        script_instance_ref: ScriptInstanceRef,
        sprite_num: u32,
    ) -> Result<(), ScriptError> {
        let instance_datum_ref = reserve_player_mut(|player| {
            player.alloc_datum(Datum::ScriptInstanceRef(script_instance_ref.clone()))
        });

        // Try to call getPropertyDescriptionList

        let result = player_call_datum_handler(
            &instance_datum_ref,
            &"getPropertyDescriptionList".to_string(),
            &vec![],
        )
        .await;

        if let Ok(prop_desc_ref) = result {
            reserve_player_mut(|player| {
                let prop_desc_datum = player.get_datum(&prop_desc_ref).clone();

                if let Datum::PropList(prop_descriptions, _) = prop_desc_datum {
                    // First pass: collect all the data we need (avoiding nested borrows)

                    let prop_data: Vec<(String, DatumRef, Vec<(String, DatumRef)>)> =
                        prop_descriptions
                            .iter()
                            .filter_map(|(prop_key_ref, prop_desc_ref)| {
                                let prop_key = player.get_datum(prop_key_ref).clone();

                                if let Datum::Symbol(prop_name) = prop_key {
                                    let prop_desc = player.get_datum(prop_desc_ref).clone();

                                    if let Datum::PropList(desc_props, _) = prop_desc {
                                        let desc_props_cloned: Vec<(String, DatumRef)> = desc_props
                                            .iter()
                                            .filter_map(|(k, v)| {
                                                let key = player.get_datum(k).clone();

                                                if let Datum::Symbol(key_name) = key {
                                                    Some((key_name.clone(), v.clone()))
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();

                                        return Some((
                                            prop_name.clone(),
                                            prop_desc_ref.clone(),
                                            desc_props_cloned,
                                        ));
                                    }
                                }

                                None
                            })
                            .collect();

                    // Second pass: check existing values and collect defaults to set
                    let mut defaults_to_set = Vec::new();

                    for (prop_name, _, desc_props) in prop_data {
                        // Check if property already has a non-void value
                        let should_set_default = if let Some(existing) =
                            script_get_prop_opt(player, &script_instance_ref, &prop_name)
                        {
                            let existing_datum = player.get_datum(&existing);

                            matches!(existing_datum, Datum::Void)
                        } else {
                            true
                        };

                        if should_set_default {
                            // Find the default value
                            for (key_name, default_value_ref) in desc_props {
                                if key_name == "default" {
                                    defaults_to_set.push((prop_name.clone(), default_value_ref));

                                    break;
                                }
                            }
                        }
                    }

                    // Third pass: set all defaults
                    for (prop_name, default_value_ref) in defaults_to_set {
                        let _ = script_set_prop(
                            player,
                            &script_instance_ref,
                            &prop_name,
                            &default_value_ref,
                            false,
                        );
                    }
                }

                Ok::<(), ScriptError>(())
            })
        } else {
            Ok(())
        }
    }

    pub fn begin_sprites(&mut self, frame_num: u32) {
        // Clean up sound channel triggers: remove tracking for any sound not on the current frame
        // This ensures sounds can play again when we return to their frame after leaving it
        let sounds_on_current_frame: HashSet<u16> = self
            .sound_channel_data
            .iter()
            .filter_map(|(frame_index, channel_index, _)| {
                if *frame_index + 1 == frame_num {
                    Some(*channel_index)
                } else {
                    None
                }
            })
            .collect();

        // Remove triggered markers for any sound not on the current frame
        // This clears tracking when we've moved past a sound's frame
        self.sound_channel_triggered
            .retain(|channel_index, _| sounds_on_current_frame.contains(channel_index));

        // clean up behaviors from previous frame
        let sprites_to_finish = reserve_player_mut(|player| {
            player
                .movie
                .score
                .channels
                .iter()
                .filter_map(|channel| channel.sprite.exited.then_some(channel.sprite.number))
                .collect_vec()
        });

        for sprite_num in sprites_to_finish {
            reserve_player_mut(|player| {
                let sprite: &mut Sprite = player.movie.score.get_sprite_mut(sprite_num as i16);
                if sprite.visible {
                    sprite.reset();
                }
            });
        }

        // Find spans that should be entered
        let spans_to_enter: Vec<_> = self
            .sprite_spans
            .iter()
            .filter(|span| Self::is_span_in_frame(span, frame_num))
            .filter(|span| {
                reserve_player_mut(|player| {
                    player
                        .movie
                        .score
                        .get_sprite(span.channel_number as i16)
                        .map_or(true, |sprite| !sprite.entered && !sprite.exited)
                })
            })
            .cloned()
            .collect();

        // Get initialization data for sprites
        let span_init_data: Vec<_> = spans_to_enter
            .iter()
            .filter_map(|span| {
                self.channel_initialization_data
                    .iter()
                    .find(|(_frame_index, channel_index, _data)| {
                        get_channel_number_from_index(*channel_index as u32)
                            == span.channel_number as u32
                            && _frame_index + 1 == span.start_frame
                    })
                    .map(|(_frame_index, channel_index, data)| (span, *channel_index, data.clone()))
            })
            .collect();

        // Initialize sprite properties (member, position, etc.)
        for (span, channel_index, data) in span_init_data.iter() {
            let sprite_num = span.channel_number as i16;
            let sprite: &mut Sprite = self.get_sprite_mut(sprite_num);
            sprite.entered = true;
            let is_sprite = span.channel_number > 0;
            if is_sprite {
                let member = CastMemberRef {
                    cast_lib: data.cast_lib as i32,
                    cast_member: data.cast_member as i32,
                };

                let _ = sprite_set_prop(sprite_num, "member", Datum::CastMember(member.clone()));
                sprite.ink = data.ink as i32;
                sprite.loc_h = data.pos_x as i32;
                sprite.loc_v = data.pos_y as i32;
                sprite.width = data.width as i32;
                sprite.height = data.height as i32;
                sprite.blend = if data.blend == 0 {
                    100
                } else {
                    data.blend as i32
                };
                sprite.skew = data.skew as f64;
                sprite.rotation = data.rotation as f64;

                reserve_player_mut(|player| {
                    let member_ref = member.clone();

                    if let Ok(cast) = player.movie.cast_manager.get_cast(member.cast_lib as u32) {
                        if let Some(real_member) = cast.members.get(&(member.cast_member as u32)) {
                            let type_str = real_member.member_type.type_string();

                            if type_str == "shape" {
                                let blend = if data.blend == 255 {
                                    100
                                } else {
                                    ((255.0 - data.blend as f32) * 100.0 / 255.0) as u8
                                };
                                sprite.blend = blend as i32;
                                sprite.ink = (data.ink / 5) as i32;
                            }
                        }
                    }
                });

                reserve_player_ref(|player| {
                    if let Some(member_ref) = &sprite.member {
                        if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                            if let CastMemberType::Bitmap(bitmap_member) = &member.member_type {
                                let bw = bitmap_member.info.width as i32;
                                let bh = bitmap_member.info.height as i32;

                                sprite.bitmap_size_owned_by_sprite =
                                    sprite.width != bw || sprite.height != bh;
                            }
                        }
                    }
                });

                match data.color_flag {
                    // fore + back are palette indexes
                    0 => {
                        // Foreground
                        sprite.fore_color = data.fore_color as i32;
                        sprite.color = ColorRef::PaletteIndex(data.fore_color);

                        // Background
                        sprite.back_color = data.back_color as i32;
                        sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                    }

                    // foreColor is RGB, backColor is palette index
                    1 => {
                        // Foreground (RGB → map to palette)
                        sprite.color = ColorRef::Rgb(
                            data.fore_color,
                            data.fore_color_g,
                            data.fore_color_b,
                        );
                        sprite.fore_color =
                            sprite.color.to_index(&SYSTEM_WIN_PALETTE) as i32;

                        // Background (palette index)
                        sprite.back_color = data.back_color as i32;
                        sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                    }

                    // foreColor is palette index, backColor is RGB
                    2 => {
                        // Foreground (palette index)
                        sprite.fore_color = data.fore_color as i32;
                        sprite.color = ColorRef::PaletteIndex(data.fore_color);

                        // Background (RGB → map to palette)
                        sprite.bg_color = ColorRef::Rgb(
                            data.back_color,
                            data.back_color_g,
                            data.back_color_b,
                        );
                        sprite.back_color =
                            sprite.bg_color.to_index(&SYSTEM_WIN_PALETTE) as i32;
                    }

                    // both fore + back are RGB
                    3 => {
                        // Foreground (RGB → map to palette)
                        sprite.color = ColorRef::Rgb(
                            data.fore_color,
                            data.fore_color_g,
                            data.fore_color_b,
                        );
                        sprite.fore_color =
                            sprite.color.to_index(&SYSTEM_WIN_PALETTE) as i32;

                        // Background (RGB → map to palette)
                        sprite.bg_color = ColorRef::Rgb(
                            data.back_color,
                            data.back_color_g,
                            data.back_color_b,
                        );
                        sprite.back_color =
                            sprite.bg_color.to_index(&SYSTEM_WIN_PALETTE) as i32;
                    }

                    _ => {
                        web_sys::console::error_1(&JsValue::from_str(&format!(
                            "Unexpected color flag: {}",
                            data.color_flag
                        )));
                    }
                }

                sprite.base_loc_h = sprite.loc_h;
                sprite.base_loc_v = sprite.loc_v;
                sprite.base_width = sprite.width;
                sprite.base_height = sprite.height;
                sprite.base_rotation = sprite.rotation;
                sprite.base_blend = sprite.blend;
                sprite.base_skew = sprite.skew;
                sprite.base_color = sprite.color.clone();
                sprite.base_bg_color = sprite.bg_color.clone();

                // Reset size flags when sprite re-enters
                sprite.has_size_tweened = false;
                sprite.has_size_changed = false;
            }
        }

        // handle score Sound 1 + Sound 2 in Effects Channels
        let first_appearances: HashMap<u16, u32> = self.sound_channel_data.iter().fold(
            HashMap::new(),
            |mut map, (frame_idx, ch_idx, _data)| {
                map.entry(*ch_idx).or_insert(*frame_idx);
                map
            },
        );

        for (frame_index, channel_index, sound_data) in self.sound_channel_data.iter() {
            if *frame_index + 1 == frame_num
                && first_appearances.get(channel_index) == Some(frame_index)
            {
                // Check if we've already triggered this sound on this frame
                if let Some(&triggered_frame) = self.sound_channel_triggered.get(channel_index) {
                    if triggered_frame == frame_num {
                        // Already triggered this sound on this frame, skip it
                        continue;
                    }
                }

                let sound_channel = if *channel_index == 3 { 2 } else { 1 };

                reserve_player_mut(|player| {
                    if player.is_playing {
                        // First check if this exact sound is already playing on this channel
                        let already_playing = player
                            .sound_manager
                            .get_channel((sound_channel - 1) as usize)
                            .map(|ch| {
                                let channel = ch.borrow();
                                // Check if actively playing or loading
                                if channel.status == SoundStatus::Playing
                                    || channel.status == SoundStatus::Loading
                                {
                                    // Check if same member
                                    if let Some(ref current_member_ref) = channel.member {
                                        let current_datum = player.get_datum(current_member_ref);

                                        if let Datum::CastMember(ref current_cast_ref) =
                                            current_datum
                                        {
                                            return current_cast_ref.cast_member
                                                == sound_data.cast_member as i32;
                                        }
                                    }
                                }

                                // Checking if the sound is looping
                                if channel.loop_count == 0 {
                                    // 0 means loop forever
                                    if let Some(ref current_member_ref) = channel.member {
                                        let current_datum = player.get_datum(current_member_ref);

                                        if let Datum::CastMember(ref current_cast_ref) =
                                            current_datum
                                        {
                                            return current_cast_ref.cast_member
                                                == sound_data.cast_member as i32;
                                        }
                                    }
                                }

                                false
                            })
                            .unwrap_or(false);

                        if !already_playing {
                            // Find the cast member by slot number
                            if let Some(cast_member) = player
                                .movie
                                .cast_manager
                                .find_member_by_slot_number(sound_data.cast_member as u32)
                            {
                                if let CastMemberType::Sound(sound_member) =
                                    &cast_member.member_type
                                {
                                    web_sys::console::log_1(&format!(
                                        "Starting Sound {} on channel_index {}: cast_member={}, loop_enabled={}",
                                        sound_channel, channel_index, sound_data.cast_member, sound_member.info.loop_enabled
                                    ).into());

                                    let cast_member_ref =
                                        CastMemberRefHandlers::member_ref_from_slot_number(
                                            cast_member.number,
                                        );

                                    let member_ref =
                                        player.alloc_datum(Datum::CastMember(CastMemberRef {
                                            cast_lib: cast_member_ref.cast_lib as i32,

                                            cast_member: cast_member_ref.cast_member as i32,
                                        }));

                                    let _ = player.puppet_sound(sound_channel, member_ref);
                                }
                            }
                        } else {
                            debug!(
                                "SoundChannel {} already playing from channel_index {}",
                                sound_channel, channel_index
                            );
                        }
                    }
                });

                // Mark that we've triggered this sound on this frame
                self.sound_channel_triggered
                    .insert(*channel_index, frame_num);
            }
        }

        // Attach behaviors and set their parameters - GROUP BY CHANNEL
        // Group spans by channel_number to process all behaviors for a sprite at once
        let spans_by_channel: std::collections::HashMap<u32, Vec<&ScoreSpriteSpan>> =
            spans_to_enter
                .iter()
                .fold(std::collections::HashMap::new(), |mut acc, span| {
                    acc.entry(span.channel_number)
                        .or_insert_with(Vec::new)
                        .push(span);
                    acc
                });

        for (channel_num, channel_spans) in spans_by_channel.iter() {
            debug!(
                "🔧 Attaching behaviors to channel {}: {} spans",
                channel_num,
                channel_spans.len()
            );

            for span in channel_spans {
                if span.scripts.is_empty() {
                    continue;
                }

                for behavior_ref in &span.scripts {
                    debug!(
                            "Creating behavior from cast {}/{} with {} parameters",
                            behavior_ref.cast_lib,
                            behavior_ref.cast_member,
                            behavior_ref.parameter.len()
                        );

                    // Create the behavior instance
                    let (script_instance_ref, datum_ref) = Self::create_behavior(
                        behavior_ref.cast_lib as i32,
                        behavior_ref.cast_member as i32,
                    );

                    // Extract the ScriptInstanceRef from datum_ref
                    let actual_instance_ref = reserve_player_mut(|player| {
                        let datum = player.get_datum(&datum_ref);
                        match datum {
                            Datum::ScriptInstanceRef(ref instance_ref) => Ok(instance_ref.clone()),
                            _ => Err(ScriptError::new("Expected ScriptInstanceRef".to_string())),
                        }
                    })
                    .expect("Failed to extract ScriptInstanceRef");

                    // Set the spriteNum property so 'the currentSpriteNum' works correctly
                    reserve_player_mut(|player| {
                        let sprite_num_ref = player.alloc_datum(Datum::Int(*channel_num as i32));
                        let _ = script_set_prop(
                            player,
                            &actual_instance_ref,
                            &"spriteNum".to_string(),
                            &sprite_num_ref,
                            false,
                        );
                    });

                    // Parameter setup
                    if !behavior_ref.parameter.is_empty() {
                        reserve_player_mut(|player| {
                            debug!("🔧 Applying {} saved parameters", behavior_ref.parameter.len());
                            for param_ref in &behavior_ref.parameter {
                                let param_datum = player.get_datum(param_ref);
                                debug!("  Parameter type: {:?}", param_datum.type_enum());
                                if let Datum::PropList(props, _) = param_datum {
                                    let props_to_set: Vec<(String, DatumRef)> = props.iter()
                                        .filter_map(|(key_ref, value_ref)| {
                                            let key = player.get_datum(key_ref);
                                            if let Datum::Symbol(key_name) = key {
                                                let value = player.get_datum(value_ref);
                                                debug!(
                                                    "    prop: {} type: {:?}",
                                                    key_name,
                                                    value.type_enum()
                                                );

                                                // Try to format value safely
                                                match value {
                                                    Datum::Int(n) => debug!("      value: {}", n),
                                                    Datum::CastMember(ref m) => debug!("      value: member {} of castLib {}", m.cast_member, m.cast_lib),
                                                    _ => debug!("      value: <{:?}>", value.type_enum()),
                                                }

                                                Some((key_name.clone(), value_ref.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();

                                    for (prop_name, value_ref) in props_to_set {
                                        let _ = script_set_prop(
                                            player,
                                            &actual_instance_ref,
                                            &prop_name,
                                            &value_ref,
                                            false,
                                        );
                                    }
                                }
                            }
                            Ok::<(), ScriptError>(())
                        })
                        .expect("Failed to set behavior parameters");
                    }

                    // Attach behavior to sprite
                    reserve_player_mut(|player| {
                        let sprite_num = *channel_num as i16;

                        let current_list_datum =
                            sprite_get_prop(player, sprite_num, "scriptInstanceList");

                        let mut list: Vec<DatumRef> = match current_list_datum {
                            Ok(Datum::List(_, items, _)) => items.clone(),
                            _ => vec![],
                        };

                        list.push(datum_ref.clone());

                        let scripts = Datum::List(DatumType::List, list.clone(), false);
                        let _ = sprite_set_prop(sprite_num, "scriptInstanceList", scripts);
                        Ok::<(), ScriptError>(())
                    })
                    .expect("Failed to attach behavior to sprite");
                }
            }
        }

        // Always ensure frame script instance exists for current frame
        // This handles looping - frame scripts need to be recreated each time we enter the frame
        if let Some(behavior_ref) = self.get_script_in_frame(frame_num) {
            // ONLY create cached instance if there are parameters!
            if !behavior_ref.parameter.is_empty() {
                // Check if we need to create/recreate the frame script instance
                let needs_creation = reserve_player_ref(|player| {
                    player.movie.frame_script_instance.is_none()
                });
                
                if needs_creation {
                    debug!(
                        "🔧 Creating frame script instance from cast {}/{} with {} parameters",
                        behavior_ref.cast_lib,
                        behavior_ref.cast_member,
                        behavior_ref.parameter.len()
                    );

                    // Create the script instance
                    let (script_instance_ref, datum_ref) = Self::create_behavior(
                        behavior_ref.cast_lib as i32,
                        behavior_ref.cast_member as i32,
                    );

                    // Extract ScriptInstanceRef
                    let actual_instance_ref = reserve_player_mut(|player| {
                        match player.get_datum(&datum_ref) {
                            Datum::ScriptInstanceRef(ref inst) => inst.clone(),
                            _ => {
                                web_sys::console::error_1(&"Expected ScriptInstanceRef".into());
                                panic!("Expected ScriptInstanceRef");
                            }
                        }
                    });

                    // Create the CastMemberRef for later use
                    let cast_member_ref = CastMemberRef {
                        cast_lib: behavior_ref.cast_lib as i32,
                        cast_member: behavior_ref.cast_member as i32,
                    };

                    // Set spriteNum property
                    reserve_player_mut(|player| {
                        let sprite_num_ref = player.alloc_datum(Datum::Int(0));
                        let _ = script_set_prop(
                            player,
                            &actual_instance_ref,
                            &"spriteNum".to_string(),
                            &sprite_num_ref,
                            false,
                        );
                    });

                    // Apply behavior parameters
                    if !behavior_ref.parameter.is_empty() {
                        reserve_player_mut(|player| {
                            debug!("  Applying {} parameters", behavior_ref.parameter.len());
                            
                            for param_ref in &behavior_ref.parameter {
                                let param_datum = player.get_datum(param_ref);
                                
                                if let Datum::PropList(props, _) = param_datum {
                                    // Collect properties first
                                    let props_to_set: Vec<(String, DatumRef)> = props.iter()
                                        .filter_map(|(key_ref, value_ref)| {
                                            let key = player.get_datum(key_ref);
                                            if let Datum::Symbol(prop_name) = key {
                                                Some((prop_name.clone(), value_ref.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    
                                    // Then set them
                                    for (prop_name, value_ref) in props_to_set {
                                        debug!("    Setting property: {}", prop_name);
                                        let _ = script_set_prop(
                                            player,
                                            &actual_instance_ref,
                                            &prop_name,
                                            &value_ref,
                                            false,
                                        );
                                    }
                                }
                            }
                        });
                    }

                    // Cache BOTH the instance and the member ref
                    reserve_player_mut(|player| {
                        player.movie.frame_script_instance = Some(actual_instance_ref);
                        player.movie.frame_script_member = Some(cast_member_ref);
                    });

                    debug!("✓ Frame script instance created and cached");
                }
            }
        }

        // Initialize sprites that don't have behaviors (the second loop)
        // BUT: Only process sprites that weren't already initialized in the first loop
        let sprites_to_init: Vec<(i16, ScoreFrameChannelData)> = self
            .channel_initialization_data
            .iter()
            .filter(|(frame_index, channel_index, data)| {
                // Only process sprites for the current frame
                if *frame_index + 1 != frame_num {
                    return false;
                }

                // Skip empty sprites
                if data.cast_lib == 0 && data.cast_member == 0 {
                    return false;
                }

                let channel_num = get_channel_number_from_index(*channel_index as u32) as i16;

                // Skip channel 0 and negative channels
                if channel_num <= 0 {
                    return false;
                }

                // Skip if sprite was already initialized via span (has behaviors)
                let already_in_span = spans_to_enter
                    .iter()
                    .any(|span| span.channel_number == channel_num as u32);

                if already_in_span {
                    return false;
                }

                // Only initialize if there's an active span for this sprite
                let has_active_span = self.sprite_spans
                    .iter()
                    .any(|span| {
                        span.channel_number == channel_num as u32 
                            && Self::is_span_in_frame(span, frame_num)
                    });
                
                if !has_active_span {
                    return false;
                }

                // Skip if already initialized
                let sprite = self.get_sprite(channel_num);

                if sprite.unwrap().entered {
                    return false;
                }
                true
            })
            .map(|(_, channel_index, data)| {
                (
                    get_channel_number_from_index(*channel_index as u32) as i16,
                    data.clone(),
                )
            })
            .collect();

        for (channel_num, data) in &sprites_to_init {
            let sprite = self.get_sprite_mut(*channel_num);
            sprite.entered = true;

            let member = CastMemberRef {
                cast_lib: data.cast_lib as i32,
                cast_member: data.cast_member as i32,
            };

            let _ = sprite_set_prop(*channel_num, "member", Datum::CastMember(member.clone()));
            sprite.ink = data.ink as i32;
            sprite.loc_h = data.pos_x as i32;
            sprite.loc_v = data.pos_y as i32;
            sprite.width = data.width as i32;
            sprite.height = data.height as i32;
            sprite.blend = if data.blend == 0 {
                100
            } else {
                data.blend as i32
            };

            sprite.skew = data.skew as f64;
            sprite.rotation = data.rotation as f64;

            reserve_player_mut(|player| {
                let member_ref = member.clone();

                if let Ok(cast) = player.movie.cast_manager.get_cast(member.cast_lib as u32) {
                    if let Some(real_member) = cast.members.get(&(member.cast_member as u32)) {
                        let type_str = real_member.member_type.type_string();
                        if type_str == "shape" {
                            let blend = if data.blend == 255 {
                                100
                            } else {
                                ((255.0 - data.blend as f32) * 100.0 / 255.0) as u8
                            };
                            sprite.blend = blend as i32;
                            sprite.ink = (data.ink / 5) as i32;
                        }
                    }
                }
            });

            reserve_player_ref(|player| {
                if let Some(member_ref) = &sprite.member {
                    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                        if let CastMemberType::Bitmap(bitmap_member) = &member.member_type {
                            let bw = bitmap_member.info.width as i32;
                            let bh = bitmap_member.info.height as i32;

                            sprite.bitmap_size_owned_by_sprite =
                                sprite.width != bw || sprite.height != bh;
                        }
                    }
                }
            });

            match data.color_flag {
                // fore + back are palette indexes
                0 => {
                    // Foreground
                    sprite.fore_color = data.fore_color as i32;
                    sprite.color = ColorRef::PaletteIndex(data.fore_color);

                    // Background
                    sprite.back_color = data.back_color as i32;
                    sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                }

                // foreColor is RGB, backColor is palette index
                1 => {
                    // Foreground (RGB → map to palette)
                    sprite.color = ColorRef::Rgb(
                        data.fore_color,
                        data.fore_color_g,
                        data.fore_color_b,
                    );
                    sprite.fore_color =
                        sprite.color.to_index(&SYSTEM_WIN_PALETTE) as i32;

                    // Background (palette index)
                    sprite.back_color = data.back_color as i32;
                    sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                }

                // foreColor is palette index, backColor is RGB
                2 => {
                    // Foreground (palette index)
                    sprite.fore_color = data.fore_color as i32;
                    sprite.color = ColorRef::PaletteIndex(data.fore_color);

                    // Background (RGB → map to palette)
                    sprite.bg_color = ColorRef::Rgb(
                        data.back_color,
                        data.back_color_g,
                        data.back_color_b,
                    );
                    sprite.back_color =
                        sprite.bg_color.to_index(&SYSTEM_WIN_PALETTE) as i32;
                }

                // both fore + back are RGB
                3 => {
                    // Foreground (RGB → map to palette)
                    sprite.color = ColorRef::Rgb(
                        data.fore_color,
                        data.fore_color_g,
                        data.fore_color_b,
                    );
                    sprite.fore_color =
                        sprite.color.to_index(&SYSTEM_WIN_PALETTE) as i32;

                    // Background (RGB → map to palette)
                    sprite.bg_color = ColorRef::Rgb(
                        data.back_color,
                        data.back_color_g,
                        data.back_color_b,
                    );
                    sprite.back_color =
                        sprite.bg_color.to_index(&SYSTEM_WIN_PALETTE) as i32;
                }

                _ => {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "Unexpected color flag: {}",
                        data.color_flag
                    )));
                }
            }

            // Also update runtime values to match (apply_tween_modifiers will handle tweening)
            sprite.base_loc_h = sprite.loc_h;
            sprite.base_loc_v = sprite.loc_v;
            sprite.base_width = sprite.width;
            sprite.base_height = sprite.height;
            sprite.base_rotation = sprite.rotation;
            sprite.base_blend = sprite.blend;
            sprite.base_skew = sprite.skew;
            sprite.base_color = sprite.color.clone();
            sprite.base_bg_color = sprite.bg_color.clone();

            // Reset size flags when sprite re-enters
            sprite.has_size_tweened = false;
            sprite.has_size_changed = false;
        }
    }

    pub fn apply_tween_modifiers(&mut self, frame: u32) {
        // Build a set of active channels for this frame
        let active_channels: std::collections::HashSet<u32> = self
            .sprite_spans
            .iter()
            .filter(|span| Self::is_span_in_frame(span, frame))
            .map(|span| span.channel_number)
            .collect();

        for channel in self.channels.iter_mut() {
            let sprite = &mut channel.sprite;
            let sprite_num = sprite.number as u16;

            // Skip if sprite isn't in an active span at this frame
            if !active_channels.contains(&(sprite_num as u32)) {
                if !sprite.puppet || !sprite.visible {
                    continue;
                }
            }

            let Some(keyframes) = self.keyframes_cache.get(&sprite_num) else {
                continue;
            };

            // ---- Position tween (additive) ----
            if let Some(path) = keyframes.path.as_ref() {
                if path
                    .tween_info
                    .as_ref()
                    .is_some_and(|t| t.is_path_tweened())
                    && path.is_active_at_frame(frame)
                {
                    if let Some((dx, dy)) = path.get_delta_at_frame(
                        frame,
                        sprite.base_loc_h,
                        sprite.base_loc_v,
                    ) {
                        debug!(
                            "    🛤️ PATH TWEEN: sprite {} frame {} - base: ({},{}), delta: ({},{}), result: ({},{})",
                            sprite_num, frame,
                            sprite.base_loc_h, sprite.base_loc_v,
                            dx, dy,
                            sprite.base_loc_h + dx, sprite.base_loc_v + dy
                        );

                        sprite.loc_h = sprite.base_loc_h + dx as i32;
                        sprite.loc_v = sprite.base_loc_v + dy as i32;
                    }
                }
            }

            // ---- Size tween (additive) ----
            if let Some(size) = keyframes.size.as_ref() {
                if size
                    .tween_info
                    .as_ref()
                    .is_some_and(|t| t.is_size_tweened())
                    && size.is_active_at_frame(frame)
                {
                    if let Some((dw, dh)) = size.get_delta_at_frame(
                        frame,
                        sprite.base_width,
                        sprite.base_height,
                    ) {
                        debug!(
                            "    📏 SIZE TWEEN: sprite {} frame {} - base: {}x{}, delta: ({},{}), result: {}x{}",
                            sprite_num, frame,
                            sprite.base_width, sprite.base_height,
                            dw, dh,
                            sprite.base_width + dw, sprite.base_height + dh
                        );

                        sprite.width = sprite.base_width + dw as i32;
                        sprite.height = sprite.base_height + dh as i32;

                        sprite.has_size_changed = true;

                        if !sprite.has_size_tweened {
                            sprite.has_size_tweened = true;
                        }
                    }
                } else if sprite.has_size_tweened {
                    sprite.has_size_tweened = false;
                }
            }

            // ---- Rotation tween (additive) ----
            if let Some(rotation) = keyframes.rotation.as_ref() {
                if rotation
                    .tween_info
                    .as_ref()
                    .is_some_and(|t| t.is_rotation_tweened())
                    && rotation.is_active_at_frame(frame)
                {
                    if let Some(dr) = rotation.get_delta_at_frame(
                        frame,
                        sprite.base_rotation,
                    ) {
                        debug!(
                            "    🔄 ROTATION TWEEN: sprite {} frame {} - base: {:.2}°, delta: {:.2}°, result: {:.2}°",
                            sprite_num, frame,
                            sprite.base_rotation,
                            dr,
                            sprite.base_rotation + dr
                        );

                        sprite.rotation = sprite.base_rotation + dr;
                    }
                }
            }

            // ---- Blend tween (absolute) ----
            if let Some(blend) = keyframes.blend.as_ref() {
                if blend
                    .tween_info
                    .as_ref()
                    .is_some_and(|t| t.is_blend_tweened())
                    && blend.is_active_at_frame(frame)
                {
                    if let Some(value) = blend.get_blend_at_frame(frame) {
                        debug!(
                            "    🎨 BLEND TWEEN: sprite {} frame {} - old: {}, new: {}",
                            sprite_num, frame,
                            sprite.blend,
                            value
                        );

                        sprite.blend = value as i32;
                    }
                }
            }

            // ---- Skew tween (additive) ----
            if let Some(skew) = keyframes.skew.as_ref() {
                if skew
                    .tween_info
                    .as_ref()
                    .is_some_and(|t| t.is_skew_tweened())
                    && skew.is_active_at_frame(frame)
                {
                    if let Some(ds) = skew.get_delta_at_frame(
                        frame,
                        sprite.base_skew,
                    ) {
                        debug!(
                            "    ↗️ SKEW TWEEN: sprite {} frame {} - base: {:.2}°, delta: {:.2}°, result: {:.2}°",
                            sprite_num, frame,
                            sprite.base_skew,
                            ds,
                            sprite.base_skew + ds
                        );

                        sprite.skew = sprite.base_skew + ds;
                    }
                }
            }

            // ---- Foreground color tween (absolute) ----
            if let Some(fore_color) = keyframes.fore_color.as_ref() {
                if fore_color
                    .tween_info
                    .as_ref()
                    .is_some_and(|t| t.is_forecolor_tweened())
                    && fore_color.is_active_at_frame(frame)
                {
                    if let Some(color) = fore_color.get_color_at_frame(frame) {
                        debug!(
                            "    🎨 FORECOLOR TWEEN: sprite {} frame {} - old: {:?}, new: {:?}",
                            sprite_num, frame,
                            sprite.color,
                            color
                        );

                        sprite.color = color;
                        sprite.fore_color = sprite.color.to_index(&SYSTEM_WIN_PALETTE) as i32;

                        if !sprite.has_fore_color {
                            sprite.has_fore_color = true;
                        }
                    }
                }
            }

            // ---- Background color tween (absolute) ----
            if let Some(back_color) = keyframes.back_color.as_ref() {
                if back_color
                    .tween_info
                    .as_ref()
                    .is_some_and(|t| t.is_backcolor_tweened())
                    && back_color.is_active_at_frame(frame)
                {
                    if let Some(color) = back_color.get_color_at_frame(frame) {
                        debug!(
                            "    🖌️ BACKCOLOR TWEEN: sprite {} frame {} - old: {:?}, new: {:?}",
                            sprite_num, frame,
                            sprite.bg_color,
                            color
                        );

                        sprite.bg_color = color;
                        sprite.back_color = sprite.bg_color.to_index(&SYSTEM_WIN_PALETTE) as i32;

                        if !sprite.has_back_color {
                            sprite.has_back_color = true;
                        }
                    }
                }
            }
        }
    }

    pub async fn end_sprites(&mut self, prev_frame: u32, next_frame: u32) -> Vec<u32> {
        let channels_to_end: Vec<u32> = self
            .sprite_spans
            .iter()
            .filter(|span| {
                Self::is_span_in_frame(span, prev_frame)
                    && !Self::is_span_in_frame(span, next_frame)
            })
            .map(|span| span.channel_number)
            .collect_vec();

        let _ = dispatch_event_endsprite(channels_to_end.clone()).await;

        channels_to_end
    }

    pub fn get_channel_count(&self) -> usize {
        return self.channels.len() - 1;
    }

    pub fn set_channel_count(&mut self, new_count: usize) {
        if new_count > self.channels.len() {
            let base_number = self.channels.len();
            let add_count = max(0, new_count - self.channels.len());
            let mut add_channels = (0..add_count)
                .map(|index| SpriteChannel::new(base_number + index))
                .collect_vec();
            self.channels.append(&mut add_channels);
        } else if new_count < self.channels.len() {
            let remove_count = self.channels.len() - new_count;
            for _ in 1..remove_count {
                self.channels.pop();
            }
        }

        JsApi::dispatch_score_changed();
    }

    #[allow(dead_code)]
    pub fn get_sprite(&self, number: i16) -> Option<&Sprite> {
        if number < 0 || number as usize > self.channels.len() - 1 {
            return None;
        }
        let channel = &self.channels.get(number as usize);
        return channel.map(|x| &x.sprite);
    }

    pub fn get_channel(&self, number: i16) -> &SpriteChannel {
        return &self.channels[number as usize];
    }

    pub fn get_sprite_mut(&mut self, number: i16) -> &mut Sprite {
        let channel = &mut self.channels[number as usize];
        return &mut channel.sprite;
    }

    pub fn load_from_score_chunk(
        &mut self,
        score_chunk: &crate::director::chunks::score::ScoreChunk,
    ) {
        self.set_channel_count(score_chunk.frame_data.header.num_channels as usize);

        self.channel_initialization_data = score_chunk.frame_data.frame_channel_data.clone();
        self.sound_channel_data = score_chunk.frame_data.sound_channel_data.clone();
        self.tempo_channel_data = score_chunk.frame_data.tempo_channel_data.clone();
        self.keyframes_cache = build_all_keyframes_cache(
            &score_chunk.frame_data.frame_channel_data,
            &score_chunk.frame_intervals
        );

        for (primary, secondary) in &score_chunk.frame_intervals {
            let is_frame_script_or_sprite_script =
                primary.channel_index == 0 || primary.channel_index > 5;
            if is_frame_script_or_sprite_script {
                // TODO support the other 5 reserved channels
                let sprite_span = ScoreSpriteSpan {
                    channel_number: get_channel_number_from_index(primary.channel_index),
                    start_frame: primary.start_frame,
                    end_frame: primary.end_frame,
                    scripts: match secondary {
                        Some(sec) => vec![ScoreBehaviorReference {
                            cast_lib: sec.cast_lib,
                            cast_member: sec.cast_member,
                            parameter: sec.parameter.clone(),
                        }],
                        None => Vec::new(),
                    },
                };
                self.sprite_spans.push(sprite_span);
            }
        }
    }

    pub fn load_from_dir(&mut self, dir: &DirectorFile) {
        let score_chunk = dir.score.as_ref().unwrap();
        let frame_labels_chunk = dir.frame_labels.as_ref();
        if frame_labels_chunk.is_some() {
            self.frame_labels = frame_labels_chunk.unwrap().labels.clone();
        }
        self.load_from_score_chunk(score_chunk);
        JsApi::dispatch_score_changed();
    }

    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            // Clear script instances for ALL sprites, not just puppeted ones
            // This prevents stale ScriptInstanceRef objects from pointing to deleted instances
            channel.sprite.script_instance_list.clear();

            if channel.sprite.puppet {
                channel.sprite.reset();
            }
        }

        JsApi::dispatch_score_changed();
    }

    pub fn get_sorted_channels(&self) -> Vec<&SpriteChannel> {
        // Build set of active channel numbers for current frame
        let current_frame = reserve_player_ref(|player| player.movie.current_frame);
        let active_channels: std::collections::HashSet<u32> = self
            .sprite_spans
            .iter()
            .filter(|span| Self::is_span_in_frame(span, current_frame))
            .map(|span| span.channel_number)
            .collect();

        return self
            .channels
            .iter()
            .filter(|x| {
                // Skip channel 0 (frame behaviors)
                if x.number == 0 {
                    return false;
                }
                // Render if: in active span OR is puppeted
                let is_active = active_channels.contains(&(x.number as u32)) || x.sprite.puppet;
                if !is_active {
                    return false;
                }
                x.sprite.member.is_some()
                    && x.sprite.member.as_ref().unwrap().is_valid()
                    && x.sprite.visible
            })
            .sorted_by(|a, b| {
                let res = a.sprite.loc_z.cmp(&b.sprite.loc_z);
                if res == std::cmp::Ordering::Equal {
                    a.number.cmp(&b.number)
                } else {
                    res
                }
            })
            .collect_vec();
    }

    pub fn get_active_script_instance_list(&self) -> Vec<ScriptInstanceRef> {
        let mut instance_list = vec![];
        for channel in &self.channels {
            for instance_ref in &channel.sprite.script_instance_list {
                instance_list.push(instance_ref.clone());
            }
        }
        return instance_list;
    }

    pub fn get_frame_tempo(&self, frame: u32) -> Option<u32> {
        // Search through tempo_channel_data to find the most recent tempo change
        // at or before the requested frame
        self.tempo_channel_data
            .iter()
            .rev() // Search backwards from the end (most recent first)
            .find(|(frame_idx, _)| *frame_idx <= frame)
            .map(|(_, tempo_data)| tempo_data.tempo as u32)
    }
}

pub fn sprite_get_prop(
    player: &mut DirPlayer,
    sprite_id: i16,
    prop_name: &str,
) -> Result<Datum, ScriptError> {
    let sprite = player.movie.score.get_sprite(sprite_id);
    match prop_name {
        "ilk" => Ok(Datum::Symbol("sprite".to_string())),
        "spriteNum" | "spriteNumber" => Ok(Datum::Int(
            sprite.map_or(sprite_id as i32, |x| x.number as i32),
        )),
        "loc" => reserve_player_mut(|player| {
            let (x, y) = sprite.map_or((0, 0), |sprite| (sprite.loc_h, sprite.loc_v));
            let x_ref = player.alloc_datum(Datum::Int(x));
            let y_ref = player.alloc_datum(Datum::Int(y));
            Ok(Datum::Point([x_ref, y_ref]))
        }),
        "width" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.width) as i32)),
        "height" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.height) as i32)),
        "blend" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.blend) as i32)),
        "ink" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.ink) as i32)),
        "left" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.0 as i32))
        }
        "top" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.1 as i32))
        }
        "right" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.2 as i32))
        }
        "bottom" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Int(rect.3 as i32))
        }
        "rect" => {
            let rect = get_sprite_rect(player, sprite_id);
            Ok(Datum::Rect([
                player.alloc_datum(Datum::Int(rect.0)),
                player.alloc_datum(Datum::Int(rect.1)),
                player.alloc_datum(Datum::Int(rect.2)),
                player.alloc_datum(Datum::Int(rect.3)),
            ]))
        }
        "color" => Ok(Datum::ColorRef(
            sprite.map_or(ColorRef::PaletteIndex(255), |sprite| sprite.color.clone()),
        )),
        "bgColor" => Ok(Datum::ColorRef(
            sprite.map_or(ColorRef::PaletteIndex(0), |sprite| sprite.bg_color.clone()),
        )),
        "skew" => Ok(Datum::Float(sprite.map_or(0.0, |sprite| sprite.skew))),
        "locH" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_h) as i32)),
        "locV" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_v) as i32)),
        "locZ" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.loc_z) as i32)),
        "member" => Ok(Datum::CastMember(
            sprite
                .and_then(|x| x.member.as_ref())
                .map(|x| x.clone())
                .unwrap_or(NULL_CAST_MEMBER_REF),
        )),
        "flipH" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_h))),
        "flipV" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_v))),
        "rotation" => Ok(Datum::Float(sprite.map_or(0.0, |sprite| sprite.rotation))),
        "scriptInstanceList" => {
            let instance_ids = sprite.map_or(vec![], |x| x.script_instance_list.clone());
            let instance_ids = instance_ids
                .iter()
                .map(|x| player.alloc_datum(Datum::ScriptInstanceRef(x.clone())))
                .collect();
            Ok(Datum::List(DatumType::List, instance_ids, false))
        }
        "castNum" => Ok(Datum::Int(sprite.map_or(0, |x| {
            x.member.as_ref().map_or(0, |y| {
                CastMemberRefHandlers::get_cast_slot_number(y.cast_lib as u32, y.cast_member as u32)
                    as i32
            })
        }))),
        "scriptNum" => {
            let script_num = sprite
                .and_then(|sprite| sprite.script_instance_list.first())
                .map(|script_instance_ref| {
                    player.allocator.get_script_instance(&script_instance_ref)
                })
                .map(|script_instance| script_instance.script.cast_member);
            Ok(Datum::Int(script_num.unwrap_or(0)))
        }
        "visible" => Ok(datum_bool(sprite.map_or(true, |sprite| sprite.visible))),
        "puppet" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.puppet))),
        "foreColor" => Ok(Datum::Int(
            sprite.map_or(255, |sprite| sprite.color.to_index(&SYSTEM_WIN_PALETTE)) as i32,
        )),
        "backColor" => Ok(Datum::Int(
            sprite.map_or(0, |sprite| sprite.bg_color.to_index(&SYSTEM_WIN_PALETTE)) as i32,
        )),
        "cursor" => {
            let cursor_ref = sprite.and_then(|sprite| sprite.cursor_ref.clone());
            match cursor_ref {
                Some(CursorRef::System(id)) => Ok(Datum::Int(id)),
                Some(CursorRef::Member(ids)) => {
                    let id_refs = ids
                        .iter()
                        .map(|id| player.alloc_datum(Datum::Int(*id)))
                        .collect();
                    Ok(Datum::List(DatumType::List, id_refs, false))
                }
                None => Ok(Datum::Int(0)),
            }
        }
        "startFrame" => {
            let current_frame = player.movie.current_frame;
            let start_frame = player
                .movie
                .score
                .sprite_spans
                .iter()
                .find(|span| {
                    span.channel_number == sprite_id as u32
                        && current_frame >= span.start_frame
                        && current_frame <= span.end_frame
                })
                .map(|span| span.start_frame)
                .unwrap_or(0);
            Ok(Datum::Int(start_frame as i32))
        }
        "endFrame" => {
            let current_frame = player.movie.current_frame;
            let end_frame = player
                .movie
                .score
                .sprite_spans
                .iter()
                .find(|span| {
                    span.channel_number == sprite_id as u32
                        && current_frame >= span.start_frame
                        && current_frame <= span.end_frame
                })
                .map(|span| span.end_frame)
                .unwrap_or(0);
            Ok(Datum::Int(end_frame as i32))
        }
        prop_name => {
            let datum_ref = sprite.and_then(|sprite| {
                reserve_player_mut(|player| {
                    sprite.script_instance_list.iter().find_map(|behavior| {
                        script_get_prop_opt(player, behavior, &prop_name.to_string())
                    })
                })
            });
            match datum_ref {
                Some(ref_) => Ok(player.get_datum(&ref_).clone()),
                None => {
                    return Err(ScriptError::new(format!(
                        "Cannot get prop {} of sprite",
                        prop_name
                    )))
                }
            }
        }
    }
}

pub fn borrow_sprite_mut<T1, F1, T2, F2>(sprite_id: i16, player_f: F2, f: F1) -> T1
where
    F1: FnOnce(&mut Sprite, T2) -> T1,
    F2: FnOnce(&DirPlayer) -> T2,
{
    reserve_player_mut(|player| {
        let arg = player_f(player);
        let sprite = player.movie.score.get_sprite_mut(sprite_id);
        f(sprite, arg)
    })
}

pub fn sprite_set_prop(sprite_id: i16, prop_name: &str, value: Datum) -> Result<(), ScriptError> {
    let result = match prop_name {
        "visible" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.visible = value.to_bool()?;
                Ok(())
            },
        ),
        "stretch" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.stretch = value?;
                Ok(())
            },
        ),
        "locH" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.loc_h = value?;
                Ok(())
            },
        ),
        "locV" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.loc_v = value?;
                Ok(())
            },
        ),
        "locZ" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.loc_z = value?;
                Ok(())
            },
        ),
        "width" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.width = value?;
                sprite.has_size_changed = true;
                Ok(())
            },
        ),
        "height" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.height = value?;
                sprite.has_size_changed = true;
                Ok(())
            },
        ),
        "ink" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.ink = value?;
                Ok(())
            },
        ),
        "blend" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.blend = value?;
                Ok(())
            },
        ),
        "rotation" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.rotation = value.to_float()?;
                } else {
                    sprite.rotation = 0.0;
                }
                Ok(())
            },
        ),
        "skew" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.skew = value.to_float()?;
                } else {
                    sprite.skew = 0.0;
                }
                Ok(())
            },
        ),
        "flipH" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.flip_h = value.to_bool()?
                } else {
                    sprite.flip_h = false;
                }
                Ok(())
            },
        ),
        "flipV" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                if value.is_number() {
                    sprite.flip_v = value.to_bool()?
                } else {
                    sprite.flip_v = false;
                }
                Ok(())
            },
        ),
        "backColor" => borrow_sprite_mut(
            sprite_id,
            |_| (),
            |sprite, _| {
                let v = value.int_value()?;
                sprite.back_color = v;
                sprite.bg_color = ColorRef::PaletteIndex(v as u8);
                sprite.has_back_color = true;
                Ok(())
            },
        ),
        "bgColor" => borrow_sprite_mut(
            sprite_id,
            |_| (),
            |sprite, _| {
                sprite.bg_color = value.to_color_ref()?.to_owned();
                sprite.back_color = sprite.bg_color.to_index(&SYSTEM_WIN_PALETTE) as i32;
                sprite.has_back_color = true;
                Ok(())
            },
        ),
        "foreColor" => borrow_sprite_mut(
            sprite_id,
            |_| (),
            |sprite, _| {
                let v = value.int_value()?;
                sprite.fore_color = v;
                sprite.color = ColorRef::PaletteIndex(v as u8);
                sprite.has_fore_color = true;
                Ok(())
            },
        ),
        "color" => borrow_sprite_mut(
            sprite_id,
            |_| (),
            |sprite, _| {
                sprite.color = value.to_color_ref()?.to_owned();
                sprite.fore_color = sprite.color.to_index(&SYSTEM_WIN_PALETTE) as i32;
                sprite.has_fore_color = true;
                Ok(())
            },
        ),
        // Member properties
        "member" => borrow_sprite_mut(
            sprite_id,
            |player| {
                // Resolve member reference
                let mem_ref = if let Datum::CastMember(cast_member) = &value {
                    Some(cast_member.clone())
                } else if value.is_string() {
                    player
                        .movie
                        .cast_manager
                        .find_member_ref_by_name(&value.string_value()?)
                } else if value.is_number() {
                    player
                        .movie
                        .cast_manager
                        .find_member_ref_by_number(value.int_value()? as u32)
                } else {
                    None
                };

                // Extract intrinsic size ONLY for Bitmap / Shape
                let intrinsic_size = mem_ref
                    .as_ref()
                    .and_then(|r| player.movie.cast_manager.find_member_by_ref(r))
                    .and_then(|m| match &m.member_type {
                        CastMemberType::Bitmap(bitmap) => {
                            Some((bitmap.info.width as i32, bitmap.info.height as i32))
                        }
                        CastMemberType::Shape(shape) => {
                            Some((shape.shape_info.width as i32, shape.shape_info.height as i32))
                        }
                        _ => None,
                    });

                Ok((mem_ref, intrinsic_size))
            },
            |sprite, value| {
                let (mem_ref, intrinsic_size) = value?;

                // Detect whether the member actually changed
                let member_changed = sprite.member != mem_ref;

                // Assign the new member
                sprite.member = mem_ref;

                // Initialize size ONLY if:
                //  - member actually changed
                if member_changed && !sprite.has_size_changed {
                    if let Some((w, h)) = intrinsic_size {
                        if w > 0 && h > 0 {
                            sprite.width = w;
                            sprite.height = h;
                            sprite.base_width = w;
                            sprite.base_height = h;
                        }
                    }
                }

                JsApi::on_sprite_member_changed(sprite_id);
                Ok(())
            },
        ),
        "memberNum" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                let value = value?;
                let new_member_ref = match &sprite.member {
                    Some(member_ref) => cast_member_ref(member_ref.cast_lib, value),
                    None => CastMemberRefHandlers::member_ref_from_slot_number(value as u32),
                };
                sprite.member = Some(new_member_ref);
                JsApi::on_sprite_member_changed(sprite_id);
                Ok(())
            },
        ),
        "castNum" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                let value = value?;
                let new_member_ref =
                    CastMemberRefHandlers::member_ref_from_slot_number(value as u32);
                sprite.member = Some(new_member_ref);
                JsApi::on_sprite_member_changed(sprite_id);
                Ok(())
            },
        ),
        "cursor" => borrow_sprite_mut(
            sprite_id,
            |player| {
                if value.is_int() {
                    Ok(CursorRef::System(value.int_value()?))
                } else if value.is_list() {
                    let mut cursor_ids = vec![];
                    for cursor_id in value.to_list()? {
                        cursor_ids.push(player.get_datum(cursor_id).int_value()?);
                    }
                    Ok(CursorRef::Member(cursor_ids))
                } else {
                    Err(ScriptError::new(
                        "cursor must be a number or a list".to_string(),
                    ))
                }
            },
            |sprite, cursor_ref| {
                sprite.cursor_ref = Some(cursor_ref?);
                Ok(())
            },
        ),
        "loc" => borrow_sprite_mut(
            sprite_id,
            |_| value.clone(),  // Pass the value through so we can use it in the sprite closure
            |sprite, value| -> Result<(), ScriptError> {
                match value {
                    Datum::Point(arr) => {
                        reserve_player_mut(|player| {
                            let x = player.get_datum(&arr[0]).int_value()?;
                            let y = player.get_datum(&arr[1]).int_value()?;
                            sprite.loc_h = x;
                            sprite.loc_v = y;
                            Ok(())
                        })
                    }
                    Datum::Void => Ok(()), // no-op
                    _ => Err(ScriptError::new(format!(
                        "loc must be a Point (received {})",
                        value.type_str()
                    ))),
                }
            },
        ),
        "rect" => reserve_player_mut(|player| {
            borrow_sprite_mut(
                sprite_id,
                |player| {
                    let sprite = player.movie.score.get_sprite(sprite_id).unwrap();
                    let member_ref = sprite.member.as_ref();
                    let cast_member =
                        member_ref.and_then(|x| player.movie.cast_manager.find_member_by_ref(&x));
                    let reg_point = cast_member
                        .map(|x| match &x.member_type {
                            CastMemberType::Bitmap(bitmap) => bitmap.reg_point,
                            _ => (0, 0),
                        })
                        .unwrap_or((0, 0));

                    reg_point
                },
                |sprite, reg_point| {
                    match value {
                        Datum::Rect(ref arr) => {
                            let left = player.get_datum(&arr[0]).int_value()?;
                            let top = player.get_datum(&arr[1]).int_value()?;
                            let right = player.get_datum(&arr[2]).int_value()?;
                            let bottom = player.get_datum(&arr[3]).int_value()?;

                            sprite.loc_h = left + reg_point.0 as i32;
                            sprite.loc_v = top + reg_point.1 as i32;
                            sprite.width = right - left;
                            sprite.height = bottom - top;
                            Ok(())
                        }
                        _ => Err(ScriptError::new("rect must be a rect".to_string())),
                    }
                },
            )
        }),
        "scriptInstanceList" => {
            let ref_list = value.to_list()?;
            let instance_refs = borrow_sprite_mut(
                sprite_id,
                |player| {
                    let mut instance_ids = vec![];
                    for ref_id in ref_list {
                        let datum = player.get_datum(ref_id);
                        match datum {
                            Datum::ScriptInstanceRef(instance_id) => {
                                instance_ids.push(instance_id.clone());
                            }
                            _ => {
                                return Err(ScriptError::new(
                                    "Cannot set non-script to scriptInstanceList".to_string(),
                                ))
                            }
                        }
                    }
                    Ok(instance_ids)
                },
                |sprite, value| {
                    let instance_ids = value?;
                    sprite.script_instance_list = instance_ids.to_owned();
                    Ok(instance_ids)
                },
            )?;
            reserve_player_mut(|player| {
                let value_ref = player.alloc_datum(Datum::Int(sprite_id as i32));
                for instance_ref in instance_refs {
                    script_set_prop(
                        player,
                        &instance_ref,
                        &"spriteNum".to_string(),
                        &value_ref,
                        false,
                    )?
                }
                Ok(())
            })
        }
        "editable" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.editable = value.to_bool()?;
                Ok(())
            },
        ),
        "quad" => borrow_sprite_mut(
            sprite_id,
            |player| {
                // quad should be a list of 4 points: [topLeft, topRight, bottomRight, bottomLeft]
                if let Ok(list) = value.to_list() {
                    if list.len() == 4 {
                        let mut points = Vec::new();
                        for point_ref in list {
                            let point_datum = player.get_datum(point_ref);
                            let point_arr = point_datum.to_point()?;
                            let x = player.get_datum(&point_arr[0]).int_value()?;
                            let y = player.get_datum(&point_arr[1]).int_value()?;
                            points.push((x, y));
                        }
                        Ok(points)
                    } else {
                        Err(ScriptError::new(
                            "quad must be a list of 4 points".to_string(),
                        ))
                    }
                } else {
                    Err(ScriptError::new("quad must be a list".to_string()))
                }
            },
            |sprite, points| {
                let points = points?;
                sprite.quad = Some([points[0], points[1], points[2], points[3]]);
                Ok(())
            },
        ),
        "puppet" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.puppet = value.to_bool()?;
                Ok(())
            },
        ),
        prop_name => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite
                    .script_instance_list
                    .iter()
                    .find_map(|behavior| {
                        reserve_player_mut(|player| {
                            let value_ref = player.alloc_datum(value.clone());
                            match script_set_prop(
                                player,
                                behavior,
                                &prop_name.to_string(),
                                &value_ref,
                                true,
                            ) {
                                Ok(_) => Some(Ok(())),
                                Err(_) => None,
                            }
                        })
                    })
                    .unwrap_or_else(|| {
                        Err(ScriptError::new(format!(
                            "Cannot set prop {} of sprite",
                            prop_name
                        )))
                    })
            },
        ),
    };
    if result.is_ok() {
        JsApi::dispatch_channel_changed(sprite_id);
    }
    result
}

pub fn concrete_sprite_hit_test(player: &DirPlayer, sprite: &Sprite, x: i32, y: i32) -> bool {
    // Don't test collision for invisible sprites
    if !sprite.visible || !sprite.member.is_some() {
        return false;
    }
    let rect = get_concrete_sprite_rect(player, sprite);

    // Don't test collision for sprites positioned far off-screen
    // If the entire rect is in negative space, it's intentionally hidden
    if rect.right < 0 || rect.bottom < 0 {
        return false;
    }

    let left = rect.left;
    let top = rect.top;
    let right = rect.right;
    let bottom = rect.bottom;
    return x >= left && x < right && y >= top && y < bottom;
}

pub fn get_sprite_at(player: &DirPlayer, x: i32, y: i32, scripted: bool) -> Option<u32> {
    for channel in player.movie.score.get_sorted_channels().iter().rev() {
        if concrete_sprite_hit_test(player, &channel.sprite, x, y)
            && (!scripted || channel.sprite.script_instance_list.len() > 0)
        {
            return Some(channel.sprite.number as u32);
        }
    }

    return None;
}

pub fn get_concrete_sprite_rect(player: &DirPlayer, sprite: &Sprite) -> IntRect {
    let member = sprite
        .member
        .as_ref()
        .and_then(|member_ref| player.movie.cast_manager.find_member_by_ref(member_ref));
    if member.is_none() {
        return IntRect::from_size(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height);
    }
    let member = member.unwrap();

    match &member.member_type {
        CastMemberType::Bitmap(bitmap_member) => {
            let sprite_bitmap = player
                .bitmap_manager
                .get_bitmap(bitmap_member.image_ref);

            if sprite_bitmap.is_none() {
                return IntRect::from(
                    sprite.loc_h,
                    sprite.loc_v,
                    sprite.width,
                    sprite.height,
                );
            }

            let src_bitmap = sprite_bitmap.unwrap();

            // Always use original registration point
            let reg_x = bitmap_member.reg_point.0;
            let reg_y = bitmap_member.reg_point.1;

            // Compute draw origin, compensating for flipH/flipV
            let mut draw_x = sprite.loc_h - reg_x as i32;
            let mut draw_y = sprite.loc_v - reg_y as i32;

            let mut dst_rect = IntRect::from(0, 0, 0, 0);
            let mut option = 0;

            if sprite.has_size_changed {
                option = 1;

                let sprite_width;
                let sprite_height;

                if sprite.has_size_tweened {
                    // For tweened sprites, width/height are already correct
                    sprite_width = sprite.width;
                    sprite_height = sprite.height;
                } else {
                    // For non-tweened sprites, handle wrong score data
                    if (bitmap_member.info.width as i32
                        + bitmap_member.info.height as i32)
                        > (sprite.width + sprite.height)
                    {
                        if sprite.flip_h && reg_x != (bitmap_member.info.width as i32 / 2) as i16 {
                            draw_x = sprite.loc_h - (bitmap_member.info.width as i32 - reg_x as i32);
                        }

                        if sprite.flip_v && reg_y != (bitmap_member.info.height as i32 / 2) as i16 {
                            draw_y = sprite.loc_v - (bitmap_member.info.height as i32 - reg_y as i32);
                        }

                        sprite_width = bitmap_member.info.width as i32;
                        sprite_height = bitmap_member.info.height as i32;
                    } else {
                        if sprite.flip_h && reg_x != (sprite.width / 2) as i16 {
                            draw_x = sprite.loc_h - (sprite.width - reg_x as i32);
                        }

                        if sprite.flip_v && reg_y != (sprite.height / 2) as i16 {
                            draw_y = sprite.loc_v - (sprite.height - reg_y as i32);
                        }

                        sprite_width = sprite.width;
                        sprite_height = sprite.height;
                    }
                }

                let left = draw_x;
                let top = draw_y;
                let right = sprite_width + left;
                let bottom = sprite_height + top;

                dst_rect = IntRect::from(left, top, right, bottom);
            } else if sprite.bitmap_size_owned_by_sprite {
                option = 7;

                if sprite.flip_h && reg_x != (bitmap_member.info.width as i32 / 2) as i16 {
                    draw_x = sprite.loc_h - (bitmap_member.info.width as i32 - reg_x as i32);
                }

                if sprite.flip_v && reg_y != (bitmap_member.info.height as i32 / 2) as i16 {
                    draw_y = sprite.loc_v - (bitmap_member.info.height as i32 - reg_y as i32);
                }

                let sprite_width = bitmap_member.info.width as i32;
                let sprite_height = bitmap_member.info.height as i32;
                let left = draw_x;
                let top = draw_y;
                let right = sprite_width + left;
                let bottom = sprite_height + top;

                dst_rect = IntRect::from(left, top, right, bottom);
            } else if sprite.width > player.movie.rect.width()
                && sprite.height > player.movie.rect.height()
            {
                option = 2;

                let sprite_width = sprite.width as i32;
                let sprite_height = sprite.height as i32;

                let left =
                    ((player.movie.rect.width() / 2) / sprite.loc_h)
                        - reg_x as i32;
                let top =
                    ((player.movie.rect.height() / 2) / sprite.loc_v)
                        - reg_y as i32;

                let right =
                    player.movie.rect.width() + reg_x as i32 + left;
                let bottom =
                    player.movie.rect.height() + reg_y as i32 + top;

                dst_rect = IntRect::from(left, top, right, bottom);
            } else if bitmap_member.info.width == 0
                && bitmap_member.info.height == 0
            {
                option = 3;

                dst_rect = IntRect::from(
                    sprite.loc_h - reg_x as i32,
                    sprite.loc_v - reg_y as i32,
                    sprite.loc_h - reg_x as i32 + sprite.width,
                    sprite.loc_v - reg_y as i32 + sprite.height,
                );
            } else if (i32::from(bitmap_member.info.width) < sprite.width
                && i32::from(bitmap_member.info.height) < sprite.height)
                || (i32::from(bitmap_member.info.width) > sprite.width
                    && i32::from(bitmap_member.info.height) > sprite.height)
            {
                option = 4;

                let sprite_width = bitmap_member.info.width as i32;
                let sprite_height = bitmap_member.info.height as i32;

                if sprite.flip_h && reg_x != (bitmap_member.info.width as i32 / 2) as i16 {
                    draw_x = sprite.loc_h - (bitmap_member.info.width as i32 - reg_x as i32);
                }

                if sprite.flip_v && reg_y != (bitmap_member.info.height as i32 / 2) as i16 {
                    draw_y = sprite.loc_v - (bitmap_member.info.height as i32 - reg_y as i32);
                }

                let left = draw_x;
                let top = draw_y;
                let right = sprite_width + left;
                let bottom = sprite_height + top;

                dst_rect = IntRect::from(left, top, right, bottom);
            } else if sprite.width > i32::from(bitmap_member.info.width)
                || sprite.height > i32::from(bitmap_member.info.height)
                || (sprite.width
                    == i32::from(bitmap_member.info.width)
                    && sprite.height
                        == i32::from(bitmap_member.info.height))
            {
                option = 5;

                let sprite_width;
                let sprite_height;

                if (bitmap_member.info.width as i32
                    + bitmap_member.info.height as i32)
                    > (sprite.width + sprite.height)
                {
                    sprite_width = bitmap_member.info.width as i32;
                    sprite_height = bitmap_member.info.height as i32;

                    if sprite.flip_h && reg_x != (bitmap_member.info.width as i32 / 2) as i16 {
                        draw_x = sprite.loc_h - (bitmap_member.info.width as i32 - reg_x as i32);
                    }

                    if sprite.flip_v && reg_y != (bitmap_member.info.height as i32 / 2) as i16 {
                        draw_y = sprite.loc_v - (bitmap_member.info.height as i32 - reg_y as i32);
                    }
                } else {
                    sprite_width = sprite.width;
                    sprite_height = sprite.height;

                    if sprite.flip_h && reg_x != (sprite.width / 2) as i16 {
                        draw_x = sprite.loc_h - (sprite.width - reg_x as i32);
                    }

                    if sprite.flip_v && reg_y != (sprite.height / 2) as i16 {
                        draw_y = sprite.loc_v - (sprite.height - reg_y as i32);
                    }
                }

                let left = draw_x;
                let top = draw_y;
                let right = sprite_width + left;
                let bottom = sprite_height + top;

                dst_rect = IntRect::from(left, top, right, bottom);
            } else {
                option = 6;

                if sprite.flip_h && reg_x != (sprite.width / 2) as i16 {
                    draw_x = sprite.loc_h - (sprite.width - reg_x as i32);
                }

                if sprite.flip_v && reg_y != (sprite.height / 2) as i16 {
                    draw_y = sprite.loc_v - (sprite.height - reg_y as i32);
                }

                dst_rect = IntRect::from(
                    draw_x,
                    draw_y,
                    sprite.loc_h - reg_x as i32 + sprite.width,
                    sprite.loc_v - reg_y as i32 + sprite.height,
                );
            }

            if sprite.number == 12 {
                debug!(
                    "Sprite {} dimensions {}x{} was chosen Option {} and got this rect: {:?}",
                    sprite.number,
                    sprite.width,
                    sprite.height,
                    option,
                    dst_rect
                );
            }

            dst_rect
        }

        CastMemberType::Shape(shape_member) => {
            let reg_x = shape_member.shape_info.reg_point.0;
            let reg_y = shape_member.shape_info.reg_point.1;
            IntRect::from(
                sprite.loc_h - reg_x as i32,
                sprite.loc_v - reg_y as i32,
                sprite.width + sprite.loc_h - reg_x as i32,
                sprite.height + sprite.loc_v - reg_y as i32,
            )
        }
        CastMemberType::Field(field_member) => {
            IntRect::from_size(sprite.loc_h, sprite.loc_v, field_member.width as i32, 12)
        } // TODO
        CastMemberType::Text(text_member) => {
            IntRect::from_size(sprite.loc_h, sprite.loc_v, text_member.width as i32, 12)
        } // TODO
        _ => IntRect::from_size(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height),
    }
}

pub fn get_score<'a>(movie: &'a Movie, score_source: &ScoreRef) -> Option<&'a Score> {
    match score_source {
        ScoreRef::Stage => Some(&movie.score),
        ScoreRef::FilmLoop(member_ref) => {
            let member = movie.cast_manager.find_member_by_ref(member_ref);
            if member.is_none() {
                return None;
            }
            let member = member.unwrap();
            match &member.member_type {
                CastMemberType::FilmLoop(film_loop_member) => Some(&film_loop_member.score),
                _ => return None,
            }
        }
    }
}

pub fn get_score_mut<'a>(movie: &'a mut Movie, score_source: &ScoreRef) -> Option<&'a mut Score> {
    match score_source {
        ScoreRef::Stage => Some(&mut movie.score),
        ScoreRef::FilmLoop(member_ref) => {
            let member = movie.cast_manager.find_mut_member_by_ref(member_ref);
            if member.is_none() {
                return None;
            }
            let member = member.unwrap();
            match &mut member.member_type {
                CastMemberType::FilmLoop(film_loop_member) => Some(&mut film_loop_member.score),
                _ => return None,
            }
        }
    }
}

pub fn get_score_sprite<'a>(
    movie: &'a Movie,
    score_source: &ScoreRef,
    channel_num: i16,
) -> Option<&'a Sprite> {
    let score = get_score(movie, score_source)?;
    score.get_sprite(channel_num)
}

pub fn get_score_sprite_mut<'a>(
    movie: &'a mut Movie,
    score_source: &ScoreRef,
    channel_num: i16,
) -> Option<&'a mut Sprite> {
    let score = get_score_mut(movie, score_source)?;
    Some(score.get_sprite_mut(channel_num))
}
