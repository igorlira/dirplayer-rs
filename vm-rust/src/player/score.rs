use std::{cell::RefCell, cmp::max, sync::Arc};

use itertools::Itertools;
use log::{debug, warn};
use wasm_bindgen::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    console_warn,
    director::{
        chunks::score::{FrameLabel, ScoreFrameChannelData, SoundChannelData, TempoChannelData},
        file::DirectorFile,
        lingo::datum::{datum_bool, Datum, DatumType},
    },
    js_api::JsApi,
    player::bitmap::bitmap::{PaletteRef, get_system_default_palette},
    player::bitmap::drawing::{should_matte_hit_test},
    player::bitmap::palette::SYSTEM_WIN_PALETTE,
    player::events::dispatch_event_endsprite_for_score,
    player::score_keyframes::{
        ChannelKeyframes,
        build_all_keyframes_cache,
        KeyframeTrack,
    },
    player::handlers::datum_handlers::player_call_datum_handler,
};

use super::{
    allocator::ScriptInstanceAllocatorTrait,
    cast_lib::{cast_member_ref, CastMemberRef, NULL_CAST_MEMBER_REF},
    cast_member::CastMemberType,
    datum_ref::DatumRef,
    geometry::{IntRect, IntRectTuple},
    handlers::datum_handlers::{
        cast_member_ref::CastMemberRefHandlers,
        script::ScriptDatumHandlers,
        sound_channel::SoundStatus,
    },
    movie::Movie,
    reserve_player_mut, reserve_player_ref,
    script::{script_get_prop_opt, script_set_prop},
    script_ref::ScriptInstanceRef,
    sprite::{ColorRef, CursorRef, Sprite},
    DirPlayer, ScriptError, PLAYER_OPT,
};

// JS bridge names use the `dirplayer_` prefix so this fork's globals don't
// collide with stock Ruffle if both are loaded on the same page (e.g. via a
// browser extension). Matching JS-side definitions live in
// src/services/flashPlayerManager.ts::initFlashBridge.
#[wasm_bindgen]
extern "C" {
    // All sprite-method Flash bridge calls now key by sprite_num — each
    // Flash sprite has its own dedicated Ruffle instance.
    #[wasm_bindgen(js_name = "dirplayer_ruffleIsPlaying")]
    fn ruffle_is_playing(sprite_num: i32) -> bool;
    #[wasm_bindgen(js_name = "dirplayer_ruffleGetFrameCount")]
    fn ruffle_get_frame_count(sprite_num: i32) -> i32;
    #[wasm_bindgen(js_name = "dirplayer_ruffleGetCurrentFrame")]
    fn ruffle_get_current_frame(sprite_num: i32) -> i32;
    /// `sprite.frame = N` semantic: seek + STOP (pin at the frame).
    /// This is the gotoAndStop path used for poster-frame Flash sprites
    /// like storyscramble's tiles. For `sprite(N).gotoFrame(...)` (seek +
    /// keep playing — required by mello's Fire/Marshmello which play
    /// looping animations under each label) use `dirplayer_ruffleGoToFrame`.
    #[wasm_bindgen(js_name = "dirplayer_ruffleGoToFrameAndStop")]
    fn ruffle_goto_frame_and_stop(sprite_num: i32, frame_or_label: &str);
    /// Classify what's under a sprite-local point in the SWF, mirroring
    /// Director's Flash `sprite.hitTest()`: 0 = #background, 1 = #normal,
    /// 2 = #button, 3 = #editText. The fork injects a synthetic MouseMove at
    /// the point (the offscreen player gets no real motion) then reads the
    /// resolved cursor + a stage shape pick. Drives `mouseOverButton`
    /// (== #button) — no dependency on continuous mouse routing.
    #[wasm_bindgen(js_name = "dirplayer_ruffleHitTest")]
    fn ruffle_hit_test(sprite_num: i32, x: f64, y: f64) -> i32;
}

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
    /// Sound channel sprite spans extracted from `frame_intervals`. Keyed by
    /// channel_index (4 = Sound1, 3 = Sound2 on D6+). Used by the audio-time
    /// score-sync logic in `run_frame_loop` so it only catches up to audio
    /// time while the playhead is inside the authored sound 1 span — outside
    /// that range, the score is meant to advance at its own tempo. The
    /// regular `sprite_spans` Vec excludes effects channels (1-5) so we
    /// stash these separately.
    pub sound_channel_spans: HashMap<u32, (u32, u32)>,
    pub tempo_channel_data: Vec<(u32, TempoChannelData)>,
    pub palette_channel_data: Vec<(u32, i16, i16)>,
    pub frame_labels: Vec<FrameLabel>,
    pub sound_channel_triggered: HashMap<u16, u32>,
    pub keyframes_cache: Arc<HashMap<u16, ChannelKeyframes>>,
    /// Sprite detail behaviors indexed by spriteListIdx (D6+)
    pub sprite_details: HashMap<u32, crate::director::chunks::score::SpriteDetailInfo>,
    /// User-defined tile patterns (VWTL). Shape `pattern` 57-64 maps to index
    /// 0-7; a non-zero `member` overrides the built-in tile with a region of
    /// that bitmap cast member (e.g. employee's blue/white checker background).
    pub custom_tiles: Vec<crate::director::chunks::tile_list::TilePatternEntry>,
    /// Track the last frame where we cleared sound triggers (to prevent double-clearing)
    pub last_sound_clear_frame: Option<u32>,
    /// D5 movies need per-frame sprite property updates from channel_initialization_data
    /// (since sprite properties can change every frame via delta compression)
    pub needs_per_frame_updates: bool,
    /// Channels that have spans from frame_intervals (not from extend_sprite_spans).
    /// Used to prevent per-frame delta initialization from showing sprites outside their span range.
    pub channels_with_frame_interval_spans: HashSet<u32>,
    /// Total frame count (used for auto-looping back to frame 1 when past last frame)
    pub frame_count: Option<u32>,
    /// Active non-puppet channel numbers derived from sprite_spans, cached per frame.
    pub active_channels_cache: RefCell<HashMap<u32, Vec<usize>>>,
    /// Sorted render order cache for the last requested frame and runtime state generation.
    pub sorted_channels_cache: RefCell<Option<(u32, u64, Vec<usize>)>>,
    /// Incremented when runtime sprite state changes in a way that affects render inclusion/order.
    pub render_channel_cache_generation: u64,
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
pub fn get_sprite_rect_in_context(player: &DirPlayer, sprite_id: i16) -> IntRectTuple {
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

/// Convert raw blend byte from score data to a 0-100 percentage.
///
/// D6+ stores blend on an inverted 0-255 scale (raw 0 → 100%, 255 → 0%,
/// 128 → ~50%), BUT the raw byte is only meaningful when the sprite's
/// "blend enabled" flag is set — `sprite_flags` (score byte 22) bit 4
/// (0x10). When the flag is clear the raw byte is an unused default (it
/// can be 0 OR 255 depending on the sprite) and the sprite is fully opaque.
///
/// This flag gate is what reconciles two same-version (D6) movies that the
/// raw value alone cannot:
///   - SpongeBob "JellyFishin'" sprite 40: flag SET, raw 127 → 50% overlay.
///   - load_bug.dcr loader (sprite 2): flag CLEAR, raw 255 → 100% (NOT 0%).
/// An earlier value-only approach (inverted for all D6+) correctly dimmed
/// sprite 40 but wrongly made the loader transparent (raw 255 → 0%); the
/// old direct-0-100 path did the reverse. Only the flag disambiguates.
/// It also matches spineworld_dcr's pDropList sprite 799 (D8+), authored at
/// `blend=0`: that has the flag SET, so raw 255 → 0% as intended.
///
/// `human_version` maps the late-Director-6 file version 1223 to exactly
/// 600 (1224+ → 700), so the threshold is `>= 600` to include D6. D5 and
/// earlier have no authored sprite blend and keep the direct path.
pub(crate) fn convert_raw_blend(raw: u8, sprite_flags: u8, dir_version: u16) -> i32 {
    if dir_version >= 600 {
        // Blend only applies when the "blend enabled" flag (bit 4) is set;
        // otherwise the raw byte is a junk default → fully opaque.
        if sprite_flags & 0x10 != 0 {
            ((255.0 - raw as f32) * 100.0 / 255.0).round() as i32
        } else {
            100
        }
    } else {
        // D5 and earlier: direct percentage, 0 = fully opaque (not set)
        if raw == 0 { 100 } else { (raw as i32).min(100) }
    }
}

impl Score {
    pub fn empty() -> Score {
        Score {
            channels: vec![],
            frame_labels: vec![],
            channel_initialization_data: vec![],
            sound_channel_data: vec![],
            sound_channel_spans: HashMap::new(),
            tempo_channel_data: vec![],
            palette_channel_data: vec![],
            sprite_spans: vec![],
            sound_channel_triggered: HashMap::new(),
            keyframes_cache: Arc::new(HashMap::new()),
            sprite_details: HashMap::new(),
            custom_tiles: Vec::new(),
            last_sound_clear_frame: None,
            needs_per_frame_updates: false,
            channels_with_frame_interval_spans: HashSet::new(),
            frame_count: None,
            active_channels_cache: RefCell::new(HashMap::new()),
            sorted_channels_cache: RefCell::new(None),
            render_channel_cache_generation: 0,
        }
    }

    pub fn invalidate_render_channel_cache(&mut self) {
        self.render_channel_cache_generation =
            self.render_channel_cache_generation.wrapping_add(1);
        self.sorted_channels_cache.replace(None);
    }

    pub fn invalidate_span_channel_cache(&mut self) {
        self.active_channels_cache.borrow_mut().clear();
        self.invalidate_render_channel_cache();
    }

    pub fn active_channel_numbers_for_frame(&self, frame_num: u32) -> Vec<usize> {
        if let Some(cached) = self.active_channels_cache.borrow().get(&frame_num) {
            return cached.clone();
        }

        let mut active_channels: Vec<usize> = self
            .sprite_spans
            .iter()
            .filter(|span| Self::is_span_in_frame(span, frame_num))
            .map(|span| span.channel_number as usize)
            .collect();
        active_channels.sort_unstable();
        active_channels.dedup();
        self.active_channels_cache
            .borrow_mut()
            .insert(frame_num, active_channels.clone());
        active_channels
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

    /// Start frame of the channel-0 (frame-script) span covering `frame`.
    /// Identifies the span so the cached frame-script instance can be recreated
    /// when the playhead crosses into a different span (even of the same behavior
    /// member, which may carry different parameters).
    pub fn get_frame_script_span_start(&self, frame: u32) -> Option<u32> {
        self.sprite_spans
            .iter()
            .find(|span| {
                span.channel_number == 0 && frame >= span.start_frame && frame <= span.end_frame
            })
            .map(|span| span.start_frame)
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
            debug!("create_behavior: script member {} not found in cast_lib {}, found in cast_lib {} instead", 
                cast_member, resolved_cast_lib, found_cast_lib);
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
                            let is_void = matches!(existing_datum, Datum::Void);
                            debug!("  [getPropertyDescriptionList] Property '{}' exists with type {:?}, is_void: {}", 
                                prop_name, existing_datum.type_enum(), is_void);
                            if !is_void {
                                match existing_datum {
                                    Datum::String(s) => debug!("    Existing value: {:?}", s),
                                    Datum::Int(n) => debug!("    Existing value: {}", n),
                                    _ => {}
                                }
                            }
                            is_void
                        } else {
                            debug!("  [getPropertyDescriptionList] Property '{}' does not exist, will set default", prop_name);
                            true
                        };

                        if should_set_default {
                            // Find the default value
                            for (key_name, default_value_ref) in desc_props {
                                if key_name == "default" {
                                    let default_value = player.get_datum(&default_value_ref);
                                    debug!("    [getPropertyDescriptionList] Will set default for '{}' to {:?}", 
                                        prop_name, default_value.type_enum());
                                    defaults_to_set.push((prop_name.clone(), default_value_ref));

                                    break;
                                }
                            }
                        } else {
                            debug!("    [getPropertyDescriptionList] Skipping default for '{}' (already has value)", prop_name);
                        }
                    }

                    // Third pass: set all defaults
                    debug!("  [getPropertyDescriptionList] Setting {} default values", defaults_to_set.len());
                    for (prop_name, default_value_ref) in defaults_to_set {
                        let default_value = player.get_datum(&default_value_ref);
                        debug!("    [getPropertyDescriptionList] Setting '{}' = {:?}", prop_name, default_value.type_enum());
                        let result = script_set_prop(
                            player,
                            &script_instance_ref,
                            &prop_name,
                            &default_value_ref,
                            false,
                        );
                        if let Err(e) = result {
                            debug!("      ⚠️ Failed: {}", e.message);
                        }
                    }
                }

                Ok::<(), ScriptError>(())
            })?;
        }

        // Director instantiates a score BEHAVIOR as a child object and calls
        // its `on new me` handler (if defined) at sprite-entry, BEFORE
        // beginSprite (and after property defaults are seeded above, so the
        // handler sees its authored/default props). Many older behaviors do
        // ALL their setup in `on new` — pengapop's Sparkle01 parks its sprite
        // offscreen (`the locV of sprite mysprite = -500`) there and defines
        // no beginSprite; without this the sparkle sprite stays at its
        // authored (visible) position and a stale `tiny_sparkle0001` frame
        // lingers on screen at game start.
        //
        // ONLY for ScriptType::Score (score behaviors). Parent scripts
        // (ScriptType::Parent) already had `on new` run when they were
        // explicitly `new()`'d — a parent-script instance sitting in a
        // sprite's script_instance_list (e.g. via scriptInstanceList.add or
        // parent-script-as-field-member) must NOT be re-`new`'d here, or it
        // gets double-initialized. Dispatched only to instances that actually
        // define `on new`, so beginSprite-only behaviors no-op; it never falls
        // through to movie-script `on new`.
        let should_call_new = reserve_player_ref(|player| {
            let Some(inst) = player.allocator.get_script_instance_opt(&script_instance_ref) else {
                return false;
            };
            let Some(script) = player.movie.cast_manager.get_script_by_ref(&inst.script) else {
                return false;
            };
            // Behaviors only (see above). Parent scripts already had `on new` run.
            if script.script_type != crate::director::enums::ScriptType::Score {
                return false;
            }
            // Only auto-call a NO-ARG `on new me`. A handler that declares extra
            // parameters (`on new me, aSprite, ...`) is meant to be called
            // explicitly with those args; auto-dispatching with none would run
            // it with VOID arguments and misbehave. `argument_name_ids` includes
            // the implicit `me`, so length <= 1 means "me only".
            match script.get_own_handler("new") {
                Some(h) => h.argument_name_ids.len() <= 1,
                None => false,
            }
        });
        if should_call_new {
            let receivers = vec![script_instance_ref.clone()];
            let _ = crate::player::events::player_invoke_event_to_instances(
                &"new".to_string(),
                &vec![],
                &receivers,
            )
            .await;
        }

        Ok(())
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
                // Capture the last on-screen rect BEFORE reset for visible stage
                // sprites that still have a member. Director keeps a channel's
                // `the rect of sprite` at its last value after the sprite leaves
                // its span (member clears to 0); 3D init scripts read
                // sprite(1).rect on the between-spans transition frame.
                let retained_rect = if matches!(score_ref, ScoreRef::Stage) {
                    player.movie.score.get_sprite(sprite_num as i16).and_then(|sprite| {
                        if sprite.visible && !sprite.puppet && sprite.member.is_some() {
                            let r = get_concrete_sprite_rect(player, sprite);
                            Some((r.left, r.top, r.right, r.bottom))
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };
                let did_reset = {
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
                        false
                    } else if sprite.visible {
                        // Visible non-puppet sprite leaving its span → full reset,
                        // but keep the last on-screen rect for empty-channel reads.
                        sprite.reset();
                        sprite.retained_rect = retained_rect;
                        true
                    } else {
                        // Invisible non-puppet exited sprite: clear ONLY the
                        // behavior lifecycle (instances + entered/exited) so a
                        // re-entered span re-creates the behavior and re-fires
                        // beginSprite — but PRESERVE the visual state (visible,
                        // member, loc). A full reset() forces `visible = true`
                        // and drops the member, which is wrong here for two
                        // reasons:
                        //  - spectral-wizard's Help scroll bar hides sprite 15
                        //    (`sprite(15).visible = 0`) for short pages; on the
                        //    second visit its stale first-visit instance lingered
                        //    (myState=#done), InstallElement was skipped, and the
                        //    shared `ourMaxScroll` stayed empty → SetScroll crashed
                        //    on `ourMaxScroll[1]`. It needs the lifecycle cleared.
                        //  - Pinball keeps its flipper-frame sprites deliberately
                        //    hidden until interaction; forcing them visible drew
                        //    every animation frame at once. It needs `visible=0`
                        //    preserved.
                        // Clearing the lifecycle satisfies the first; preserving
                        // the visual state satisfies the second.
                        sprite.script_instance_list.clear();
                        sprite.entered = false;
                        sprite.exited = false;
                        true
                    }
                };
                // Invalidate the cached scriptInstanceList so stale ScriptInstanceRefs
                // don't prevent deallocation of old script instances.
                if did_reset {
                    player.remove_script_instance_list_cache(sprite_num as i16);
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

        // Get dir_version for blend conversion (D8+ uses inverted 0-255 scale for all sprites)
        let dir_version = reserve_player_ref(|player| player.movie.dir_version);

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
                // Score "stretch" flag (sprite ink byte bit 0x80): authoritative
                // signal for whether the sprite was resized off its member's
                // natural size. Drives get_concrete_sprite_rect's sprite-vs-bitmap
                // dimension choice and `the stretch of sprite`.
                sprite.stretch = data.stretch as i32;
                // Apply the score channel's flipH/flipV bits (sprite_flags bit 5
                // / bit 6). Previously only Lingo `sprite.flipH =` set these, so
                // score-authored flipped Flash/bitmap sprites rendered
                // un-mirrored — bogey_nights' end-game grab hands (sprites 17/20,
                // authored flipH/flipV in the score) reached from the wrong side.
                // A behavior that sets flipH in exitFrame still wins (scripts run
                // after the channel update), matching Director's puppet semantics.
                sprite.flip_h = data.flip_h();
                sprite.flip_v = data.flip_v();

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
                    sprite.blend = convert_raw_blend(data.blend, data.sprite_flags, dir_version);
                    // Shape ink encoding: mask off the high bit and divide by 5
                    sprite.ink = if dir_version > 700 { ((data.ink & 0x7F) / 5) as i32 } else { data.ink as i32 }
                } else {
                    // Non-shape sprites: mask off the high bit (bit 7 is a flag, not part of ink number)
                    sprite.ink = (data.ink & 0x7F) as i32;
                    sprite.blend = convert_raw_blend(data.blend, data.sprite_flags, dir_version);
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
                sprite.explicit_lingo_size = false;
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
                // Score "stretch" flag (sprite ink byte bit 0x80): authoritative
                // signal for whether the sprite was resized off its member's
                // natural size. Drives get_concrete_sprite_rect's sprite-vs-bitmap
                // dimension choice and `the stretch of sprite`.
                sprite.stretch = data.stretch as i32;
                sprite.ink = (data.ink & 0x7F) as i32;
                sprite.blend = convert_raw_blend(data.blend, data.sprite_flags, dir_version);

                // Attach a sprite-script behavior that appears mid-span. D5
                // sprites can change member per frame and bring a scriptId with
                // them; begin_sprites only attaches at span-enter (using the
                // span's START frame data), so a script that shows up on a
                // later frame would never bind. 'hackeys clickbutton (script 13,
                // `on mouseDown` → click=3) appears on channel 6 at frame 2
                // when the channel switches from member 21 to member 22+script
                // 13 — without this, clicking never registers and the kick
                // never fires. Guarded against re-attachment because the
                // per-frame deltas re-fire every loop of `go the frame`.
                if dir_version < 600 && data.sprite_list_idx_lo != 0 {
                    let script_cast_lib = if data.sprite_list_idx_hi == 0
                        || data.sprite_list_idx_hi == 65535 {
                        1
                    } else {
                        data.sprite_list_idx_hi as i32
                    };
                    let script_member = data.sprite_list_idx_lo as i32;
                    let script_ref = CastMemberRef {
                        cast_lib: script_cast_lib,
                        cast_member: script_member,
                    };
                    let already_attached = reserve_player_ref(|player| {
                        self.get_sprite(sprite_num).map_or(false, |s| {
                            s.script_instance_list.iter().any(|inst_ref| {
                                player.allocator.get_script_instance(inst_ref).script == script_ref
                            })
                        })
                    });
                    if !already_attached {
                        if let Some((instance_ref, _datum)) =
                            Self::create_behavior(script_cast_lib, script_member, None)
                        {
                            reserve_player_mut(|player| {
                                let sprite_num_ref = player.alloc_datum(Datum::Int(sprite_num as i32));
                                let _ = script_set_prop(
                                    player,
                                    &instance_ref,
                                    &"spriteNum".to_string(),
                                    &sprite_num_ref,
                                    false,
                                );
                            });
                            self.get_sprite_mut(sprite_num)
                                .script_instance_list
                                .push(instance_ref);
                        }
                    }
                }
            }
        }

        // D6+ per-frame sprite initialization from accumulated delta data.
        // Only initializes sprites that have NO spans at all — their lifecycle
        // isn't managed by frame_intervals so they need to be entered from raw data.
        // Does NOT update already-entered sprites (their properties are managed
        // by spans or Lingo scripts).
        if dir_version >= 600 && !self.needs_per_frame_updates {
            // Channels that have ANY span (frame_intervals or extended).
            // These are managed by the span system — don't double-initialize from delta data.
            let channels_with_any_span: HashSet<u32> = self.sprite_spans
                .iter()
                .filter(|span| span.channel_number > 0)
                .map(|span| span.channel_number)
                .collect();

            let mut latest_by_channel: std::collections::HashMap<u16, ScoreFrameChannelData> = std::collections::HashMap::new();
            for (frame_index, channel_index, data) in self.channel_initialization_data.iter() {
                if frame_index + 1 <= frame_num {
                    latest_by_channel.insert(*channel_index, data.clone());
                }
            }

            for (channel_index, data) in latest_by_channel.iter() {
                let channel_number = get_channel_number_from_index(*channel_index as u32);
                if channel_number < 1 || data.cast_member == 0 {
                    continue;
                }
                let sprite_num = channel_number as i16;
                let already_entered = self.get_sprite(sprite_num)
                    .map_or(false, |s| s.entered);

                if !already_entered && !channels_with_any_span.contains(&channel_number) {
                    // Only enter from delta data if the channel has NO spans at all.
                    // Channels with spans (frame_intervals or extended) have their
                    // lifecycle managed by the span system.
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

                    match &score_ref {
                        ScoreRef::Stage => {
                            let _ = sprite_set_prop(sprite_num, "member", Datum::CastMember(member.clone()));
                        }
                        ScoreRef::FilmLoop(_) => {
                            let sprite = self.get_sprite_mut(sprite_num);
                            sprite.member = Some(member.clone());
                        }
                    }

                    let sprite = self.get_sprite_mut(sprite_num);
                    sprite.loc_h = data.pos_x as i32;
                    sprite.loc_v = data.pos_y as i32;
                    if data.width != 0 { sprite.width = data.width as i32; }
                    if data.height != 0 { sprite.height = data.height as i32; }
                    sprite.skew = data.skew as f64;
                    sprite.rotation = data.rotation as f64;
                    sprite.moveable = data.moveable;
                    sprite.trails = data.trails;
                    sprite.stretch = data.stretch as i32;
                    sprite.ink = (data.ink & 0x7F) as i32;
                    sprite.blend = convert_raw_blend(data.blend, data.sprite_flags, dir_version);
                    // Set base_* values so tweens can work
                    sprite.base_loc_h = sprite.loc_h;
                    sprite.base_loc_v = sprite.loc_v;
                    sprite.base_width = sprite.width;
                    sprite.base_height = sprite.height;
                    sprite.base_rotation = sprite.rotation;
                    sprite.base_blend = sprite.blend;
                    sprite.base_skew = sprite.skew;
                    sprite.entered = true;
                }
            }
        }

        // D6-ONLY per-frame MEMBER swap within a span. Some D6 movies
        // "score-record" a sprite that changes its cast member every frame
        // inside a single span — e.g. SpongeBob "JellyFishin'"'s instructions/
        // controls text field (sprite 46) cycling members 36→37→38 across
        // frames 36-39. In D6 the span system sets the member only at span
        // entry, so without this the sprite is stuck on the first page. We
        // update ONLY the member (not pos/size, to avoid disturbing tweens or
        // Lingo-driven motion) when the current frame's delta names a
        // different, non-empty member for an already-entered, non-puppet sprite.
        //
        // Restricted to D6 (==600): D7+ (e.g. dir_version 700, raw 1406) carry
        // per-frame member changes inside the keyframe-bearing 52+ byte spans
        // we now parse, so applying this delta-based swap there double-updates
        // and fights the span data. Only D6 lacks those member keyframes.
        if dir_version == 600 {
            let member_updates: Vec<(i16, CastMemberRef)> = self.channel_initialization_data
                .iter()
                .filter_map(|(frame_idx, channel_idx, data)| {
                    if frame_idx + 1 != frame_num || data.cast_member == 0 {
                        return None;
                    }
                    let channel_number = get_channel_number_from_index(*channel_idx as u32);
                    if channel_number < 1 {
                        return None;
                    }
                    let sprite_num = channel_number as i16;
                    let sprite = self.get_sprite(sprite_num)?;
                    if !sprite.entered || sprite.puppet {
                        return None;
                    }
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
                    if sprite.member.as_ref() == Some(&member) {
                        return None; // already on the right member
                    }
                    Some((sprite_num, member))
                })
                .collect();
            for (sprite_num, member) in member_updates {
                match &score_ref {
                    ScoreRef::Stage => {
                        let _ = sprite_set_prop(sprite_num, "member", Datum::CastMember(member));
                    }
                    ScoreRef::FilmLoop(_) => {
                        self.get_sprite_mut(sprite_num).member = Some(member);
                    }
                }
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

                                        if let Datum::CastMember(current_cast_ref) =
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

                                        if let Datum::CastMember(current_cast_ref) =
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

        // Mark ALL entering channels as entered — even those without channel_initialization_data
        // (e.g., channel 0 / frame scripts). Without this, channels that only appear in
        // spans_to_enter (not in span_init_data) would never get entered=true and
        // would re-enter every frame cycle, leaking script instances.
        for span in &spans_to_enter {
            let sprite = self.get_sprite_mut(span.channel_number as i16);
            sprite.entered = true;
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
                            Datum::ScriptInstanceRef(instance_ref) => Ok(instance_ref.clone()),
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
                            debug!(
                                "[BEHAVIOR-APPLY] frame_interval: applying {} params for cast {}/{}",
                                behavior_ref.parameter.len(), behavior_ref.cast_lib, behavior_ref.cast_member
                            );
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
                                                    Datum::CastMember(m) => debug!("      value: member {} of castLib {}", m.cast_member, m.cast_lib),
                                                    _ => debug!("      value: <{:?}>", value.type_enum()),
                                                }

                                                Some((key_name.clone(), value_ref.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();

                                    for (prop_name, value_ref) in &props_to_set {
                                        let val_str = match player.get_datum(value_ref) {
                                            Datum::Int(n) => format!("{}", n),
                                            Datum::Float(f) => format!("{:.4}", f),
                                            Datum::String(s) => format!("{:?}", s),
                                            Datum::Vector(v) => format!("vector({:.2},{:.2},{:.2})", v[0], v[1], v[2]),
                                            Datum::Symbol(s) => format!("#{}", s),
                                            other => format!("<{:?}>", other.type_enum()),
                                        };
                                        let result = script_set_prop(
                                            player,
                                            &actual_instance_ref,
                                            prop_name,
                                            value_ref,
                                            false,
                                        );
                                        if let Err(e) = &result {
                                            warn!(
                                                "[BEHAVIOR-APPLY] FAILED {}.{} = {}: {}",
                                                behavior_ref.cast_member, prop_name, val_str, e.message
                                            );
                                        }
                                    }
                                    // Log all property values for debugging
                                    let summary: Vec<String> = props_to_set.iter().map(|(name, vref)| {
                                        let v = match player.get_datum(vref) {
                                            Datum::Int(n) => format!("{}", n),
                                            Datum::Float(f) => format!("{:.4}", f),
                                            Datum::String(s) => format!("{:?}", &s[..s.len().min(30)]),
                                            Datum::Vector(v) => format!("v({:.1},{:.1},{:.1})", v[0], v[1], v[2]),
                                            Datum::Symbol(s) => format!("#{}", s),
                                            other => format!("<{:?}>", other.type_enum()),
                                        };
                                        format!("{}={}", name, v)
                                    }).collect();
                                    debug!(
                                        "[BEHAVIOR-APPLY] cast {}/{}: [{}]",
                                        behavior_ref.cast_lib, behavior_ref.cast_member,
                                        summary.join(", ")
                                    );
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
            if sprite_list_idx == 0 && !(dir_version < 600 && data.sprite_list_idx_lo != 0) {
                continue;
            }

            if dir_version >= 600 && sprite_list_idx != 0 {
                // D6+ path: spriteListIdx references the sprite detail table
                // Check if sprite already has behaviors (skip if fully initialized)
                let channel_num = span.channel_number;
                let has_behaviors = reserve_player_ref(|player| {
                    let score = match &score_ref {
                        ScoreRef::Stage => &player.movie.score,
                        ScoreRef::FilmLoop(member_ref) => {
                            match player.movie.cast_manager.find_member_by_ref(member_ref) {
                                Some(member) => match &member.member_type {
                                    super::cast_member::CastMemberType::FilmLoop(film_loop) => &film_loop.score,
                                    _ => &player.movie.score,
                                },
                                None => &player.movie.score,
                            }
                        }
                    };
                    score.get_sprite(channel_num as i16)
                        .map_or(false, |s| !s.script_instance_list.is_empty())
                });
                if has_behaviors {
                    continue;
                }

                let (detail_info_opt, _details_count) = reserve_player_ref(|player| {
                    let info = player.movie.score.sprite_details.get(&sprite_list_idx).cloned();
                    let count = player.movie.score.sprite_details.len();
                    (info, count)
                });

                if let Some(detail_info) = detail_info_opt {
                    if detail_info.behaviors.is_empty() {
                        continue;
                    }

                    debug!(
                        "Attaching {} behaviors from spriteListIdx {} to channel {}",
                        detail_info.behaviors.len(), sprite_list_idx, channel_num
                    );

                    for behavior in &detail_info.behaviors {
                        debug!(
                            "   Creating behavior from spriteDetail cast {}/{} for channel {}",
                            behavior.cast_lib, behavior.cast_member, channel_num
                        );

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

                        let actual_instance_ref = reserve_player_mut(|player| {
                            let datum = player.get_datum(&datum_ref);
                            match datum {
                                Datum::ScriptInstanceRef(instance_ref) => Ok(instance_ref.clone()),
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

                        // Apply behavior parameters from initializer data
                        if !behavior.parameter.is_empty() {
                            debug!(
                                "[BEHAVIOR-APPLY] sprite_details: applying {} params for cast {}/{}",
                                behavior.parameter.len(), behavior.cast_lib, behavior.cast_member
                            );
                            reserve_player_mut(|player| {
                                for param_ref in &behavior.parameter {
                                    let param_datum = player.get_datum(param_ref);
                                    debug!("  [sprite_details] Parameter type: {:?}", param_datum.type_enum());
                                    if let Datum::PropList(props, _) = param_datum {
                                        let props_to_set: Vec<(String, DatumRef)> = props.iter()
                                            .filter_map(|(key_ref, value_ref)| {
                                                let key = player.get_datum(key_ref);
                                                if let Datum::Symbol(key_name) = key {
                                                    let value = player.get_datum(value_ref);
                                                    debug!("    [sprite_details] prop: {} type: {:?}", key_name, value.type_enum());
                                                    match value {
                                                        Datum::String(s) => debug!("      [sprite_details] value: {:?}", s),
                                                        Datum::Int(n) => debug!("      [sprite_details] value: {}", n),
                                                        _ => debug!("      [sprite_details] value: <{:?}>", value.type_enum()),
                                                    }
                                                    Some((key_name.clone(), value_ref.clone()))
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();
                                        let prop_count = props_to_set.len();
                                        for (prop_name, value_ref) in props_to_set {
                                            let result = script_set_prop(
                                                player,
                                                &actual_instance_ref,
                                                &prop_name,
                                                &value_ref,
                                                false,
                                            );
                                            if let Err(e) = &result {
                                                warn!(
                                                    "[BEHAVIOR-APPLY] FAILED to set {}: {}",
                                                    prop_name, e.message
                                                );
                                            }
                                        }
                                        debug!(
                                            "[BEHAVIOR-APPLY] set {} properties on cast {}/{}",
                                            prop_count, behavior.cast_lib, behavior.cast_member
                                        );
                                    }
                                }
                                Ok::<(), ScriptError>(())
                            }).expect("Failed to set sprite detail behavior parameters");
                        } else {
                            debug!("⚠️ [sprite_details] No parameters to apply for behavior cast {}/{}", 
                                behavior.cast_lib, behavior.cast_member);
                        }

                        // Attach behavior to sprite
                        let score_ref_clone = score_ref.clone();
                        reserve_player_mut(|player| {
                            let sprite_num = channel_num as i16;
                            let sprite = match &score_ref_clone {
                                ScoreRef::Stage => player.movie.score.get_sprite_mut(sprite_num),
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
                }
            } else if dir_version < 600 && data.sprite_list_idx_lo != 0 {
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
                debug!(
                    "D5 sprite ch={}: scriptId=({},{}), sprite_member=({},{})",
                    channel_num, resolved_cast_lib, script_member,
                    sprite_cast_lib, data.cast_member
                );

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
                                Datum::ScriptInstanceRef(instance_ref) => Ok(instance_ref.clone()),
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

        // D6+ behavior attachment for sprites entered by the per-frame delta block.
        // These sprites have entered=true but no behaviors yet. Iterate latest_by_channel
        // to find their spriteListIdx and attach behaviors from sprite_details.
        if dir_version >= 600 {
            let mut latest_by_channel: std::collections::HashMap<u16, ScoreFrameChannelData> = std::collections::HashMap::new();
            for (frame_index, channel_index, data) in self.channel_initialization_data.iter() {
                if frame_index + 1 <= frame_num {
                    latest_by_channel.insert(*channel_index, data.clone());
                }
            }

            for (channel_index, data) in &latest_by_channel {
                let sprite_list_idx = data.sprite_list_idx();
                if sprite_list_idx == 0 {
                    continue;
                }

                let channel_num = get_channel_number_from_index(*channel_index as u32);
                if channel_num < 1 {
                    continue;
                }

                // Only process sprites that are entered but have no behaviors yet
                let (is_entered, has_behaviors) = reserve_player_ref(|player| {
                    let score = match &score_ref {
                        ScoreRef::Stage => &player.movie.score,
                        ScoreRef::FilmLoop(member_ref) => {
                            match player.movie.cast_manager.find_member_by_ref(member_ref) {
                                Some(member) => match &member.member_type {
                                    super::cast_member::CastMemberType::FilmLoop(film_loop) => &film_loop.score,
                                    _ => &player.movie.score,
                                },
                                None => &player.movie.score,
                            }
                        }
                    };
                    score.get_sprite(channel_num as i16)
                        .map_or((false, false), |s| (s.entered, !s.script_instance_list.is_empty()))
                });

                if !is_entered || has_behaviors {
                    continue;
                }

                let detail_info_opt = reserve_player_ref(|player| {
                    player.movie.score.sprite_details.get(&sprite_list_idx).cloned()
                });

                if let Some(detail_info) = detail_info_opt {
                    if detail_info.behaviors.is_empty() {
                        continue;
                    }

                    for behavior in &detail_info.behaviors {
                        let behavior_result = Self::create_behavior(
                            behavior.cast_lib as i32,
                            behavior.cast_member as i32,
                            default_cast_lib,
                        );

                        let (_, datum_ref) = match behavior_result {
                            Some(result) => result,
                            None => continue,
                        };

                        let actual_instance_ref = reserve_player_mut(|player| {
                            let datum = player.get_datum(&datum_ref);
                            match datum {
                                Datum::ScriptInstanceRef(instance_ref) => Ok(instance_ref.clone()),
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

                        // Apply behavior parameters
                        if !behavior.parameter.is_empty() {
                            debug!("🔧 [delta-data] Applying {} saved parameters for behavior cast {}/{}", 
                                behavior.parameter.len(), behavior.cast_lib, behavior.cast_member);
                            reserve_player_mut(|player| {
                                for param_ref in &behavior.parameter {
                                    let param_datum = player.get_datum(param_ref);
                                    debug!("  [delta-data] Parameter type: {:?}", param_datum.type_enum());
                                    if let Datum::PropList(props, _) = param_datum {
                                        let props_to_set: Vec<(String, DatumRef)> = props.iter()
                                            .filter_map(|(key_ref, value_ref)| {
                                                let key = player.get_datum(key_ref);
                                                if let Datum::Symbol(key_name) = key {
                                                    let value = player.get_datum(value_ref);
                                                    debug!("    [delta-data] prop: {} type: {:?}", key_name, value.type_enum());
                                                    match value {
                                                        Datum::String(s) => debug!("      [delta-data] value: {:?}", s),
                                                        Datum::Int(n) => debug!("      [delta-data] value: {}", n),
                                                        _ => debug!("      [delta-data] value: <{:?}>", value.type_enum()),
                                                    }
                                                    Some((key_name.clone(), value_ref.clone()))
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();
                                        for (prop_name, value_ref) in props_to_set {
                                            debug!("      [delta-data] Setting property {} on script instance", prop_name);
                                            let result = script_set_prop(
                                                player,
                                                &actual_instance_ref,
                                                &prop_name,
                                                &value_ref,
                                                false,
                                            );
                                            if let Err(e) = result {
                                                debug!("      [delta-data] ⚠️ Failed to set property {}: {}", prop_name, e.message);
                                            } else {
                                                debug!("      [delta-data] ✅ Successfully set property {}", prop_name);
                                            }
                                        }
                                    }
                                }
                                Ok::<(), ScriptError>(())
                            }).expect("Failed to set behavior parameters");
                        } else {
                            debug!("⚠️ [delta-data] No parameters to apply for behavior cast {}/{}", 
                                behavior.cast_lib, behavior.cast_member);
                        }

                        let score_ref_clone = score_ref.clone();
                        reserve_player_mut(|player| {
                            let sprite = match &score_ref_clone {
                                ScoreRef::Stage => player.movie.score.get_sprite_mut(channel_num as i16),
                                ScoreRef::FilmLoop(member_ref) => {
                                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
                                        if let super::cast_member::CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                                            film_loop.score.get_sprite_mut(channel_num as i16)
                                        } else {
                                            player.movie.score.get_sprite_mut(channel_num as i16)
                                        }
                                    } else {
                                        player.movie.score.get_sprite_mut(channel_num as i16)
                                    }
                                }
                            };
                            sprite.script_instance_list.push(actual_instance_ref.clone());
                            Ok::<(), ScriptError>(())
                        })
                        .expect("Failed to attach delta-data behavior to sprite");
                    }
                }
            }
        }

        // Frame script lifecycle: keep the instance alive across frames within the same
        // span (so script properties like markList persist), discard it when the script
        // changes or when we leave the span.
        // Frame scripts need instances so that `me` resolves to ScriptInstanceRef (not ScriptRef)
        // in their handlers — e.g., `me.spriteNum` in beginSprite/enterFrame handlers.
        let new_script_member = self.get_script_in_frame(frame_num).map(|b| CastMemberRef {
            cast_lib: b.cast_lib as i32,
            cast_member: b.cast_member as i32,
        });
        // Identity of the channel-0 span at this frame. The same behavior member can
        // be dropped on several consecutive spans with different parameters (the Game
        // Loop is `#Nonlooping` on the intro frames and `#CostumeChange`/`#Looping` on
        // the gameplay frames). Tracking the span start lets us recreate + re-apply
        // parameters when crossing a span boundary instead of carrying the previous
        // span's stale `pType` (which made the gameplay `case pType of` fall through →
        // no `go(the frame)` → the playhead marched through every frame to the end).
        let new_span_start = self.get_frame_script_span_start(frame_num);

        // Discard the cached instance if the script member OR the span changed (or it
        // no longer applies). Without this, the previous span's instance lingers when
        // entering a new span, keeping stale properties/parameters.
        //
        // CRITICAL: only the MAIN movie (Stage) drives `player.movie.frame_script_instance`.
        // Film loops / nested scores have their own frame numbering, and at a film-loop
        // frame with no channel-0 script `new_script_member` is None — without this gate
        // the film loop's begin_sprites discards the MAIN movie's channel-0 frame-script
        // instance every tick, forcing it to be recreated each frame (the "Game Loop"
        // frame script in Trick-or-Treat-Beat was recreated 329×, resetting its state).
        let is_stage = matches!(score_ref, ScoreRef::Stage);
        let should_discard = is_stage && reserve_player_ref(|player| {
            player.movie.frame_script_instance.is_some()
                && (player.movie.frame_script_member != new_script_member
                    || player.movie.frame_script_span_start != new_span_start)
        });
        if should_discard {
            reserve_player_mut(|player| {
                player.movie.frame_script_instance = None;
                player.movie.frame_script_member = None;
                player.movie.frame_script_span_start = None;
            });
        }

        if let Some(behavior_ref) = self.get_script_in_frame(frame_num)
            .filter(|_| is_stage)
        {
            // Only create when no instance is cached (covers initial entry and post-discard).
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
                        Datum::ScriptInstanceRef(inst) => inst.clone(),
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

                // Cache the instance, member ref, AND the span it belongs to so we
                // recreate (and re-apply params) when the playhead enters a new span.
                reserve_player_mut(|player| {
                    player.movie.frame_script_instance = Some(actual_instance_ref);
                    player.movie.frame_script_member = Some(cast_member_ref);
                    player.movie.frame_script_span_start = new_span_start;
                });

                debug!("✓ Frame script instance created and cached");
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
            sprite.stretch = data.stretch as i32;

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
                sprite.blend = convert_raw_blend(data.blend, data.sprite_flags, dir_version);
                // Shape ink encoding: mask off the high bit and divide by 5
                sprite.ink = ((data.ink & 0x7F) / 5) as i32;
            } else {
                // Non-shape sprites use standard encoding
                sprite.ink = data.ink as i32;
                sprite.blend = convert_raw_blend(data.blend, data.sprite_flags, dir_version);
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
        let active_channels = self.active_channel_numbers_for_frame(frame);

        for channel_number in active_channels {
            let Some(channel) = self.channels.get_mut(channel_number) else {
                continue;
            };
            let sprite = &mut channel.sprite;
            let sprite_num = sprite.number as u16;

            // In Director, puppeted sprites are controlled by Lingo and
            // score tweens do NOT apply to them.
            if sprite.puppet {
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

        self.invalidate_render_channel_cache();
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

        self.invalidate_render_channel_cache();
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
        // Out-of-range writes silently land on channel 0 instead of panicking
        // on a negative-i16-cast-to-usize index. The cleaner fix is to make
        // every call-site bounds-check explicitly, but there are 30+ callers
        // and a panic at this level was bringing the whole VM down on
        // legitimate Lingo patterns like `sprite(N).prop = X` where N came
        // from a list-lookup that can return -1/0. Lingo entry points
        // (`sprite_set_prop` / `sprite_get_prop`) guard separately and
        // short-circuit before reaching here, so this branch is just a
        // last-line safety net.
        let idx = if number < 0 || number as usize >= self.channels.len() {
            log::warn!(
                "get_sprite_mut: out-of-range sprite number {} (channels.len()={}), clamping to 0",
                number,
                self.channels.len()
            );
            0
        } else {
            number as usize
        };
        let channel = &mut self.channels[idx];
        return &mut channel.sprite;
    }

    pub fn load_from_score_chunk(
        &mut self,
        score_chunk: &crate::director::chunks::score::ScoreChunk,
        dir_version: u16,
    ) {
        self.set_channel_count(score_chunk.frame_data.header.num_channels as usize);

        // Clear previous sprite_spans so they don't accumulate across movie transitions
        self.sprite_spans.clear();
        self.sprite_details.clear();
        self.invalidate_span_channel_cache();

        self.channel_initialization_data = score_chunk.frame_data.frame_channel_data.clone();
        self.sound_channel_data = score_chunk.frame_data.sound_channel_data.clone();
        self.tempo_channel_data = score_chunk.frame_data.tempo_channel_data.clone();
        self.palette_channel_data = score_chunk.frame_data.palette_channel_data.clone();
        self.keyframes_cache = Arc::new(build_all_keyframes_cache(
            &score_chunk.frame_data.frame_channel_data,
            &score_chunk.frame_intervals
        ));

        // Capture sound-channel sprite spans (channel_index 3 = Sound2,
        // 4 = Sound1 on D6+). These are excluded from the regular
        // sprite_spans Vec below, but the audio-time score-sync code
        // needs Sound1's span range so it only catches up while the
        // playhead is inside the authored span.
        for (primary, _) in &score_chunk.frame_intervals {
            if primary.channel_index == 3 || primary.channel_index == 4 {
                let entry = self.sound_channel_spans
                    .entry(primary.channel_index)
                    .or_insert((primary.start_frame, primary.end_frame));
                entry.0 = entry.0.min(primary.start_frame);
                entry.1 = entry.1.max(primary.end_frame);
            }
        }

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

        // Record which channels have spans from frame_intervals (before extend adds more)
        self.channels_with_frame_interval_spans = self.sprite_spans
            .iter()
            .filter(|span| span.channel_number > 0)
            .map(|span| span.channel_number)
            .collect();

        // For filmloops (and any score with empty frame_intervals), generate
        // sprite_spans from frame_channel_data to ensure sprites can be rendered
        if self.sprite_spans.is_empty() && !self.channel_initialization_data.is_empty() {
            self.generate_sprite_spans_from_channel_data(dir_version);
        }

        // For D6+ movies with incomplete frame_intervals, extend sprite_spans
        // to cover sprites that exist in channel_initialization_data but have no span.
        // This ensures those sprites get proper lifecycle (beginSprite events).
        if dir_version >= 600 && !self.sprite_spans.is_empty() {
            self.extend_sprite_spans_from_channel_data();
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

        // Collect the exact frames each channel actually holds a sprite. We do
        // NOT collapse to min/max — a channel is commonly reused by different
        // sprites with EMPTY frames in between (sprite A in 1-10, nothing in
        // 11-20, sprite B in 21-30). A single 1-30 span would keep the sprite
        // visible across the 11-20 gap (norman shows sprites that shouldn't be
        // on the frame). Instead build one span per CONTIGUOUS run of frames.
        let mut channel_frames: HashMap<u32, Vec<u32>> = HashMap::new();

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
            channel_frames.entry(channel_number).or_default().push(frame_num);
        }

        // Create a sprite span for each contiguous run of frames per channel.
        for (channel_number, mut frames) in channel_frames {
            frames.sort_unstable();
            frames.dedup();
            let mut i = 0;
            while i < frames.len() {
                let start_frame = frames[i];
                let mut end_frame = start_frame;
                while i + 1 < frames.len() && frames[i + 1] == end_frame + 1 {
                    end_frame = frames[i + 1];
                    i += 1;
                }
                self.sprite_spans.push(ScoreSpriteSpan {
                    channel_number,
                    start_frame,
                    end_frame,
                    scripts: Vec::new(),
                });
                i += 1;
            }
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

            // Don't extend into gaps between multi-frame spans from frame_intervals.
            // Multi-frame spans (start != end) define intentional sprite lifecycles —
            // gaps between them are deliberate (e.g. a black overlay disappearing
            // between fade-out and fade-in). Single-frame spans just represent
            // individual behavior script entries, not lifecycle boundaries.
            if self.channels_with_frame_interval_spans.contains(&channel_number) {
                if let Some(spans) = channel_spans.get(&channel_number) {
                    // Only consider multi-frame spans for gap detection
                    let multi_frame_spans: Vec<_> = spans.iter()
                        .filter(|(s, e)| s != e)
                        .collect();
                    let has_mf_span_before = multi_frame_spans.iter().any(|&&(_, end)| end < frame_num);
                    let has_mf_span_after = multi_frame_spans.iter().any(|&&(start, _)| start > frame_num);
                    if has_mf_span_before && has_mf_span_after {
                        continue;
                    }
                }
            }


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
                // Skip frames already covered by a span (including ones we just added)
                let already_covered = channel_spans
                    .get(&channel_number)
                    .map(|spans| spans.iter().any(|(s, e)| start_frame >= *s && start_frame <= *e))
                    .unwrap_or(false);
                if already_covered {
                    continue;
                }

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
        self.custom_tiles = dir.tile_list.as_ref()
            .map(|t| t.tiles.clone())
            .unwrap_or_default();
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

        self.invalidate_render_channel_cache();
        JsApi::dispatch_score_changed();
    }

    pub fn get_sorted_channels(&self, frame_num: u32) -> Vec<&SpriteChannel> {
        let generation = self.render_channel_cache_generation;
        if let Some((cached_frame, cached_generation, cached_indices)) =
            self.sorted_channels_cache.borrow().as_ref()
        {
            if *cached_frame == frame_num && *cached_generation == generation {
                return cached_indices
                    .iter()
                    .filter_map(|index| self.channels.get(*index))
                    .collect();
            }
        }

        let mut channel_indices = self.active_channel_numbers_for_frame(frame_num);
        let mut seen = channel_indices.iter().copied().collect::<HashSet<usize>>();
        for channel in &self.channels {
            if channel.number != 0 && channel.sprite.puppet && seen.insert(channel.number) {
                channel_indices.push(channel.number);
            }
        }

        channel_indices.retain(|index| {
            self.channels.get(*index).is_some_and(|channel| {
                channel.number != 0
                    && channel
                        .sprite
                        .member
                        .as_ref()
                        .is_some_and(|member| member.is_valid())
                    && channel.sprite.visible
            })
        });

        channel_indices.sort_by(|a, b| {
            let a_channel = &self.channels[*a];
            let b_channel = &self.channels[*b];
            let res = a_channel.sprite.loc_z.cmp(&b_channel.sprite.loc_z);
            if res == std::cmp::Ordering::Equal {
                a_channel.number.cmp(&b_channel.number)
            } else {
                res
            }
        });

        self.sorted_channels_cache
            .replace(Some((frame_num, generation, channel_indices.clone())));

        channel_indices
            .iter()
            .filter_map(|index| self.channels.get(*index))
            .collect()
    }

    pub fn get_active_script_instance_list_for_frame(&self, frame_num: u32) -> Vec<ScriptInstanceRef> {
        let active_channels = self.active_channel_numbers_for_frame(frame_num);
        let total: usize = active_channels
            .iter()
            .filter_map(|channel_num| self.channels.get(*channel_num))
            .map(|channel| channel.sprite.script_instance_list.len())
            .sum();
        let mut instance_list = Vec::with_capacity(total);
        for channel_num in active_channels {
            let Some(channel) = self.channels.get(channel_num) else {
                continue;
            };
            instance_list.extend(channel.sprite.script_instance_list.iter().cloned());
        }
        instance_list
    }

    pub fn get_frame_tempo(&self, frame: u32) -> Option<u32> {
        // Search through tempo_channel_data to find the most recent tempo change
        // at or before the requested frame.
        // Note: frame_idx is 0-based (from score parsing), frame is 1-based (current_frame).
        // frame_idx 0 = Director frame 1, so we need frame_idx < frame.
        let tempo_data = self.tempo_channel_data
            .iter()
            .rev()
            .find(|(frame_idx, _)| *frame_idx < frame)
            .map(|(_, td)| td)?;

        let tempo = tempo_data.tempo;
        if tempo == 0 {
            return None;
        }

        // D6+ tempo encoding: special codes 246-255
        match tempo {
            246 => {
                // FPS mode: actual FPS is in tempo_cue_point
                let fps = tempo_data.tempo_cue_point;
                if fps > 0 { Some(fps as u32) } else { None }
            }
            247 => {
                // Delay mode: tempo_cue_point is delay in seconds
                // Convert to a very low FPS to approximate the delay
                // (actual wait-for-delay should be handled separately)
                None // Fall back to movie frame rate; delay handled elsewhere
            }
            248 => {
                // Wait for mouse click - not an FPS value
                None
            }
            254 | 255 => {
                // Wait for sound channel - not an FPS value
                None
            }
            1..=120 => {
                // Direct FPS value (valid for all versions)
                Some(tempo as u32)
            }
            _ => {
                // Other values: pre-D6 special codes or video wait
                // For safety, treat as no tempo change
                None
            }
        }
    }

    pub fn get_frame_palette(&self, frame: u32) -> PaletteRef {
        self.palette_channel_data
            .iter()
            .rev()
            .find(|(frame_idx, _, _)| *frame_idx < frame)
            .map(|(_, cast_lib, member)| {
                if *member < 0 {
                    // Negative member = built-in palette
                    PaletteRef::from(*member, *cast_lib, 0)
                } else if *member > 0 {
                    // Positive member = cast member palette
                    PaletteRef::Member(CastMemberRef {
                        cast_lib: *cast_lib as i32,
                        cast_member: *member as i32,
                    })
                } else {
                    // member == 0: use system default
                    PaletteRef::BuiltIn(get_system_default_palette())
                }
            })
            .unwrap_or_else(|| PaletteRef::BuiltIn(get_system_default_palette()))
    }
}

pub fn sprite_get_prop(
    player: &mut DirPlayer,
    sprite_id: i16,
    prop_name: &str,
) -> Result<Datum, ScriptError> {
    // Clear any previous cached ref. Only set for scriptInstanceList.
    player.last_sprite_prop_ref = None;
    // Use context-aware sprite lookup to support filmloop behaviors
    let sprite = get_sprite_in_context(player, sprite_id);
    match prop_name {
        "ilk" => Ok(Datum::Symbol("sprite".to_string())),
        "spriteNum" | "spriteNumber" => Ok(Datum::Int(
            sprite.map_or(sprite_id as i32, |x| x.number as i32),
        )),
        "loc" => {
            let sprite = get_sprite_in_context(player, sprite_id);
            let (x, y) = sprite.map_or((0, 0), |sprite| (sprite.loc_h, sprite.loc_v));
            Ok(Datum::Point([x as f64, y as f64], 0))
        },
        // Director: `sprite.width` / `sprite.height` return the *displayed*
        // dimensions (= sprite.rect width/height), not the raw score-channel
        // values. For an unscaled bitmap sprite they match bitmap-native dims;
        // for a stretched sprite they match the score-authored width. The
        // heuristics live in `get_concrete_sprite_rect`, which the rect-based
        // getters (left/top/right/bottom/rect) already use. Without this,
        // scripts that compute layer offsets via
        // `destRect = rect(L, T, L + sprite(chan).width, ...)` read the
        // cell-size 48 instead of the overlay's 29×29 and stretch the
        // composite — verified in Trick or Treat Beat's `buildTileAnims`.
        "width" => {
            let rect = get_sprite_rect_in_context(player, sprite_id);
            Ok(Datum::Int((rect.2 - rect.0) as i32))
        }
        "height" => {
            let rect = get_sprite_rect_in_context(player, sprite_id);
            Ok(Datum::Int((rect.3 - rect.1) as i32))
        }
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
            Ok(Datum::Rect([rect.0 as f64, rect.1 as f64, rect.2 as f64, rect.3 as f64], 0))
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
        // `the type of sprite` (Director 11.5): "can be tested and set". An
        // occupied channel reports its member's type symbol (#bitmap, #flash,
        // …); an empty channel reports 0 (matching `sprite(ch).type = 0`
        // clearing a channel). bogeyman tests `if sprite(pBogey).type = #flash`.
        "type" => {
            let member_ref = sprite.and_then(|s| s.member.clone());
            match member_ref {
                Some(m) if m.is_valid() => {
                    match player.movie.cast_manager.find_member_by_ref(&m) {
                        Some(member) => {
                            Ok(Datum::Symbol(member.member_type.type_string().to_string()))
                        }
                        None => Ok(Datum::Int(0)),
                    }
                }
                _ => Ok(Datum::Int(0)),
            }
        }
        "camera" => {
            // Shockwave3D sprite camera — returns the active camera as a Shockwave3dObjectRef
            let member_ref = sprite.and_then(|s| s.member.as_ref()).cloned().unwrap_or(NULL_CAST_MEMBER_REF);
            let cam_name = sprite.and_then(|s| s.w3d_camera.as_ref()).cloned()
                .unwrap_or_else(|| "DefaultView".to_string());
            Ok(Datum::Shockwave3dObjectRef(crate::director::lingo::datum::Shockwave3dObjectRef {
                cast_lib: member_ref.cast_lib,
                cast_member: member_ref.cast_member,
                object_type: "camera".to_string(),
                name: cam_name,
            }))
        }
        "cameraCount" => {
            let count = sprite.map_or(1, |s| {
                1 + s.w3d_cameras.len() as i32
            });
            Ok(Datum::Int(count))
        }
        "flipH" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_h))),
        "flipV" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.flip_v))),
        "rotation" => Ok(Datum::Float(sprite.map_or(0.0, |sprite| sprite.rotation))),
        "scriptInstanceList" => {
            // Return cached list datum if available, so that
            // sprite.scriptInstanceList.add(x) modifies the live list.
            if let Some(cached_ref) = player.script_instance_list_cache.get(&sprite_id).cloned() {
                // Set last_sprite_prop_ref so callers use this DatumRef
                // instead of allocating a new one (which would create
                // a separate copy that .add() wouldn't sync back).
                player.last_sprite_prop_ref = Some(cached_ref.clone());
                Ok(player.get_datum(&cached_ref).clone())
            } else {
                let initial_ids = sprite.map_or(vec![], |x| x.script_instance_list.clone());
                let instance_ids: VecDeque<DatumRef> = initial_ids
                    .iter()
                    .map(|x| player.alloc_datum(Datum::ScriptInstanceRef(x.clone())))
                    .collect();
                let list = Datum::List(DatumType::List, instance_ids, false);
                let list_ref = player.alloc_datum(list.clone());
                player.cache_script_instance_list(sprite_id, list_ref.clone(), initial_ids);
                player.last_sprite_prop_ref = Some(list_ref);
                Ok(list)
            }
        }
        "memberNum" => Ok(Datum::Int(sprite.map_or(0, |x| {
            x.member.as_ref().map_or(0, |y| y.cast_member)
        }))),
        "castNum" => Ok(Datum::Int(sprite.map_or(0, |x| {
            x.member.as_ref().map_or(0, |y| {
                // Director 4 predates multiple cast libraries: `the castNum of
                // sprite` is the bare member number, and scripts round-trip it
                // through `cast <n>` (e.g. the Animator behavior). D5+ uses the
                // slot-encoded value (cast_lib << 16 | member).
                if player.movie.dir_version < 500 {
                    y.cast_member
                } else {
                    CastMemberRefHandlers::get_cast_slot_number(y.cast_lib as u32, y.cast_member as u32)
                        as i32
                }
            })
        }))),
        "scriptNum" => {
            let fallback = sprite.map_or(vec![], |sprite| sprite.script_instance_list.clone());
            let script_ids = player.get_sprite_script_instance_ids(
                sprite_id,
                fallback.as_slice(),
            );
            let script_num = script_ids
                .first()
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
                    let id_refs: VecDeque<DatumRef> = ids
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
        // Flash (SWF) sprite properties — keyed by sprite_num because
        // each Flash sprite has its own dedicated Ruffle instance.
        "playing" => {
            if sprite.and_then(|s| s.member.as_ref()).is_some() {
                Ok(datum_bool(ruffle_is_playing(sprite_id as i32)))
            } else {
                Ok(datum_bool(false))
            }
        }
        "frameCount" => {
            // Prefer the SWF header's FrameCount (parsed from the member bytes)
            // over asking Ruffle: the header is correct even while the instance
            // is still (re)loading, whereas ruffle_get_frame_count returns 0 in
            // that window. A 0 here poisons movies that seed state from
            // frameCount (bogey_nights #hiding). Fall back to Ruffle for
            // compressed CWS members the header parser doesn't handle.
            let header_fc = sprite
                .and_then(|s| s.member.as_ref())
                .and_then(|m| player.movie.cast_manager.find_member_by_ref(m))
                .and_then(|cm| match &cm.member_type {
                    crate::player::cast_member::CastMemberType::Flash(f) => {
                        crate::player::cast_member::CastMember::parse_swf_frame_count(&f.data)
                    }
                    _ => None,
                });
            match header_fc {
                Some(n) => Ok(Datum::Int(n as i32)),
                None if sprite.and_then(|s| s.member.as_ref()).is_some() => {
                    Ok(Datum::Int(ruffle_get_frame_count(sprite_id as i32)))
                }
                None => Ok(Datum::Int(0)),
            }
        }
        "currentFrame" | "frame" => {
            // A behavior's own `property frame` / `property currentFrame` takes
            // precedence over the Flash playhead reading. Many behaviors track
            // an animation frame in a property literally named `frame`
            // (bogey_nights' bogeyman behavior does). Without checking the
            // behaviors first, `sprite(N).frame` was unconditionally hijacked
            // into the Ruffle currentFrame for ANY sprite with a member — the
            // behavior's real value was unreachable and the paired setter
            // blanked the SWF (see the setter arm). Only when no behavior owns
            // the property do we read the Flash playhead (storyscramble tiles).
            let behavior_val = sprite.and_then(|sprite| {
                reserve_player_mut(|player| {
                    sprite.script_instance_list.iter().find_map(|behavior| {
                        script_get_prop_opt(player, behavior, &prop_name.to_string())
                    })
                })
            });
            match behavior_val {
                Some(ref_) => {
                    let datum_clone = player.get_datum(&ref_).clone();
                    player.last_sprite_prop_ref = Some(ref_);
                    Ok(datum_clone)
                }
                None if sprite.and_then(|s| s.member.as_ref()).is_some() => {
                    Ok(Datum::Int(ruffle_get_current_frame(sprite_id as i32)))
                }
                None => Ok(Datum::Int(0)),
            }
        }
        "actionsEnabled" | "buttonsEnabled" | "imageEnabled" | "sound" | "static" => {
            // Flash properties that default to true/1
            Ok(datum_bool(true))
        }
        "quality" => Ok(Datum::String("high".to_string())),
        "scaleMode" => Ok(Datum::String("showAll".to_string())),
        "playBackMode" => Ok(Datum::Int(0)), // 0 = normal
        "centerRegPoint" => Ok(datum_bool(true)),
        "defaultRectMode" => Ok(Datum::Int(0)),
        "eventPassMode" => Ok(Datum::Int(0)),
        "clickMode" => Ok(Datum::Int(0)),
        "fixedRate" => Ok(Datum::Int(0)),
        "streamMode" => Ok(Datum::Int(0)),
        "broadcastProps" => Ok(datum_bool(false)),
        "linked" => Ok(datum_bool(false)),
        "posterFrame" => Ok(Datum::Int(1)),
        "mouseOverButton" => {
            // Flash sprite property: TRUE when the mouse pointer is over a
            // button within the SWF, FALSE when outside the sprite or over a
            // non-button object (e.g. the background). Per the Director
            // dictionary this is exactly `hitTest(mouseLoc) = #button`, so we
            // reuse the Flash hit-test classifier (2 == #button). Splat's
            // titleJump gates `on mouseDown ... go(6)` on
            // `sprite(3).mouseOverButton = 1`; a constant FALSE left the Flash
            // start button dead.
            //
            // The offscreen Ruffle player never sees real mouse motion (we only
            // capture its frames and forward clicks), so the classifier itself
            // injects a synthetic MouseMove at this point before reading the
            // resolved cursor — no dependency on continuous mouse routing. We
            // pass sprite-local pixels (mouseLoc minus the sprite's top-left).
            if sprite.and_then(|s| s.member.as_ref()).is_some() {
                let rect = get_sprite_rect_in_context(player, sprite_id);
                let (mx, my) = player.mouse_loc;
                let lx = (mx - rect.0 as i32) as f64;
                let ly = (my - rect.1 as i32) as f64;
                Ok(datum_bool(ruffle_hit_test(sprite_id as i32, lx, ly) == 2))
            } else {
                Ok(datum_bool(false))
            }
        }
        "viewScale" => Ok(Datum::Float(100.0)),
        "originMode" => Ok(Datum::Int(0)),
        "originH" | "originV" => Ok(Datum::Int(0)),
        "viewH" | "viewV" => Ok(Datum::Int(0)),
        "flashRect" | "defaultRect" => {
            let w = sprite.map_or(0, |s| s.width);
            let h = sprite.map_or(0, |s| s.height);
            Ok(Datum::Rect([0.0, 0.0, w as f64, h as f64], 0))
        }
        "originPoint" | "viewPoint" => {
            Ok(Datum::Point([0.0, 0.0], 0))
        }
        "bytesStreamed" | "bufferSize" | "streamSize" => Ok(Datum::Int(0)),
        "scale" => Ok(Datum::Float(100.0)),
        "editable" => Ok(datum_bool(sprite.map_or(false, |sprite| sprite.editable))),
        "scriptList" | "scriptlist" => {
            // Returns a list of [memberRef, propertiesString] for each behavior
            // attached to this sprite. Used by trigger behaviors (Mouse Left etc.)
            // for event routing.
            use crate::director::lingo::datum::DatumType;
            let behaviors: Vec<(crate::player::cast_lib::CastMemberRef, String)> = sprite
                .map(|s| {
                    reserve_player_mut(|player| {
                        s.script_instance_list.iter().filter_map(|beh_ref| {
                            let inst = player.allocator.get_script_instance_opt(beh_ref)?;
                            let member_ref = inst.script.clone();
                            // Serialize properties as "[#name: value, ...]"
                            let mut parts: Vec<String> = Vec::new();
                            for (key, val_ref) in &inst.properties {
                                let val = player.get_datum(val_ref);
                                let val_str = match val {
                                    Datum::Int(i) => i.to_string(),
                                    Datum::Float(f) => format!("{:.4}", f),
                                    Datum::String(s) => format!("\"{}\"", s),
                                    Datum::Symbol(s) => format!("#{}", s),
                                    Datum::Void => "VOID".to_string(),
                                    _ => "VOID".to_string(),
                                };
                                parts.push(format!("#{}: {}", key.as_str(), val_str));
                            }
                            let props_str = format!("[{}]", parts.join(", "));
                            Some((member_ref, props_str))
                        }).collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default();

            let items: VecDeque<DatumRef> = behaviors.into_iter().map(|(member_ref, props_str)| {
                let member_datum = player.alloc_datum(Datum::CastMember(member_ref));
                let props_datum = player.alloc_datum(Datum::String(props_str));
                let inner = VecDeque::from([member_datum, props_datum]);
                player.alloc_datum(Datum::List(DatumType::List, inner, false))
            }).collect();

            Ok(Datum::List(DatumType::List, items, false))
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
                // Some(ref_) => Ok(player.get_datum(&ref_).clone()),

                Some(ref_) => {
                    // Return a clone of the Datum AND cache the underlying DatumRef
                    // in last_sprite_prop_ref so the bytecode handler uses the real
                    // ref instead of allocating a fresh slot. Without this, mutating
                    // a behavior-stored list/proplist via sprite(N).propName would
                    // mutate a *clone*, severing the link to the script instance's
                    // storage (e.g. setaProp(sprite(N).pCustomData, key, val) would
                    // not propagate back to the behavior's pCustomData).
                    let datum_clone = player.get_datum(&ref_).clone();
                    player.last_sprite_prop_ref = Some(ref_);
                    Ok(datum_clone)
                }

                None => {
                    // Unknown sprite props may be custom behavior properties — return VOID
                    warn!(
                        "Unknown sprite prop '{}' — returning VOID", prop_name
                    );
                    Ok(Datum::Void)
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

fn resolve_sprite_member_assignment(
    player: &DirPlayer,
    value: &Datum,
) -> Result<(Option<CastMemberRef>, Option<(i32, i32)>, bool), ScriptError> {
    let mem_ref = if let Datum::CastMember(cast_member) = value {
        Some(cast_member.clone())
    } else if value.is_string() {
        let name = value.string_value()?;
        player.movie.cast_manager.find_member_ref_by_name(&name)
    } else if value.is_number() {
        player
            .movie
            .cast_manager
            .find_member_ref_by_number(value.int_value()? as u32)
    } else {
        None
    };

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
                CastMemberType::Flash(flash) => {
                    let (l, t, r, b) = flash.effective_rect();
                    Some(((r - l) as i32, (b - t) as i32))
                }
                _ => None,
            };
            let is_film_loop = matches!(&m.member_type, CastMemberType::FilmLoop(_));
            (size, is_film_loop)
        })
        .unwrap_or((None, false));

    Ok((mem_ref, intrinsic_size, is_film_loop))
}

fn sprite_set_prop_is_noop(
    sprite_id: i16,
    prop_name: &str,
    value: &Datum,
) -> Result<bool, ScriptError> {
    reserve_player_ref(|player| {
        let Some(sprite) = player.movie.score.get_sprite(sprite_id) else {
            return Ok(false);
        };

        match prop_name {
            "visible" | "visibility" => Ok(sprite.visible == value.to_bool()?),
            "stretch" => Ok(sprite.stretch == value.int_value()?),
            "locH" => Ok(sprite.loc_h == value.int_value()?),
            "locV" => Ok(sprite.loc_v == value.int_value()?),
            "locZ" => {
                if matches!(value, Datum::Void) {
                    Ok(true)
                } else {
                    Ok(sprite.loc_z == value.int_value()?)
                }
            }
            "width" => {
                let width = value.int_value()?;
                Ok(sprite.width == width && sprite.has_size_changed)
            }
            "height" => {
                let height = value.int_value()?;
                Ok(sprite.height == height && sprite.has_size_changed)
            }
            "left" => {
                let (left, _, _, _) = get_sprite_rect_in_context(player, sprite_id);
                Ok(left == value.int_value()?)
            }
            "top" => {
                let (_, top, _, _) = get_sprite_rect_in_context(player, sprite_id);
                Ok(top == value.int_value()?)
            }
            "right" => {
                let (left, _, _, _) = get_sprite_rect_in_context(player, sprite_id);
                let width = value.int_value()? - left;
                Ok(sprite.width == width && sprite.has_size_changed)
            }
            "bottom" => {
                let (_, top, _, _) = get_sprite_rect_in_context(player, sprite_id);
                let height = value.int_value()? - top;
                Ok(sprite.height == height && sprite.has_size_changed)
            }
            "ink" => Ok(sprite.ink == value.int_value()?),
            "blend" => Ok(sprite.blend == value.int_value()?),
            "rotation" => {
                let rotation = if value.is_number() {
                    value.to_float()?
                } else {
                    0.0
                };
                Ok(sprite.rotation == rotation)
            }
            "skew" => {
                let skew = if value.is_number() {
                    value.to_float()?
                } else {
                    0.0
                };
                Ok(sprite.skew == skew)
            }
            "flipH" => {
                let flip_h = if value.is_number() {
                    value.to_bool()?
                } else {
                    false
                };
                Ok(sprite.flip_h == flip_h)
            }
            "flipV" => {
                let flip_v = if value.is_number() {
                    value.to_bool()?
                } else {
                    false
                };
                Ok(sprite.flip_v == flip_v)
            }
            "backColor" | "backcolor" => {
                let back_color = value.int_value()?;
                Ok(
                    sprite.back_color == back_color
                        && sprite.bg_color == ColorRef::PaletteIndex(back_color as u8)
                        && sprite.has_back_color,
                )
            }
            "bgColor" => {
                let bg_color = value.to_color_ref()?.to_owned();
                Ok(
                    sprite.bg_color == bg_color
                        && sprite.back_color
                            == bg_color.to_index(&SYSTEM_WIN_PALETTE) as i32
                        && sprite.has_back_color,
                )
            }
            "foreColor" | "forecolor" => {
                let fore_color = value.int_value()?;
                Ok(
                    sprite.fore_color == fore_color
                        && sprite.color == ColorRef::PaletteIndex(fore_color as u8)
                        && sprite.has_fore_color,
                )
            }
            "color" => {
                let color = value.to_color_ref()?.to_owned();
                Ok(
                    sprite.color == color
                        && sprite.fore_color == color.to_index(&SYSTEM_WIN_PALETTE) as i32
                        && sprite.has_fore_color,
                )
            }
            "member" => {
                let (mem_ref, _, _) = resolve_sprite_member_assignment(player, value)?;
                Ok(sprite.member == mem_ref)
            }
            "memberNum" => {
                let value = value.int_value()?;
                let actual_member_num = if value > 65535 {
                    (value as u32 & 0xFFFF) as i32
                } else {
                    value
                };
                let new_member_ref = match &sprite.member {
                    Some(member_ref) => cast_member_ref(member_ref.cast_lib, actual_member_num),
                    None => CastMemberRefHandlers::member_ref_from_slot_number(value as u32),
                };
                Ok(sprite.member.as_ref() == Some(&new_member_ref))
            }
            "castNum" => {
                let new_member_ref =
                    CastMemberRefHandlers::member_ref_from_slot_number(value.int_value()? as u32);
                Ok(sprite.member.as_ref() == Some(&new_member_ref))
            }
            "loc" => match value {
                Datum::Point(vals, _) => {
                    Ok(sprite.loc_h == vals[0] as i32 && sprite.loc_v == vals[1] as i32)
                }
                Datum::List(_, list, _) if list.len() == 2 => {
                    let x = player.get_datum(&list[0]).int_value()?;
                    let y = player.get_datum(&list[1]).int_value()?;
                    Ok(sprite.loc_h == x && sprite.loc_v == y)
                }
                Datum::Void => Ok(true),
                // Mirror the assignment path: silently ignore type-mismatched
                // values so a value-equality check on a never-applied
                // assignment doesn't error.
                _ => Ok(false),
            },
            "rect" => {
                let rect_values = match value {
                    Datum::Rect(vals, _) => Some([
                        vals[0] as i32,
                        vals[1] as i32,
                        vals[2] as i32,
                        vals[3] as i32,
                    ]),
                    Datum::List(_, items, _) if items.len() == 4 => Some([
                        player.get_datum(&items[0]).int_value()?,
                        player.get_datum(&items[1]).int_value()?,
                        player.get_datum(&items[2]).int_value()?,
                        player.get_datum(&items[3]).int_value()?,
                    ]),
                    Datum::Point(vals, _) => {
                        let x = vals[0] as i32;
                        let y = vals[1] as i32;
                        Some([x, y, x + sprite.width, y + sprite.height])
                    }
                    _ => None,
                };

                let member_ref = sprite.member.as_ref();
                let cast_member =
                    member_ref.and_then(|x| player.movie.cast_manager.find_member_by_ref(x));
                let reg_point = cast_member
                    .map(|x| match &x.member_type {
                        CastMemberType::Bitmap(bitmap) => bitmap.reg_point,
                        CastMemberType::FilmLoop(film_loop) => {
                            let w = film_loop.initial_rect.width();
                            let h = film_loop.initial_rect.height();
                            ((w / 2) as i16, (h / 2) as i16)
                        }
                        CastMemberType::Flash(flash) => flash.reg_point,
                        _ => (0, 0),
                    })
                    .unwrap_or((0, 0));

                match rect_values {
                    Some([left, top, right, bottom]) => Ok(
                        sprite.loc_h == left + reg_point.0 as i32
                            && sprite.loc_v == top + reg_point.1 as i32
                            && sprite.width == right - left
                            && sprite.height == bottom - top,
                    ),
                    None => Err(ScriptError::new(format!(
                        "rect must be a rect (got {})",
                        value.type_str()
                    ))),
                }
            }
            "scriptInstanceList" => {
                let ref_list = value.to_list()?;
                if ref_list.len() != sprite.script_instance_list.len() {
                    return Ok(false);
                }
                for (ref_id, existing_ref) in ref_list.iter().zip(sprite.script_instance_list.iter()) {
                    let datum = player.get_datum(ref_id);
                    let Datum::ScriptInstanceRef(instance_ref) = datum else {
                        return Err(ScriptError::new(
                            "Cannot set non-script to scriptInstanceList".to_string(),
                        ));
                    };
                    if instance_ref.id() != existing_ref.id() {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            "editable" => Ok(sprite.editable == value.to_bool()?),
            "quad" => {
                let list = value
                    .to_list()
                    .map_err(|_| ScriptError::new("quad must be a list".to_string()))?;
                if list.len() != 4 {
                    return Err(ScriptError::new(
                        "quad must be a list of 4 points".to_string(),
                    ));
                }
                let mut points = Vec::new();
                for point_ref in list {
                    let point_datum = player.get_datum(point_ref);
                    let (vals, _flags) = point_datum.to_point_inline()?;
                    points.push((vals[0] as i32, vals[1] as i32));
                }
                Ok(sprite.quad == Some([points[0], points[1], points[2], points[3]]))
            }
            "puppet" => Ok(sprite.puppet == value.to_bool()?),
            "moveableSprite" | "moveable" => Ok(sprite.moveable == value.to_bool()?),
            "constraint" => Ok(sprite.constraint == value.int_value()?),
            "trails" => Ok(sprite.trails == value.to_bool()?),
            _ => Ok(false),
        }
    })
}

pub fn sprite_set_prop(sprite_id: i16, prop_name: &str, value: Datum) -> Result<(), ScriptError> {
    // Director silently ignores property writes to invalid sprite refs. A
    // script doing `sprite(N).prop = X` where N came from a list-lookup
    // returning -1 (not-found sentinel) is legitimate Lingo — verified in
    // Trick or Treat Beat where the panic surfaced via a behavior script.
    // Also bounds-check the upper end against `channels.len()` to keep the
    // unsigned-cast indexing in `get_sprite_mut` safe.
    if sprite_id < 0 {
        return Ok(());
    }
    let in_range = reserve_player_ref(|player| {
        (sprite_id as usize) < player.movie.score.channels.len()
    });
    if !in_range {
        return Ok(());
    }
    if sprite_set_prop_is_noop(sprite_id, prop_name, &value)? {
        return Ok(());
    }

    reserve_player_mut(|player| { player.stage_dirty = true; });
    let result = match prop_name {
        // Flash (SWF) sprite frame setter — `mySprite.frame = N` on a Flash
        // member must navigate that sprite's embedded Ruffle player, not be
        // treated as a behaviour-property assignment. Each Flash sprite has
        // its own Ruffle instance keyed by sprite_num, so multiple sprites
        // sharing a single Flash cast member can independently pin to
        // different frames (storyscramble's 3 story tiles use cast 2:1 but
        // display poster frames 2/4/6 simultaneously).
        "frame" | "currentFrame" => {
            // Mirror the getter: a behavior that declares its own `property
            // frame`/`currentFrame` OWNS this assignment — store it there and
            // do NOT touch the Flash playhead. Only sprites with no such
            // behavior property route to Ruffle gotoAndStop (poster-frame Flash
            // tiles like storyscramble's). This stops a behavior's animation
            // counter being misrouted into the Flash bridge — bogey_nights sets
            // `sprite(16).frame = VOID` every frame, which was calling
            // gotoAndStop("VOID") (a nonexistent label) and blanking the SWF.
            let declared = borrow_sprite_mut(
                sprite_id,
                |_| {},
                |sprite, _| {
                    sprite.script_instance_list.iter().find_map(|behavior| {
                        reserve_player_mut(|player| {
                            let value_ref = player.alloc_datum(value.clone());
                            match script_set_prop(
                                player,
                                behavior,
                                &prop_name.to_string(),
                                &value_ref,
                                true, // only if the behavior already declares it
                            ) {
                                Ok(_) => Some(()),
                                Err(_) => None,
                            }
                        })
                    })
                },
            );
            if declared.is_some() {
                return Ok(());
            }
            // No behavior owns `frame`: this is a Flash playhead navigation.
            // Director ignores a VOID frame value (no valid target), so guard
            // it — otherwise value.string_value() yields "VOID", which the
            // bridge treats as a (nonexistent) label and blanks the sprite.
            if matches!(value, Datum::Void) {
                return Ok(());
            }
            let frame_or_label = value.string_value()?;
            let has_member = reserve_player_ref(|player| {
                player
                    .movie
                    .score
                    .get_sprite(sprite_id)
                    .and_then(|s| s.member.as_ref().map(|_| ()))
                    .is_some()
            });
            if has_member {
                // Record the asserted numeric frame on the SPRITE so it survives
                // a member swap and re-projects onto a freshly-created Ruffle
                // instance (StoryScramble poster tiles / bogeyman pre-swap
                // frame). Non-numeric (label) targets don't set it.
                if let Ok(n) = frame_or_label.parse::<i32>() {
                    reserve_player_mut(|player| {
                        player.movie.score.get_sprite_mut(sprite_id).flash_asserted_frame = Some(n);
                    });
                }
                ruffle_goto_frame_and_stop(sprite_id as i32, &frame_or_label);
            }
            Ok(())
        }
        "visible" | "visibility" => borrow_sprite_mut(
            sprite_id,
            |_| {},
            |sprite, _| {
                sprite.visible = value.to_bool()?;
                sprite.has_visible_mod = true;
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
                // Director sets `the stretch of sprite` TRUE on any manual
                // resize; this makes get_concrete_sprite_rect honor the new
                // size verbatim instead of snapping back to the bitmap's.
                sprite.stretch = 1;
                Ok(())
            },
        ),
        "height" => borrow_sprite_mut(
            sprite_id,
            |player| value.int_value(),
            |sprite, value| {
                sprite.height = value?;
                sprite.has_size_changed = true;
                sprite.stretch = 1;
                Ok(())
            },
        ),
        "left" => borrow_sprite_mut(
            sprite_id,
            |player| {
                let rect = get_sprite_rect_in_context(player, sprite_id);
                (rect.0, value.int_value())
            },
            |sprite, args| {
                let (current_left, new_left) = args;
                let new_left = new_left?;
                sprite.loc_h += new_left - current_left;
                Ok(())
            },
        ),
        "top" => borrow_sprite_mut(
            sprite_id,
            |player| {
                let rect = get_sprite_rect_in_context(player, sprite_id);
                (rect.1, value.int_value())
            },
            |sprite, args| {
                let (current_top, new_top) = args;
                let new_top = new_top?;
                sprite.loc_v += new_top - current_top;
                Ok(())
            },
        ),
        "right" => borrow_sprite_mut(
            sprite_id,
            |player| {
                let rect = get_sprite_rect_in_context(player, sprite_id);
                (rect.0, value.int_value())
            },
            |sprite, args| {
                let (left, new_right) = args;
                let new_right = new_right?;
                sprite.width = new_right - left;
                sprite.has_size_changed = true;
                sprite.stretch = 1;
                Ok(())
            },
        ),
        "bottom" => borrow_sprite_mut(
            sprite_id,
            |player| {
                let rect = get_sprite_rect_in_context(player, sprite_id);
                (rect.1, value.int_value())
            },
            |sprite, args| {
                let (top, new_bottom) = args;
                let new_bottom = new_bottom?;
                sprite.height = new_bottom - top;
                sprite.has_size_changed = true;
                sprite.stretch = 1;
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
                sprite.has_blend_mod = true;
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
        // Shockwave3D camera assignment
        "camera" => {
            let cam_name = match &value {
                Datum::Shockwave3dObjectRef(r) => r.name.clone(),
                Datum::String(s) => s.clone(),
                _ => "DefaultView".to_string(),
            };
            borrow_sprite_mut(
                sprite_id,
                |_player| Ok(cam_name.clone()),
                |sprite, name: Result<String, ScriptError>| {
                    sprite.w3d_camera = Some(name.unwrap_or_default());
                    Ok(())
                },
            )
        }
        // Member properties
        "member" => borrow_sprite_mut(
            sprite_id,
            |player| resolve_sprite_member_assignment(player, &value),
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
                    // FurniFactory2-style scaling guard: if the sprite's current
                    // dimensions are exactly 2× the new member's intrinsic size,
                    // a `1_resize` beginSprite handler (or equivalent) has
                    // already doubled this sprite — don't snap it back to 1×
                    // just because a hover handler swapped the member.
                    let sprite_is_doubled = intrinsic_size
                        .map(|(w, h)| w > 0 && h > 0
                            && sprite.width == w * 2 && sprite.height == h * 2)
                        .unwrap_or(false);
                    if !sprite_is_doubled {
                        if let Some((w, h)) = intrinsic_size {
                            if w > 0 && h > 0 {
                                sprite.width = w;
                                sprite.height = h;
                                sprite.base_width = w;
                                sprite.base_height = h;
                            }
                        }
                        sprite.has_size_changed = false;
                        // A `member` assignment that resets to intrinsic size
                        // also clears any prior explicit Lingo sizing.
                        sprite.explicit_lingo_size = false;
                    }
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
                let cr = cursor_ref?;
                // Track hidden cursor state for pointer lock detection.
                // Pointer lock is only activated when the game also sets _mouse.mouseLoc.
                match &cr {
                    crate::player::sprite::CursorRef::System(id) => {
                        if *id == 200 || *id == -1 {
                            reserve_player_mut(|p| { p.cursor_is_hidden = true; });
                        } else {
                            reserve_player_mut(|p| {
                                p.cursor_is_hidden = false;
                                p.wants_pointer_lock = false;
                            });
                        }
                    }
                    _ => {
                        // Member cursor (custom bitmap) = visible, not mouselook
                        reserve_player_mut(|p| {
                            p.cursor_is_hidden = false;
                            p.wants_pointer_lock = false;
                        });
                    }
                }
                sprite.cursor_ref = Some(cr);
                Ok(())
            },
        ),
        "loc" => borrow_sprite_mut(
            sprite_id,
            // flag (D6+) so the sprite-mut closure doesn't need to re-borrow.
            |player| Ok::<_, ScriptError>(value.clone()),
            |sprite, prep| -> Result<(), ScriptError> {
                let value = prep?;
                match value {
                    Datum::Point(vals, _) => {
                        sprite.loc_h = vals[0] as i32;
                        sprite.loc_v = vals[1] as i32;
                        Ok(())
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
                    // Director silently ignores type-mismatched assignments
                    // here (e.g. `sprite.loc = 116`, a known script typo for
                    // `.locV`). Mirror that — warn but keep the game running.
                    _ => {
                        log::warn!(
                            "Ignoring sprite {} loc assignment with non-Point value ({})",
                            sprite_id, value.type_str()
                        );
                        Ok(())
                    }
                }
            },
        ),
        "rect" => reserve_player_mut(|player| {
            // Extract the target rect from `value`.
            let rect_values = match value {
                Datum::Rect(ref vals, _) => {
                    Some([vals[0] as i32, vals[1] as i32, vals[2] as i32, vals[3] as i32])
                }
                Datum::List(_, ref items, _) if items.len() == 4 => {
                    Some([
                        player.get_datum(&items[0]).int_value()?,
                        player.get_datum(&items[1]).int_value()?,
                        player.get_datum(&items[2]).int_value()?,
                        player.get_datum(&items[3]).int_value()?,
                    ])
                }
                Datum::Point(ref vals, _) => {
                    let s = player.movie.score.get_sprite(sprite_id);
                    let (sw, sh) = s.map(|s| (s.width, s.height)).unwrap_or((0, 0));
                    let x = vals[0] as i32;
                    let y = vals[1] as i32;
                    Some([x, y, x + sw, y + sh])
                }
                _ => return Err(ScriptError::new(format!(
                    "rect must be a rect (got {})", value.type_str()
                ))),
            };
            let [left, top, right, bottom] = rect_values
                .ok_or_else(|| ScriptError::new("rect parse failed".to_string()))?;
            let new_width = right - left;
            let new_height = bottom - top;

            // Probe approach: clone the sprite, apply the new dimensions with
            // loc=0,0, then ask the *real* renderer logic what rect it would
            // produce. Whatever offset it picks (scaled reg point, bbox-driven
            // reg, centerRegPoint, vector-shape natural-rect scaling, …)
            // becomes the offset we need to invert. This guarantees the
            // round-trip property `sprite.rect = R → sprite.left == R.left`
            // that Director provides, regardless of which member type or
            // heuristic path the renderer takes. Fugue No.4's `cueCursor`
            // relies on this for `if sprite(6).left < 20`.
            let mut probe: Sprite = match player.movie.score.get_sprite(sprite_id) {
                Some(s) => s.clone(),
                None => return Err(ScriptError::new(format!(
                    "rect: sprite {} not found", sprite_id
                ))),
            };
            probe.loc_h = 0;
            probe.loc_v = 0;
            probe.width = new_width;
            probe.height = new_height;
            probe.has_size_changed = true;
            // Setting `the rect of sprite` is a resize: mark it stretched so
            // get_concrete_sprite_rect uses these dims verbatim (STRETCH path)
            // instead of snapping to the bitmap's natural size. Must match the
            // real sprite below or the reg-offset round-trip breaks.
            probe.stretch = 1;
            let probe_rect = get_concrete_sprite_rect(player, &probe);
            // probe_rect.left = -effective_reg_x with loc=0, so to make the
            // real getter return `left`, write loc_h = left - probe_rect.left.
            let new_loc_h = left - probe_rect.left;
            let new_loc_v = top - probe_rect.top;

            let s = player.movie.score.get_sprite_mut(sprite_id);
            s.loc_h = new_loc_h;
            s.loc_v = new_loc_v;
            s.width = new_width;
            s.height = new_height;
            s.has_size_changed = true;
            s.stretch = 1;
            Ok(())
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
                // Invalidate the cached scriptInstanceList so the next
                // getter call rebuilds it from the updated Vec.
                player.remove_script_instance_list_cache(sprite_id);
                player.refresh_stage_behavior_channel_cache_entry(sprite_id);
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
                            let (vals, _flags) = point_datum.to_point_inline()?;
                            let x = vals[0] as i32;
                            let y = vals[1] as i32;
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
        // `the type of sprite` setter. Director games activate a
        // script-created sprite in an otherwise-empty channel by assigning
        // e.g. `sprite(ch).type = #bitmap`. bogey_nights' Eustice `#sneeze`
        // spawns its boogy/spit splashes exactly this way — `newchannel`
        // finds a free channel, then `sprite(ch).type = #bitmap` before
        // setting ink/member. dirplayer renders a channel only if it has a
        // score span OR is a puppet (see get_sorted_channels), so without a
        // `type` setter the spawned sprites got a member but never became
        // renderable ("many on the score but not visible"). A non-empty type
        // activates the channel via the puppet flag; #none/0/VOID clears it.
        "type" => {
            let activate = match &value {
                Datum::Void => false,
                Datum::Int(n) => *n != 0,
                Datum::Symbol(s) => {
                    !s.eq_ignore_ascii_case("none") && !s.eq_ignore_ascii_case("empty")
                }
                _ => true,
            };
            borrow_sprite_mut(
                sprite_id,
                |_| {},
                |sprite, _| {
                    if activate {
                        // Activate the channel so a script-created sprite
                        // renders even without a score span. The caller sets
                        // .member/.ink/.loc separately (bogey_nights' Eustice).
                        sprite.puppet = true;
                    } else {
                        // Per the Director 11.5 reference, `sprite(ch).type = 0`
                        // CLEARS the channel — empty it and stop it rendering.
                        sprite.member = None;
                        sprite.puppet = false;
                    }
                    Ok(())
                },
            )
        }
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
                // First pass: try to set on a behavior that already declares/has the property
                let first_pass = sprite
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
                    });
                match first_pass {
                    Some(r) => r,
                    None => {
                        // No behavior declares this property. Director allows dynamic
                        // creation of behavior properties via assignment, so create it
                        // on the first behavior (e.g. cs `sprite(N).pCustomData = ...`
                        // on a sprite whose only behavior doesn't declare pCustomData).
                        if let Some(first_behavior) = sprite.script_instance_list.first().cloned() {
                            reserve_player_mut(|player| {
                                let value_ref = player.alloc_datum(value.clone());
                                script_set_prop(
                                    player,
                                    &first_behavior,
                                    &prop_name.to_string(),
                                    &value_ref,
                                    false,
                                )
                            })
                        } else {
                            eprintln!(
                                "Warning: Cannot set prop {} of sprite (no behaviors)",
                                prop_name
                            );
                            Ok(())
                        }
                    }
                }
            },
        ),
    };
    if result.is_ok() {
        let affects_render_order = prop_name.eq_ignore_ascii_case("visible")
            || prop_name.eq_ignore_ascii_case("visibility")
            || prop_name.eq_ignore_ascii_case("locZ")
            || prop_name.eq_ignore_ascii_case("member")
            || prop_name.eq_ignore_ascii_case("memberNum")
            || prop_name.eq_ignore_ascii_case("castNum")
            || prop_name.eq_ignore_ascii_case("puppet")
            // `type` activates/clears a channel via the puppet flag, so it
            // changes which channels render (bogey_nights' spawned splashes).
            || prop_name.eq_ignore_ascii_case("type");
        if affects_render_order {
            reserve_player_mut(|player| {
                player.movie.score.invalidate_render_channel_cache();
            });
        }
        if prop_name.eq_ignore_ascii_case("puppet") || prop_name.eq_ignore_ascii_case("type") {
            reserve_player_mut(|player| {
                player.refresh_stage_behavior_channel_cache_entry(sprite_id);
            });
        }
        if prop_name.eq_ignore_ascii_case("visible")
            || prop_name.eq_ignore_ascii_case("visibility")
            || prop_name.eq_ignore_ascii_case("puppet")
            || prop_name.eq_ignore_ascii_case("member")
            || prop_name.eq_ignore_ascii_case("memberNum")
            || prop_name.eq_ignore_ascii_case("castNum")
        {
            reserve_player_mut(|player| {
                player.invalidate_active_stage_filmloop_cache();
            });
        }
        JsApi::dispatch_channel_changed(sprite_id);
    }
    result
}

/// For matte-ink sprites, check if a point (in stage coordinates) hits an opaque pixel.
/// Returns true if the sprite is not matte-ink, or if the matte is not yet computed,
/// or if the pixel at the given point is opaque.
fn matte_pixel_hit_test(player: &DirPlayer, sprite: &Sprite, rect: &IntRect, hit_x: i32, hit_y: i32) -> bool {
    if !should_matte_hit_test(sprite.ink as u32) {
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

pub fn is_active_sprite(player: &DirPlayer, sprite: &Sprite) -> bool {
    // Per Director docs, an "active sprite" has a sprite script (behavior) OR cast member script.
    if player.sprite_has_script_instance_ids(
        sprite.number as i16,
        sprite.script_instance_list.as_slice(),
    ) {
        return true;
    }
    // Also check the cached scriptInstanceList — behaviors added via
    // sprite.scriptInstanceList.add() only update the cached Datum::List,
    // not the sprite's internal script_instance_list Vec.
    if let Some(member_ref) = sprite.member.as_ref() {
        if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
            if member.get_member_script_ref().is_some() || member.get_script_id().is_some() {
                return true;
            }
        }
    }
    false
}

/// Non-editable text/field members are transparent to mouse events.
/// Clicks pass through them to the sprite underneath (e.g. a button).
/// Only sprite behavior scripts that actually define mouse handlers
/// (mouseDown, mouseUp, mouseUpOutSide) prevent pass-through.
fn is_click_transparent_sprite(player: &DirPlayer, sprite: &Sprite) -> bool {
    if sprite.editable {
        return false;
    }
    // A sprite with `the blend of sprite = 0` is fully transparent on screen
    // and should not eat clicks. Common idiom: a black/white overlay sprite
    // that fades in/out during intros; once faded to blend=0 it stays in the
    // score but is visually gone, so clicks must reach what's underneath.
    // Only treat as transparent when there's no behavior actively trapping
    // mouse events on it — script-attached overlays may set blend=0 as a
    // hidden hit zone and need to keep their clicks.
    if sprite.blend == 0 && !sprite_has_mouse_handler(player, sprite) {
        return true;
    }
    // Translucent Shape / VectorShape overlays (blend < 50) with no mouse
    // handler are also click-transparent — Director's hit test on these
    // follows the visual: a semi-transparent highlight bar (e.g. spineworld
    // pDropList's pSelSprite = sprite(802), GUI_dropsel at blend=20) sits
    // ON TOP of an interactive sprite below and must let clicks pass through
    // so the underlying sprite gets the mouseUp. Limited to shapes because
    // bitmaps at low blend are usually still hit-test targets (e.g. ghosted
    // buttons in a disabled state).
    if sprite.blend < 50 && !sprite_has_mouse_handler(player, sprite) {
        if let Some(member_ref) = sprite.member.as_ref() {
            if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                if matches!(
                    &member.member_type,
                    CastMemberType::Shape(_) | CastMemberType::VectorShape(_)
                ) {
                    return true;
                }
            }
        }
    }
    // Check if it's a non-editable text or field member
    let is_non_editable_text_or_field = if let Some(member_ref) = sprite.member.as_ref() {
        if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
            match &member.member_type {
                CastMemberType::Field(field) => !field.editable,
                CastMemberType::Text(text) => {
                    if let Some(ref info) = text.info {
                        !info.editable
                    } else {
                        true
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    } else {
        false
    };
    if !is_non_editable_text_or_field {
        return false;
    }
    !sprite_has_mouse_handler(player, sprite)
}

fn sprite_has_mouse_handler(player: &DirPlayer, sprite: &Sprite) -> bool {
    sprite_has_handler(player, sprite, &["mouseDown", "mouseUp", "mouseUpOutSide"])
}

/// Returns true when any behaviour on `sprite` (sprite-attached, Lingo-added,
/// or cast-member-attached) defines a handler matching any of `names`. Used to
/// decide whether a sprite that would otherwise be click-transparent (e.g.
/// non-editable text/field, blend=0 overlay) should actually keep its clicks,
/// and to decide whether clicking a non-editable sprite should still steal
/// keyboard focus (because a `keyDown` behaviour needs it).
///
/// Three places a behaviour can live, all of which need checking:
///   1. `sprite.script_instance_list` — score-authored behaviours.
///   2. The Lingo-side cached `scriptInstanceList` (Datum::List held in
///      `player.script_instance_list_cache`) — populated when a script
///      does `sprite.scriptInstanceList.add(...)` at runtime, which does
///      NOT mirror back into the sprite's internal Vec.
///   3. The cast member's attached "BehaviorScript" (Director's member
///      script export) — covers movies that attach the handler at the
///      cast level rather than per-sprite.
pub fn sprite_has_handler(player: &DirPlayer, sprite: &Sprite, names: &[&str]) -> bool {
    let script_has_any = |script: &crate::player::script::Script| -> bool {
        names.iter().any(|n| script.get_own_handler(n).is_some())
    };

    // (1) Score-authored sprite behaviours.
    for instance_ref in &sprite.script_instance_list {
        if let Some(instance) = player.allocator.get_script_instance_opt(instance_ref) {
            if let Some(script) = player.movie.cast_manager.get_script_by_ref(&instance.script) {
                if script_has_any(script) {
                    return true;
                }
            }
        }
    }

    // (2) Runtime-added behaviours via `sprite.scriptInstanceList.add(...)`.
    if let Some(cached_ref) = player.script_instance_list_cache.get(&(sprite.number as i16)) {
        if let crate::director::lingo::datum::Datum::List(_, items, _) = player.get_datum(cached_ref) {
            for item_ref in items {
                if let crate::director::lingo::datum::Datum::ScriptInstanceRef(id) =
                    player.get_datum(item_ref)
                {
                    if let Some(instance) = player.allocator.get_script_instance_opt(id) {
                        if let Some(script) =
                            player.movie.cast_manager.get_script_by_ref(&instance.script)
                        {
                            if script_has_any(script) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }

    // (3) Cast member's attached script.
    if let Some(member_ref) = sprite.member.as_ref() {
        if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
            if let Some(script_ref) = member.get_member_script_ref() {
                if let Some(script) = player.movie.cast_manager.get_script_by_ref(script_ref) {
                    if script_has_any(script) {
                        return true;
                    }
                }
            }
            // Fallback: behavior scripts attached to Field/Text members live
            // in the cast's lctx, not as Script-type cast members, so
            // get_script_by_ref above can return None for them. Consult
            // get_script_id() → lctx.scripts directly. This is the path
            // used by member-attached BehaviorScripts in Director 11.5
            // (e.g. Fugue No.4's Narrative / Portuguese / Spanish fields).
            if let Some(script_id) = member.get_script_id() {
                if let Ok(cast) = player.movie.cast_manager.get_cast(member_ref.cast_lib as u32) {
                    if let Some(lctx) = cast.lctx.as_ref() {
                        if let Some(script_chunk) = lctx.scripts.get(&script_id) {
                            let matches = script_chunk.handlers.iter().any(|h| {
                                lctx.names.get(h.name_id as usize)
                                    .map(|n| names.iter().any(|target| n.eq_ignore_ascii_case(target)))
                                    .unwrap_or(false)
                            });
                            if matches {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

pub fn get_sprite_at(player: &DirPlayer, x: i32, y: i32, scripted: bool) -> Option<u32> {
    for channel in player.movie.score.get_sorted_channels(player.movie.current_frame).iter().rev() {
        if concrete_sprite_hit_test(player, &channel.sprite, x, y)
            && (!scripted || is_active_sprite(player, &channel.sprite))
            && !is_click_transparent_sprite(player, &channel.sprite)
        {
            return Some(channel.sprite.number as u32);
        }
    }

    return None;
}

/// Version of `get_concrete_sprite_rect` that applies the stage auto-scale
/// factor — used by the renderer so content fills drawRect when it exceeds
/// movie.rect. Hit-testing / Lingo APIs keep using the unscaled variant so
/// sprite state stays in movie coordinates.
///
/// Uses a uniform scale (min of horizontal/vertical factors) to preserve the
/// movie's aspect ratio — drawRect asymmetry results in letterboxing rather
/// than stretching. This lets `get_concrete_sprite_rect`'s heuristics decide
/// the "actual" sprite rect, and the scaling here just amplifies that result
/// consistently across bitmap/text/etc.
pub fn get_concrete_sprite_render_rect(player: &DirPlayer, sprite: &Sprite) -> IntRect {
    let rect = get_concrete_sprite_rect(player, sprite);
    let layout = crate::player::stage::stage_layout(player);
    let (sx, sy) = crate::player::stage::stage_scale(player);
    if (sx - 1.0).abs() < 1e-6 && (sy - 1.0).abs() < 1e-6 {
        return rect;
    }
    IntRect::from(
        (layout.draw_rect[0] + rect.left as f64 * sx).round() as i32,
        (layout.draw_rect[1] + rect.top as f64 * sy).round() as i32,
        (layout.draw_rect[0] + rect.right as f64 * sx).round() as i32,
        (layout.draw_rect[1] + rect.bottom as f64 * sy).round() as i32,
    )
}

pub fn get_concrete_sprite_rect(player: &DirPlayer, sprite: &Sprite) -> IntRect {
    let member = sprite
        .member
        .as_ref()
        .and_then(|member_ref| player.movie.cast_manager.find_member_by_ref(member_ref));
    if member.is_none() {
        // Empty channel: Director keeps `the rect of sprite` at the value it had
        // before the sprite left its span (member→0). retained_rect carries that
        // last rect so transition-frame reads of sprite(n).rect stay meaningful.
        if let Some((l, t, r, b)) = sprite.retained_rect {
            return IntRect::from(l, t, r, b);
        }
        return IntRect::from_size(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height);
    }
    let member = member.unwrap();

    // Shockwave3D directToStage members render the 3D world into the sprite's
    // viewport at the member's `defaultRect` size (Director: defaultRect
    // "controls the default size used for all new sprites"), positioned by the
    // member regPoint — NOT the raw score channel width/height. Splat's "scene"
    // sets `defaultRect = rect(0,0,620,410)` + directToStage; without this it
    // rendered at the 640×480 stage default, distorting the 3D aspect. Gated on
    // directToStage + a non-degenerate defaultRect + no explicit score stretch,
    // so non-directToStage / un-sized 3D members keep their existing behavior.
    if let CastMemberType::Shockwave3d(w3d) = &member.member_type {
        let dr = w3d.info.default_rect;
        let (dw, dh) = (dr.2 - dr.0, dr.3 - dr.1);
        if w3d.info.direct_to_stage && dw > 0 && dh > 0 && sprite.stretch == 0 {
            let reg_x = w3d.info.reg_point.0;
            let reg_y = w3d.info.reg_point.1;
            return IntRect::from_size(sprite.loc_h - reg_x, sprite.loc_v - reg_y, dw, dh);
        }
    }

    match &member.member_type {
        CastMemberType::Bitmap(bitmap_member) => {
            // Get registration point from bitmap member
            let mut reg_x = bitmap_member.reg_point.0;
            let mut reg_y = bitmap_member.reg_point.1;

            // Prefer actual pixel dimensions from bitmap_manager; info dimensions
            // reflect the score channel's bounding box and can differ from the
            // real bitmap size, which causes the heuristic to mis-classify the
            // sprite as a bbox and stretch/clip it incorrectly.
            let (bitmap_width, bitmap_height) =
                if let Some(bmp) = player.bitmap_manager.get_bitmap(bitmap_member.image_ref) {
                    (bmp.width as i32, bmp.height as i32)
                } else {
                    (bitmap_member.info.width as i32, bitmap_member.info.height as i32)
                };

            // Choose sprite vs bitmap dimensions from the Score's authoritative
            // "stretch" flag (sprite ink byte bit 0x80, plumbed to
            // sprite.stretch). This replaces the old size heuristic that tried
            // to GUESS whether the score's width/height were an intentional
            // resize or just an approximate bounding box.
            //
            // - stretched → the sprite was resized off the member's natural
            //   size (manual handle drag, Lingo `the width/height of sprite`,
            //   or a size tween) → use the sprite (score) dimensions verbatim.
            // - not stretched → the bitmap displays at its natural size; the
            //   score's width/height are an approximate bounding box and are
            //   ignored → use the bitmap's real pixel dimensions.
            //
            // Verified against Director: a 4x4 fill stretched to 352x282
            // (nomiss) and a 5x5 fill to 338x253 (SpongeBob) both carry
            // stretch=1; natural-size sprites whose score box drifts a few px
            // from the bitmap (PinBall, Junkbot) carry stretch=0.
            let stretched = sprite.stretch != 0 || sprite.has_size_tweened;
            // A ≤1px difference from the bitmap's natural size in BOTH axes is
            // NOT a real resize — it's the rounding Director's score applies to
            // a center-registered bitmap of odd dimension. The sprite rect is
            // stored as `2 * regPoint`, i.e. the bitmap size rounded DOWN to
            // even, and the stretch bit ends up set. e.g. a 760x521 bitmap with
            // reg (380,260) gets a 760x520 score box with stretch=1; honoring
            // it verbatim drops the last row. A genuine resize differs by far
            // more than a pixel, so snapping to the decoded bitmap size here is
            // safe and only ever corrects this off-by-one.
            let near_natural = bitmap_width > 0 && bitmap_height > 0
                && (sprite.width - bitmap_width).abs() <= 1
                && (sprite.height - bitmap_height).abs() <= 1;
            let (sprite_width, sprite_height, _size_path) =
                if stretched && !near_natural && sprite.width > 0 && sprite.height > 0 {
                    (sprite.width, sprite.height, "STRETCH")
                } else if bitmap_width > 0 && bitmap_height > 0 {
                    (bitmap_width, bitmap_height, "NATURAL")
                } else if sprite.width > 0 && sprite.height > 0 {
                    // Bitmap not loaded yet — fall back to the score box.
                    (sprite.width, sprite.height, "NATURAL_FALLBACK")
                } else {
                    (bitmap_width, bitmap_height, "ZERO")
                };
            debug!("[BITMAP_RECT] sprite#{} member={:?} path={} result={}x{} loc=({},{}) reg=({},{}) sprite={}x{} bitmap(decoded)={}x{} info={}x{} stretch={} flags(tweened={} owned={} changed={})",
                sprite.number, sprite.member, _size_path, sprite_width, sprite_height,
                sprite.loc_h, sprite.loc_v,
                reg_x, reg_y,
                sprite.width, sprite.height, bitmap_width, bitmap_height,
                bitmap_member.info.width, bitmap_member.info.height, sprite.stretch,
                sprite.has_size_tweened, sprite.bitmap_size_owned_by_sprite, sprite.has_size_changed);

            // If centerRegPoint is enabled, set reg point to bitmap center.
            // Use bitmap coordinates here so the subsequent scaling step
            // correctly converts to sprite coordinates (sprite_width/2).
            if bitmap_member.info.center_reg_point && bitmap_width > 0 && bitmap_height > 0 {
                reg_x = (bitmap_width / 2) as i16;
                reg_y = (bitmap_height / 2) as i16;
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
            // #adjust boxType: displayed width = `field.width` + chrome
            // (2*border + 2*margin + boxDropShadow). `field.width` is
            // the authored TEXT-AREA width — Director's sprite display
            // rect adds the chrome padding so the box visually frames
            // the text area. E.g. MarioNetQuest's "How to Play":
            // field.width=324, chrome=24 → 348 displayed (matches
            // Director's `the rect of sprite` width). For runtime-
            // authored CS fields where field.width is the script's
            // .window XML setting, the same `field.width + chrome`
            // works because the chrome is small/zero.
            //
            // Why not use sprite.width directly: many .dir-loaded
            // fields with little text content have a `sprite.width`
            // that's some default oversized score authoring — using
            // it would render a much-too-wide box. Building width
            // from the authored field.width + the field's own chrome
            // settings gives a consistent, Director-faithful result.
            //
            // Other boxTypes (#scroll, #fixed, #limit) honor
            // sprite.width unconditionally — those are user-resizable
            // containers independent of the member's authored width.
            let is_adjust_for_width = field_member.box_type == "adjust";
            let member_authored_w = field_member.width as i32;
            let chrome_w = (2 * field_member.border as i32)
                + (2 * field_member.margin as i32)
                + (field_member.box_drop_shadow as i32);
            let field_width = if is_adjust_for_width && member_authored_w > 0 {
                member_authored_w + chrome_w
            } else {
                sprite.width
            };
            let rect_height = (field_member.rect_bottom - field_member.rect_top).max(0) as i32;
            let extras = (2 * field_member.border as i32)
                + (2 * field_member.margin as i32)
                + (4 * field_member.box_drop_shadow as i32);
            let is_adjust = field_member.box_type == "adjust";

            // Measure the actual rendered text height using the same logic as
            // the text member path: bitmap font measurement when available,
            // Canvas2D native measurement otherwise. Wrap at the inner content
            // width (field_width minus border/margin/shadow) so multi-line
            // wrapped text reports the real height.
            let measured_height: Option<i32> = if field_width > 0 && !field_member.text.is_empty() {
                use crate::player::font::{measure_text, measure_text_wrapped, FontManager};
                use crate::player::handlers::datum_handlers::cast_member::font::FontMemberHandlers;

                let cache_key = FontManager::cache_key(&field_member.font);
                let bitmap_font = player.font_manager.font_cache.get(&cache_key).cloned();
                let inner_width = (field_width - extras).max(1) as u16;

                let from_bitmap = bitmap_font.as_ref().map(|f| {
                    if field_member.word_wrap {
                        measure_text_wrapped(
                            &field_member.text,
                            f,
                            inner_width,
                            true,
                            field_member.fixed_line_space,
                            field_member.top_spacing,
                            0,
                            0,
                        ).1 as i32
                    } else {
                        measure_text(
                            &field_member.text,
                            f,
                            None,
                            field_member.fixed_line_space,
                            field_member.top_spacing,
                            0,
                        ).1 as i32
                    }
                }).filter(|h| *h > 0);

                from_bitmap.or_else(|| {
                    let font_name = if !field_member.font.is_empty() {
                        field_member.font.as_str()
                    } else {
                        "Arial"
                    };
                    let font_size = if field_member.font_size > 0 { field_member.font_size } else { 12 };
                    // Build the bold/italic/underline bitflag the same way the
                    // renderer does (see webgl2/mod.rs Field branch) so the
                    // Canvas2D measurement uses the same glyph widths.
                    let style_lc = field_member.font_style.to_lowercase();
                    let mut style: u8 = 0;
                    if style_lc.contains("bold") { style |= 1; }
                    if style_lc.contains("italic") { style |= 2; }
                    if style_lc.contains("underline") { style |= 4; }
                    let (_, h) = FontMemberHandlers::measure_text_native_styled(
                        &field_member.text,
                        font_name,
                        font_size,
                        if style == 0 { None } else { Some(style) },
                        field_member.word_wrap,
                        if field_member.word_wrap { inner_width as i32 } else { 0 },
                        field_member.top_spacing,
                        0,
                        field_member.fixed_line_space,
                    );
                    if h > 0 { Some(h as i32) } else { None }
                })
            } else { None };

            let measured_plus_extras = measured_height.map(|h| h + extras);

            // Box type comes from STxT (lowercase, no `#`) or from Lingo
            // (`member.boxType = #fixed` may serialize with the `#` prefix
            // and arbitrary case). Normalize before comparing.
            let box_type_norm = field_member
                .box_type
                .trim()
                .trim_start_matches('#')
                .to_ascii_lowercase();
            let _ = box_type_norm; // (used only for legacy debug logs; safe to drop later)
            let (field_height, height_arm) = if is_adjust && measured_plus_extras.is_some() {
                // #adjust is the ONLY box type that auto-grows. #fixed clips
                // overflow, #scroll adds a scrollbar (also clips visually),
                // #limit rejects text that doesn't fit. All three keep the
                // authored sprite_rect height regardless of how tall the
                // measured text is — typing past the bottom is supposed to
                // disappear or be refused, not stretch the field.
                let m = measured_plus_extras.unwrap();
                (m.max(sprite.height.max(1)), "adjust+measured")
            } else if field_member.word_wrap && is_adjust && field_member.text_height > 0 {
                ((field_member.text_height as i32 + extras).max(sprite.height), "wrap+adjust+text_height")
            } else if field_member.word_wrap
                && measured_plus_extras.is_some()
                && sprite.height > 0
                && {
                    // #scroll/#fixed/#limit fields normally clip to the authored
                    // height (handled by the `sprite.height>0` arm below), but a
                    // short dialogue that overflows by only a few lines would then
                    // lose text. This happens in Summer Resort: "sign.text" is an
                    // authored 16px (one-line) #scroll field, but its messages wrap
                    // to two lines because we substitute a wider font for the
                    // authored "Osaka" — the second line ("…resort!") was clipped.
                    // Grow to the measured content height, but only when the
                    // overflow is small so genuinely large scrolling documents
                    // (help panels wrapping to thousands of px) still clip + scroll
                    // as authored. MAX_GROW caps the *extra* height (~10 lines).
                    const MAX_GROW: i32 = 160;
                    let m = measured_plus_extras.unwrap();
                    m > sprite.height && (m - sprite.height) <= MAX_GROW
                }
            {
                (measured_plus_extras.unwrap(), "wrap+scroll+grow-dialogue")
            } else if sprite.height > 0 {
                (sprite.height, "sprite.height>0")
            } else if field_member.text_height > 0 {
                (field_member.text_height as i32 + extras, "text_height>0")
            } else if rect_height > 0 && !is_adjust {
                (rect_height + extras, "rect_height>0+non-adjust")
            } else {
                (sprite.height, "fallback-sprite.height")
            };
            let field_height = field_height.max(1);

            debug!(
                "Field sprite_rect #{}: field_width={} sprite.width={} sprite.height={} sprite.loc_h={} sprite.loc_v={} rect_top={} rect_bottom={} rect_height={} border={} margin={} box_drop_shadow={} extras={} box_type={} word_wrap={} is_adjust={} text_height={} font={} font_size={} font_style={} measured_height={:?} field_height={} height_arm={}",
                sprite.number,
                field_width,
                sprite.width,
                sprite.height,
                sprite.loc_h,
                sprite.loc_v,
                field_member.rect_top,
                field_member.rect_bottom,
                rect_height,
                field_member.border,
                field_member.margin,
                field_member.box_drop_shadow,
                extras,
                field_member.box_type,
                field_member.word_wrap,
                is_adjust,
                field_member.text_height,
                field_member.font,
                field_member.font_size,
                field_member.font_style,
                measured_height,
                field_height,
                height_arm,
            );

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

            // Measure the actual text dimensions when the font is available.
            // For #adjust box type, Director expands/shrinks to fit, so use
            // measured directly. For other box types, use measured as a floor
            // when it exceeds the stored size — prevents clipping when the
            // stored height is stale or too small for the current text.
            // Detect system font (no bitmap font in cache for this name).
            // System fonts render via Canvas2D which has slightly larger
            // glyph metrics — add 4px padding to height to avoid clipping.
            let is_system_font = {
                use crate::player::font::FontManager;
                let cache_key = FontManager::cache_key(&text_member.font);
                !player.font_manager.font_cache.contains_key(&cache_key)
            };

            let measured_height = if text_width > 0 {
                use crate::player::font::{measure_text, measure_text_wrapped, FontManager};
                let cache_key = FontManager::cache_key(&text_member.font);
                // Only use bitmap font if it actually exists for this font name.
                // Don't fall back to system_font here — its char_height won't
                // match text_member.font_size and gives a wrong measurement.
                // For system fonts (Arial, etc.), use a font-size-based estimate
                // since native text uses Canvas2D which we can't measure here.
                let font = player.font_manager.font_cache.get(&cache_key).cloned();
                // Force wrap-aware measurement for puppet text members
                // even when `word_wrap` is false on the member. Director
                // grows puppet sprites to fit all visible content; if
                // a long single-line text exceeds the box width and we
                // measure unwrapped, the height computes as 1 line and
                // the rendered text overflows (the renderer still
                // wraps, but the sprite_rect is too short to show the
                // wrapped tail). Score-authored sprites continue to
                // honor the member's word_wrap flag.
                let force_wrap = sprite.puppet;
                let from_bitmap = font.map(|f| {
                    if text_member.word_wrap || force_wrap {
                        measure_text_wrapped(
                            &text_member.text, &f, text_width as u16, true,
                            text_member.fixed_line_space,
                            text_member.top_spacing,
                            text_member.bottom_spacing,
                            text_member.char_spacing,
                        ).1 as i32
                    } else {
                        measure_text(
                            &text_member.text, &f, None,
                            text_member.fixed_line_space,
                            text_member.top_spacing,
                            text_member.bottom_spacing,
                        ).1 as i32
                    }
                }).filter(|h| *h > 0);

                from_bitmap.or_else(|| {
                    // System-font estimate: prefer fixed_line_space (only set
                    // when XMED has explicit line_spacing — page_height is the
                    // member box height, NOT line spacing). Fall back to a
                    // Director-style line height: max(font_size + 3, ceil(font_size * 1.15)).
                    // Verified: font_size=13 → 16, font_size=24 → 28.
                    let per_line = if text_member.fixed_line_space > 0 {
                        text_member.fixed_line_space as i32
                    } else if text_member.font_size > 0 {
                        let fs = text_member.font_size as i32;
                        let scaled = (fs * 23 + 19) / 20;  // ceil(fs * 1.15)
                        let min_pad = fs + 3;
                        scaled.max(min_pad)
                    } else {
                        return None;
                    };
                    let line_count = text_member.text.matches(|c| c == '\n' || c == '\r').count() as i32 + 1;
                    Some(per_line * line_count
                        + text_member.top_spacing as i32
                        + text_member.bottom_spacing as i32)
                })
            } else { None };

            let stored_height = if info_height > sprite.height {
                info_height
            } else {
                sprite.height
            };

            // Director's `#fixed` (and `#scroll` / `#limit`) box types keep
            // the sprite locked to its authored / Lingo-set height — content
            // that overflows is clipped, not grown into. Only `#adjust`
            // grows the box to fit measured content — and even then, only
            // when the text relies on RUN-TIME WRAP (single paragraph, no
            // \n). When the text has explicit \n breaks the stored height
            // already includes them (set in cast_member.rs from Paige's
            // doc_bottom = page_height) and re-measuring with our PFR
            // metrics overshoots Director (CS Junkbot credits: stored 375,
            // re-measured 414, sprite_rect should be 375 to match Director).
            //
            // The previous unconditional `max(measured, stored)` made
            // fixed-size displays balloon (CS clock display 52×18 → 52×48),
            // and the always-grow #adjust path made the credits text
            // sprite extend below member bounds.
            // For `#adjust` text, Director uses `info_height` as the
            // sprite height when the stored value already accounts for
            // the laid-out content. Two cases:
            //   1. info_height is reasonably close to the measured
            //      content height (within 30%) — the stored value IS
            //      the laid-out total. Trust it; the score's sprite.height
            //      and our PFR rasterization both overshoot by a few px
            //      otherwise (Junkbot READY: member 30, sprite 33).
            //   2. info_height is much smaller than measured — it's
            //      single-line authored, content has been added via
            //      runtime text/html assignment that didn't update the
            //      stored info. Grow to measured (CS recycler help member
            //      82: info_height=36, measured=108).
            // The 70% threshold is empirical: tested across Junkbot
            // (READY 30/33, credits 72/72, level title 375/414) and
            // CS members (recycler 36/108).
            // Distinguishes the two #adjust scenarios:
            //   A. Authored multi-paragraph layout — info_height is the full
            //      laid-out total. Director clips overflow rather than growing
            //      the sprite rect (Junkbot help member 124: info_h=310,
            //      measured at 120 px wrap = 540 because the help text spans
            //      30+ wrapped lines). Detected by explicit \r/\n breaks in
            //      the text — multi-paragraph authored content always carries
            //      them, and Lingo writes that produce multi-paragraph text
            //      keep them too.
            //   B. Runtime-grown wrap content — info_height is the single-
            //      line authored value, the text is one paragraph that wraps
            //      at the box width (CS recycler help member 82: info_h=36,
            //      measured=108). No \r/\n in text → fall through to the
            //      70% / measured.max(info_height) rule.
            //
            // par_runs.len() > 1 is NOT a safe discriminator here — single-
            // paragraph members can still carry start+end par_run markers
            // (recycler member 82 has 2 par_runs but no breaks), which would
            // cause that path to incorrectly trust the single-line info_h.
            let text_has_breaks = text_member.text.contains('\n')
                || text_member.text.contains('\r');
            // Prefer the cast_member-computed `text_member.height` over
            // raw `text_info.height`. They diverge when info_h (TextInfo
            // offset 48) ≠ page_h (Paige doc_bottom): cast_member.rs
            // resolves to page_h for #adjust + breaks, which Director
            // also reports as `member.height`. Junkbot V2 level-name
            // member 5: info_h=331 (from header), page_h=361, Director
            // and our `member.rect` both report 361 — but our score.rs
            // path was returning 331 (info_h), making sprite.rect 30 px
            // shorter than the bitmap and clipping all but 1-2 names.
            let stored_member_h = text_member.height as i32;
            let preferred_authored = if stored_member_h > 0 {
                stored_member_h
            } else {
                info_height
            };
            // Puppet-sprite text members get the simple "fit measured
            // content" rule regardless of boxType: the authored
            // info_height/text_member.height were typically set when the
            // member was first created (often 100×100 default from
            // `new(#text)`), but the script then assigns runtime text via
            // `member.text = ...` or HTML and expects the sprite to grow
            // to display all of it. Without this, multi-line text gets
            // clipped at the stale authored height (e.g. tutorial dialog
            // "Mission 1: Tutorial" + body text — the body's second/third
            // lines were cut off even though boxType is #fixed, because
            // the puppet's authored height was the default-small).
            // Director's standard "boxType #fixed clips, #adjust grows"
            // semantics apply to score-authored sprites where the
            // authored height represents the scene designer's intent;
            // puppet sprites have no such intent baked in.
            let text_height = if sprite.puppet && measured_height.unwrap_or(0) > 0 {
                let measured_h = measured_height.unwrap();
                measured_h.max(preferred_authored)
            } else if text_member.box_type == "adjust" && preferred_authored > 0 {
                // Non-puppet #adjust text members: trust the authored
                // member.height when text has explicit breaks (the value
                // is the laid-out total Director rendered into); for
                // wrap-only single-paragraph text, fall back to the 70%
                // rule that grows to measured when the stored value is
                // much smaller than the content needs.
                //
                // Trying to grow has_breaks members to our measured size
                // overshoots Director — our PFR metrics for "Arial *"
                // and friends produce slightly taller wrapped lines than
                // Director's, so a Buggy info-card with member.height=110
                // would expand to ~150 and visibly cover adjacent UI like
                // the CLOSE button below it. Director apparently fits the
                // same text into the authored 110 px and our render
                // simply clips at the box boundary — visually a few
                // pixels of the last line may be cut, but layout
                // adjacency is preserved (Junkbot credits: stored 375
                // matches Director, our re-measure 414 would inflate).
                if text_has_breaks {
                    preferred_authored
                } else {
                    let measured_h = measured_height.unwrap_or(preferred_authored);
                    if (preferred_authored as u32) * 10 >= (measured_h as u32) * 7 {
                        preferred_authored
                    } else {
                        measured_h.max(preferred_authored)
                    }
                }
            } else if text_member.box_type == "adjust" {
                match measured_height {
                    Some(m) if m > stored_height => m,
                    _ => stored_height,
                }
            } else if stored_member_h > 0
                && (sprite.skew.abs() > 0.001 || sprite.rotation.abs() > 0.1)
            {
                // Rotated/skewed #fixed text: use member.rect.height as the
                // quad height. `sprite.height` (and therefore stored_height)
                // is the rotated bounding-box height, not the authored quad
                // — FurniFactory displayComputer member 7 has
                // member.rect.height=36 but sprite.height=48 because the
                // sprite is skewed. Pairing this with the upload_height =
                // render_height fix in webgl2/mod.rs gives 1:1 texture-to-
                // quad mapping at the authored member size.
                //
                // Restricted to transformed sprites: for flat #fixed text
                // members, sprite.height IS the authored quad and using
                // member.rect.height (which can be the FULL Paige page_h
                // — much taller than the visible score-placed sprite area)
                // would inflate the sprite_rect and let text overflow into
                // adjacent UI (worldbuilder unit-info card #291 was
                // half-covering the button below).
                stored_member_h
            } else {
                stored_height
            };

            // System fonts render via Canvas2D with slightly larger glyph
            // metrics than the stored member box accounts for. Add 4px to
            // the height to avoid descender / antialias clipping.
            let text_height = if is_system_font {
                text_height + 4
            } else {
                text_height
            };

            // Calculate draw position based on registration point from TextInfo.
            // For centerRegPoint, use the current (measured) text_height so the
            // sprite rect is centered around sprite.loc after word wrap changes.
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
                "[TEXT_RECT] sprite#{} text='{}' info={}x{} member={}x{} sprite={}x{} is_sys_font={} -> {}x{}",
                sprite.number,
                &text_member.text[..text_member.text.len().min(30)],
                info_width, info_height,
                text_member.width, text_member.height,
                sprite.width, sprite.height,
                is_system_font,
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
        CastMemberType::VectorShape(vs) => {
            // Director positions a vector shape on stage so that the
            // member's `regPoint` (in member-pixel coords) aligns with
            // the sprite's `loc`. Because vector shapes scale to sprite
            // dimensions (default scaleMode = #autoSize), the regPoint
            // offset is scaled by sprite.width/member.width before being
            // subtracted from loc:
            //
            //   sprite_left = loc_h - regPoint.x * (sprite.width  / member.width)
            //   sprite_top  = loc_v - regPoint.y * (sprite.height / member.height)
            //
            // Verified against figure8 Slider Groove:
            //   regPoint=(1,2), member.width=113, sprite.width=200,
            //   loc=(91,378)  →  91 - 1*(200/113) ≈ 89  (matches Director).
            //
            // Falls back to centering when regPoint or member dims are
            // missing (e.g. Lingo `new(#vectorShape)` synthesized members).
            let mw = vs.member_width as i32;
            let mh = vs.member_height as i32;
            let (reg_x, reg_y) = if mw > 0 && mh > 0 {
                let sx = sprite.width as f32 / mw as f32;
                let sy = sprite.height as f32 / mh as f32;
                (
                    (vs.reg_point.0 as f32 * sx).round() as i32,
                    (vs.reg_point.1 as f32 * sy).round() as i32,
                )
            } else {
                (sprite.width / 2, sprite.height / 2)
            };
            IntRect::from(
                sprite.loc_h - reg_x,
                sprite.loc_v - reg_y,
                sprite.loc_h - reg_x + sprite.width,
                sprite.loc_v - reg_y + sprite.height,
            )
        }
        CastMemberType::Flash(flash_member) => {
            let (l, t, r, b) = flash_member.effective_rect();
            let flash_width = if r != l { (r - l) as i32 } else { sprite.width };
            let flash_height = if b != t { (b - t) as i32 } else { sprite.height };
            let _ = (l, t);

            let mut reg_x = flash_member.reg_point.0 as i32;
            let mut reg_y = flash_member.reg_point.1 as i32;

            // Director anchors a Flash sprite at the member's STORED regPoint
            // even when centerRegPoint is set — it keeps that stored point synced
            // to the member's center, and `member.regPoint` returns it verbatim
            // (bogey_nights' arm: point(109, 63), NOT the geometric center
            // (69, 43)). So only synthesize a center when there is no stored
            // reg point; otherwise honor it, matching Director's placement.
            if reg_x == 0 && reg_y == 0
                && flash_member.flash_info.as_ref().map_or(true, |i| i.center_reg_point)
                && flash_width > 0 && flash_height > 0
            {
                reg_x = flash_width / 2;
                reg_y = flash_height / 2;
            }

            // Scale registration point proportionally when sprite is stretched
            let mut scaled_reg_x = if flash_width > 0 {
                ((reg_x * sprite.width) as f32 / flash_width as f32).round() as i32
            } else {
                reg_x
            };
            let mut scaled_reg_y = if flash_height > 0 {
                ((reg_y * sprite.height) as f32 / flash_height as f32).round() as i32
            } else {
                reg_y
            };

            // When the sprite is flipped, Director mirrors the content around the
            // registration point, moving the bounding box to the opposite side of
            // loc. The renderers flip only the texture WITHIN the rect (webgl2 tex
            // coords / CPU copy), so mirror the reg point here to place the rect on
            // the correct side. Without this, bogey_nights' straw (always flipH)
            // landed on the wrong side of the spit splash even though longarm
            // (usually flipH:0) looked right.
            if sprite.flip_h {
                scaled_reg_x = sprite.width - scaled_reg_x;
            }
            if sprite.flip_v {
                scaled_reg_y = sprite.height - scaled_reg_y;
            }

            IntRect::from(
                sprite.loc_h - scaled_reg_x,
                sprite.loc_v - scaled_reg_y,
                sprite.loc_h - scaled_reg_x + sprite.width,
                sprite.loc_v - scaled_reg_y + sprite.height,
            )
        }
        CastMemberType::Shockwave3d(w3d) => {
            let default_rect = w3d.info.default_rect;
            let member_width = (default_rect.2 - default_rect.0).max(1) as i32;
            let member_height = (default_rect.3 - default_rect.1).max(1) as i32;

            let reg_x = w3d.info.reg_point.0 as i32;
            let reg_y = w3d.info.reg_point.1 as i32;

            calc_shockwave3d_rect(
                player,
                sprite,
                member_width,
                member_height,
                reg_x,
                reg_y,
            )
        }
        _ => IntRect::from_size(sprite.loc_h, sprite.loc_v, sprite.width, sprite.height),
    }
}

fn rect_intersection_area(a: &IntRect, b: &IntRect) -> i32 {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = a.right.min(b.right);
    let bottom = a.bottom.min(b.bottom);

    let w = (right - left).max(0);
    let h = (bottom - top).max(0);
    w * h
}

fn calc_shockwave3d_rect(
    player: &DirPlayer,
    sprite: &Sprite,
    member_width: i32,
    member_height: i32,
    reg_x: i32,
    reg_y: i32,
) -> IntRect {
    let sprite_width = sprite.width.max(1);
    let sprite_height = sprite.height.max(1);

    let scaled_reg_x = if member_width > 0 {
        reg_x as f32 * sprite_width as f32 / member_width as f32
    } else {
        reg_x as f32
    };

    let scaled_reg_y = if member_height > 0 {
        reg_y as f32 * sprite_height as f32 / member_height as f32
    } else {
        reg_y as f32
    };

    // Candidate 1: loc already in stage coordinates
    let left1 = (sprite.loc_h as f32 - scaled_reg_x).floor() as i32;
    let top1 = (sprite.loc_v as f32 - scaled_reg_y).floor() as i32;
    let rect1 = IntRect::from(left1, top1, left1 + sprite_width, top1 + sprite_height);

    // Candidate 2: loc is center-origin
    let stage_center_x = player.movie.rect.width() / 2;
    let stage_center_y = player.movie.rect.height() / 2;

    let loc2_h = sprite.loc_h + stage_center_x;
    let loc2_v = sprite.loc_v + stage_center_y;

    let left2 = (loc2_h as f32 - scaled_reg_x).floor() as i32;
    let top2 = (loc2_v as f32 - scaled_reg_y).floor() as i32;
    let rect2 = IntRect::from(left2, top2, left2 + sprite_width, top2 + sprite_height);

    let movie_rect = IntRect::from(
        0,
        0,
        player.movie.rect.width(),
        player.movie.rect.height(),
    );

    let overlap1 = rect_intersection_area(&rect1, &movie_rect);
    let overlap2 = rect_intersection_area(&rect2, &movie_rect);

    if overlap2 > overlap1 {
        rect2
    } else {
        rect1
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
