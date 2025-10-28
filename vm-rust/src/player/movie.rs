use std::collections::HashMap;

use chrono::Local;

use crate::{
    director::{
        file::DirectorFile,
        lingo::datum::{datum_bool, Datum},
    },
    utils::PATH_SEPARATOR,
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
    pub exit_lock: bool,
    pub dir_version: u16,
    pub item_delimiter: char,
    pub alert_hook: Option<ScriptReceiver>,
    pub base_path: String,
    pub file_name: String,
    pub stage_color: (u8, u8, u8),
    pub frame_rate: u16,
    pub file: Option<DirectorFile>,
    pub update_lock: bool,
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
    }

    pub fn get_prop(&self, prop: &str) -> Result<Datum, ScriptError> {
        match prop {
            "alertHook" => match self.alert_hook.to_owned() {
                Some(ScriptReceiver::Script(script_ref)) => Ok(Datum::ScriptRef(script_ref)),
                Some(ScriptReceiver::ScriptInstance(script_instance_id)) => {
                    Ok(Datum::ScriptInstanceRef(script_instance_id))
                }
                None => Ok(Datum::Int(0)),
            },
            "exitLock" => Ok(datum_bool(self.exit_lock)),
            "itemDelimiter" => Ok(Datum::String(self.item_delimiter.into())),
            "runMode" => Ok(Datum::String("Plugin".to_string())), // Plugin / Author
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
            "lastChannel" => Ok(Datum::Int(self.score.get_channel_count() as i32)),
            "moviePath" => {
                let mut result = self.base_path.clone();
                if !result.is_empty() && !result.ends_with(PATH_SEPARATOR) {
                    result.push_str(PATH_SEPARATOR);
                }
                Ok(Datum::String(result))
            }
            "platform" => Ok(Datum::String("Windows,32".to_string())),
            "frame" => Ok(Datum::Int(self.current_frame as i32)),
            "productVersion" => Ok(Datum::String("10.1".to_string())),
            "stageRight" => Ok(Datum::Int(self.rect.right as i32)),
            "stageLeft" => Ok(Datum::Int(self.rect.left as i32)),
            "stageTop" => Ok(Datum::Int(self.rect.top as i32)),
            "stageBottom" => Ok(Datum::Int(self.rect.bottom as i32)),
            "traceLogFile" => Ok(Datum::String("".to_string())), // TODO
            "traceScript" => Ok(Datum::Int(0)),                  // TODO
            "movieName" => Ok(Datum::String(self.file_name.to_owned())),
            "updateLock" => Ok(Datum::Int(if self.update_lock { 1 } else { 0 })),
            _ => Err(ScriptError::new(format!("Cannot get movie prop {prop}"))),
        }
    }

    pub fn set_prop(
        &mut self,
        prop: &str,
        value: Datum,
        datums: &DatumAllocator,
    ) -> Result<(), ScriptError> {
        match prop {
            "exitLock" => {
                self.exit_lock = value.int_value()? == 1;
            }
            "itemDelimiter" => {
                self.item_delimiter = (value.string_value()?).as_bytes()[0] as char;
            }
            "debugPlaybackEnabled" => {
                // TODO
            }
            "alertHook" => {
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
            "traceScript" => {
                // TODO
                return Ok(());
            }
            "traceLogFile" => {
                // TODO
                return Ok(());
            }
            "updateLock" => {
                self.update_lock = value.int_value()? != 0;
            }
            _ => return Err(ScriptError::new(format!("Cannot set movie prop {prop}"))),
        }
        Ok(())
    }
}
