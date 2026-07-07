use std::collections::{HashMap, VecDeque};

use chrono::Local;

use crate::{
    director::{
        file::DirectorFile,
        lingo::datum::{datum_bool, Datum},
    },
    utils::PATH_SEPARATOR, reserve_player_ref, reserve_player_mut,
    player::ColorRef, player::ScriptInstanceRef, CastMemberRef,
};

use super::{
    allocator::DatumAllocator, bitmap::manager::BitmapManager, cast_manager::CastManager,
    geometry::IntRect, net_manager::NetManager, score::Score, ScriptError, ScriptReceiver,
};

pub struct Movie {
    pub rect: IntRect,
    pub cast_manager: CastManager,
    pub score: Score,
    pub current_frame: u32,
    pub puppet_tempo: u32,
    pub random_seed: Option<i32>,
    pub exit_lock: bool,
    pub dir_version: u16,
    pub item_delimiter: char,
    pub alert_hook: Option<ScriptReceiver>,
    pub base_path: String,
    pub file_name: String,
    pub stage_color: (u8, u8, u8),
    pub stage_color_ref: ColorRef,
    pub frame_rate: u16,
    pub file: Option<DirectorFile>,
    pub update_lock: bool,
    pub mouse_down_script: Option<ScriptReceiver>,
    pub mouse_up_script: Option<ScriptReceiver>,
    pub key_down_script: Option<ScriptReceiver>,
    pub key_up_script: Option<ScriptReceiver>,
    pub timeout_script: Option<ScriptReceiver>,
    pub allow_custom_caching: bool,
    pub trace_script: bool,
    pub trace_log_file: String,
    pub debug_playback_enabled: bool,
    /// `the editShortCutsEnabled` — when FALSE, the player disables built-in
    /// cut/copy/paste keyboard shortcuts. Web player has no native edit menu,
    /// so this is stored for round-trip read/write but has no side effect.
    /// Default TRUE for movies authored in Director 8+ (Scripting Dict p.890).
    pub edit_shortcuts_enabled: bool,
    pub mouse_down: bool,
    /// Tracks the right mouse button independently of `mouse_down` — set by
    /// `right_mouse_down(x, y)` / `right_mouse_up(x, y)` from the JS host.
    /// Read by `the rightMouseDown` / `the rightMouseUp`.
    pub right_mouse_down: bool,
    pub click_loc: (i32, i32),
    pub frame_script_instance: Option<ScriptInstanceRef>,
    pub frame_script_member: Option<CastMemberRef>,
    /// Start frame of the channel-0 (frame-script) SPAN the cached instance was
    /// created for. The same behavior member can appear in several consecutive
    /// spans with DIFFERENT parameters (e.g. the Game Loop dropped on the intro
    /// frames as `#Nonlooping` and on the gameplay frames as `#CostumeChange`);
    /// the instance must be recreated (and its params re-applied) when the span
    /// changes, not only when the member changes — otherwise the gameplay span
    /// inherits the intro span's stale `pType`.
    pub frame_script_span_start: Option<u32>,
    pub sound_device: String,
}

impl Movie {
    /// Construct an empty movie with default state (no cast/score loaded).
    /// Used for the player's primary movie and for nested linked-movie
    /// (`#movie`) playback, which builds a second Movie from the linked .dcr
    /// and drives it through the same engine.
    pub fn empty() -> Self {
        Movie {
            rect: IntRect::from(0, 0, 0, 0),
            cast_manager: CastManager::empty(),
            score: Score::empty(),
            current_frame: 1,
            puppet_tempo: 0,
            random_seed: None,
            exit_lock: false,
            dir_version: 0,
            item_delimiter: ',',
            alert_hook: None,
            base_path: "".to_string(),
            file_name: "".to_string(),
            stage_color: (255, 255, 255),
            stage_color_ref: ColorRef::PaletteIndex(255),
            frame_rate: 30,
            file: None,
            update_lock: false,
            mouse_down_script: None,
            mouse_up_script: None,
            key_down_script: None,
            key_up_script: None,
            timeout_script: None,
            allow_custom_caching: false,
            trace_script: false,
            trace_log_file: String::new(),
            debug_playback_enabled: false,
            edit_shortcuts_enabled: true,
            mouse_down: false,
            right_mouse_down: false,
            click_loc: (0, 0),
            frame_script_instance: None,
            frame_script_member: None,
            frame_script_span_start: None,
            sound_device: String::new(),
        }
    }

    pub async fn load_from_file(
        &mut self,
        file: DirectorFile,
        net_manager: &mut NetManager,
        bitmap_manager: &mut BitmapManager,
        dir_cache: &mut HashMap<Box<str>, DirectorFile>,
    ) {
        self.dir_version = file.version;
        // Determine stage color based on Director version and color mode
        let stage_color_ref = if self.dir_version < 700 {
            // Director 6 and below: always palette index
            ColorRef::PaletteIndex(file.config.pre_d7_stage_color as u8)
        } else {
            // Director 7+: check is_rgb flag
            if file.config.d7_stage_color_is_rgb != 0 {
                // RGB mode
                ColorRef::Rgb(
                    file.config.d7_stage_color_r,
                    file.config.d7_stage_color_g,
                    file.config.d7_stage_color_b,
                )
            } else {
                // Palette index mode (index stored in d7_stage_color_r)
                ColorRef::PaletteIndex(file.config.d7_stage_color_r)
            }
        };
        
        // Store as RGB tuple for backward compatibility (if needed elsewhere)
        self.stage_color = (
            file.config.d7_stage_color_r,
            file.config.d7_stage_color_g,
            file.config.d7_stage_color_b,
        );

        self.base_path = file.base_path.to_string();
        self.rect = IntRect {
            left: file.config.movie_left as i32,
            top: file.config.movie_top as i32,
            right: file.config.movie_right as i32,
            bottom: file.config.movie_bottom as i32,
        };
        self.cast_manager
            .load_from_dir(&file, net_manager, bitmap_manager, dir_cache)
            .await;
        self.score.load_from_dir(&file);
        self.file_name = file.file_name.to_string();
        self.frame_rate = file.config.frame_rate;
        self.file = Some(file);

        // Store the resolved color reference
        self.stage_color_ref = stage_color_ref;
    }

    pub fn get_prop(&self, prop: &str) -> Result<Datum, ScriptError> {
        match_ci!(prop, {
            "alertHook" => match self.alert_hook.to_owned() {
                Some(ScriptReceiver::Script(script_ref)) => Ok(Datum::ScriptRef(script_ref)),
                Some(ScriptReceiver::ScriptInstance(script_instance_id)) => {
                    Ok(Datum::ScriptInstanceRef(script_instance_id))
                }
                Some(ScriptReceiver::ScriptText(text)) => Ok(Datum::String(text)),
                None => Ok(Datum::Int(0)),
            },
            "exitlock" => Ok(datum_bool(self.exit_lock)),
            "itemdelimiter" => Ok(Datum::String(self.item_delimiter.into())),
            "runmode" => Ok(Datum::String("Plugin".to_string())), // Plugin / Author
            "date" => {
                // TODO localize formatting
                let time = Local::now();
                let formatted = time.format("%m/%d/%Y").to_string();
                Ok(Datum::String(formatted))
            },
            "long time" => {
                let time = Local::now();
                let formatted = time.format("%H:%M:%S %p").to_string();
                Ok(Datum::String(formatted))
            },
            "lastChannel" => Ok(Datum::Int(self.score.get_channel_count() as i32)),
            "moviePath" => {
                let mut result = self.base_path.clone();
                if !result.is_empty() && !result.ends_with(PATH_SEPARATOR) {
                    result.push_str(PATH_SEPARATOR);
                }
                Ok(Datum::String(result))
            },
            "platform" => Ok(Datum::String("Windows,32".to_string())),
            "frame" => Ok(Datum::Int(self.current_frame as i32)),
            "productversion" => Ok(Datum::String("11.0".to_string())),
            "moviename" | "movie" => Ok(Datum::String(self.file_name.to_owned())),
            "updatelock" => Ok(Datum::Int(if self.update_lock { 1 } else { 0 })),
            "path" => Ok(Datum::String(self.base_path.to_owned())),
            "mouseDownScript" | "mouseUpScript" | "keyDownScript" | "keyUpScript" | "timeoutScript" => {
                let script = if prop.eq_ignore_ascii_case("mouseDownScript") {
                    &self.mouse_down_script
                } else if prop.eq_ignore_ascii_case("mouseUpScript") {
                    &self.mouse_up_script
                } else if prop.eq_ignore_ascii_case("keyDownScript") {
                    &self.key_down_script
                } else if prop.eq_ignore_ascii_case("keyUpScript") {
                    &self.key_up_script
                } else {
                    &self.timeout_script
                };
                match script.to_owned() {
                    Some(ScriptReceiver::Script(script_ref)) => Ok(Datum::ScriptRef(script_ref)),
                    Some(ScriptReceiver::ScriptInstance(id)) => Ok(Datum::ScriptInstanceRef(id)),
                    Some(ScriptReceiver::ScriptText(text)) => Ok(Datum::String(text)),
                    None => Ok(Datum::Int(0)),
                }
            },
            "allowCustomCaching" => Ok(datum_bool(self.allow_custom_caching)),
            "timer" => {
                reserve_player_ref(|player| {
                    let elapsed = chrono::Local::now()
                        .signed_duration_since(player.start_time)
                        .num_milliseconds();
                    // Convert to ticks (60ths of a second)
                    let ticks = (elapsed * 60) / 1000;
                    Ok(Datum::Int(ticks as i32))
                })
            },
            "lastKey" => {
                // `the lastKey` returns ticks (1/60 s) since the last key event.
                // Before any key is pressed, fall back to ticks since movie start
                // (matches Director's monotonic behaviour for these accessors).
                reserve_player_ref(|player| {
                    let reference = player
                        .keyboard_manager
                        .last_key_time
                        .unwrap_or(player.start_time);
                    let elapsed = chrono::Local::now()
                        .signed_duration_since(reference)
                        .num_milliseconds();
                    let ticks = (elapsed * 60) / 1000;
                    Ok(Datum::Int(ticks as i32))
                })
            },
            "mouseDown" => {
                Ok(datum_bool(self.mouse_down))
            },
            // `the mouseUp` is the inverse of `the mouseDown` — true while
            // the mouse button is in the up (released) state. Director uses
            // this in per-frame polling like storyscramble's Draggable
            // behavior (`if the mouseUp then …`) to detect button release.
            "mouseUp" => {
                Ok(datum_bool(!self.mouse_down))
            },
            "rightMouseDown" => Ok(datum_bool(self.right_mouse_down)),
            "rightMouseUp" => Ok(datum_bool(!self.right_mouse_down)),
            // `the trace` toggles the same Lingo-tracing facility as the Trace
            // button / `the traceScript` (Director 11.5 Scripting Dictionary).
            "trace" | "traceScript" => Ok(datum_bool(self.trace_script)),
            "activeWindow" => Ok(Datum::Stage),
            // `the windowList` is the Player property listing all open
            // movie-in-a-window (MIAW) windows (Director 11.5 Scripting
            // Dictionary). dirplayer has no MIAW support, so it's always empty —
            // `count(the windowList)` is then 0, matching a player with only the
            // Stage open.
            "windowList" => Ok(Datum::List(
                crate::director::lingo::datum::DatumType::List,
                VecDeque::new(),
                false,
            )),
            "rollOver" => {
                reserve_player_ref(|player| {
                    let sprite = super::score::get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
                    Ok(Datum::Int(sprite.unwrap_or(0) as i32))
                })
            },
            "randomSeed" => Ok(Datum::Int(self.random_seed.unwrap_or(0))),
            "maxInteger" => Ok(Datum::Int(i32::MAX)),
            "memorySize" => Ok(Datum::Int(256 * 1024 * 1024)), // 256 MB
            // `_system.colorDepth` — monitor color depth (Director 11.5
            // Scripting Dictionary, valid values 1/2/4/8/16/32). The web
            // canvas is true-color, so report 32. Setting it is a no-op
            // (see set_prop), matching a monitor that can't change depth.
            "colorDepth" => Ok(Datum::Int(32)),
            "active3dRenderer" => Ok(Datum::String("#openGL".to_string())),
            "scriptExecutionStyle" => Ok(Datum::Int(9)),
            "xtraList" => {
                // Return a list of prop lists, each with #name and #fileName
                use crate::player::xtra::manager::get_registered_xtra_names;
                reserve_player_mut(|player| {
                    let names = get_registered_xtra_names();
                    let mut items = VecDeque::new();
                    for name in names {
                        let name_key = player.alloc_datum(Datum::Symbol("name".to_string()));
                        let name_val = player.alloc_datum(Datum::String(name.to_string()));
                        let file_key = player.alloc_datum(Datum::Symbol("fileName".to_string()));
                        let file_val = player.alloc_datum(Datum::String(format!("{}.x32", name)));
                        let entry = player.alloc_datum(Datum::PropList(VecDeque::from(vec![
                            (name_key, name_val), (file_key, file_val),
                        ]), false));
                        items.push_back(entry);
                    }
                    Ok(Datum::List(
                        crate::director::lingo::datum::DatumType::List,
                        items,
                        false,
                    ))
                })
            },
            "soundDevice" => Ok(Datum::String(if self.sound_device.is_empty() { "DirectSound".to_string() } else { self.sound_device.clone() })),
            "soundDeviceList" => {
                reserve_player_mut(|player| {
                    let device = player.alloc_datum(Datum::String("WebAudio".to_string()));
                    Ok(Datum::List(
                        crate::director::lingo::datum::DatumType::List,
                        VecDeque::from(vec![device]),
                        false,
                    ))
                })
            },
            "desktopRectList" => {
                reserve_player_mut(|player| {
                    let w = player.movie.rect.right as i32;
                    let h = player.movie.rect.bottom as i32;
                    let rect = player.alloc_datum(Datum::Rect([0.0, 0.0, w as f64, h as f64], 0));
                    Ok(Datum::List(
                        crate::director::lingo::datum::DatumType::List,
                        VecDeque::from(vec![rect]),
                        false,
                    ))
                })
            },
            "labelList" => {
                // Director's `the labelList` ends each label (including the
                // last) with a `\r`, so `the number of lines in the labelList`
                // returns label_count + 1 — script idioms like
                //   numMarkers = the number of lines in the labelList
                //   repeat with i = 1 to numMarkers
                //     L = (the labelList).line[i]
                //     ...
                // expect the trailing empty line to be present (the last
                // iteration sees L = "") so downstream lists end with the
                // same shape they would in Director.
                let mut s = String::new();
                for fl in &self.score.frame_labels {
                    s.push_str(&fl.label);
                    s.push('\r');
                }
                Ok(Datum::String(s))
            },
            "debugplaybackenabled" => Ok(datum_bool(self.debug_playback_enabled)),
            "editShortCutsEnabled" => Ok(datum_bool(self.edit_shortcuts_enabled)),
            // No-op system prop: nothing to preload-abort in dirplayer.
            // Return the Director default (FALSE) so read-backs don't error.
            "preLoadEventAbort" => Ok(datum_bool(false)),
            _ => Err(ScriptError::new(format!("Cannot get movie prop {prop}")))
        })
    }

    pub fn set_prop(
        &mut self,
        prop: &str,
        value: Datum,
        datums: &DatumAllocator,
    ) -> Result<(), ScriptError> {
        match_ci!(prop, {
            "exitLock" => {
                self.exit_lock = value.int_value()? == 1;
                Ok(())
            },
            "itemDelimiter" => {
                self.item_delimiter = (value.string_value()?).chars().next().unwrap();
                Ok(())
            },
            "debugPlaybackEnabled" => {
                self.debug_playback_enabled = value.int_value()? != 0;
                Ok(())
            },
            "editShortCutsEnabled" => {
                self.edit_shortcuts_enabled = value.int_value()? != 0;
                Ok(())
            },
            "alertHook" => {
                match value {
                    Datum::Int(0) => {
                        self.alert_hook = None;
                        Ok(())
                    }
                    Datum::ScriptRef(script_ref) => {
                        self.alert_hook = Some(ScriptReceiver::Script(script_ref));
                        Ok(())
                    }
                    Datum::ScriptInstanceRef(script_instance_id) => {
                        self.alert_hook = Some(ScriptReceiver::ScriptInstance(script_instance_id));
                        Ok(())
                    }
                    _ => Err(ScriptError::new(
                        "Object or 0 expected for alertHook value".to_string(),
                    )),
                }
            },
            // `the trace` is an alias for the Trace-button / `the traceScript`
            // tracing toggle (Director 11.5 Scripting Dictionary).
            "trace" | "traceScript" => {
                self.trace_script = value.int_value()? != 0;
                Ok(())
            },
            "traceLogFile" => {
                self.trace_log_file = value.string_value()?;
                Ok(())
            },
            "updateLock" => {
                self.update_lock = value.int_value()? != 0;
                Ok(())
            },
            "mouseDownScript" | "mouseUpScript" | "keyDownScript" | "keyUpScript" | "timeoutScript" => {
                let target = if prop.eq_ignore_ascii_case("mouseDownScript") {
                    &mut self.mouse_down_script
                } else if prop.eq_ignore_ascii_case("mouseUpScript") {
                    &mut self.mouse_up_script
                } else if prop.eq_ignore_ascii_case("keyDownScript") {
                    &mut self.key_down_script
                } else if prop.eq_ignore_ascii_case("keyUpScript") {
                    &mut self.key_up_script
                } else {
                    &mut self.timeout_script
                };
                match value {
                    Datum::Int(0) | Datum::Void => {
                        *target = None;
                        Ok(())
                    }
                    Datum::String(script_text) => {
                        if script_text.is_empty() {
                            *target = None;
                        } else {
                            *target = Some(ScriptReceiver::ScriptText(script_text));
                        }
                        Ok(())
                    }
                    Datum::ScriptRef(script_ref) => {
                        *target = Some(ScriptReceiver::Script(script_ref));
                        Ok(())
                    }
                    Datum::ScriptInstanceRef(script_instance_id) => {
                        *target = Some(ScriptReceiver::ScriptInstance(script_instance_id));
                        Ok(())
                    }
                    _ => Err(ScriptError::new(
                        format!("String, object or 0 expected for {} value", prop),
                    )),
                }
            },
            "allowCustomCaching" => {
                self.allow_custom_caching = value.int_value()? != 0;
                Ok(())
            },
            "puppetTempo" => {
                self.puppet_tempo = value.int_value()? as u32;
                Ok(())
            },
            "colorDepth" | "useFastQuads" | "romanLingo" | "allowSaveLocal" | "cpuHogTicks"
            | "preLoadEventAbort" => {
                // Read-only / no-op in practice; ignore sets like Director does.
                // preLoadEventAbort gates whether a preload event handler may
                // abort preloading — dirplayer loads synchronously, so there
                // is nothing to abort (netjack's startMovie sets it).
                Ok(())
            },
            "stageColor" => {
                match value {
                    Datum::Int(color_index) => {
                        self.stage_color_ref = ColorRef::PaletteIndex(color_index as u8);
                        Ok(())
                    }
                    Datum::ColorRef(color_ref) => {
                        self.stage_color_ref = color_ref;
                        Ok(())
                    }
                    _ => {
                        Err(ScriptError::new("Integer color index expected for stageColor".to_string()))
                    }
                }
            },
            "timeoutLength" | "timeoutKeyDown" | "timeoutMouse" | "timeoutPlay"
            | "timeoutLapsed" | "soundEnabled" | "soundLevel"
            | "beepOn" | "centerStage" | "exitLock" | "fixStageSize" => {
                // Anim props that are set via property_type 0x07 - accept silently
                Ok(())
            },
            "randomSeed" => {
                self.random_seed = Some(value.int_value()?);
                Ok(())
            },
            "soundDevice" => {
                // Accept the sound device setting (DirectSound, MacroMix, QT3Mix, etc.)
                // In WASM we use WebAudio, so this is stored but not acted upon
                self.sound_device = value.string_value().unwrap_or_default();
                Ok(())
            },
            "preferred3drenderer" | "milesfast" => {
                // 3D renderer preference / sound settings — accept silently in WASM
                Ok(())
            },
            _ => Err(ScriptError::new(format!("Cannot set movie prop {prop}")))
        })
    }

    /// Get the current effective tempo (puppetTempo overrides frameTempo)
    pub fn get_effective_tempo(&self) -> u32 {
        if self.puppet_tempo > 0 {
            self.puppet_tempo
        } else {
            // Get tempo from current frame, or fall back to movie frame_rate
            self.score.get_frame_tempo(self.current_frame)
                .unwrap_or(self.frame_rate as u32)
        }
    }
    
    /// Calculate frame delay in milliseconds based on tempo
    pub fn get_frame_delay_ms(&self) -> f64 {
        let tempo = self.get_effective_tempo();
        if tempo == 0 {
            return 1000.0 / 30.0; // Default to 30fps if tempo is 0
        }
        
        // Director tempo: frames per second
        // So delay = 1000ms / tempo
        1000.0 / tempo as f64
    }

    pub fn next_random_int(&mut self, max: i32) -> Option<i32> {
        let seed = self.random_seed?;
        let seed_u32 = seed as u32;

        // Note: This does not match the Director implementation exactly - there is no public knowledge of the seed algorithm.
        let next_seed = seed_u32.wrapping_mul(214013).wrapping_add(2531011);
        self.random_seed = Some(next_seed as i32);
        let value = (next_seed % (max as u32)) as i32 + 1;
        Some(value)
    }
}
