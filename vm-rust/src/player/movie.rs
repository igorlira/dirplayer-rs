use std::collections::HashMap;

use chrono::Local;

use crate::{
    director::{
        file::DirectorFile,
        lingo::datum::{datum_bool, Datum},
    },
    utils::PATH_SEPARATOR, reserve_player_ref,
    player::ColorRef, player::ScriptInstanceRef, CastMemberRef,
};

use super::{
    allocator::DatumAllocator, bitmap::manager::BitmapManager, cast_manager::CastManager,
    geometry::IntRect, net_manager::NetManager, score::Score, ScriptError, ScriptReceiver,
    reserve_player_mut,
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
    pub mouse_down: bool,
    pub click_loc: (i32, i32),
    pub frame_script_instance: Option<ScriptInstanceRef>,
    pub frame_script_member: Option<CastMemberRef>,
    pub sound_device: String,
}

impl Movie {
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
        let prop_lower = prop.to_lowercase();
        match prop_lower.as_str() {
            "alerthook" => match self.alert_hook.to_owned() {
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
            }
            "long time" => {
                let time = Local::now();
                let formatted = time.format("%H:%M:%S %p").to_string();
                Ok(Datum::String(formatted))
            }
            "lastchannel" => Ok(Datum::Int(self.score.get_channel_count() as i32)),
            "moviepath" => {
                let mut result = self.base_path.clone();
                if !result.is_empty() && !result.ends_with(PATH_SEPARATOR) {
                    result.push_str(PATH_SEPARATOR);
                }
                Ok(Datum::String(result))
            }
            "platform" => Ok(Datum::String("Windows,32".to_string())),
            "frame" => Ok(Datum::Int(self.current_frame as i32)),
            "productversion" => Ok(Datum::String("10.1".to_string())),
            "stageright" => Ok(Datum::Int(self.rect.right as i32)),
            "stageleft" => Ok(Datum::Int(self.rect.left as i32)),
            "stagetop" => Ok(Datum::Int(self.rect.top as i32)),
            "stagebottom" => Ok(Datum::Int(self.rect.bottom as i32)),
            "moviename" => Ok(Datum::String(self.file_name.to_owned())),
            "updatelock" => Ok(Datum::Int(if self.update_lock { 1 } else { 0 })),
            "path" => Ok(Datum::String(self.base_path.to_owned())),
            "mousedownscript" | "mouseupscript" | "keydownscript" | "keyupscript" | "timeoutscript" => {
                let script = match prop_lower.as_str() {
                    "mousedownscript" => &self.mouse_down_script,
                    "mouseupscript" => &self.mouse_up_script,
                    "keydownscript" => &self.key_down_script,
                    "keyupscript" => &self.key_up_script,
                    "timeoutscript" => &self.timeout_script,
                    _ => unreachable!(),
                };
                match script.to_owned() {
                    Some(ScriptReceiver::Script(script_ref)) => Ok(Datum::ScriptRef(script_ref)),
                    Some(ScriptReceiver::ScriptInstance(id)) => Ok(Datum::ScriptInstanceRef(id)),
                    Some(ScriptReceiver::ScriptText(text)) => Ok(Datum::String(text)),
                    None => Ok(Datum::Int(0)),
                }
            }
            "allowcustomcaching" => Ok(datum_bool(self.allow_custom_caching)),
            "timer" => {
                reserve_player_ref(|player| {
                    let elapsed = chrono::Local::now()
                        .signed_duration_since(player.start_time)
                        .num_milliseconds();
                    // Convert to ticks (60ths of a second)
                    let ticks = (elapsed * 60) / 1000;
                    Ok(Datum::Int(ticks as i32))
                })
            }
            "mousedown" => {
                Ok(datum_bool(self.mouse_down))
            }
            "tracescript" => Ok(datum_bool(self.trace_script)),
            "activewindow" => Ok(Datum::Stage),
            "rollover" => {
                reserve_player_ref(|player| {
                    let sprite = super::score::get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
                    Ok(Datum::Int(sprite.unwrap_or(0) as i32))
                })
            }
            "randomseed" => Ok(Datum::Int(self.random_seed.unwrap_or(0))),
            "maxinteger" => Ok(Datum::Int(i32::MAX)),
            "memorysize" => Ok(Datum::Int(256 * 1024 * 1024)), // 256 MB
            "active3drenderer" => Ok(Datum::String("#openGL".to_string())),
            "scriptexecutionstyle" => Ok(Datum::Int(9)),
            "xtralist" => {
                // Return a list of prop lists, each with #name and #fileName
                use crate::player::xtra::manager::get_registered_xtra_names;
                reserve_player_mut(|player| {
                    let names = get_registered_xtra_names();
                    let mut items = Vec::new();
                    for name in names {
                        let name_key = player.alloc_datum(Datum::Symbol("name".to_string()));
                        let name_val = player.alloc_datum(Datum::String(name.to_string()));
                        let file_key = player.alloc_datum(Datum::Symbol("fileName".to_string()));
                        let file_val = player.alloc_datum(Datum::String(format!("{}.x32", name)));
                        let entry = player.alloc_datum(Datum::PropList(vec![
                            (name_key, name_val), (file_key, file_val),
                        ], false));
                        items.push(entry);
                    }
                    Ok(Datum::List(
                        crate::director::lingo::datum::DatumType::List,
                        items,
                        false,
                    ))
                })
            }
            "sounddevice" => Ok(Datum::String(if self.sound_device.is_empty() { "DirectSound".to_string() } else { self.sound_device.clone() })),
            "sounddevicelist" => {
                reserve_player_mut(|player| {
                    let device = player.alloc_datum(Datum::String("WebAudio".to_string()));
                    Ok(Datum::List(
                        crate::director::lingo::datum::DatumType::List,
                        vec![device],
                        false,
                    ))
                })
            }
            "desktoprectlist" => {
                reserve_player_mut(|player| {
                    let w = player.movie.rect.right as i32;
                    let h = player.movie.rect.bottom as i32;
                    let l = player.alloc_datum(Datum::Int(0));
                    let t = player.alloc_datum(Datum::Int(0));
                    let r = player.alloc_datum(Datum::Int(w));
                    let b = player.alloc_datum(Datum::Int(h));
                    let rect = player.alloc_datum(Datum::Rect([l, t, r, b]));
                    Ok(Datum::List(
                        crate::director::lingo::datum::DatumType::List,
                        vec![rect],
                        false,
                    ))
                })
            }
            "labellist" => {
                let s = self
                    .score
                    .frame_labels
                    .iter()
                    .map(|fl| fl.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\r");
                Ok(Datum::String(s))
            }
            "debugplaybackenabled" => Ok(Datum::Int(0)),
            _ => Err(ScriptError::new(format!("Cannot get movie prop {prop}"))),
        }
    }

    pub fn set_prop(
        &mut self,
        prop: &str,
        value: Datum,
        datums: &DatumAllocator,
    ) -> Result<(), ScriptError> {
        let prop_lower = prop.to_lowercase();
        match prop_lower.as_str() {
            "exitlock" => {
                self.exit_lock = value.int_value()? == 1;
            }
            "itemdelimiter" => {
                self.item_delimiter = (value.string_value()?).chars().next().unwrap();
            }
            "debugplaybackenabled" => {
                // TODO
            }
            "alerthook" => {
                return match value {
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
            }
            "tracescript" => {
                self.trace_script = value.int_value()? != 0;
            }
            "tracelogfile" => {
                self.trace_log_file = value.string_value()?;
            }
            "updatelock" => {
                self.update_lock = value.int_value()? != 0;
            }
            "mousedownscript" | "mouseupscript" | "keydownscript" | "keyupscript" | "timeoutscript" => {
                let target = match prop_lower.as_str() {
                    "mousedownscript" => &mut self.mouse_down_script,
                    "mouseupscript" => &mut self.mouse_up_script,
                    "keydownscript" => &mut self.key_down_script,
                    "keyupscript" => &mut self.key_up_script,
                    "timeoutscript" => &mut self.timeout_script,
                    _ => unreachable!(),
                };
                return match value {
                    Datum::Int(0) | Datum::Void => {
                        *target = None;
                        Ok(())
                    }
                    Datum::String(script_text) => {
                        if script_text.is_empty() {
                            // EMPTY clears the script
                            *target = None;
                        } else {
                            // Store everything, including comments like "--nothing".
                            // In Director, setting mouseDownScript to a comment means
                            // "intercept the event but do nothing" - the presence of
                            // the script blocks normal event propagation to sprites.
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
            }
            "allowcustomcaching" => {
                self.allow_custom_caching = value.int_value()? != 0;
            }
            "puppettempo" => {
                self.puppet_tempo = value.int_value()? as u32;
            }
            "colordepth" | "usefastquads" | "romanlingo" | "allowsavelocal" => {
                // Read-only / no-op in practice; ignore sets like Director does
            }
            "sounddevice" => {
                // Accept the sound device setting (DirectSound, MacroMix, QT3Mix, etc.)
                // In WASM we use WebAudio, so this is stored but not acted upon
                self.sound_device = value.string_value().unwrap_or_default();
            }
            "timeoutlength" | "timeoutkeydown" | "timeoutmouse" | "timeoutplay"
            | "timeoutlapsed" | "soundenabled" | "soundlevel"
            | "beepon" | "centerstage" | "fixstagesize" => {
                // Anim props that are set via property_type 0x07 - accept silently
            }
            "randomseed" => {
                self.random_seed = Some(value.int_value()?);
            }
            "stagecolor" => {
                match value {
                    Datum::Int(color_index) => {
                        self.stage_color_ref = ColorRef::PaletteIndex(color_index as u8);
                    }
                    Datum::ColorRef(color_ref) => {
                        self.stage_color_ref = color_ref;
                    }
                    _ => {
                        return Err(ScriptError::new("Integer color index expected for stageColor".to_string()));
                    }
                }
            }
            _ => return Err(ScriptError::new(format!("Cannot set movie prop {prop}"))),
        }
        Ok(())
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
