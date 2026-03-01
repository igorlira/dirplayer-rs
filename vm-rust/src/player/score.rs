use std::cmp::max;

use itertools::Itertools;
use log::debug;
use wasm_bindgen::JsValue;
use std::collections::{HashMap, HashSet};

use crate::{
    console_warn,
    director::{
        chunks::score::{FrameLabel, ScoreFrameChannelData, SoundChannelData, TempoChannelData},
        file::DirectorFile,
        lingo::datum::{datum_bool, Datum, DatumType},
    },
    js_api::JsApi,
    player::bitmap::drawing::should_matte_sprite,
    player::bitmap::palette::SYSTEM_WIN_PALETTE,
    player::events::{dispatch_event_endsprite, dispatch_event_endsprite_for_score},
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
    DirPlayer, ScriptError, PLAYER_OPT,
};

#[derive(Clone, Debug)]
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
    /// Sprite detail behaviors indexed by spriteListIdx (D6+)
    pub sprite_details: HashMap<u32, crate::director::chunks::score::SpriteDetailInfo>,
    /// Track the last frame where we cleared sound triggers (to prevent double-clearing)
    pub last_sound_clear_frame: Option<u32>,
    /// D5 movies need per-frame sprite property updates from channel_initialization_data
    /// (since sprite properties can change every frame via delta compression)
    pub needs_per_frame_updates: bool,
    /// Total frame count (used for auto-looping back to frame 1 when past last frame)
    pub frame_count: Option<u32>,
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

/// Get a sprite from the appropriate score based on the current_score_context.
/// When running filmloop behaviors, this returns sprites from the filmloop's score.
/// Otherwise, returns sprites from the main stage.
pub fn get_sprite_in_context<'a>(player: &'a DirPlayer, sprite_id: i16) -> Option<&'a Sprite> {
    match &player.current_score_context {
        ScoreRef::Stage => player.movie.score.get_sprite(sprite_id),
        ScoreRef::FilmLoop(member_ref) => {
            player.movie.cast_manager.find_member_by_ref(member_ref)
                .and_then(|member| {
                    if let CastMemberType::FilmLoop(film_loop) = &member.member_type {
                        film_loop.score.get_sprite(sprite_id)
                    } else {
                        None
                    }
                })
        }
    }
}

/// Get a mutable reference to a sprite from the appropriate score based on current_score_context.
/// Note: For filmloop sprites, this returns None and the caller should handle the fallback.
/// This is due to Rust's borrow checker limitations with the cast_manager lookup.
pub fn get_sprite_in_context_mut<'a>(player: &'a mut DirPlayer, sprite_id: i16) -> Option<&'a mut Sprite> {
    match &player.current_score_context {
        ScoreRef::Stage => Some(player.movie.score.get_sprite_mut(sprite_id)),
        ScoreRef::FilmLoop(_member_ref) => {
            // For filmloop context, we return None here.
            // The caller (borrow_sprite_mut) will fall back to the main stage sprite,
            // which is correct because filmloop sprites are rendered copies and
            // modifications should typically go to the source sprite or be handled specially.
            // The read-only get_sprite_in_context is the important fix for the original bug.
            None
        }
    }
}

/// Get sprite rect using current score context
fn get_sprite_rect_in_context(player: &DirPlayer, sprite_id: i16) -> IntRectTuple {
    let sprite = get_sprite_in_context(player, sprite_id);
    let sprite = match sprite {
        Some(sprite) => sprite,
        None => return (0, 0, 0, 0),
    };
    let rect = get_concrete_sprite_rect(player, sprite);
    return (rect.left, rect.top, rect.right, rect.bottom);
}

pub fn get_channel_number_from_index(index: u32) -> u32 {
    match index {
        0..=4 => 0,
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
            sprite_details: HashMap::new(),
            last_sound_clear_frame: None,
            needs_per_frame_updates: false,
            frame_count: None,
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

    /// Create a behavior script instance.
    ///
    /// `default_cast_lib` is used to resolve cast_lib when it's 65535 or -1 (which means
    /// "use the parent's cast library", commonly used in filmloops).
    /// If the script is not found in the resolved cast library, we search all cast libraries.
    fn create_behavior(cast_lib: i32, cast_member: i32, default_cast_lib: Option<i32>) -> Option<(ScriptInstanceRef, DatumRef)> {
        // Resolve cast_lib 65535 or -1 to the default (filmloop's) cast library
        let resolved_cast_lib = if cast_lib == 65535 || cast_lib == -1 {
            default_cast_lib.unwrap_or(1) // Fall back to cast lib 1 if no default provided
        } else {
            cast_lib
        };

        let mut script_ref = CastMemberRef {
            cast_lib: resolved_cast_lib,
            cast_member,
        };

        // Check if the script exists in the resolved cast library
        // For cast 65535 (relative cast reference), we only use the filmloop's own cast.
        // We do NOT search other casts because that would attach unrelated behaviors
        // from the main movie to filmloop sprites.
        let script_exists = reserve_player_mut(|player| {
            player.movie.cast_manager.get_script_by_ref(&script_ref).is_some()
        });
        let found_in_other_lib: Option<i32> = None;

        // Update script_ref if we found the script in a different cast library
        if let Some(found_cast_lib) = found_in_other_lib {
            web_sys::console::log_1(
                &format!("create_behavior: script member {} not found in cast_lib {}, found in cast_lib {} instead",
                    cast_member, resolved_cast_lib, found_cast_lib).into(),
            );
            script_ref.cast_lib = found_cast_lib;
        }

        if !script_exists {
            web_sys::console::warn_1(
                &format!("Script not found: {:?} (original cast_lib: {}, default_cast_lib: {:?}), skipping behavior creation",
                    script_ref, cast_lib, default_cast_lib).into(),
            );
            return None;
        }

        let (script_instance_ref, datum_ref) =
            match ScriptDatumHandlers::create_script_instance(&script_ref) {
                Ok(result) => result,

                Err(e) => {
                    web_sys::console::error_1(
                        &format!("Failed to create script instance: {}", e.message).into(),
                    );
                    return None;
                }
            };

        Some((script_instance_ref.clone(), datum_ref.clone()))
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

    pub fn begin_sprites(&mut self, score_ref: ScoreRef, frame_num: u32) {
        // Clean up sound channel triggers - but only once per frame to prevent double-triggering
        // Check if we already processed this frame
        let already_processed = self.last_sound_clear_frame == Some(frame_num);

        if !already_processed {
            // Track that we're processing this frame
            self.last_sound_clear_frame = Some(frame_num);

            // For film loops, clear all triggers when:
            // 1. Frame is 1 (starting fresh or looped back) - this allows sounds to play for new sprites using same film loop
            // 2. Frame wrapped around (triggered_frame > frame_num)
            // This ensures sounds play each time a new sprite uses the film loop, even if it's the same member
            let should_clear_all = frame_num == 1 ||
                self.sound_channel_triggered.values().any(|&triggered_frame| triggered_frame > frame_num);

            if should_clear_all {
                // Clear all triggers to allow sounds to replay
                self.sound_channel_triggered.clear();
            } else {
                // Normal progression - only clear triggers for sounds that are no longer on the current frame
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
            }
        }

        // clean up behaviors from previous frame
        let sprites_to_finish = reserve_player_mut(|player| {
            let score = match &score_ref {
                ScoreRef::Stage => &player.movie.score,
                ScoreRef::FilmLoop(member_ref) => {
                    match player.movie.cast_manager.find_member_by_ref(member_ref) {
                        Some(member) => {
                            match &member.member_type {
                                super::cast_member::CastMemberType::FilmLoop(film_loop) => &film_loop.score,
                                _ => return Vec::new(),
                            }
                        }
                        None => return Vec::new(),
                    }
                }
            };
            score
                .channels
                .iter()
                .filter_map(|channel| channel.sprite.exited.then_some(channel.sprite.number))
                .collect_vec()
        });

        for sprite_num in sprites_to_finish {
            reserve_player_mut(|player| {
                let score = match &score_ref {
                    ScoreRef::Stage => &mut player.movie.score,
                    ScoreRef::FilmLoop(member_ref) => {
                        match player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                            Some(member) => {
                                match &mut member.member_type {
                                    super::cast_member::CastMemberType::FilmLoop(film_loop) => &mut film_loop.score,
                                    _ => return,
                                }
                            }
                            None => return,
                        }
                    }
                };
                let sprite: &mut Sprite = score.get_sprite_mut(sprite_num as i16);
                if sprite.puppet {
                    // Puppeted sprites keep their state across frame transitions.
                    // Just clear the exited flag so they remain active.
                    sprite.exited = false;
                } else if sprite.visible {
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
                // Check the correct score's sprites based on score_ref
                reserve_player_mut(|player| {
                    let score = match &score_ref {
                        ScoreRef::Stage => &player.movie.score,
                        ScoreRef::FilmLoop(member_ref) => {
                            match player.movie.cast_manager.find_member_by_ref(member_ref) {
                                Some(member) => {
                                    match &member.member_type {
                                        super::cast_member::CastMemberType::FilmLoop(film_loop) => &film_loop.score,
                                        _ => return false,
                                    }
                                }
                                None => return false,
                            }
                        }
                    };
                    score
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
                // Log spriteListIdx values for D6+ behavior debugging
                let sprite_list_idx = data.sprite_list_idx();
                if sprite_list_idx != 0 {
                    debug!(
                        "Sprite channel {} has spriteListIdx: {}",
                        sprite_num, sprite_list_idx
                    );
                }

                // Resolve cast_lib to the correct value:
                // - cast_lib 65535 is a "relative cast" reference - for the main stage
                //   it resolves to the default cast (1), for filmloops it stays as 65535
                // - cast_lib 0 means "default cast" = cast 1 (D5 uses 0 for single cast)
                let resolved_cast_lib = if data.cast_lib == 65535 && matches!(score_ref, ScoreRef::Stage) {
                    1
                } else if data.cast_lib == 0 {
                    1
                } else {
                    data.cast_lib as i32
                };

                let member = CastMemberRef {
                    cast_lib: resolved_cast_lib,
                    cast_member: data.cast_member as i32,
                };

                // For Stage sprites, use sprite_set_prop which handles intrinsic size
                // initialization and other side effects. For FilmLoop sprites, set
                // member directly since sprite_set_prop always writes to main stage score.
                match &score_ref {
                    ScoreRef::Stage => {
                        let _ = sprite_set_prop(sprite_num, "member", Datum::CastMember(member.clone()));
                    }
                    ScoreRef::FilmLoop(_) => {
                        sprite.member = Some(member.clone());
                    }
                }
                sprite.loc_h = data.pos_x as i32;
                sprite.loc_v = data.pos_y as i32;
                // Only set width/height when non-zero (0 means "use member's natural size")
                if data.width != 0 {
                    sprite.width = data.width as i32;
                }
                if data.height != 0 {
                    sprite.height = data.height as i32;
                }
                sprite.skew = data.skew as f64;
                sprite.rotation = data.rotation as f64;
                sprite.moveable = data.moveable;
                sprite.trails = data.trails;

                // Check if member is a shape to determine ink/blend handling
                // Use find_member_by_ref which handles relative cast references (65535)
                let is_shape = reserve_player_ref(|player| {
                    if let Some(real_member) = player.movie.cast_manager.find_member_by_ref(&member) {
                        return real_member.member_type.type_string() == "shape";
                    }
                    false
                });

                if is_shape {
                    // Shape sprites use different ink/blend encoding
                    sprite.blend = if data.blend == 255 {
                        100
                    } else {
                        ((255.0 - data.blend as f32) * 100.0 / 255.0) as i32
                    };
                    // Shape ink encoding: mask off the high bit and divide by 5
                    sprite.ink = ((data.ink & 0x7F) / 5) as i32;
                } else {
                    // Non-shape sprites: mask off the high bit (bit 7 is a flag, not part of ink number)
                    sprite.ink = (data.ink & 0x7F) as i32;
                    sprite.blend = if data.blend == 0 {
                        100
                    } else {
                        data.blend as i32
                    };
                }

                // Get bitmap's palette for RGB<->index conversion
                // Use the bitmap's actual palette instead of SYSTEM_WIN_PALETTE
                let bitmap_palette: Option<Vec<(u8, u8, u8)>> = reserve_player_ref(|player| {
                    if let Some(member_ref) = &sprite.member {
                        if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                            if let CastMemberType::Bitmap(bitmap_member) = &member.member_type {
                                let bw = bitmap_member.info.width as i32;
                                let bh = bitmap_member.info.height as i32;

                                sprite.bitmap_size_owned_by_sprite =
                                    sprite.width != bw || sprite.height != bh;

                                // Get the bitmap's palette colors
                                let bitmap = player.bitmap_manager.get_bitmap(bitmap_member.image_ref);
                                if let Some(bitmap) = bitmap {
                                    use crate::player::bitmap::bitmap::{PaletteRef, BuiltInPalette};
                                    use crate::player::bitmap::palette::{
                                        SYSTEM_MAC_PALETTE, GRAYSCALE_PALETTE, PASTELS_PALETTE,
                                        VIVID_PALETTE, NTSC_PALETTE, METALLIC_PALETTE, WEB_216_PALETTE,
                                        RAINBOW_PALETTE,
                                    };
                                    use crate::player::handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers;

                                    match &bitmap.palette_ref {
                                        PaletteRef::BuiltIn(builtin) => {
                                            let palette: &[(u8, u8, u8)] = match builtin {
                                                BuiltInPalette::SystemMac => &SYSTEM_MAC_PALETTE,
                                                BuiltInPalette::SystemWin | BuiltInPalette::SystemWinDir4 | BuiltInPalette::Vga => &SYSTEM_WIN_PALETTE,
                                                BuiltInPalette::GrayScale => &GRAYSCALE_PALETTE,
                                                BuiltInPalette::Pastels => &PASTELS_PALETTE,
                                                BuiltInPalette::Vivid => &VIVID_PALETTE,
                                                BuiltInPalette::Ntsc => &NTSC_PALETTE,
                                                BuiltInPalette::Metallic => &METALLIC_PALETTE,
                                                BuiltInPalette::Web216 => &WEB_216_PALETTE,
                                                BuiltInPalette::Rainbow => &RAINBOW_PALETTE,
                                            };
                                            return Some(palette.to_vec());
                                        }
                                        PaletteRef::Member(palette_member_ref) => {
                                            let slot_number = CastMemberRefHandlers::get_cast_slot_number(
                                                palette_member_ref.cast_lib as u32,
                                                palette_member_ref.cast_member as u32,
                                            );
                                            let palettes = player.movie.cast_manager.palettes();
                                            if let Some(palette_member) = palettes.get(slot_number as usize) {
                                                return Some(palette_member.colors.clone());
                                            }
                                        }
                                        PaletteRef::Default => {
                                            // Use system default
                                        }
                                    }
                                }
                            }
                        }
                    }
                    None
                });

                // Use bitmap's palette if available, otherwise fall back to SYSTEM_WIN_PALETTE
                let palette_for_index: &[(u8, u8, u8)] = bitmap_palette.as_deref().unwrap_or(&SYSTEM_WIN_PALETTE);

                match data.color_flag {
                    // fore + back are palette indexes
                    0 => {
                        sprite.fore_color = data.fore_color as i32;
                        sprite.color = ColorRef::PaletteIndex(data.fore_color);

                        sprite.back_color = data.back_color as i32;
                        sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                    }

                    // foreColor is RGB, backColor is palette index
                    1 => {
                        sprite.color = ColorRef::Rgb(
                            data.fore_color,
                            data.fore_color_g,
                            data.fore_color_b,
                        );
                        sprite.fore_color =
                            sprite.color.to_index(palette_for_index) as i32;

                        sprite.back_color = data.back_color as i32;
                        sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                    }

                    // foreColor is palette index, backColor is RGB
                    2 => {
                        sprite.fore_color = data.fore_color as i32;
                        sprite.color = ColorRef::PaletteIndex(data.fore_color);

                        sprite.bg_color = ColorRef::Rgb(
                            data.back_color,
                            data.back_color_g,
                            data.back_color_b,
                        );
                        sprite.back_color =
                            sprite.bg_color.to_index(palette_for_index) as i32;
                    }

                    // both fore + back are RGB
                    3 => {
                        sprite.color = ColorRef::Rgb(
                            data.fore_color,
                            data.fore_color_g,
                            data.fore_color_b,
                        );
                        sprite.fore_color =
                            sprite.color.to_index(palette_for_index) as i32;

                        sprite.bg_color = ColorRef::Rgb(
                            data.back_color,
                            data.back_color_g,
                            data.back_color_b,
                        );
                        sprite.back_color =
                            sprite.bg_color.to_index(palette_for_index) as i32;
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

                // Reset size flags when sprite re-enters.
                sprite.has_size_tweened = false;
                let has_explicit_size = data.width != 0 || data.height != 0;
                sprite.has_size_changed = has_explicit_size;
                // Score data dimensions are authoritative - dont let bitmap intrinsic
                // size override them in renderer
                if has_explicit_size {
                    sprite.bitmap_size_owned_by_sprite = false;
                }
            }
        }

        // D5 per-frame sprite property updates:
        // In D5, sprite properties (member, position, ink, etc.) can change every frame
        // via delta-compressed score data. Update already-entered, non-puppeted sprites
        // from the current frame's channel_initialization_data.
        if self.needs_per_frame_updates {
            // Collect updates first to avoid borrow conflicts
            let updates: Vec<(i16, ScoreFrameChannelData)> = self.channel_initialization_data
                .iter()
                .filter_map(|(frame_idx, channel_idx, data)| {
                    if frame_idx + 1 != frame_num {
                        return None;
                    }
                    let channel_number = get_channel_number_from_index(*channel_idx as u32);
                    if channel_number < 1 {
                        return None; // Skip frame scripts and effect channels
                    }
                    let sprite_num = channel_number as i16;
                    // Skip if this sprite was just entered above (already initialized)
                    if spans_to_enter.iter().any(|s| s.channel_number == channel_number) {
                        return None;
                    }
                    let sprite = self.get_sprite(sprite_num)?;
                    if !sprite.entered || sprite.puppet {
                        return None;
                    }
                    Some((sprite_num, data.clone()))
                })
                .collect();

            for (sprite_num, data) in updates {
                let resolved_cast_lib = if data.cast_lib == 65535 && matches!(score_ref, ScoreRef::Stage) {
                    1
                } else if data.cast_lib == 0 {
                    1
                } else {
                    data.cast_lib as i32
                };

                let member = CastMemberRef {
                    cast_lib: resolved_cast_lib,
                    cast_member: data.cast_member as i32,
                };

                // Update member if changed
                let current_member = self.get_sprite(sprite_num).and_then(|s| s.member.clone());
                match &score_ref {
                    ScoreRef::Stage => {
                        if current_member.as_ref() != Some(&member) {
                            let _ = sprite_set_prop(sprite_num, "member", Datum::CastMember(member.clone()));
                        }
                    }
                    ScoreRef::FilmLoop(_) => {
                        let sprite = self.get_sprite_mut(sprite_num);
                        sprite.member = Some(member.clone());
                    }
                }
                let sprite = self.get_sprite_mut(sprite_num);
                sprite.loc_h = data.pos_x as i32;
                sprite.loc_v = data.pos_y as i32;
                // Only update width/height when non-zero (0 means "use member's natural size").
                if data.width != 0 {
                    sprite.width = data.width as i32;
                    sprite.has_size_changed = true;
                }
                if data.height != 0 {
                    sprite.height = data.height as i32;
                    sprite.has_size_changed = true;
                }
                sprite.skew = data.skew as f64;
                sprite.rotation = data.rotation as f64;
                sprite.moveable = data.moveable;
                sprite.trails = data.trails;
                sprite.ink = (data.ink & 0x7F) as i32;
                sprite.blend = if data.blend == 0 { 100 } else { data.blend as i32 };
            }
        }

        // handle score Sound 1 + Sound 2 in Effects Channels
        // Build a map of (frame_index, channel_index) -> cast_member for quick lookup
        let sound_by_frame_channel: HashMap<(u32, u16), u8> = self.sound_channel_data.iter()
            .map(|(frame_idx, ch_idx, data)| ((*frame_idx, *ch_idx), data.cast_member))
            .collect();

        for (frame_index, channel_index, sound_data) in self.sound_channel_data.iter() {
            if *frame_index + 1 == frame_num {
                // Check if this is the start of a new sound span
                // A sound triggers when:
                // 1. It's the first frame (frame_index == 0), OR
                // 2. The previous frame had no sound on this channel, OR
                // 3. The previous frame had a different cast_member on this channel
                let prev_frame_sound = if *frame_index > 0 {
                    sound_by_frame_channel.get(&(*frame_index - 1, *channel_index))
                } else {
                    None
                };

                let is_new_sound_span = match prev_frame_sound {
                    None => true, // No sound on previous frame
                    Some(&prev_cast_member) => prev_cast_member != sound_data.cast_member, // Different sound
                };

                if !is_new_sound_span {
                    // This is a continuation of the same sound, skip
                    continue;
                }
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
                            // For film loops, look up the sound in the film loop's cast library
                            // For the main score, use the global slot number lookup
                            let sound_member_opt = match &score_ref {
                                ScoreRef::FilmLoop(filmloop_member_ref) => {
                                    // Look up sound in the film loop's cast library
                                    let cast_member_ref = CastMemberRef {
                                        cast_lib: filmloop_member_ref.cast_lib,
                                        cast_member: sound_data.cast_member as i32,
                                    };
                                    player.movie.cast_manager.find_member_by_ref(&cast_member_ref)
                                        .map(|m| (m, cast_member_ref))
                                }
                                ScoreRef::Stage => {
                                    // Find the cast member by slot number (global)
                                    player.movie.cast_manager
                                        .find_member_by_slot_number(sound_data.cast_member as u32)
                                        .map(|m| {
                                            let ref_ = CastMemberRefHandlers::member_ref_from_slot_number(m.number);
                                            (m, CastMemberRef {
                                                cast_lib: ref_.cast_lib as i32,
                                                cast_member: ref_.cast_member as i32,
                                            })
                                        })
                                }
                            };

                            if let Some((cast_member, cast_member_ref)) = sound_member_opt {
                                if let CastMemberType::Sound(_) =
                                    &cast_member.member_type
                                {
                                    let member_ref =
                                        player.alloc_datum(Datum::CastMember(cast_member_ref));

                                    let _ = player.puppet_sound(sound_channel, member_ref);
                                }
                            } else {
                                debug!(
                                    "Sound member not found: cast_member={} score_ref={:?}",
                                    sound_data.cast_member, score_ref
                                );
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

        // Extract default cast_lib for resolving 65535 references (used in filmloops)
        let default_cast_lib: Option<i32> = match &score_ref {
            ScoreRef::Stage => None,
            ScoreRef::FilmLoop(member_ref) => Some(member_ref.cast_lib),
        };

        // Debug: Log how many channels have behaviors
        let total_scripts: usize = spans_by_channel.values()
            .flat_map(|spans| spans.iter())
            .map(|span| span.scripts.len())
            .sum();
        if total_scripts > 0 {
            debug!(
                "🔧 begin_sprites: {} channels, {} total behavior scripts to attach (score_ref: {:?})",
                spans_by_channel.len(), total_scripts,
                match &score_ref {
                    ScoreRef::Stage => "Stage".to_string(),
                    ScoreRef::FilmLoop(m) => format!("FilmLoop {}:{}", m.cast_lib, m.cast_member),
                }
            );
        }

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
                            "Creating behavior from cast {}/{} with {} parameters (default_cast_lib: {:?}) for channel {}",
                            behavior_ref.cast_lib,
                            behavior_ref.cast_member,
                            behavior_ref.parameter.len(),
                            default_cast_lib,
                            channel_num
                        );

                    // Create the behavior instance
                    let behavior_result = Self::create_behavior(
                        behavior_ref.cast_lib as i32,
                        behavior_ref.cast_member as i32,
                        default_cast_lib,
                    );

                    // Skip this behavior if creation failed (script not found)
                    let (script_instance_ref, datum_ref) = match behavior_result {
                        Some(result) => result,
                        None => {
                            debug!("Skipping behavior from cast {}/{} - script not found",
                                behavior_ref.cast_lib, behavior_ref.cast_member);
                            continue;
                        }
                    };

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

                    // Attach behavior to sprite - need to use the correct score (stage or filmloop)
                    let score_ref_clone = score_ref.clone();
                    reserve_player_mut(|player| {
                        let sprite_num = *channel_num as i16;

                        // Get mutable access to the correct sprite based on score_ref
                        let sprite = match &score_ref_clone {
                            ScoreRef::Stage => {
                                player.movie.score.get_sprite_mut(sprite_num)
                            }
                            ScoreRef::FilmLoop(member_ref) => {
                                // For filmloops, we need to get the sprite from the filmloop's score
                                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                                    if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                                        film_loop.score.get_sprite_mut(sprite_num)
                                    } else {
                                        // Fallback to stage score if filmloop not found
                                        player.movie.score.get_sprite_mut(sprite_num)
                                    }
                                } else {
                                    // Fallback to stage score if member not found
                                    player.movie.score.get_sprite_mut(sprite_num)
                                }
                            }
                        };

                        // Add the behavior to the sprite's script_instance_list
                        sprite.script_instance_list.push(actual_instance_ref.clone());
                        Ok::<(), ScriptError>(())
                    })
                    .expect("Failed to attach behavior to sprite");
                }
            }
        }

        // Attach behaviors from spriteListIdx (D6+ sprite detail mechanism)
        // This is an alternative to frame_intervals for behavior attachment
        // NOTE: spriteListIdx references the MAIN MOVIE's sprite detail table, not local filmloop tables
        //
        // Log summary of available sprite_details for diagnosis
        let (sprite_details_info, dir_version) = reserve_player_ref(|player| {
            let count = player.movie.score.sprite_details.len();
            let max_idx = player.movie.score.sprite_details.keys().max().cloned();
            ((count, max_idx), player.movie.dir_version)
        });
        debug!(
            "🔍 begin_sprites: main movie has {} sprite_details, max index: {:?}, score_ref: {:?}",
            sprite_details_info.0, sprite_details_info.1, score_ref
        );

        for (span, _channel_index, data) in span_init_data.iter() {
            let sprite_list_idx = data.sprite_list_idx();
            if sprite_list_idx == 0 {
                continue;
            }

            // Look up behaviors for this spriteListIdx in the MAIN MOVIE's sprite_details
            // (spriteListIdx is a global index, not local to filmloops)
            let (detail_info_opt, details_count) = reserve_player_ref(|player| {
                let info = player.movie.score.sprite_details.get(&sprite_list_idx).cloned();
                let count = player.movie.score.sprite_details.len();
                (info, count)
            });

            if let Some(detail_info) = detail_info_opt {
                // D6+ path: spriteListIdx references the sprite detail table
                if detail_info.behaviors.is_empty() {
                    continue;
                }

                let channel_num = span.channel_number;
                debug!(
                    "Attaching {} behaviors from spriteListIdx {} to channel {}",
                    detail_info.behaviors.len(), sprite_list_idx, channel_num
                );

                for behavior in &detail_info.behaviors {
                    debug!(
                        "   Creating behavior from spriteDetail cast {}/{} for channel {}",
                        behavior.cast_lib, behavior.cast_member, channel_num
                    );

                    // Create the behavior instance
                    let sprite_cast_lib = if data.cast_lib == 0 || data.cast_lib == 65535 {
                        default_cast_lib.unwrap_or(1)
                    } else {
                        data.cast_lib as i32
                    };
                    let sprite_member = CastMemberRef {
                        cast_lib: sprite_cast_lib,
                        cast_member: data.cast_member as i32,
                    };
                    let behavior_result = Self::create_behavior(
                        behavior.cast_lib as i32,
                        behavior.cast_member as i32,
                        default_cast_lib,
                    );

                    let (script_instance_ref, datum_ref) = match behavior_result {
                        Some(result) => result,
                        None => {
                            debug!("Skipping spriteDetail behavior from cast {}/{} - script not found",
                                behavior.cast_lib, behavior.cast_member);
                            continue;
                        }
                    };

                    // Extract the ScriptInstanceRef from datum_ref
                    let actual_instance_ref = reserve_player_mut(|player| {
                        let datum = player.get_datum(&datum_ref);
                        match datum {
                            Datum::ScriptInstanceRef(ref instance_ref) => Ok(instance_ref.clone()),
                            _ => Err(ScriptError::new("Expected ScriptInstanceRef".to_string())),
                        }
                    })
                    .expect("Failed to extract ScriptInstanceRef");

                    // Set the spriteNum property
                    reserve_player_mut(|player| {
                        let sprite_num_ref = player.alloc_datum(Datum::Int(channel_num as i32));
                        let _ = script_set_prop(
                            player,
                            &actual_instance_ref,
                            &"spriteNum".to_string(),
                            &sprite_num_ref,
                            false,
                        );
                    });

                    // Attach behavior to sprite
                    let score_ref_clone = score_ref.clone();
                    reserve_player_mut(|player| {
                        let sprite_num = channel_num as i16;

                        let sprite = match &score_ref_clone {
                            ScoreRef::Stage => {
                                player.movie.score.get_sprite_mut(sprite_num)
                            }
                            ScoreRef::FilmLoop(member_ref) => {
                                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                                    if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                                        film_loop.score.get_sprite_mut(sprite_num)
                                    } else {
                                        player.movie.score.get_sprite_mut(sprite_num)
                                    }
                                } else {
                                    player.movie.score.get_sprite_mut(sprite_num)
                                }
                            }
                        };

                        sprite.script_instance_list.push(actual_instance_ref.clone());
                        Ok::<(), ScriptError>(())
                    })
                    .expect("Failed to attach spriteDetail behavior to sprite");
                }
            } else if data.sprite_list_idx_lo != 0 && dir_version < 600 {
                // D5 path: sprite_list_idx_hi/lo are scriptId castLib/member
                let script_cast_lib = data.sprite_list_idx_hi as i32;
                let script_member = data.sprite_list_idx_lo as i32;
                let channel_num = span.channel_number;

                // Resolve cast_lib 0 or 65535 to default
                let resolved_cast_lib = if script_cast_lib == 0 || script_cast_lib == 65535 {
                    default_cast_lib.unwrap_or(1)
                } else {
                    script_cast_lib
                };

                let sprite_cast_lib = if data.cast_lib == 0 || data.cast_lib == 65535 {
                    default_cast_lib.unwrap_or(1)
                } else {
                    data.cast_lib as i32
                };
                let sprite_member = CastMemberRef {
                    cast_lib: sprite_cast_lib,
                    cast_member: data.cast_member as i32,
                };
                web_sys::console::log_1(&format!(
                    "D5 sprite ch={}: scriptId=({},{}), sprite_member=({},{})",
                    channel_num, resolved_cast_lib, script_member,
                    sprite_cast_lib, data.cast_member
                ).into());

                let behavior_result = Self::create_behavior(
                    resolved_cast_lib,
                    script_member,
                    default_cast_lib,
                );

                match behavior_result {
                    Some((script_instance_ref, datum_ref)) => {
                        let actual_instance_ref = reserve_player_mut(|player| {
                            let datum = player.get_datum(&datum_ref);
                            match datum {
                                Datum::ScriptInstanceRef(ref instance_ref) => Ok(instance_ref.clone()),
                                _ => Err(ScriptError::new("Expected ScriptInstanceRef".to_string())),
                            }
                        })
                        .expect("Failed to extract ScriptInstanceRef");

                        reserve_player_mut(|player| {
                            let sprite_num_ref = player.alloc_datum(Datum::Int(channel_num as i32));
                            let _ = script_set_prop(
                                player,
                                &actual_instance_ref,
                                &"spriteNum".to_string(),
                                &sprite_num_ref,
                                false,
                            );
                        });

                        let score_ref_clone = score_ref.clone();
                        reserve_player_mut(|player| {
                            let sprite_num = channel_num as i16;

                            let sprite = match &score_ref_clone {
                                ScoreRef::Stage => {
                                    player.movie.score.get_sprite_mut(sprite_num)
                                }
                                ScoreRef::FilmLoop(member_ref) => {
                                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                                        if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                                            film_loop.score.get_sprite_mut(sprite_num)
                                        } else {
                                            player.movie.score.get_sprite_mut(sprite_num)
                                        }
                                    } else {
                                        player.movie.score.get_sprite_mut(sprite_num)
                                    }
                                }
                            };

                            sprite.script_instance_list.push(actual_instance_ref.clone());
                            Ok::<(), ScriptError>(())
                        })
                        .expect("Failed to attach D5 sprite script to sprite");
                    }
                    None => {
                        // Script not found — store the scriptId on the sprite as a fallback
                        // reference for potential future event-time resolution
                        let score_ref_clone = score_ref.clone();
                        reserve_player_mut(|player| {
                            let sprite_num = channel_num as i16;
                            let sprite = match &score_ref_clone {
                                ScoreRef::Stage => {
                                    player.movie.score.get_sprite_mut(sprite_num)
                                }
                                ScoreRef::FilmLoop(member_ref) => {
                                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                                        if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                                            film_loop.score.get_sprite_mut(sprite_num)
                                        } else {
                                            player.movie.score.get_sprite_mut(sprite_num)
                                        }
                                    } else {
                                        player.movie.score.get_sprite_mut(sprite_num)
                                    }
                                }
                            };
                            Ok::<(), ScriptError>(())
                        })
                        .expect("Failed to set D5 sprite scriptId fallback");
                    }
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
                    let behavior_result = Self::create_behavior(
                        behavior_ref.cast_lib as i32,
                        behavior_ref.cast_member as i32,
                        default_cast_lib,
                    );

                    // Skip if creation failed (script not found)
                    let (script_instance_ref, datum_ref) = match behavior_result {
                        Some(result) => result,
                        None => {
                            debug!("Skipping frame script from cast {}/{} - script not found",
                                behavior_ref.cast_lib, behavior_ref.cast_member);
                            // Don't cache anything if script not found
                            return; // Exit early from begin_sprites
                        }
                    };

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

        // Initialize filmloop child sprites
        for channel in self.channels.iter() {
            if let Some(member_ref) = &channel.sprite.member {
                reserve_player_mut(|player| {
                    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                        if let CastMemberType::FilmLoop(_) = &member.member_type {
                            // Filmloop sprites need their children initialized too
                            let filmloop_score_ref = ScoreRef::FilmLoop(member_ref.clone());
                            if let Some(filmloop_member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                                if let CastMemberType::FilmLoop(film_loop) = &mut filmloop_member.member_type {
                                    let current_frame = film_loop.current_frame;
                                    // Make sure filmloop sprites are entered and have data
                                    film_loop.score.begin_sprites(filmloop_score_ref, current_frame);
                                    film_loop.score.apply_tween_modifiers(current_frame);
                                }
                            }
                        }
                    }
                });
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

            // Resolve cast_lib 65535 to cast 1 ONLY for main stage sprites.
            // Cast 65535 is a "relative cast" reference - for filmloops it should
            // stay as 65535 so it resolves relative to the filmloop's cast.
            // For the main stage, it should resolve to the default cast (1).
            let resolved_cast_lib = if data.cast_lib == 65535 && matches!(score_ref, ScoreRef::Stage) {
                1
            } else {
                data.cast_lib as i32
            };

            let member = CastMemberRef {
                cast_lib: resolved_cast_lib,
                cast_member: data.cast_member as i32,
            };

            // Set member directly on the sprite instead of using sprite_set_prop,
            // because sprite_set_prop always writes to main stage score,
            // but we need to set it on this score's sprite (may be filmloop).
            sprite.member = Some(member.clone());
            sprite.loc_h = data.pos_x as i32;
            sprite.loc_v = data.pos_y as i32;
            sprite.width = data.width as i32;
            sprite.height = data.height as i32;
            sprite.skew = data.skew as f64;
            sprite.rotation = data.rotation as f64;
            sprite.moveable = data.moveable;
            sprite.trails = data.trails;

            // Check if member is a shape to determine ink/blend handling
            // Use find_member_by_ref which handles relative cast references (65535)
            let is_shape = reserve_player_ref(|player| {
                if let Some(real_member) = player.movie.cast_manager.find_member_by_ref(&member) {
                    return real_member.member_type.type_string() == "shape";
                }
                false
            });

            if is_shape {
                // Shape sprites use different ink/blend encoding
                sprite.blend = if data.blend == 255 {
                    100
                } else {
                    ((255.0 - data.blend as f32) * 100.0 / 255.0) as i32
                };
                // Shape ink encoding: mask off the high bit and divide by 5
                sprite.ink = ((data.ink & 0x7F) / 5) as i32;
            } else {
                // Non-shape sprites use standard encoding
                sprite.ink = data.ink as i32;
                sprite.blend = if data.blend == 0 {
                    100
                } else {
                    data.blend as i32
                };
            }

            // Get bitmap's palette for RGB<->index conversion
            let bitmap_palette: Option<Vec<(u8, u8, u8)>> = reserve_player_ref(|player| {
                if let Some(member_ref) = &sprite.member {
                    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                        if let CastMemberType::Bitmap(bitmap_member) = &member.member_type {
                            let bw = bitmap_member.info.width as i32;
                            let bh = bitmap_member.info.height as i32;

                            sprite.bitmap_size_owned_by_sprite =
                                sprite.width != bw || sprite.height != bh;

                            // Get the bitmap's palette colors
                            let bitmap = player.bitmap_manager.get_bitmap(bitmap_member.image_ref);
                            if let Some(bitmap) = bitmap {
                                use crate::player::bitmap::bitmap::{PaletteRef, BuiltInPalette};
                                use crate::player::bitmap::palette::{
                                    SYSTEM_MAC_PALETTE, GRAYSCALE_PALETTE, PASTELS_PALETTE,
                                    VIVID_PALETTE, NTSC_PALETTE, METALLIC_PALETTE, WEB_216_PALETTE,
                                    RAINBOW_PALETTE,
                                };
                                use crate::player::handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers;

                                match &bitmap.palette_ref {
                                    PaletteRef::BuiltIn(builtin) => {
                                        let palette: &[(u8, u8, u8)] = match builtin {
                                            BuiltInPalette::SystemMac => &SYSTEM_MAC_PALETTE,
                                            BuiltInPalette::SystemWin | BuiltInPalette::SystemWinDir4 | BuiltInPalette::Vga => &SYSTEM_WIN_PALETTE,
                                            BuiltInPalette::GrayScale => &GRAYSCALE_PALETTE,
                                            BuiltInPalette::Pastels => &PASTELS_PALETTE,
                                            BuiltInPalette::Vivid => &VIVID_PALETTE,
                                            BuiltInPalette::Ntsc => &NTSC_PALETTE,
                                            BuiltInPalette::Metallic => &METALLIC_PALETTE,
                                            BuiltInPalette::Web216 => &WEB_216_PALETTE,
                                            BuiltInPalette::Rainbow => &RAINBOW_PALETTE,
                                        };
                                        return Some(palette.to_vec());
                                    }
                                    PaletteRef::Member(palette_member_ref) => {
                                        let slot_number = CastMemberRefHandlers::get_cast_slot_number(
                                            palette_member_ref.cast_lib as u32,
                                            palette_member_ref.cast_member as u32,
                                        );
                                        let palettes = player.movie.cast_manager.palettes();
                                        if let Some(palette_member) = palettes.get(slot_number as usize) {
                                            return Some(palette_member.colors.clone());
                                        }
                                    }
                                    PaletteRef::Default => {
                                        // Use system default
                                    }
                                }
                            }
                        }
                    }
                }
                None
            });

            // Use bitmap's palette if available, otherwise fall back to SYSTEM_WIN_PALETTE
            let palette_for_index: &[(u8, u8, u8)] = bitmap_palette.as_deref().unwrap_or(&SYSTEM_WIN_PALETTE);

            match data.color_flag {
                // fore + back are palette indexes
                0 => {
                    sprite.fore_color = data.fore_color as i32;
                    sprite.color = ColorRef::PaletteIndex(data.fore_color);

                    sprite.back_color = data.back_color as i32;
                    sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                }

                // foreColor is RGB, backColor is palette index
                1 => {
                    sprite.color = ColorRef::Rgb(
                        data.fore_color,
                        data.fore_color_g,
                        data.fore_color_b,
                    );
                    sprite.fore_color =
                        sprite.color.to_index(palette_for_index) as i32;

                    sprite.back_color = data.back_color as i32;
                    sprite.bg_color = ColorRef::PaletteIndex(data.back_color);
                }

                // foreColor is palette index, backColor is RGB
                2 => {
                    sprite.fore_color = data.fore_color as i32;
                    sprite.color = ColorRef::PaletteIndex(data.fore_color);

                    sprite.bg_color = ColorRef::Rgb(
                        data.back_color,
                        data.back_color_g,
                        data.back_color_b,
                    );
                    sprite.back_color =
                        sprite.bg_color.to_index(palette_for_index) as i32;
                }

                // both fore + back are RGB
                3 => {
                    sprite.color = ColorRef::Rgb(
                        data.fore_color,
                        data.fore_color_g,
                        data.fore_color_b,
                    );
                    sprite.fore_color =
                        sprite.color.to_index(palette_for_index) as i32;

                    // Background (RGB → map to palette using bitmap's palette)
                    sprite.bg_color = ColorRef::Rgb(
                        data.back_color,
                        data.back_color_g,
                        data.back_color_b,
                    );
                    sprite.back_color =
                        sprite.bg_color.to_index(palette_for_index) as i32;
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

            // In Director, puppeted sprites are controlled by Lingo and
            // score tweens do NOT apply to them.
            if sprite.puppet {
                continue;
            }

            // Skip if sprite isn't in an active span at this frame
            if !active_channels.contains(&(sprite_num as u32)) {
                continue;
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

    pub async fn end_sprites(&mut self, score_ref: ScoreRef, prev_frame: u32, next_frame: u32) -> Vec<u32> {
        let channels_to_end: Vec<u32> = self
            .sprite_spans
            .iter()
            .filter(|span| {
                Self::is_span_in_frame(span, prev_frame)
                    && !Self::is_span_in_frame(span, next_frame)
            })
            .map(|span| span.channel_number)
            .collect_vec();

        let _ = dispatch_event_endsprite_for_score(score_ref, channels_to_end.clone()).await;

        channels_to_end
    }

    pub fn get_channel_count(&self) -> usize {
        return self.channels.len().saturating_sub(1);
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
        dir_version: u16,
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

        // For filmloops (and any score with empty frame_intervals), generate
        // sprite_spans from frame_channel_data to ensure sprites can be rendered
        if self.sprite_spans.is_empty() && !self.channel_initialization_data.is_empty() {
            self.generate_sprite_spans_from_channel_data(dir_version);
        }

        // Copy sprite detail behaviors (D6+)
        self.sprite_details = score_chunk.sprite_details.clone();

        // Compute frame count for auto-looping (applies to all movie versions).
        // Always run this even if generate_sprite_spans_from_channel_data already set
        // frame_count, because we also need to consider the score chunk header and frame labels.
        {
            let header_fc = score_chunk.frame_data.header.frame_count;
            let span_max = self.sprite_spans.iter()
                .map(|span| span.end_frame)
                .max()
                .unwrap_or(1);
            let init_data_max = self.channel_initialization_data.iter()
                .map(|(frame_idx, _, _)| frame_idx + 1)
                .max()
                .unwrap_or(1);
            let keyframes_max = self.keyframes_cache.values()
                .filter_map(|channel_kf| channel_kf.path.as_ref())
                .flat_map(|path_kf| path_kf.keyframes.iter())
                .map(|kf| kf.frame)
                .max()
                .unwrap_or(1);
            let labels_max = self.frame_labels.iter()
                .map(|fl| fl.frame_num as u32)
                .max()
                .unwrap_or(1);
            let current = self.frame_count.unwrap_or(1);
            self.frame_count = Some(current.max(header_fc).max(span_max).max(init_data_max).max(keyframes_max).max(labels_max));
        }
    }

    /// Generate sprite_spans from channel_initialization_data.
    /// This is used for filmloops and D5 movies which don't have frame_intervals
    /// but do have frame_channel_data with sprite and frame script information.
    fn generate_sprite_spans_from_channel_data(&mut self, dir_version: u16) {
        use std::collections::HashMap;

        // Group by channel: find min/max frame for each channel with data
        let mut channel_frames: HashMap<u32, (u32, u32)> = HashMap::new();

        // Collect frame script data (channel 0) separately
        // Each frame may have a different script, so we track per-frame
        let mut frame_scripts: Vec<(u32, u16, u16)> = Vec::new(); // (frame_num, cast_lib, cast_member)

        for (frame_idx, channel_idx, data) in &self.channel_initialization_data {
            if *channel_idx == 0 {
                // D5 path: channel 0 holds frame scripts
                if dir_version < 600 && data.cast_member != 0 {
                    let frame_num = *frame_idx + 1; // 0-based → 1-based
                    // D5 has a single cast library; cast_lib 0 means "default cast" which is 1
                    let cast_lib = if data.cast_lib == 0 { 1 } else { data.cast_lib };
                    frame_scripts.push((frame_num, cast_lib, data.cast_member));
                }
                continue;
            }
            // Skip other effect channels (1-5)
            if *channel_idx < 6 {
                continue;
            }
            // Skip empty sprites
            if data.cast_member == 0 {
                continue;
            }

            let channel_number = get_channel_number_from_index(*channel_idx as u32);
            let frame_num = *frame_idx + 1; // frame_idx is 0-based, frames are 1-based

            channel_frames
                .entry(channel_number)
                .and_modify(|(min_frame, max_frame)| {
                    if frame_num < *min_frame {
                        *min_frame = frame_num;
                    }
                    if frame_num > *max_frame {
                        *max_frame = frame_num;
                    }
                })
                .or_insert((frame_num, frame_num));
        }

        // Create sprite spans for each channel
        for (channel_number, (start_frame, end_frame)) in channel_frames {
            let sprite_span = ScoreSpriteSpan {
                channel_number,
                start_frame,
                end_frame,
                scripts: Vec::new(), // Filmloop sprites don't have behavior scripts in this context
            };
            self.sprite_spans.push(sprite_span);
        }

        // Create frame script spans (channel 0)
        // Merge consecutive frames with the same script into a single span
        frame_scripts.sort_by_key(|(frame_num, _, _)| *frame_num);
        let mut i = 0;
        while i < frame_scripts.len() {
            let (start_frame, cast_lib, cast_member) = frame_scripts[i];
            let mut end_frame = start_frame;
            // Extend span while next frame has the same script
            while i + 1 < frame_scripts.len() {
                let (next_frame, next_lib, next_member) = frame_scripts[i + 1];
                if next_frame == end_frame + 1 && next_lib == cast_lib && next_member == cast_member {
                    end_frame = next_frame;
                    i += 1;
                } else {
                    break;
                }
            }
            self.sprite_spans.push(ScoreSpriteSpan {
                channel_number: 0,
                start_frame,
                end_frame,
                scripts: vec![ScoreBehaviorReference {
                    cast_lib,
                    cast_member,
                    parameter: Vec::new(),
                }],
            });
            i += 1;
        }

        // D5 movies need per-frame sprite property updates
        self.needs_per_frame_updates = true;

        // Compute D5 frame count for auto-looping
        let init_data_max = self.channel_initialization_data.iter()
            .map(|(frame_idx, _, _)| frame_idx + 1)
            .max()
            .unwrap_or(1);
        let span_max = self.sprite_spans.iter()
            .map(|span| span.end_frame)
            .max()
            .unwrap_or(1);
        self.frame_count = Some(init_data_max.max(span_max));

        debug!(
            "Generated {} sprite_spans from channel_data (per-frame updates enabled, frame_count={})",
            self.sprite_spans.len(),
            self.frame_count.unwrap_or(0)
        );
    }

    /// Extend sprite_spans with frame_channel_data when frame_intervals are incomplete.
    /// This handles cases where frame_channel_data has a cast_member on a frame that
    /// isn't covered by any existing sprite_span for that channel.
    fn extend_sprite_spans_from_channel_data(&mut self) {
        use std::collections::HashMap;

        // Build a map of channel -> list of (start_frame, end_frame) from existing sprite_spans
        let mut channel_spans: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for span in &self.sprite_spans {
            channel_spans
                .entry(span.channel_number)
                .or_insert_with(Vec::new)
                .push((span.start_frame, span.end_frame));
        }

        // Find frames with non-zero cast_member that aren't covered by any span
        // Group by channel: find frames with cast_member that need new spans
        let mut missing_frames: HashMap<u32, Vec<u32>> = HashMap::new();

        for (frame_idx, channel_idx, data) in &self.channel_initialization_data {
            // Skip effect channels (channels 0-5 in raw data)
            if *channel_idx < 6 {
                continue;
            }
            // Only consider frames with a cast_member (keyframes)
            if data.cast_member == 0 {
                continue;
            }

            let channel_number = get_channel_number_from_index(*channel_idx as u32);
            let frame_num = *frame_idx + 1; // frame_idx is 0-based, frames are 1-based

            // Check if this frame is covered by an existing span
            let is_covered = channel_spans
                .get(&channel_number)
                .map(|spans| spans.iter().any(|(start, end)| frame_num >= *start && frame_num <= *end))
                .unwrap_or(false);

            if !is_covered {
                missing_frames
                    .entry(channel_number)
                    .or_insert_with(Vec::new)
                    .push(frame_num);
            }
        }

        // Create new spans for missing frames
        // For each channel with missing frames, find the contiguous ranges
        for (channel_number, mut frames) in missing_frames {
            frames.sort();
            frames.dedup();

            if frames.is_empty() {
                continue;
            }

            // Find the end frame by looking at all frame_channel_data for this channel
            let channel_frames: Vec<_> = self.channel_initialization_data.iter()
                .filter(|(_, ch, _)| get_channel_number_from_index(*ch as u32) == channel_number)
                .collect();

            for &start_frame in &frames {
                // Find the end of this span
                let mut end_frame = start_frame;

                for (frame_idx, _, data) in &channel_frames {
                    let frame = *frame_idx + 1;
                    if frame <= start_frame {
                        continue;
                    }

                    // Check if this frame is covered by an existing span
                    let is_covered_by_existing = channel_spans
                        .get(&channel_number)
                        .map(|spans| spans.iter().any(|(s, e)| frame >= *s && frame <= *e))
                        .unwrap_or(false);

                    if is_covered_by_existing {
                        break;
                    }

                    if data.cast_member != 0 {
                        end_frame = frame;
                    } else {
                        end_frame = frame;
                    }
                }

                // Create a new span
                let sprite_span = ScoreSpriteSpan {
                    channel_number,
                    start_frame,
                    end_frame,
                    scripts: Vec::new(),
                };

                self.sprite_spans.push(sprite_span);

                // Update our tracking
                channel_spans
                    .entry(channel_number)
                    .or_insert_with(Vec::new)
                    .push((start_frame, end_frame));
            }
        }
    }

    pub fn load_from_dir(&mut self, dir: &DirectorFile) {
        let frame_labels_chunk = dir.frame_labels.as_ref();
        if frame_labels_chunk.is_some() {
            self.frame_labels = frame_labels_chunk.unwrap().labels.clone();
        }
        if let Some(score_chunk) = dir.score.as_ref() {
            self.load_from_score_chunk(score_chunk, dir.version);
        } else {
            console_warn!("No score chunk found in movie - score will be empty");
        }
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

    pub fn get_sorted_channels(&self, frame_num: u32) -> Vec<&SpriteChannel> {
        // Build set of active channel numbers for the specified frame
        let active_channels: std::collections::HashSet<u32> = self
            .sprite_spans
            .iter()
            .filter(|span| Self::is_span_in_frame(span, frame_num))
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
                // Sort by loc_z ascending (lower values first, drawn first/behind)
                // Sprites with higher loc_z are drawn later and appear on top
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
        let total: usize = self.channels.iter().map(|c| c.sprite.script_instance_list.len()).sum();
        let mut instance_list = Vec::with_capacity(total);
        for channel in &self.channels {
            for instance_ref in &channel.sprite.script_instance_list {
                instance_list.push(instance_ref.clone());
            }
        }
        instance_list
    }

    pub fn get_frame_tempo(&self, frame: u32) -> Option<u32> {
        // Search through tempo_channel_data to find the most recent tempo change
        // at or before the requested frame.
        // Note: frame_idx is 0-based (from score parsing), frame is 1-based (current_frame).
        // frame_idx 0 = Director frame 1, so we need frame_idx < frame.
        self.tempo_channel_data
            .iter()
            .rev() // Search backwards from the end (most recent first)
            .find(|(frame_idx, _)| *frame_idx < frame)
            .map(|(_, tempo_data)| tempo_data.tempo as u32)
    }
}

pub fn sprite_get_prop(
    player: &mut DirPlayer,
    sprite_id: i16,
    prop_name: &str,
) -> Result<Datum, ScriptError> {
    // Use context-aware sprite lookup to support filmloop behaviors
    let sprite = get_sprite_in_context(player, sprite_id);
    match prop_name {
        "ilk" => Ok(Datum::Symbol("sprite".to_string())),
        "spriteNum" | "spriteNumber" => Ok(Datum::Int(
            sprite.map_or(sprite_id as i32, |x| x.number as i32),
        )),
        "loc" => reserve_player_mut(|player| {
            let sprite = get_sprite_in_context(player, sprite_id);
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
            let rect = get_sprite_rect_in_context(player, sprite_id);
            Ok(Datum::Int(rect.0 as i32))
        }
        "top" => {
            let rect = get_sprite_rect_in_context(player, sprite_id);
            Ok(Datum::Int(rect.1 as i32))
        }
        "right" => {
            let rect = get_sprite_rect_in_context(player, sprite_id);
            Ok(Datum::Int(rect.2 as i32))
        }
        "bottom" => {
            let rect = get_sprite_rect_in_context(player, sprite_id);
            Ok(Datum::Int(rect.3 as i32))
        }
        "rect" => {
            let rect = get_sprite_rect_in_context(player, sprite_id);
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
        "memberNum" => Ok(Datum::Int(sprite.map_or(0, |x| {
            x.member.as_ref().map_or(0, |y| y.cast_member)
        }))),
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
        "visible" | "visibility" => Ok(datum_bool(sprite.map_or(true, |sprite| sprite.visible))),
        "puppet" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.puppet))),
        "moveableSprite" | "moveable" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.moveable))),
        "constraint" => Ok(Datum::Int(sprite.map_or(0, |sprite| sprite.constraint))),
        "trails" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.trails))),
        "foreColor" | "forecolor" => Ok(Datum::Int(
            sprite.map_or(255, |sprite| sprite.fore_color) as i32,
        )),
        "backColor" | "backcolor" => Ok(Datum::Int(
            sprite.map_or(0, |sprite| sprite.back_color) as i32,
        )),
        "cursor" => {
            let cursor_ref = sprite.and_then(|sprite| sprite.cursor_ref.clone());
            match cursor_ref {
                Some(CursorRef::System(id)) => Ok(Datum::Int(id)),
                Some(CursorRef::Member(ids)) => {
                    let id_refs = ids
                        .iter()
                        .map(|id| {
                            let member_ref = CastMemberRefHandlers::member_ref_from_slot_number(*id as u32);
                            player.alloc_datum(Datum::CastMember(member_ref))
                        })
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
        "castLibNum" => Ok(Datum::Int(sprite.map_or(0, |x| {
            x.member.as_ref().map_or(0, |y| y.cast_lib)
        }))),
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
        // Check if we're in a filmloop context
        let in_filmloop_context = !matches!(player.current_score_context, ScoreRef::Stage);

        // For filmloop sprites, modifications go to the main stage sprite at the same channel
        // This is because filmloop sprites are rendered copies
        // Note: Read access via get_sprite_in_context correctly returns filmloop sprites
        let sprite = if in_filmloop_context {
            // In filmloop context, but modifications still go to main stage
            // (The filmloop sprite data is read-only rendered state)
            player.movie.score.get_sprite_mut(sprite_id)
        } else {
            player.movie.score.get_sprite_mut(sprite_id)
        };
        f(sprite, arg)
    })
}

pub fn sprite_set_prop(sprite_id: i16, prop_name: &str, value: Datum) -> Result<(), ScriptError> {
    let result = match prop_name {
        "visible" | "visibility" => borrow_sprite_mut(
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
                let val = value?;
                sprite.loc_h = val;
                Ok(())
            },
        ),
        "locV" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                let val = value?;
                sprite.loc_v = val;
                Ok(())
            },
        ),
        "locZ" => {
            // Handle Void as a no-op (Director behavior when setting locZ = VOID)
            if matches!(value, Datum::Void) {
                return Ok(());
            }
            borrow_sprite_mut(
                sprite_id,
                |player| value.int_value(),
                |sprite, value| {
                    sprite.loc_z = value?;
                    Ok(())
                },
            )
        }
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
        "backColor" | "backcolor" => borrow_sprite_mut(
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
        "foreColor" | "forecolor" => borrow_sprite_mut(
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
                // Also check if the member is a film loop
                let (intrinsic_size, is_film_loop) = mem_ref
                    .as_ref()
                    .and_then(|r| player.movie.cast_manager.find_member_by_ref(r))
                    .map(|m| {
                        let size = match &m.member_type {
                            CastMemberType::Bitmap(bitmap) => {
                                Some((bitmap.info.width as i32, bitmap.info.height as i32))
                            }
                            CastMemberType::Shape(shape) => {
                                Some((shape.shape_info.width() as i32, shape.shape_info.height() as i32))
                            }
                            CastMemberType::VectorShape(vs) => {
                                Some((vs.width().ceil() as i32, vs.height().ceil() as i32))
                            }
                            _ => None,
                        };
                        let is_film_loop = matches!(&m.member_type, CastMemberType::FilmLoop(_));
                        (size, is_film_loop)
                    })
                    .unwrap_or((None, false));

                Ok((mem_ref, intrinsic_size, is_film_loop))
            },
            |sprite, value| {
                let (mem_ref, intrinsic_size, is_film_loop) = value?;

                // Detect whether the member actually changed
                let member_changed = sprite.member != mem_ref;

                // Assign the new member
                sprite.member = mem_ref.clone();

                // Initialize size and reset rotation/skew ONLY if:
                //  - member actually changed
                if member_changed {
                    if sprite.puppet {
                        sprite.reset_for_member_change();
                    }
                    if let Some((w, h)) = intrinsic_size {
                        if w > 0 && h > 0 {
                            sprite.width = w;
                            sprite.height = h;
                            sprite.base_width = w;
                            sprite.base_height = h;
                        }
                    }

                    sprite.has_size_changed = false;
                }

                // If the new member is a film loop, reset its frame and sound triggers
                // This ensures sounds play when a new sprite starts using the film loop
                if is_film_loop && member_changed {
                    if let Some(ref r) = mem_ref {
                        // We need to do this outside borrow_sprite_mut since we need mutable access to cast_manager
                        // Store the member ref to reset later
                        unsafe {
                            if let Some(player) = PLAYER_OPT.as_mut() {
                                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(r) {
                                    if let CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                                        film_loop.current_frame = 1;
                                        film_loop.score.sound_channel_triggered.clear();
                                        film_loop.score.last_sound_clear_frame = None;
                                    }
                                }
                            }
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
                // Check if value looks like a slot number (cast_lib << 16 | cast_member)
                // Director's castNum getter returns slot numbers, and some scripts
                // incorrectly pass castNum to memberNum setter
                let actual_member_num = if value > 65535 {
                    // Value is a slot number, extract just the member part (lower 16 bits)
                    (value as u32 & 0xFFFF) as i32
                } else {
                    value
                };
                let new_member_ref = match &sprite.member {
                    Some(member_ref) => cast_member_ref(member_ref.cast_lib, actual_member_num),
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
                        let datum = player.get_datum(cursor_id);
                        let slot = match datum {
                            Datum::CastMember(member_ref) => {
                                CastMemberRefHandlers::get_cast_slot_number(
                                    member_ref.cast_lib as u32,
                                    member_ref.cast_member as u32,
                                ) as i32
                            }
                            _ => datum.int_value()?,
                        };
                        cursor_ids.push(slot);
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
                    // Director auto-coerces a 2-element list to a point for loc
                    Datum::List(_, list, _) if list.len() == 2 => {
                        reserve_player_mut(|player| {
                            let x = player.get_datum(&list[0]).int_value()?;
                            let y = player.get_datum(&list[1]).int_value()?;
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
                            CastMemberType::FilmLoop(film_loop) => {
                                // For filmloops, registration point is the center of the content bounds
                                let w = film_loop.initial_rect.width();
                                let h = film_loop.initial_rect.height();
                                ((w / 2) as i16, (h / 2) as i16)
                            }
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
        "moveableSprite" | "moveable" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.moveable = value.to_bool()?;
                Ok(())
            },
        ),
        "constraint" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                let val = value?;
                sprite.constraint = val;
                Ok(())
            },
        ),
        "trails" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.trails = value.to_bool()?;
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

/// For matte-ink sprites, check if a point (in stage coordinates) hits an opaque pixel.
/// Returns true if the sprite is not matte-ink, or if the matte is not yet computed,
/// or if the pixel at the given point is opaque.
fn matte_pixel_hit_test(player: &DirPlayer, sprite: &Sprite, rect: &IntRect, hit_x: i32, hit_y: i32) -> bool {
    if !should_matte_sprite(sprite.ink as u32) {
        return true;
    }
    let member_ref = match sprite.member.as_ref() {
        Some(r) => r,
        None => return true,
    };
    let member = match player.movie.cast_manager.find_member_by_ref(member_ref) {
        Some(m) => m,
        None => return true,
    };
    let bmp_member = match member.member_type.as_bitmap() {
        Some(b) => b,
        None => return true,
    };
    let bitmap = match player.bitmap_manager.get_bitmap(bmp_member.image_ref) {
        Some(b) => b,
        None => return true,
    };
    let matte = match bitmap.matte.as_ref() {
        Some(m) => m,
        None => return true, // Matte not yet computed, fall back to bounding box
    };

    let rect_w = (rect.right - rect.left).max(1);
    let rect_h = (rect.bottom - rect.top).max(1);

    // Map stage coordinates to bitmap coordinates (handling scaling)
    let bx = ((hit_x - rect.left) as u32 * bitmap.width as u32 / rect_w as u32) as u16;
    let by = ((hit_y - rect.top) as u32 * bitmap.height as u32 / rect_h as u32) as u16;

    matte.get_bit(bx, by)
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

    // Check for rotation or skew that requires coordinate transformation
    let has_rotation = sprite.rotation.abs() > 0.1;
    let has_skew_flip = sprite.has_skew_flip();

    if has_rotation || has_skew_flip {
        // Transform mouse coordinates using INVERSE transform
        // to check against the original (untransformed) sprite rect
        let center_x = sprite.loc_h as f64;
        let center_y = sprite.loc_v as f64;

        // Translate to registration point
        let dx = x as f64 - center_x;
        let dy = y as f64 - center_y;

        // Apply inverse rotation (negate the angle)
        let theta = -sprite.rotation * std::f64::consts::PI / 180.0;
        let cos_theta = theta.cos();
        let sin_theta = theta.sin();

        let rx = dx * cos_theta - dy * sin_theta;
        let mut ry = dx * sin_theta + dy * cos_theta;

        // Apply inverse skew flip (negate y after inverse rotation)
        if has_skew_flip {
            ry = -ry;
        }

        // Translate back
        let transformed_x = (rx + center_x) as i32;
        let transformed_y = (ry + center_y) as i32;

        if transformed_x >= rect.left && transformed_x < rect.right
            && transformed_y >= rect.top && transformed_y < rect.bottom
        {
            return matte_pixel_hit_test(player, sprite, &rect, transformed_x, transformed_y);
        }
        return false;
    }

    let left = rect.left;
    let top = rect.top;
    let right = rect.right;
    let bottom = rect.bottom;
    if x >= left && x < right && y >= top && y < bottom {
        return matte_pixel_hit_test(player, sprite, &rect, x, y);
    }
    false
}

fn is_active_sprite(player: &DirPlayer, sprite: &Sprite) -> bool {
    // Per Director docs, an "active sprite" has a sprite script (behavior) OR cast member script.
    if sprite.script_instance_list.len() > 0 {
        return true;
    }
    if let Some(member_ref) = sprite.member.as_ref() {
        if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
            if member.get_member_script_ref().is_some() || member.get_script_id().is_some() {
                return true;
            }
        }
    }
    false
}

pub fn get_sprite_at(player: &DirPlayer, x: i32, y: i32, scripted: bool) -> Option<u32> {
    for channel in player.movie.score.get_sorted_channels(player.movie.current_frame).iter().rev() {
        if concrete_sprite_hit_test(player, &channel.sprite, x, y)
            && (!scripted || is_active_sprite(player, &channel.sprite))
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
            // Get registration point from bitmap member
            let mut reg_x = bitmap_member.reg_point.0;
            let mut reg_y = bitmap_member.reg_point.1;

            // Get bitmap dimensions from info
            let bitmap_width = bitmap_member.info.width as i32;
            let bitmap_height = bitmap_member.info.height as i32;

            // Determine the actual dimensions to use for the sprite rectangle.
            // In some cases, we need to use the bitmap's original dimensions instead of
            // the sprite's dimensions. This matches Director's behavior where sprites
            // can either have explicitly set dimensions or inherit from their bitmap.
            // Check if score dimensions represent a uniform scale of the bitmap.
            // If scale_x and scale_y differ by more than 10%, the score dimensions
            // are just a bounding box (not an intentional stretch) and we should
            // use the bitmap's natural size instead.
            let aspect_ratio_matches = if bitmap_width > 0 && bitmap_height > 0
                && sprite.width > 0 && sprite.height > 0
            {
                let scale_x = sprite.width as f32 / bitmap_width as f32;
                let scale_y = sprite.height as f32 / bitmap_height as f32;
                let ratio = if scale_x > scale_y { scale_x / scale_y } else { scale_y / scale_x };
                ratio <= 1.1
            } else {
                true // can't check, assume match
            };

            // Check if the sprite is intentionally stretched significantly larger
            // than the bitmap in both dimensions. In that case, trust sprite dimensions
            // regardless of aspect ratio differences.
            // Also require that the scale factors aren't wildly uneven (ratio < 2.5),
            // otherwise the sprite dims are likely a bounding box, not a real stretch.
            let intentionally_stretched = sprite.width > 0 && sprite.height > 0
                && bitmap_width > 0 && bitmap_height > 0
                && sprite.width > bitmap_width * 3 / 2
                && sprite.height > bitmap_height * 3 / 2
                && {
                    // Check max(scale_x/scale_y, scale_y/scale_x) < 2.5
                    // Using integer math: 2*a < 5*b AND 2*b < 5*a
                    let a = sprite.width as i64 * bitmap_height as i64;
                    let b = sprite.height as i64 * bitmap_width as i64;
                    2 * a < 5 * b && 2 * b < 5 * a
                };

            let is_debug_sprite = false;

            let (sprite_width, sprite_height) = if sprite.has_size_tweened {
                // Size tween is active — use the tweened dimensions directly,
                // bypassing all bitmap-vs-sprite heuristics.
                (sprite.width, sprite.height)
            } else if sprite.bitmap_size_owned_by_sprite
                && bitmap_width >= 10 && bitmap_height >= 10 {
                if is_debug_sprite {
                    web_sys::console::log_1(&format!("Sprite {}: chose BITMAP_SIZE_OWNED_BY_SPRITE (sprite {}x{}, bitmap {}x{})",
                        sprite.number, sprite.width, sprite.height, bitmap_width, bitmap_height).into());
                }
                // Sprite size is owned by bitmap, and bitmap is not tiny
                (bitmap_width, bitmap_height)
            } else if !aspect_ratio_matches
                && bitmap_width >= 10 && bitmap_height >= 10 {
                if is_debug_sprite {
                    web_sys::console::log_1(&format!("Sprite {}: chose ASPECT_RATIO_MISMATCH (sprite {}x{}, bitmap {}x{}, aspect_ratio_matches={}, has_size_changed={}, bitmap_size_owned_by_sprite={}, intentionally_stretched={})",
                        sprite.number, sprite.width, sprite.height, bitmap_width, bitmap_height, aspect_ratio_matches, sprite.has_size_changed,  sprite.bitmap_size_owned_by_sprite, intentionally_stretched).into());
                }
                if intentionally_stretched {
                    (sprite.width, sprite.height)
                } else {
                    // Score dimensions have a different aspect ratio than the bitmap,
                    // meaning they're a bounding box, not an intentional stretch.
                    // Use bitmap's natural dimensions.
                    (bitmap_width, bitmap_height)
                }
            } else if !sprite.has_size_changed
                && (bitmap_width + bitmap_height) > (sprite.width + sprite.height)
                && bitmap_width >= 10 && bitmap_height >= 10 {
                if is_debug_sprite {
                    web_sys::console::log_1(&format!("Sprite {}: chose NO_SIZE_CHANGE_BITMAP_LARGER (sprite {}x{}, bitmap {}x{}, has_size_changed={})",
                        sprite.number, sprite.width, sprite.height, bitmap_width, bitmap_height, sprite.has_size_changed).into());
                }
                // Sprite hasn't been explicitly resized and bitmap is larger (by sum).
                (bitmap_width, bitmap_height)
            } else if (sprite.width != bitmap_width || sprite.height != bitmap_height)
                && (sprite.width as i64 * bitmap_height as i64 != sprite.height as i64 * bitmap_width as i64)
                && bitmap_width >= 10 && bitmap_height >= 10 {
                if is_debug_sprite {
                    web_sys::console::log_1(&format!("Sprite {}: chose NON_PROPORTIONAL_SCALE (sprite {}x{}, bitmap {}x{})",
                        sprite.number, sprite.width, sprite.height, bitmap_width, bitmap_height).into());
                }
                // Score dimensions differ from bitmap AND are not a clean proportional
                // scale - they are an approximate bounding box. Use bitmap's natural size.
                (bitmap_width, bitmap_height)
            } else if sprite.has_size_changed && sprite.width != bitmap_width ||  sprite.has_size_changed && sprite.height != bitmap_height {
                if is_debug_sprite {
                    web_sys::console::log_1(&format!("Sprite {}: chose SIZE_CHANGED_BY_LINGO (sprite {}x{}, bitmap {}x{}, aspect_ratio_matches={}, has_size_changed={}, bitmap_size_owned_by_sprite={}, intentionally_stretched={})",
                        sprite.number, sprite.width, sprite.height, bitmap_width, bitmap_height, aspect_ratio_matches, sprite.has_size_changed,  sprite.bitmap_size_owned_by_sprite, intentionally_stretched).into());
                }
                // Sprite dimensions differ from base — something (Lingo script,
                // member change) has explicitly modified the size at runtime.
                // Use the current dimensions directly.
                (sprite.width, sprite.height)
            } else {
                if is_debug_sprite {
                    web_sys::console::log_1(&format!("Sprite {}: chose DEFAULT (sprite {}x{}, bitmap {}x{}, aspect_ratio_matches={}, has_size_changed={}, bitmap_size_owned={})",
                        sprite.number, sprite.width, sprite.height, bitmap_width, bitmap_height,
                        aspect_ratio_matches, sprite.has_size_changed, sprite.bitmap_size_owned_by_sprite).into());
                }
                // Use sprite's explicit dimensions (default case)
                (sprite.width, sprite.height)
            };

            // If centerRegPoint is enabled, calculate the centered registration point
            if bitmap_member.info.center_reg_point && sprite_width > 0 && sprite_height > 0 {
                reg_x = (sprite_width / 2) as i16;
                reg_y = (sprite_height / 2) as i16;
            }

            // Step 1: Calculate scaled registration offset
            // The registration point needs to be scaled proportionally when sprite is stretched.
            let scaled_reg_x = if bitmap_width > 0 {
                ((reg_x as i32 * sprite_width) as f32 / bitmap_width as f32).round() as i32
            } else {
                reg_x as i32
            };

            let scaled_reg_y = if bitmap_height > 0 {
                ((reg_y as i32 * sprite_height) as f32 / bitmap_height as f32).round() as i32
            } else {
                reg_y as i32
            };

            // Step 2: Apply flips by mirroring the registration offset
            // When flipped, the registration point's position relative to the sprite changes.
            let final_reg_x = if sprite.flip_h {
                sprite_width - scaled_reg_x
            } else {
                scaled_reg_x
            };

            let final_reg_y = if sprite.flip_v {
                sprite_height - scaled_reg_y
            } else {
                scaled_reg_y
            };

            // Step 3: Create rect centered on registration point, then translate to sprite position
            // The rect is positioned so the registration point sits at (loc_h, loc_v).
            let left = sprite.loc_h - final_reg_x;
            let top = sprite.loc_v - final_reg_y;
            let right = left + sprite_width;
            let bottom = top + sprite_height;

            IntRect::from(left, top, right, bottom)
        }
        CastMemberType::Shape(_shape_member) => {
            // Shapes use rect origin (0,0) — no registration point offset
            let left = sprite.loc_h;
            let top = sprite.loc_v;

            IntRect::from(
                left,
                top,
                left + sprite.width,
                top + sprite.height,
            )
        }
        CastMemberType::Field(field_member) => {
            let field_width = sprite.width;
            let rect_height = (field_member.rect_bottom - field_member.rect_top).max(0) as i32;
            let extras = (2 * field_member.border as i32)
                + (2 * field_member.margin as i32)
                + (4 * field_member.box_drop_shadow as i32);
            // For "adjust" fields, use sprite.height as initial estimate.
            // The WebGL2 renderer will override with measure_text for accurate height.
            let is_adjust = field_member.box_type == "adjust";
            let field_height = if field_member.word_wrap && is_adjust && field_member.text_height > 0 {
                // Word wrap + adjust box type: expand to fit all wrapped text content.
                (field_member.text_height as i32 + extras).max(sprite.height)
            } else if sprite.height > 0 {
                // Fixed/scroll fields and non-wrapping fields: use sprite.height.
                sprite.height
            } else if field_member.text_height > 0 {
                field_member.text_height as i32 + extras
            } else if rect_height > 0 && field_member.box_type != "adjust" {
                rect_height + extras
            } else {
                sprite.height
            };
            let field_height = field_height.max(1);
            IntRect::from_size(sprite.loc_h, sprite.loc_v, field_width, field_height)
        }
        CastMemberType::Button(button_member) => {
            // Button dimensions come from initialRect (field.rect_*), not sprite bbox
            let field = &button_member.field;
            let rect_w = (field.rect_right - field.rect_left).max(0) as i32;
            let rect_h = (field.rect_bottom - field.rect_top).max(0) as i32;
            let extras = (2 * field.border as i32)
                + (2 * field.margin as i32);
            // Use rect dimensions if available, otherwise fall back to sprite dimensions
            let btn_w = if rect_w > 0 { rect_w + extras } else { sprite.width };
            let btn_h = if rect_h > 0 { rect_h + extras } else { sprite.height };
            // For checkbox/radio, add 16px width for the indicator
            let extra_w = match button_member.button_type {
                super::cast_member::ButtonType::CheckBox | super::cast_member::ButtonType::RadioButton => 16,
                _ => 0,
            };
            IntRect::from_size(sprite.loc_h, sprite.loc_v, btn_w + extra_w, btn_h.max(1))
        }
        CastMemberType::Text(text_member) => {
            // Use member dimensions (from TextInfo or TextMember), falling back to sprite
            let (info_width, info_height) = if let Some(info) = &text_member.info {
                (
                    if info.width > 0 { info.width as i32 } else { 0 },
                    if info.height > 0 { info.height as i32 } else { 0 },
                )
            } else {
                (0, 0)
            };

            let text_width = if info_width > 0 {
                info_width
            } else if text_member.width > 0 {
                text_member.width as i32
            } else {
                sprite.width
            };

            let text_height = if info_height > 0 {
                info_height
            } else if text_member.height > 0 {
                text_member.height as i32
            } else {
                sprite.height
            };

            // Calculate draw position based on registration point from TextInfo
            let (draw_x, draw_y) = if let Some(info) = &text_member.info {
                if info.center_reg_point {
                    (sprite.loc_h - text_width / 2, sprite.loc_v - text_height / 2)
                } else if info.reg_x != 0 || info.reg_y != 0 {
                    (sprite.loc_h - info.reg_x, sprite.loc_v - info.reg_y)
                } else {
                    (sprite.loc_h, sprite.loc_v)
                }
            } else {
                (sprite.loc_h, sprite.loc_v)
            };

            debug!(
                "[TEXT_RECT] sprite#{} text='{}' info={}x{} member={}x{} sprite={}x{} -> {}x{}",
                sprite.number,
                &text_member.text[..text_member.text.len().min(30)],
                info_width, info_height,
                text_member.width, text_member.height,
                sprite.width, sprite.height,
                text_width, text_height,
            );
            IntRect::from_size(draw_x, draw_y, text_width, text_height)
        }
        CastMemberType::FilmLoop(film_loop) => {
            // The filmloop's rect is stored in info as:
            // - reg_point = (left, top) coordinates of the rect
            // - width = right coordinate
            // - height = bottom coordinate
            let rect_left = film_loop.info.reg_point.0 as i32;
            let rect_top = film_loop.info.reg_point.1 as i32;
            let rect_right = film_loop.info.width as i32;
            let rect_bottom = film_loop.info.height as i32;
            let info_width = (rect_right - rect_left).max(1);
            let info_height = (rect_bottom - rect_top).max(1);

            // Use sprite dimensions if available, otherwise info dimensions
            let use_width = if sprite.width > 0 { sprite.width } else { info_width };
            let use_height = if sprite.height > 0 { sprite.height } else { info_height };

            // Film loops always use center registration (loc is the center point)
            let reg_x = use_width / 2;
            let reg_y = use_height / 2;

            IntRect::from(
                sprite.loc_h - reg_x,
                sprite.loc_v - reg_y,
                sprite.loc_h - reg_x + use_width,
                sprite.loc_v - reg_y + use_height,
            )
        }
        CastMemberType::VectorShape(_vs) => {
            // VectorShapes with centerRegPoint use center registration
            let reg_x = sprite.width / 2;
            let reg_y = sprite.height / 2;
            IntRect::from(
                sprite.loc_h - reg_x,
                sprite.loc_v - reg_y,
                sprite.loc_h - reg_x + sprite.width,
                sprite.loc_v - reg_y + sprite.height,
            )
        }
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
