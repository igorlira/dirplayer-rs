pub mod allocator;
pub mod bitmap;
pub mod bytecode;
pub mod cast_lib;
pub mod cast_manager;
pub mod cast_member;
pub mod commands;
pub mod compare;
pub mod context_vars;
pub mod datum_formatting;
pub mod datum_operations;
pub mod datum_ref;
pub mod debug;
pub mod eval;
pub mod events;
pub mod font;
pub mod geometry;
pub mod handlers;
pub mod keyboard;
pub mod keyboard_events;
pub mod keyboard_map;
pub mod movie;
pub mod net_manager;
pub mod net_task;
pub mod profiling;
pub mod scope;
pub mod score;
pub mod script;
pub mod script_ref;
pub mod sprite;
pub mod stage;
pub mod timeout;
pub mod xtra;

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use allocator::{
    DatumAllocator, DatumAllocatorTrait, ResetableAllocator, ScriptInstanceAllocatorTrait,
};
use async_std::{
    channel::{self, Receiver, Sender},
    future::{self, timeout},
    sync::Mutex,
    task::spawn_local,
};
use cast_manager::CastPreloadReason;
use cast_member::{CastMemberType, CastMemberTypeId};
use datum_ref::DatumRef;
use fxhash::FxHashMap;
use handlers::datum_handlers::script_instance::ScriptInstanceUtils;
use log::warn;
use manual_future::{ManualFuture, ManualFutureCompleter};
use net_manager::NetManager;
use profiling::{end_profiling, start_profiling};
use scope::ScopeResult;
use score::{get_score_sprite_mut, ScoreRef};
use script::script_get_prop_opt;
use script_ref::ScriptInstanceRef;
use sprite::Sprite;
use xtra::multiuser::{MultiuserXtraManager, MULTIUSER_XTRA_MANAGER_OPT};

use crate::{
    console_warn,
    director::{
        chunks::handler::{Bytecode, HandlerDef},
        enums::ScriptType,
        file::{read_director_file_bytes, DirectorFile},
        lingo::{
            constants::{get_anim2_prop_name, get_anim_prop_name},
            datum::{datum_bool, Datum, DatumType, VarRef},
        },
    },
    js_api::JsApi,
    player::{
        bytecode::handler_manager::{player_execute_bytecode, BytecodeHandlerContext},
        datum_formatting::format_datum,
        geometry::IntRect,
        profiling::get_profiler_report,
        scope::Scope,
    },
    rendering::with_canvas_renderer_mut,
    utils::{get_base_url, get_basename_no_extension, get_elapsed_ticks},
};

use self::{
    bytecode::handler_manager::StaticBytecodeHandlerManager,
    cast_lib::CastMemberRef,
    cast_manager::CastManager,
    commands::{run_command_loop, PlayerVMCommand},
    debug::{Breakpoint, BreakpointContext, BreakpointManager},
    events::{
        player_dispatch_global_event, player_invoke_global_event, player_unwrap_result,
        player_wait_available, run_event_loop, PlayerVMEvent,
    },
    font::{player_load_system_font, BitmapFont, FontManager},
    handlers::manager::BuiltInHandlerManager,
    keyboard::KeyboardManager,
    movie::Movie,
    net_manager::NetManagerSharedState,
    scope::ScopeRef,
    score::{get_sprite_at, Score},
    script::{Script, ScriptHandlerRef},
    sprite::{ColorRef, CursorRef},
    timeout::TimeoutManager,
};

use crate::player::handlers::datum_handlers::date::DateObject;
use crate::player::handlers::datum_handlers::math::MathObject;
use crate::player::handlers::datum_handlers::sound_channel::{
    AudioData, SoundChannelDatumHandlers, SoundManager,
};
use crate::player::handlers::datum_handlers::xml::{XmlDocument, XmlNode};

use crate::player::handlers::datum_handlers::player_call_datum_handler;

pub enum HandlerExecutionResult {
    Advance,
    Stop,
    Jump,
    Error(ScriptError),
}

pub struct HandlerExecutionResultContext {
    pub result: HandlerExecutionResult,
}

pub struct PlayerVMExecutionItem {
    pub command: PlayerVMCommand,
    pub completer: Option<ManualFutureCompleter<Result<DatumRef, ScriptError>>>,
}

pub const MAX_STACK_SIZE: usize = 50;

pub struct DirPlayer {
    pub net_manager: NetManager,
    pub movie: Movie,
    pub is_playing: bool,
    pub is_script_paused: bool,
    pub next_frame: Option<u32>,
    pub queue_tx: Sender<PlayerVMExecutionItem>,
    pub globals: FxHashMap<String, DatumRef>,
    pub scopes: Vec<Scope>,
    pub bytecode_handler_manager: StaticBytecodeHandlerManager,
    pub breakpoint_manager: BreakpointManager,
    pub current_breakpoint: Option<BreakpointContext>,
    pub stage_size: (u32, u32),
    pub bitmap_manager: bitmap::manager::BitmapManager,
    pub cursor: CursorRef,
    pub start_time: chrono::DateTime<chrono::Local>,
    pub timeout_manager: TimeoutManager,
    pub title: String,
    pub bg_color: ColorRef,
    pub keyboard_focus_sprite: i16,
    pub text_selection_start: u16,
    pub text_selection_end: u16,
    pub mouse_loc: (i32, i32),
    pub last_mouse_down_time: i64,
    pub is_double_click: bool,
    pub mouse_down_sprite: i16,
    pub subscribed_member_refs: Vec<CastMemberRef>, // TODO move to debug module
    pub is_subscribed_to_channel_names: bool,       // TODO move to debug module
    pub font_manager: FontManager,
    pub keyboard_manager: KeyboardManager,
    pub float_precision: u8,
    pub last_handler_result: DatumRef,
    pub hovered_sprite: Option<i16>,
    pub allocator: DatumAllocator,
    pub dir_cache: HashMap<Box<str>, DirectorFile>,
    pub scope_count: u32,
    pub external_params: HashMap<String, String>,
    // XML document storage - maps XML document IDs to parsed XML structures
    pub xml_documents: HashMap<u32, XmlDocument>,
    // XML node storage - maps node IDs to XML nodes
    pub xml_nodes: HashMap<u32, XmlNode>,
    // Counter for generating unique XML IDs
    pub next_xml_id: u32,
    pub sound_manager: SoundManager,
    pub date_objects: HashMap<u32, DateObject>,
    pub math_objects: HashMap<u32, MathObject>,
}

impl DirPlayer {
    pub fn new<'a>(tx: Sender<PlayerVMExecutionItem>) -> DirPlayer {
        let sound_manager = SoundManager::new(8).expect("Sound manager failed to initialize"); // 8 sound channels (Director standard)

        let mut result = DirPlayer {
            movie: Movie {
                rect: IntRect::from(0, 0, 0, 0),
                cast_manager: CastManager::empty(),
                score: Score::empty(),
                current_frame: 1,
                puppet_tempo: 0,
                exit_lock: false,
                dir_version: 0,
                item_delimiter: '.',
                alert_hook: None,
                base_path: "".to_string(),
                file_name: "".to_string(),
                stage_color: (0, 0, 0),
                frame_rate: 30,
                file: None,
                update_lock: false,
            },
            net_manager: NetManager {
                base_path: None,
                tasks: HashMap::new(),
                task_states: HashMap::new(),
                shared_state: Arc::new(Mutex::new(NetManagerSharedState::new())),
            },
            is_playing: false,
            is_script_paused: false,
            next_frame: None,
            queue_tx: tx,
            globals: FxHashMap::default(),
            scopes: Vec::with_capacity(MAX_STACK_SIZE),
            bytecode_handler_manager: StaticBytecodeHandlerManager {},
            breakpoint_manager: BreakpointManager::new(),
            current_breakpoint: None,
            stage_size: (100, 100),
            bitmap_manager: bitmap::manager::BitmapManager::new(),
            cursor: CursorRef::System(0),
            start_time: chrono::Local::now(), // supposed to be time at which computer started, but we don't have access from browser. this is sufficient for calculating elapsed time.
            timeout_manager: TimeoutManager::new(),
            title: "".to_string(),
            bg_color: ColorRef::Rgb(0, 0, 0),
            keyboard_focus_sprite: -1, // Setting keyboardFocusSprite to -1 returns keyboard focus control to the Score, and setting it to 0 disables keyboard entry into any editable sprite.
            mouse_loc: (0, 0),
            last_mouse_down_time: 0,
            is_double_click: false,
            mouse_down_sprite: 0,
            subscribed_member_refs: vec![],
            is_subscribed_to_channel_names: false,
            font_manager: FontManager::new(),
            keyboard_manager: KeyboardManager::new(),
            text_selection_start: 0,
            text_selection_end: 0,
            float_precision: 4,
            last_handler_result: DatumRef::Void,
            hovered_sprite: None,
            allocator: DatumAllocator::default(),
            dir_cache: HashMap::new(),
            scope_count: 0,
            external_params: HashMap::new(),
            xml_documents: HashMap::new(),
            xml_nodes: HashMap::new(),
            next_xml_id: 1000,
            sound_manager: sound_manager,
            date_objects: HashMap::new(),
            math_objects: HashMap::new(),
        };

        // Initialize the actorList as a global variable
        let actor_list_datum = result.alloc_datum(Datum::List(DatumType::List, vec![], false));
        result
            .globals
            .insert("actorList".to_string(), actor_list_datum);

        // Initialize VOID constant
        result.globals.insert("VOID".to_string(), DatumRef::Void);

        for i in 0..MAX_STACK_SIZE {
            result.scopes.push(Scope::default(i));
        }
        result
    }

    pub async fn load_movie_from_file(&mut self, path: &str) {
        let task_id = self.net_manager.preload_net_thing(path.to_owned());
        self.net_manager.await_task(task_id).await;
        let task = self.net_manager.get_task(task_id).unwrap();
        let data_bytes = self
            .net_manager
            .get_task_result(Some(task_id))
            .unwrap()
            .unwrap();

        let movie_file = read_director_file_bytes(
            &data_bytes,
            &get_basename_no_extension(task.resolved_url.path()),
            &get_base_url(&task.resolved_url).to_string(),
        )
        .unwrap();
        self.load_movie_from_dir(movie_file).await;
    }

    async fn load_movie_from_dir(&mut self, dir: DirectorFile) {
        self.movie
            .load_from_file(
                dir,
                &mut self.net_manager,
                &mut self.bitmap_manager,
                &mut self.dir_cache,
            )
            .await;
        let (r, g, b) = self.movie.stage_color;
        self.bg_color = ColorRef::Rgb(r, g, b);

        // Load all fonts from cast members into the font manager
        web_sys::console::log_1(&"Loading fonts from cast members...".into());
        self.movie
            .cast_manager
            .load_fonts_into_manager(&mut self.font_manager);

        with_canvas_renderer_mut(|renderer| {
            if let Some(renderer) = renderer.as_mut() {
                renderer.set_size(
                    self.movie.rect.width() as u32,
                    self.movie.rect.height() as u32,
                );
            }
        });

        JsApi::dispatch_movie_loaded(self.movie.file.as_ref().unwrap());
    }

    pub fn play(&mut self) {
        if self.is_playing {
            return;
        }
        self.is_playing = true;
        self.is_script_paused = false;
        // TODO runVM()
        async_std::task::spawn_local(async move {
            if let Err(err) = player_invoke_global_event(&"prepareMovie".to_string(), &vec![]).await
            {
                reserve_player_mut(|player| player.on_script_error(&err));
                return;
            }
            reserve_player_mut(|player| {
                // player.movie.score.begin_sprites(player.movie.current_frame);
                player.begin_all_sprites();
            });
            run_frame_loop().await;
        });
    }

    pub fn begin_all_sprites(&mut self) {
        self.movie.score.begin_sprites(self.movie.current_frame);
        for channel in self.movie.score.channels.iter() {
            let member_ref = channel.sprite.member.as_ref();
            let member_type = member_ref
                .and_then(|x| self.movie.cast_manager.find_member_by_ref(&x))
                .map(|x| x.member_type.member_type_id());

            match member_type {
                Some(CastMemberTypeId::FilmLoop) => {
                    let film_loop = self
                        .movie
                        .cast_manager
                        .find_mut_member_by_ref(member_ref.unwrap())
                        .unwrap()
                        .member_type
                        .as_film_loop_mut()
                        .unwrap();
                    film_loop.score.begin_sprites(self.movie.current_frame);
                }
                _ => {}
            }
        }
    }

    pub fn end_all_sprites(&mut self) -> Vec<(ScoreRef, u32)> {
        let next_frame = self.get_next_frame();
        let mut all_ended_sprite_nums: Vec<(ScoreRef, u32)> = vec![];
        let ended_sprite_nums = self
            .movie
            .score
            .end_sprites(self.movie.current_frame, next_frame);
        all_ended_sprite_nums.extend(ended_sprite_nums.iter().map(|&x| (ScoreRef::Stage, x)));

        for channel in self.movie.score.channels.iter() {
            let member_ref = channel.sprite.member.as_ref();
            let member_type = member_ref
                .and_then(|x| self.movie.cast_manager.find_member_by_ref(&x))
                .map(|x| x.member_type.member_type_id());

            match member_type {
                Some(CastMemberTypeId::FilmLoop) => {
                    let next_frame = self.get_next_frame();
                    let film_loop = self
                        .movie
                        .cast_manager
                        .find_mut_member_by_ref(member_ref.unwrap())
                        .unwrap()
                        .member_type
                        .as_film_loop_mut()
                        .unwrap();
                    let ended_sprite_nums = film_loop
                        .score
                        .end_sprites(self.movie.current_frame, next_frame);
                    all_ended_sprite_nums.extend(
                        ended_sprite_nums
                            .iter()
                            .map(|&x| (ScoreRef::FilmLoop(member_ref.unwrap().clone()), x)),
                    );
                }
                _ => {}
            }
        }
        // for sprite_num in ended_sprite_nums.iter() {
        //   let sprite = self.movie.score.get_sprite_mut(*sprite_num as i16);
        //   sprite.exited = true;
        // }
        all_ended_sprite_nums
    }

    pub fn pause_script(&mut self) {
        self.is_script_paused = true;
    }

    pub fn resume_script(&mut self) {
        self.is_script_paused = false;
    }

    pub fn resume_breakpoint(&mut self) {
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            spawn_local(breakpoint.completer.complete(()));
        }
    }

    pub fn get_datum(&self, id: &DatumRef) -> &Datum {
        self.allocator.get_datum(id)
    }

    pub fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum {
        self.allocator.get_datum_mut(id)
    }

    pub fn get_fps(&self) -> u32 {
        if self.movie.puppet_tempo > 0 {
            self.movie.puppet_tempo
        } else {
            self.movie.frame_rate as u32
        }
    }

    pub fn get_hydrated_globals(&self) -> FxHashMap<String, &Datum> {
        self.globals
            .iter()
            .map(|(k, v)| (k.to_owned(), self.get_datum(v)))
            .collect()
    }

    #[allow(dead_code)]
    pub fn get_global(&self, name: &String) -> Option<&Datum> {
        self.globals
            .get(name)
            .map(|datum_ref| self.get_datum(datum_ref))
    }

    pub fn get_next_frame(&self) -> u32 {
        if !self.is_playing {
            return self.movie.current_frame;
        } else if let Some(next_frame) = self.next_frame {
            return next_frame;
        } else {
            return self.movie.current_frame + 1;
        }
    }

    pub fn advance_frame(&mut self) {
        if !self.is_playing {
            return;
        }

        let prev_frame = self.movie.current_frame;
        let next_frame = self.get_next_frame();

        // Always advance logic (scripts, behaviors)
        self.next_frame = None;
        self.movie.current_frame = next_frame;

        // Only dispatch and render if updateLock is off
        if !self.movie.update_lock && prev_frame != self.movie.current_frame {
            JsApi::dispatch_frame_changed(self.movie.current_frame);
        }
    }

    pub fn stop(&mut self) {
        // TODO dispatch stop movie
        self.is_playing = false;
        self.next_frame = None;
        //scopes.clear();
        // currentBreakpoint?.completer.completeError(CancelledException());
        // currentBreakpoint = null;
        self.timeout_manager.clear();
        //notifyListeners();

        warn!("Profiler report: {}", get_profiler_report());
    }

    pub fn reset(&mut self) {
        self.stop();
        self.scopes.clear();
        self.globals.clear();
        self.allocator.reset();
        self.timeout_manager.clear();
        // netManager.clear();
        self.movie.score.reset();
        self.movie.current_frame = 1;
        // TODO cancel breakpoints
        self.current_breakpoint = None;
        // notifyListeners();

        JsApi::dispatch_frame_changed(self.movie.current_frame);
        JsApi::dispatch_scope_list(self);
        JsApi::dispatch_script_error_cleared();
        JsApi::dispatch_global_list(self);
    }

    pub fn alloc_datum(&mut self, datum: Datum) -> DatumRef {
        return self.allocator.alloc_datum(datum).unwrap();
    }

    fn get_movie_prop(&mut self, prop: &str) -> Result<DatumRef, ScriptError> {
        match prop {
            "stage" => Ok(self.alloc_datum(Datum::Stage)),
            "time" => Ok(self.alloc_datum(Datum::String(
                chrono::Local::now().format("%H:%M %p").to_string(),
            ))),
            "milliSeconds" => Ok(self.alloc_datum(Datum::Int(
                chrono::Local::now()
                    .signed_duration_since(self.start_time)
                    .num_milliseconds() as i32,
            ))),
            "keyboardFocusSprite" => {
                Ok(self.alloc_datum(Datum::Int(self.keyboard_focus_sprite as i32)))
            }
            "frameTempo" => Ok(self.alloc_datum(Datum::Int(self.movie.puppet_tempo as i32))),
            "mouseLoc" => Ok(self.alloc_datum(Datum::IntPoint(self.mouse_loc))),
            "mouseH" => Ok(self.alloc_datum(Datum::Int(self.mouse_loc.0 as i32))),
            "mouseV" => Ok(self.alloc_datum(Datum::Int(self.mouse_loc.1 as i32))),
            "rollover" => {
                let sprite = get_sprite_at(self, self.mouse_loc.0, self.mouse_loc.1, false);
                Ok(self.alloc_datum(Datum::Int(sprite.unwrap_or(0) as i32)))
            }
            "keyCode" => Ok(self.alloc_datum(Datum::Int(self.keyboard_manager.key_code() as i32))),
            "shiftDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_shift_down()))),
            "optionDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_alt_down()))),
            "commandDown" => {
                Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_command_down())))
            }
            "controlDown" => {
                Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_control_down())))
            }
            "altDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_alt_down()))),
            "key" => Ok(self.alloc_datum(Datum::String(self.keyboard_manager.key()))),
            "floatPrecision" => Ok(self.alloc_datum(Datum::Int(self.float_precision as i32))),
            "doubleClick" => Ok(self.alloc_datum(datum_bool(self.is_double_click))),
            "ticks" => Ok(self.alloc_datum(Datum::Int(get_elapsed_ticks(self.start_time)))),
            "frameLabel" => {
                let frame_label = self
                    .movie
                    .score
                    .frame_labels
                    .iter()
                    .filter(|&label| label.frame_num <= self.movie.current_frame as i32)
                    .max_by_key(|label| label.frame_num)
                    .map(|label| label.label.clone());
                Ok(self.alloc_datum(Datum::String(
                    frame_label.unwrap_or_else(|| "0".to_string()),
                )))
            }
            "currentSpriteNum" => {
                // TODO: this can also be called by a static script
                let script_instance_ref = self
                    .scopes
                    .get(self.current_scope_ref())
                    .and_then(|scope| scope.receiver.clone());

                let datum_ref = script_instance_ref
                    .and_then(|x| script_get_prop_opt(self, &x, &"spriteNum".to_owned()));
                if let Some(datum_ref) = datum_ref {
                    let datum = self.get_datum(&datum_ref);
                    let sprite_num = datum.int_value()?;
                    Ok(self.alloc_datum(Datum::Int(sprite_num)))
                } else {
                    Ok(self.alloc_datum(Datum::Int(0)))
                }
            }
            // Return the actual DatumRef from globals
            "actorList" => {
                // Return the reference to the global actorList, not a clone of its contents
                Ok(self
                    .globals
                    .get("actorList")
                    .unwrap_or(&DatumRef::Void)
                    .clone())
            }
            _ => {
                let datum = self.movie.get_prop(prop)?;
                Ok(self.alloc_datum(datum))
            }
        }
    }

    fn get_player_prop(&mut self, prop: &String) -> Result<DatumRef, ScriptError> {
        match prop.as_str() {
            "traceScript" => Ok(self.alloc_datum(datum_bool(false))), // TODO
            "productVersion" => Ok(self.alloc_datum(Datum::String("10.1".to_string()))), // TODO
            _ => Err(ScriptError::new(format!("Unknown player prop {}", prop))),
        }
    }

    fn set_player_prop(&mut self, prop: &String, value: &DatumRef) -> Result<(), ScriptError> {
        match prop.as_str() {
            "traceScript" => {
                // TODO
                Ok(())
            }
            _ => Err(ScriptError::new(format!("Cannot set player prop {}", prop))),
        }
    }

    fn get_anim_prop(&self, prop_id: u16) -> Result<Datum, ScriptError> {
        let prop_name = get_anim_prop_name(prop_id);
        match prop_name {
            "colorDepth" => Ok(Datum::Int(32)),
            "timer" => Ok(Datum::Int(get_elapsed_ticks(self.start_time))),
            _ => Err(ScriptError::new(format!("Unknown anim prop {}", prop_name))),
        }
    }

    fn get_anim2_prop(&self, prop_id: u16) -> Result<Datum, ScriptError> {
        let prop_name = get_anim2_prop_name(prop_id);
        match prop_name {
            "number of castLibs" => Ok(Datum::Int(self.movie.cast_manager.casts.len() as i32)),
            "number of castMembers" => Ok(Datum::Int(
                self.movie
                    .cast_manager
                    .casts
                    .iter()
                    .map(|cast_lib| cast_lib.members.len() as i32)
                    .sum(),
            )),
            _ => Err(ScriptError::new(format!(
                "Unknown anim2 prop {}",
                prop_name
            ))),
        }
    }

    fn set_movie_prop(&mut self, prop: &str, value: Datum) -> Result<(), ScriptError> {
        match prop {
            "keyboardFocusSprite" => {
                // TODO switch focus
                self.keyboard_focus_sprite = value.int_value()? as i16;
                Ok(())
            }
            "selStart" => {
                self.text_selection_start = value.int_value()? as u16;
                Ok(())
            }
            "selEnd" => {
                self.text_selection_end = value.int_value()? as u16;
                Ok(())
            }
            "floatPrecision" => {
                self.float_precision = value.int_value()? as u8;
                Ok(())
            }
            "centerStage" => {
                // TODO
                Ok(())
            }
            "actorList" => {
                // Setting actorList - update the global variable
                match value {
                    Datum::List(list_type, list_items, sorted) => {
                        let new_actor_list =
                            self.alloc_datum(Datum::List(list_type, list_items, sorted));
                        self.globals.insert("actorList".to_string(), new_actor_list);
                        Ok(())
                    }
                    _ => Err(ScriptError::new("actorList must be a list".to_string())),
                }
            }
            _ => self.movie.set_prop(prop, value, &self.allocator),
        }
    }

    fn on_script_error(&mut self, err: &ScriptError) {
        warn!("[!!] play failed with error: {}", err.message);
        self.stop();

        JsApi::dispatch_script_error(self, &err);
    }

    fn get_ctx_current_bytecode<'a>(&'a self, ctx: &'a BytecodeHandlerContext) -> &'a Bytecode {
        let scope = self.scopes.get(ctx.scope_ref).unwrap();
        let bytecode_index = scope.bytecode_index;
        let handler_def = unsafe { &*ctx.handler_def_ptr };
        handler_def.bytecode_array.get(bytecode_index).unwrap()
    }

    pub fn push_scope(&mut self) -> ScopeRef {
        if (self.scope_count + 1) as usize >= MAX_STACK_SIZE {
            panic!("Stack overflow");
        }
        let scope_ref = self.scope_count;
        let scope = self.scopes.get_mut(scope_ref as ScopeRef).unwrap();
        scope.reset();
        self.scope_count += 1;
        scope_ref as ScopeRef
    }

    pub fn pop_scope(&mut self) {
        self.scope_count -= 1;
    }

    pub fn current_scope_ref(&self) -> ScopeRef {
        (self.scope_count - 1) as ScopeRef
    }

    // Lingo: sound(channelNum)
    pub fn get_sound_channel(&mut self, channel_num: i32) -> Result<DatumRef, ScriptError> {
        // if channel_num < 1 || channel_num > self.sound_manager.num_channels() as i32 {
        //     return Err(ScriptError::new(format!(
        //         "Sound channel {} out of range (1-{})",
        //         channel_num,
        //         self.sound_manager.num_channels()
        //     )));
        // }

        // Channel numbers in Lingo are 1-based, convert to 0-based index
        let channel_idx = (channel_num - 1) as usize;
        Ok(self.alloc_datum(Datum::SoundChannel(channel_idx as u16)))
    }

    // Lingo: puppetSound channelNum, memberRef
    pub fn puppet_sound(
        &mut self,
        channel_num: i32,
        member_ref: DatumRef,
    ) -> Result<(), ScriptError> {
        let sound_channel = self.get_sound_channel(channel_num)?;
        SoundChannelDatumHandlers::handle_play_file(self, &sound_channel, &member_ref)
    }

    // Lingo: sound stop channelNum
    pub fn sound_stop(&mut self, channel_num: i32) -> Result<(), ScriptError> {
        let sound_channel = self.get_sound_channel(channel_num)?;
        SoundChannelDatumHandlers::handle_stop(self, &sound_channel)
    }

    pub fn load_sound_member(&self, member_ref: &DatumRef) -> Result<AudioData, ScriptError> {
        // TODO: Get the cast member from your cast storage
        // let cast_member = self.get_cast_member(member_ref)?;

        // Get the raw sound data
        // let sound_data = cast_member.get_sound_data()?;

        // Load and decode it
        // load_director_sound(sound_data)
        //     .map_err(|e| ScriptError::new(format!("Failed to load sound: {}", e)))

        Err(ScriptError::new("Not implemented".to_string()))
    }

    pub fn has_custom_font(&self, font_name: &str) -> bool {
        if font_name.is_empty() || font_name == "System" {
            return false;
        }
        self.font_manager.get_font_immutable(font_name).is_some()
    }

    pub fn list_available_fonts(&self) -> Vec<String> {
        let mut fonts: Vec<String> = self
            .font_manager
            .font_cache
            .keys()
            .map(|k| k.clone())
            .collect();
        fonts.sort();
        fonts
    }
}

pub fn player_alloc_datum(datum: Datum) -> DatumRef {
    // let mut player_opt = PLAYER_LOCK.try_write().unwrap();
    unsafe {
        let player = PLAYER_OPT.as_mut().unwrap();
        player.alloc_datum(datum)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScriptErrorCode {
    HandlerNotFound,
    Generic,
}

#[derive(Debug)]
pub struct ScriptError {
    pub code: ScriptErrorCode,
    pub message: String,
}

impl ScriptError {
    pub fn new(message: String) -> ScriptError {
        Self::new_code(ScriptErrorCode::Generic, message)
    }

    pub fn new_code(code: ScriptErrorCode, message: String) -> ScriptError {
        ScriptError { code, message }
    }
}

pub fn player_handle_scope_return(scope: &ScopeResult) {
    if scope.passed {
        reserve_player_mut(|player| {
            let scope_ref = player.current_scope_ref();
            let last_scope = player.scopes.get_mut(scope_ref);
            if let Some(last_scope) = last_scope {
                last_scope.passed = true;
            }
        });
    }
}

async fn player_call_global_handler(
    handler_name: &String,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let receiver_handler = unsafe {
        // let player_opt = PLAYER_LOCK.try_read().unwrap();
        let player = PLAYER_OPT.as_ref().unwrap();

        let mut receiver_handler = None;

        // "new" invocations should always go through the built-in handler
        if handler_name != "new" {
            // Director appears to support customFunc(firstArg, ..) invocations
            // where firstArg is a script or script instance
            receiver_handler = ScriptInstanceUtils::get_handler_from_first_arg(&args, handler_name);

            if receiver_handler.is_none() {
                receiver_handler = player
                    .movie
                    .score
                    .get_active_script_instance_list()
                    .iter()
                    .find_map(|instance_receiver_ref| {
                        let script_instance =
                            player.allocator.get_script_instance(instance_receiver_ref);
                        let script = player
                            .movie
                            .cast_manager
                            .get_script_by_ref(&script_instance.script)
                            .unwrap();
                        script
                            .get_own_handler_ref(&handler_name)
                            .map(|handler_pair| (Some(instance_receiver_ref.clone()), handler_pair))
                    });
            }

            if receiver_handler.is_none() {
                receiver_handler =
                    get_active_static_script_refs(&player.movie, &player.get_hydrated_globals())
                        .iter()
                        .find_map(|script_ref| {
                            let script = player.movie.cast_manager.get_script_by_ref(script_ref);
                            script
                                .and_then(|x| x.get_own_handler_ref(&handler_name))
                                .map(|handler_pair| (None, handler_pair))
                        });
            }
        }

        receiver_handler
    };

    if let Some(receiver_handler) = receiver_handler {
        let receiver = receiver_handler.0;
        let handler_ref = receiver_handler.1;
        let scope =
            player_call_script_handler_raw_args(receiver, handler_ref.to_owned(), args, true)
                .await?;
        player_handle_scope_return(&scope);
        return Ok(scope.return_value);
    } else if BuiltInHandlerManager::has_async_handler(handler_name) {
        return BuiltInHandlerManager::call_async_handler(handler_name, args).await;
    } else {
        return BuiltInHandlerManager::call_handler(handler_name, args);
    }
}

pub fn reserve_player_ref<T, F>(callback: F) -> T
where
    F: FnOnce(&DirPlayer) -> T,
{
    // let player_opt = PLAYER_LOCK.try_read().unwrap();
    // let player = player_opt.as_ref().unwrap();
    // callback(player)
    unsafe {
        let player = PLAYER_OPT.as_ref().unwrap();
        callback(player)
    }
}

pub fn reserve_player_mut<T, F>(callback: F) -> T
where
    F: FnOnce(&mut DirPlayer) -> T,
{
    // let mut player_opt = PLAYER_LOCK.try_write().unwrap();
    // let player = player_opt.as_mut().unwrap();
    unsafe {
        let player = PLAYER_OPT.as_mut().unwrap();
        callback(player)
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum ScriptReceiver {
    Script(CastMemberRef),
    ScriptInstance(ScriptInstanceRef),
}

pub async fn player_call_script_handler(
    receiver: Option<ScriptInstanceRef>,
    handler_ref: ScriptHandlerRef,
    arg_list: &Vec<DatumRef>,
) -> Result<ScopeResult, ScriptError> {
    player_call_script_handler_raw_args(receiver, handler_ref, arg_list, false).await
}

pub async fn player_call_script_handler_raw_args(
    receiver: Option<ScriptInstanceRef>,
    handler_ref: ScriptHandlerRef,
    arg_list: &Vec<DatumRef>,
    use_raw_arg_list: bool,
) -> Result<ScopeResult, ScriptError> {
    let (script_member_ref, handler_name) = &handler_ref;
    let (scope_ref, handler_ptr, script_ptr) = reserve_player_mut(|player| {
        let (script_ptr, handler_ptr, handler_name_id, script_type) = {
            let script_rc = player
                .movie
                .cast_manager
                .get_script_by_ref(&script_member_ref)
                .unwrap();
            let script = script_rc.as_ref();
            let script_ptr = script as *const Script;
            let handler = script.get_own_handler(&handler_name);

            if let Some(handler_rc) = handler {
                let handler_name_id = handler_rc.name_id;
                let handler_ptr: *const HandlerDef = handler_rc.as_ref();
                Ok((script_ptr, handler_ptr, handler_name_id, script.script_type))
            } else {
                Err(ScriptError::new_code(
                    ScriptErrorCode::HandlerNotFound,
                    format!(
                        "Handler {handler_name} not found for script {}",
                        script.name
                    ),
                ))
            }
        }?;

        let receiver_arg = if let Some(script_instance_ref) = receiver.as_ref() {
            Some(Datum::ScriptInstanceRef(script_instance_ref.clone()))
        } else if script_type != ScriptType::Movie {
            // TODO: check if this is right
            Some(Datum::ScriptRef(handler_ref.0.clone()))
        } else {
            None
        };

        let scope_ref = player.push_scope();
        {
            let scope = player.scopes.get_mut(scope_ref).unwrap();
            scope.script_ref = script_member_ref.clone();
            scope.receiver = receiver;
            scope.handler_name_id = handler_name_id;
        };

        if let Some(receiver_arg) = receiver_arg {
            if !use_raw_arg_list {
                let arg_ref = player.alloc_datum(receiver_arg);
                let scope = player.scopes.get_mut(scope_ref).unwrap();
                scope.args.push(arg_ref);
            }
        }

        let scope = player.scopes.get_mut(scope_ref).unwrap();
        scope.args.extend_from_slice(arg_list);

        Ok((scope_ref, handler_ptr, script_ptr))
    })?;

    let ctx = BytecodeHandlerContext {
        scope_ref,
        handler_def_ptr: handler_ptr,
        script_ptr,
    };

    let mut should_return = false;

    loop {
        let bytecode_index =
            reserve_player_ref(|player| player.scopes.get(scope_ref).unwrap().bytecode_index);
        // let profile_token = start_profiling(get_opcode_name(&bytecode.opcode));
        if let Some(breakpoint) = reserve_player_ref(|player| {
            player
                .breakpoint_manager
                .find_breakpoint_for_bytecode(
                    unsafe { &(&*script_ptr).name },
                    &handler_name,
                    bytecode_index,
                )
                .cloned()
        }) {
            player_trigger_breakpoint(
                breakpoint,
                script_member_ref.to_owned(),
                handler_ref.to_owned(),
                bytecode_index,
            )
            .await;
        }
        let result = player_execute_bytecode(&ctx).await?; // TODO catch error

        match result {
            HandlerExecutionResult::Advance => {
                reserve_player_mut(|player| {
                    player.scopes.get_mut(scope_ref).unwrap().bytecode_index += 1;
                });
            }
            HandlerExecutionResult::Stop => {
                should_return = true;
            }
            HandlerExecutionResult::Error(err) => {
                return Err(err);
            }
            HandlerExecutionResult::Jump => {}
        }

        // end_profiling(profile_token);

        if should_return {
            break;
        }
    }

    let scope = reserve_player_mut(|player| {
        let result = {
            let scope = player.scopes.get(scope_ref).unwrap();
            player.last_handler_result = scope.return_value.clone();

            ScopeResult {
                passed: scope.passed,
                return_value: scope.return_value.clone(),
            }
        };
        player.pop_scope();
        result
    });

    return Ok(scope);
}

pub async fn run_frame_loop() {
    let mut fps: u32;
    unsafe {
        let player = PLAYER_OPT.as_ref().unwrap();
        if !player.is_playing {
            return;
        }
        fps = player.get_fps();
    }

    let mut is_playing = true;
    let mut is_script_paused = false;
    while is_playing {
        if !is_script_paused {
            player_wait_available().await;
            reserve_player_mut(|player| {
                // player.movie.score.begin_sprites(player.movie.current_frame);
                player.begin_all_sprites();
            });
            player_wait_available().await;
            player_unwrap_result(
                player_invoke_global_event(&"prepareFrame".to_string(), &vec![]).await,
            );
            player_unwrap_result(
                player_invoke_global_event(&"enterFrame".to_string(), &vec![]).await,
            );

            // Send stepFrame to all items in actorList
            let actor_list_snapshot = reserve_player_ref(|player| {
                let actor_list_ref = player
                    .globals
                    .get("actorList")
                    .unwrap_or(&DatumRef::Void)
                    .clone();
                let actor_list_datum = player.get_datum(&actor_list_ref);
                match actor_list_datum {
                    Datum::List(_, items, _) => items.clone(),
                    _ => vec![],
                }
            });

            // Call stepFrame on each actor in the snapshot
            for (idx, actor_ref) in actor_list_snapshot.iter().enumerate() {
                // Verify the actor is still in actorList
                let still_active = reserve_player_ref(|player| {
                    let actor_list_ref = player
                        .globals
                        .get("actorList")
                        .unwrap_or(&DatumRef::Void)
                        .clone();
                    let actor_list_datum = player.get_datum(&actor_list_ref);
                    match actor_list_datum {
                        Datum::List(_, items, _) => items.contains(&actor_ref),
                        _ => false,
                    }
                });

                if !still_active {
                    continue;
                }

                // Don't catch errors, let them propagate
                let result =
                    player_call_datum_handler(&actor_ref, &"stepFrame".to_string(), &vec![]).await;

                // Propagate the error
                if let Err(err) = result {
                    web_sys::console::log_1(
                        &format!("❌ stepFrame[{}] error: {}", idx, err.message).into(),
                    );
                    // Stop the frame loop and show the error
                    reserve_player_mut(|player| {
                        player.on_script_error(&err);
                    });
                    return; // Exit the frame loop
                }
            }
        }

        timeout(
            Duration::from_millis(1000 / fps as u64),
            future::pending::<()>(),
        )
        .await
        .unwrap_err();
        player_wait_available().await;

        let mut prev_frame = 0;
        let mut new_frame = 0;
        reserve_player_mut(|player| {
            is_playing = player.is_playing;
            is_script_paused = player.is_script_paused;
            fps = player.get_fps();
            if !player.is_playing {
                return;
            }
            prev_frame = player.movie.current_frame;
            if !player.is_script_paused {
                new_frame = player.get_next_frame();
            } else {
                new_frame = prev_frame;
            }
        });
        if !is_playing {
            return;
        }
        if new_frame > 1 && prev_frame <= 1 {
            unsafe {
                let player = PLAYER_OPT.as_mut().unwrap();
                player
                    .movie
                    .cast_manager
                    .preload_casts(
                        CastPreloadReason::AfterFrameOne,
                        &mut player.net_manager,
                        &mut player.bitmap_manager,
                        &mut player.dir_cache,
                    )
                    .await;
            }
        }
        if !is_script_paused {
            let frame_skipped =
                reserve_player_ref(|player| player.next_frame.is_some() || !player.is_playing);
            if !frame_skipped {
                player_unwrap_result(
                    player_invoke_global_event(&"exitFrame".to_string(), &vec![]).await,
                );
            }
            let ended_sprite_nums = reserve_player_mut(|player| {
                let next_frame = player.get_next_frame(); // an exitFrame handler may have changed the next frame
                                                          // player.movie.score.end_sprites(prev_frame, next_frame)
                player.end_all_sprites()
            });
            player_wait_available().await;
            reserve_player_mut(|player| {
                for (score_source, sprite_num) in ended_sprite_nums.iter() {
                    // let sprite = player.movie.score.get_sprite_mut(*sprite_num as i16);
                    if let Some(sprite) =
                        get_score_sprite_mut(&mut player.movie, score_source, *sprite_num as i16)
                    {
                        sprite.exited = true;
                    }
                }
            });
            (is_playing, is_script_paused) = reserve_player_mut(|player| {
                player.advance_frame();
                (player.is_playing, player.is_script_paused)
            });
        };
    }
}

pub async fn player_trigger_breakpoint(
    breakpoint: Breakpoint,
    script_ref: CastMemberRef,
    handler_ref: ScriptHandlerRef,
    bytecode_index: usize,
) {
    let (future, completer) = ManualFuture::new();
    let breakpoint_ctx = BreakpointContext {
        breakpoint,
        script_ref,
        handler_ref,
        bytecode_index,
        completer,
    };
    reserve_player_mut(|player| {
        player.current_breakpoint = Some(breakpoint_ctx);
        player.pause_script();
        JsApi::dispatch_scope_list(player);
    });
    future.await;
    reserve_player_mut(|player| {
        player.resume_script();
    });
}

pub async fn player_is_playing() -> bool {
    unsafe { PLAYER_OPT.as_ref().unwrap().is_playing }
}

static mut PLAYER_TX: Option<Sender<PlayerVMExecutionItem>> = None;
static mut PLAYER_EVENT_TX: Option<Sender<PlayerVMEvent>> = None;
pub static mut PLAYER_OPT: Option<DirPlayer> = None;

pub fn player_semaphone() -> &'static Mutex<()> {
    static MAP: OnceLock<Mutex<()>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(()))
}

pub fn init_player() {
    console_log::init_with_level(log::Level::Error).unwrap_or(());
    let (tx, rx) = channel::unbounded();
    let (event_tx, event_rx) = channel::unbounded();
    unsafe {
        PLAYER_TX = Some(tx.clone());
        PLAYER_EVENT_TX = Some(event_tx.clone());
        MULTIUSER_XTRA_MANAGER_OPT = Some(MultiuserXtraManager::new());
    }

    unsafe {
        PLAYER_OPT = Some(DirPlayer::new(tx));
    }
    // let mut player = //PLAYER_LOCK.try_write().unwrap();
    // *player = Some(DirPlayer::new(tx, allocator_rx, allocator_tx));

    async_std::task::spawn_local(async move {
        // player_load_system_font().await;
        async_std::task::spawn_local(async move {
            run_command_loop(rx).await;
        });
        async_std::task::spawn_local(async move {
            run_event_loop(event_rx).await;
        });
    });
}

fn get_active_static_script_refs<'a>(
    movie: &'a Movie,
    globals: &'a FxHashMap<String, &'a Datum>,
) -> Vec<CastMemberRef> {
    let frame_script = movie.score.get_script_in_frame(movie.current_frame);
    let movie_scripts = movie.cast_manager.get_movie_scripts();
    let movie_scripts = movie_scripts.as_ref().unwrap();

    let mut active_script_refs: Vec<CastMemberRef> = vec![];
    for script in movie_scripts {
        active_script_refs.push(script.member_ref.clone());
    }
    if let Some(frame_script) = frame_script {
        active_script_refs.push(CastMemberRef {
            cast_lib: frame_script.cast_lib.into(),
            cast_member: frame_script.cast_member.into(),
        });
    }
    for global in globals.values() {
        if let Datum::VarRef(VarRef::Script(script_ref)) = global {
            active_script_refs.push(script_ref.clone());
        }
    }
    return active_script_refs;
}

// #[allow(dead_code)]
// fn get_active_scripts<'a>(
//   movie: &'a Movie,
//   globals: &'a HashMap<String, Datum>
// ) -> Vec<&'a Rc<Script>> {
//   let frame_script = movie.score.get_script_in_frame(movie.current_frame);
//   let mut movie_scripts = movie.cast_manager.get_movie_scripts();

//   let mut active_scripts: Vec<&Rc<Script>> = vec![];
//   active_scripts.append(&mut movie_scripts);
//   if let Some(frame_script) = frame_script {
//     let script = movie.cast_manager
//       .get_cast(frame_script.cast_lib as u32)
//       .unwrap()
//       .get_script_for_member(frame_script.cast_member.into())
//       .unwrap();
//     active_scripts.push(script);
//   }
//   for global in globals.values() {
//     if let Datum::VarRef(VarRef::Script(script_ref)) = global {
//       active_scripts.push(
//         movie.cast_manager.get_script_by_ref(script_ref).unwrap()
//       );
//     }
//   }
//   return active_scripts;
// }

async fn player_ext_call<'a>(
    name: String,
    args: &Vec<DatumRef>,
    scope_ref: ScopeRef,
) -> (HandlerExecutionResult, DatumRef) {
    // let formatted_args: Vec<String> = reserve_player_ref(|player| {
    //   args.iter().map(|datum_ref| format_datum(*datum_ref, player)).collect()
    // });
    // warn!("ext_call: {name}({})", formatted_args.join(", "));
    match name.as_str() {
        "return" => {
            let return_value = if let Some(return_value) = args.first() {
                reserve_player_mut(|player| {
                    player.scopes.get_mut(scope_ref).unwrap().return_value = return_value.clone();
                });
                return_value.clone()
            } else {
                DatumRef::Void
            };
            (HandlerExecutionResult::Stop, return_value)
        }
        _ => {
            let result = player_call_global_handler(&name, args).await;

            match result {
                Ok(result_datum_ref) => {
                    reserve_player_mut(|player| {
                        player.last_handler_result = result_datum_ref.clone();
                        player.scopes.get_mut(scope_ref).unwrap().return_value =
                            result_datum_ref.clone();
                    });
                    (HandlerExecutionResult::Advance, result_datum_ref)
                }
                Err(err) => (HandlerExecutionResult::Error(err), DatumRef::Void),
            }
        }
    }
}

fn player_duplicate_datum(datum: &DatumRef) -> DatumRef {
    let datum_type = reserve_player_ref(|player| player.get_datum(datum).type_enum());
    let new_datum = match datum_type {
        DatumType::PropList => {
            let (props, sorted) = reserve_player_mut(|player| {
                let (props, sorted) = player.get_datum(datum).to_map_tuple().unwrap();
                (props.clone(), sorted)
            });
            let mut new_props = Vec::new();
            for (key, value) in props {
                let new_key = player_duplicate_datum(&key);
                let new_value = player_duplicate_datum(&value);
                new_props.push((new_key, new_value));
            }
            Datum::PropList(new_props, sorted)
        }
        DatumType::List => {
            let (list_type, list, sorted) = reserve_player_ref(|player| {
                let (a, b, c) = player.get_datum(datum).to_list_tuple().unwrap();
                (a.clone(), b.clone(), c)
            });
            let mut new_list = Vec::new();
            for item in list {
                let new_item = player_duplicate_datum(&item);
                new_list.push(new_item);
            }
            Datum::List(list_type.clone(), new_list, sorted)
        }
        DatumType::BitmapRef => reserve_player_mut(|player| {
            let bitmap_ref = player.get_datum(datum).to_bitmap_ref().unwrap();
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();
            let new_bitmap = bitmap.clone();
            let new_bitmap_ref = player.bitmap_manager.add_bitmap(new_bitmap);
            Datum::BitmapRef(new_bitmap_ref)
        }),
        _ => reserve_player_ref(|player| player.get_datum(datum).clone()),
    };
    let new_datum_ref = player_alloc_datum(new_datum);
    new_datum_ref
}
