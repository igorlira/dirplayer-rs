pub mod net_manager;
pub mod movie;
pub mod geometry;
pub mod cast_manager;
pub mod cast_lib;
pub mod net_task;
pub mod cast_member;
pub mod score;
pub mod sprite;
pub mod script;
pub mod scope;
pub mod bytecode;
pub mod handlers;
pub mod debug;
pub mod compare;
pub mod datum_operations;
pub mod eval;
pub mod datum_formatting;
pub mod context_vars;
pub mod profiling;
pub mod stage;
pub mod bitmap;
pub mod timeout;
pub mod xtra;
pub mod font;
pub mod commands;
pub mod events;
pub mod keyboard;
pub mod keyboard_map;
pub mod keyboard_events;

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_std::{channel::{self, Sender}, future::{self, timeout}, sync::Mutex, task::spawn_local};
use chrono::Local;
use manual_future::{ManualFutureCompleter, ManualFuture};
use net_manager::NetManager;
use lazy_static::lazy_static;
use nohash_hasher::IntMap;

use crate::{console_warn, director::{chunks::handler::Bytecode, enums::ScriptType, file::{read_director_file_bytes, DirectorFile}, lingo::{constants::{get_anim2_prop_name, get_anim_prop_name}, datum::{datum_bool, Datum, DatumType, VarRef}}}, js_api::JsApi, player::{bytecode::handler_manager::{player_execute_bytecode, BytecodeHandlerContext}, datum_formatting::format_datum, geometry::IntRect, profiling::get_profiler_report, scope::Scope}, utils::{get_base_url, get_basename_no_extension}};

use self::{bytecode::handler_manager::StaticBytecodeHandlerManager, cast_lib::CastMemberRef, cast_manager::CastManager, commands::{run_command_loop, PlayerVMCommand}, debug::{Breakpoint, BreakpointContext, BreakpointManager}, events::{player_dispatch_global_event, player_invoke_global_event, player_unwrap_result, player_wait_available, run_event_loop, PlayerVMEvent}, font::{player_load_system_font, FontManager}, handlers::manager::BuiltInHandlerManager, keyboard::KeyboardManager, movie::Movie, net_manager::NetManagerSharedState, scope::ScopeRef, score::{get_sprite_at, Score}, script::{Script, ScriptHandlerRef, ScriptInstance, ScriptInstanceId}, sprite::{ColorRef, CursorRef}, timeout::TimeoutManager};

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

pub struct DirPlayer {
  pub net_manager: NetManager,
  pub movie: Movie,
  pub is_playing: bool,
  pub is_script_paused: bool,
  pub next_frame: Option<u32>,
  pub queue_tx: Sender<PlayerVMExecutionItem>,
  pub globals: HashMap<String, DatumRef>,
  pub script_instances: IntMap<ScriptInstanceId, ScriptInstance>,
  pub script_id_counter: u32,
  pub scopes: Vec<Scope>,
  pub bytecode_handler_manager: StaticBytecodeHandlerManager,
  pub datums: DatumRefMap,
  datum_id_counter: u32,
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
  pub mouse_down_sprite: i16,
  pub subscribed_member_refs: Vec<CastMemberRef>, // TODO move to debug module
  pub is_subscribed_to_channel_names: bool, // TODO move to debug module
  pub font_manager: FontManager,
  pub keyboard_manager: KeyboardManager,
  pub float_precision: u8,
  pub last_handler_result: DatumRef,
  pub hovered_sprite: Option<i16>,
  pub timer_tick_start: u32,
}

pub type DatumRef = u32;
pub type DatumRefMap = IntMap<DatumRef, Datum>;
pub static VOID_DATUM_REF: DatumRef = 0;

impl DirPlayer {
  pub fn new<'a>(tx: Sender<PlayerVMExecutionItem>) -> DirPlayer {
    DirPlayer {
      movie: Movie { 
        rect: IntRect::from(0, 0, 0, 0),
        cast_manager: CastManager::empty(),
        score: Score::empty(),
        current_frame: 1,
        puppet_tempo: 0,
        exit_lock: false,
        dir_version: 0,
        item_delimiter: ".".to_string(),
        alert_hook: None,
        base_path: "".to_string(),
        stage_color: (0, 0, 0),
      },
      net_manager: NetManager {
        base_path: None,
        tasks: HashMap::new(),
        task_states: HashMap::new(),
        shared_state: Arc::new(Mutex::new(NetManagerSharedState::new()))
      },
      is_playing: false,
      is_script_paused: false,
      next_frame: None,
      queue_tx: tx,
      globals: HashMap::new(),
      script_instances: IntMap::default(),
      script_id_counter: 0,
      scopes: vec![],
      bytecode_handler_manager: StaticBytecodeHandlerManager {},
      datums: IntMap::default(),
      datum_id_counter: 0,
      breakpoint_manager: BreakpointManager::new(),
      current_breakpoint: None,
      stage_size: (100, 100),
      bitmap_manager: bitmap::manager::BitmapManager::new(),
      cursor: CursorRef::System(0),
      start_time: chrono::Local::now(),
      timeout_manager: TimeoutManager::new(),
      title: "".to_string(),
      bg_color: ColorRef::Rgb(0, 0, 0),
      keyboard_focus_sprite: -1, // Setting keyboardFocusSprite to -1 returns keyboard focus control to the Score, and setting it to 0 disables keyboard entry into any editable sprite.
      mouse_loc: (0, 0),
      mouse_down_sprite: 0,
      subscribed_member_refs: vec![],
      is_subscribed_to_channel_names: false,
      font_manager: FontManager::new(),
      keyboard_manager: KeyboardManager::new(),
      text_selection_start: 0,
      text_selection_end: 0,
      float_precision: 4,
      last_handler_result: VOID_DATUM_REF,
      hovered_sprite: None,
      timer_tick_start: get_ticks(),
    }
  }

  pub async fn load_movie_from_file(&mut self, path: &str) -> DirectorFile  {
    let task_id = self.net_manager.preload_net_thing(path.to_owned());
    self.net_manager.await_task(task_id).await;
    let task = self.net_manager.get_task(task_id).unwrap();
    let data_bytes = self.net_manager.get_task_result(Some(task_id)).unwrap().unwrap();

    let movie_file = read_director_file_bytes(
      &data_bytes, 
      &get_basename_no_extension(task.resolved_url.path()), 
      &get_base_url(&task.resolved_url).to_string(),
    ).unwrap();
    self.load_movie_from_dir(&movie_file).await;
    return movie_file;
  }

  async fn load_movie_from_dir(&mut self, dir: &DirectorFile) {
    self.movie.load_from_file(&dir, &mut self.net_manager, &mut self.bitmap_manager).await;
    let (r, g, b) = self.movie.stage_color;
    self.bg_color = ColorRef::Rgb(r, g, b);
    JsApi::dispatch_movie_loaded(&dir);
  }

  pub fn play(&mut self) {
    if self.is_playing {
      return;
    }
    self.is_playing = true;
    self.is_script_paused = false;
    // TODO runVM()
    async_std::task::spawn_local(async move {
      if let Err(err) = player_invoke_global_event(&"prepareMovie".to_string(), &vec![]).await {
        reserve_player_mut(|player| player.on_script_error(&err));
        return;
      }
      run_frame_loop(PLAYER_LOCK.clone()).await;
    });
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

  pub fn get_datum(&self, id: DatumRef) -> &Datum {
    get_datum(id, &self.datums)
  }

  pub fn reserve_script_instance_id(&mut self) -> ScriptInstanceId {
    self.script_id_counter += 1;
    let id = self.script_id_counter;
    id
  }

  pub fn get_datum_mut(&mut self, id: DatumRef) -> &mut Datum {
    self.datums.get_mut(&id).unwrap()
  }

  pub fn get_fps(&self) -> u32 {
    if self.movie.puppet_tempo > 0 { self.movie.puppet_tempo } else { 1 }
  }

  pub fn get_hydrated_globals(&self) -> HashMap<String, &Datum> {
    self.globals.iter().map(|(k, v)| (k.to_owned(), self.datums.get(v).unwrap())).collect()
  }

  #[allow(dead_code)]
  pub fn get_global(&self, name: &String) -> Option<&Datum> {
    self.globals.get(name).map(|datum_ref| get_datum(*datum_ref, &self.datums))
  }

  pub fn advance_frame(&mut self) {
    if !self.is_playing {
      return;
    }
    let prev_frame = self.movie.current_frame;
    if let Some(next_frame) = self.next_frame {
      self.movie.current_frame = next_frame;
      self.next_frame = None;
    } else {
      self.movie.current_frame += 1;
    }
    if prev_frame != self.movie.current_frame {
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
  }

  pub fn reset(&mut self) {
    self.stop();
    self.scopes.clear();
    self.globals.clear();
    self.datums.clear();
    self.script_instances.clear();
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
    if datum.is_void() {
      return VOID_DATUM_REF;
    }
    
    self.datum_id_counter += 1;
    let id = self.datum_id_counter;
    self.datums.insert(id, datum);
    id
  }

  fn get_movie_prop(&self, prop: &String) -> Result<Datum, ScriptError> {
    match prop.as_str() {
      "stage" => Ok(Datum::Stage),
      "milliSeconds" => Ok(Datum::Int(chrono::Local::now().signed_duration_since(self.start_time).num_milliseconds() as i32)),
      "keyboardFocusSprite" => Ok(Datum::Int(self.keyboard_focus_sprite as i32)),
      "frameTempo" => Ok(Datum::Int(self.movie.puppet_tempo as i32)),
      "mouseLoc" => Ok(Datum::IntPoint(self.mouse_loc)),
      "mouseH" => Ok(Datum::Int(self.mouse_loc.0 as i32)),
      "mouseV" => Ok(Datum::Int(self.mouse_loc.1 as i32)),
      "rollover" => {
        let sprite = get_sprite_at(self, self.mouse_loc.0, self.mouse_loc.1, false);
        Ok(Datum::Int(sprite.unwrap_or(0) as i32))
      }
      "keyCode" => Ok(Datum::Int(self.keyboard_manager.key_code() as i32)),
      "shiftDown" => Ok(datum_bool(self.keyboard_manager.is_shift_down())),
      "optionDown" => Ok(datum_bool(self.keyboard_manager.is_alt_down())), // TODO: return true only on mac
      "commandDown" => Ok(datum_bool(self.keyboard_manager.is_command_down())),
      "controlDown" => Ok(datum_bool(self.keyboard_manager.is_control_down())),
      "altDown" => Ok(datum_bool(self.keyboard_manager.is_alt_down())),
      "key" => Ok(Datum::String(self.keyboard_manager.key())),
      "floatPrecision" => Ok(Datum::Int(self.float_precision as i32)),
      "doubleClick" => Ok(datum_bool(false)), // TODO
      _ => self.movie.get_prop(prop),
    }
  }

  fn get_anim_prop(&self, prop_id: u16) -> Result<Datum, ScriptError> {
    let prop_name = get_anim_prop_name(prop_id);
    match prop_name.as_str() {
      "colorDepth" => Ok(Datum::Int(32)),
      "timer" => Ok(Datum::Int(get_ticks() as i32 - self.timer_tick_start as i32)),
      _ => Err(ScriptError::new(format!("Unknown anim prop {}", prop_name)))
    }
  }

  fn get_anim2_prop(&self, prop_id: u16) -> Result<Datum, ScriptError> {
    let prop_name = get_anim2_prop_name(prop_id);
    match prop_name.as_str() {
      "number of castLibs" => Ok(Datum::Int(self.movie.cast_manager.casts.len() as i32)),
      _ => Err(ScriptError::new(format!("Unknown anim2 prop {}", prop_name)))
    }
  }

  fn set_movie_prop(&mut self, prop: &String, value: Datum) -> Result<(), ScriptError> {
    match prop.as_str() {
      "keyboardFocusSprite" => {
        // TODO switch focus
        self.keyboard_focus_sprite = value.int_value(&self.datums)? as i16;
        Ok(())
      },
      "selStart" => {
        self.text_selection_start = value.int_value(&self.datums)? as u16;
        Ok(())
      },
      "selEnd" => {
        self.text_selection_end = value.int_value(&self.datums)? as u16;
        Ok(())
      },
      "floatPrecision" => {
        self.float_precision = value.int_value(&self.datums)? as u8;
        Ok(())
      },
      _ => {
        self.movie.set_prop(prop, value, &self.datums)
      }
    }
  }

  fn on_script_error(&mut self, err: &ScriptError) {
    console_warn!("[!!] play failed with error: {}", err.message);
    console_warn!("Profiler report: {}", get_profiler_report());
    self.stop();

    JsApi::dispatch_script_error(self, &err);
  }
}

pub fn player_alloc_datum(datum: Datum) -> DatumRef {
  let mut player_opt = PLAYER_LOCK.try_lock().unwrap();
  let player = player_opt.as_mut().unwrap();
  player.alloc_datum(datum)
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScriptErrorCode {
  HandlerNotFound,
  Generic
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

pub fn player_handle_scope_return(scope: &Scope) {
  if scope.passed {
    reserve_player_mut(|player| {
      let last_scope = player.scopes.last_mut();
      if let Some(last_scope) = last_scope {
        last_scope.passed = true;
      }
    });
  }
}

async fn player_call_global_handler(handler_name: &String, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
  let receiver_handlers: Vec<ScriptHandlerRef> = {
    let player_opt = PLAYER_LOCK.try_lock().unwrap();
    let player = player_opt.as_ref().unwrap();
  
    let receiver_refs = get_active_static_script_refs(&player.movie, &player.get_hydrated_globals());

    let mut result = vec![];
    for script_ref in receiver_refs {
      let script = player.movie.cast_manager.get_script_by_ref(&script_ref).unwrap();
      let handler_pair = script.get_own_handler_ref(&handler_name);

      if let Some(handler_pair) = handler_pair {
        //player_dispatch_async(PlayerVMCommand::CallHandler).await;
        result.push(handler_pair);
      }
    }
    result
  };

  if let Some(handler_ref) = receiver_handlers.first() {
    let scope = player_call_script_handler(None, handler_ref.to_owned(), args).await?;
    player_handle_scope_return(&scope);
    return Ok(scope.return_value);
  } else if BuiltInHandlerManager::has_async_handler(handler_name) {
    return BuiltInHandlerManager::call_async_handler(handler_name, args).await;
  } else {
    return BuiltInHandlerManager::call_handler(handler_name, args);
  }
}

pub fn reserve_player_ref<T, F>(callback: F) -> T where F: FnOnce(&DirPlayer) -> T {
  let player_opt = PLAYER_LOCK.try_lock().unwrap();
  let player = player_opt.as_ref().unwrap();
  callback(player)
}

pub fn reserve_player_mut<T, F>(callback: F) -> T where F: FnOnce(&mut DirPlayer) -> T {
  let mut player_opt = PLAYER_LOCK.try_lock().unwrap();
  let player = player_opt.as_mut().unwrap();
  callback(player)
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum ScriptReceiver {
  Script(CastMemberRef),
  ScriptInstance(ScriptInstanceId),
}

pub async fn player_call_script_handler(
  receiver: Option<ScriptInstanceId>, 
  handler_ref: ScriptHandlerRef,
  arg_list: &Vec<DatumRef>,
) -> Result<Scope, ScriptError> {
  player_call_script_handler_raw_args(receiver, handler_ref, arg_list, false).await
}

pub async fn player_call_script_handler_raw_args(
  receiver: Option<ScriptInstanceId>, 
  handler_ref: ScriptHandlerRef,
  arg_list: &Vec<DatumRef>,
  use_raw_arg_list: bool,
) -> Result<Scope, ScriptError> {
  let handler_ref_clone = handler_ref.clone();
  let (script_member_ref, handler_name) = handler_ref.to_owned();
  let script_type = reserve_player_ref(|player| {
    let script = player.movie.cast_manager.get_script_by_ref(&script_member_ref).unwrap();
    script.script_type
  });
  let new_args = reserve_player_mut(|player| {
    let mut new_args = vec![];
    let receiver_arg = if let Some(script_instance_id) = receiver {
      Some(Datum::ScriptInstanceRef(script_instance_id))
    } else if script_type != ScriptType::Movie {
      // TODO: check if this is right
      Some(Datum::ScriptRef(handler_ref.0.clone()))
    } else {
      None
    };
    if let Some(receiver_arg) = receiver_arg {
      if !use_raw_arg_list {
        new_args.push(player.alloc_datum(receiver_arg));
      }
    }
    new_args.extend(arg_list);
    new_args
  });

  let (bytecode_array, scope_ref, script_name) = reserve_player_mut(|player| {
    let script = player.movie.cast_manager.get_script_by_ref(&script_member_ref).unwrap();
    let handler = script.get_own_handler(&handler_name);
    if let Some(handler) = handler {
      // let scope_ref = player.scope_id_counter + 1;
      let scope_ref = player.scopes.len();
      let scope = Scope::new(
        scope_ref,
        script_member_ref.clone(), 
        receiver, 
        handler_ref_clone, 
        new_args,
      );
      if player.scopes.len() >= 50 {
        return Err(ScriptError::new("Stack overflow".to_string()));
      }
      // player.scope_id_counter += 1;
      player.scopes.push(scope);
  
      Ok((handler.bytecode_array.to_owned(), scope_ref, script.name.to_owned()))
    } else {
      Err(ScriptError::new_code(ScriptErrorCode::HandlerNotFound, format!("Handler {handler_name} not found for script {}", script.name)))
    }
  })?;

  let ctx = BytecodeHandlerContext {
    scope_ref,
  };

  // self.scopes.push(scope);
  // let scope = self.scopes.last().unwrap();
  let mut should_return = false;

  loop {
    let bytecode_index = reserve_player_ref(|player| player.scopes.get(scope_ref).unwrap().bytecode_index);
    let bytecode = bytecode_array.get(bytecode_index).unwrap();
    // let profile_token = start_profiling(get_opcode_name(&bytecode.opcode));
    // TODO breakpoint
    if let Some(breakpoint) = reserve_player_ref(|player| {
      player.breakpoint_manager
        .find_breakpoint_for_bytecode(&script_name, &handler_name, bytecode_index)
        .cloned()
    }) {
      player_trigger_breakpoint(breakpoint, script_member_ref.to_owned(), handler_ref.to_owned(), bytecode.to_owned()).await;
    }
    let context = player_execute_bytecode(&bytecode, &ctx).await?; // TODO catch error
    let result = context.result;

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
    let result = player.scopes.pop().unwrap();
    player.last_handler_result = result.return_value;
    result
  });

  return Ok(scope);
}

pub async fn run_frame_loop(player_arc: Arc<Mutex<Option<DirPlayer>>>) {
  let mut fps: u32;
  {
    let player_opt = player_arc.try_lock().unwrap();
    let player = player_opt.as_ref().unwrap();
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
      player_unwrap_result(player_invoke_global_event(&"prepareFrame".to_string(), &vec![]).await);
      player_unwrap_result(player_invoke_global_event(&"enterFrame".to_string(), &vec![]).await);
    }
    timeout(Duration::from_millis(1000 / fps as u64), future::pending::<()>()).await.unwrap_err();
    player_wait_available().await;
    reserve_player_mut(|player| {
      is_playing = player.is_playing;
      is_script_paused = player.is_script_paused;
      fps = player.get_fps();
      if !player.is_playing {
        return;
      }
      if !player.is_script_paused {
        player.advance_frame();
      }
    });
    if !is_playing {
      return;
    }
    if !is_script_paused {
      let frame_skipped = reserve_player_ref(|player| {
        player.next_frame.is_some() || !player.is_playing
      });
      if !frame_skipped {
        // TODO only call this after timeout completes
        player_unwrap_result(player_invoke_global_event(&"exitFrame".to_string(), &vec![]).await);
      }
    };   
  }
}

pub async fn player_trigger_breakpoint(breakpoint: Breakpoint, script_ref: CastMemberRef, handler_ref: ScriptHandlerRef, bytecode: Bytecode) {
  let (future, completer) = ManualFuture::new();
  let breakpoint_ctx = BreakpointContext {
    breakpoint,
    script_ref,
    handler_ref,
    bytecode,
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
  let player_opt = PLAYER_LOCK.lock().await;
  let player = player_opt.as_ref().unwrap();
  player.is_playing
}

static mut PLAYER_TX: Option<Sender<PlayerVMExecutionItem>> = None;
static mut PLAYER_EVENT_TX: Option<Sender<PlayerVMEvent>> = None;
lazy_static! {
  pub static ref PLAYER_LOCK: Arc<Mutex<Option<DirPlayer>>> = Arc::new(Mutex::new(None));
  pub static ref PLAYER_SEMAPHONE: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

pub fn init_player() {
  let (tx, rx) = channel::unbounded();
  let (event_tx, event_rx) = channel::unbounded();
  unsafe { 
    PLAYER_TX = Some(tx.clone()); 
    PLAYER_EVENT_TX = Some(event_tx.clone());
  }

  let mut player = PLAYER_LOCK.try_lock().unwrap();
  *player = Some(DirPlayer::new(tx));

  async_std::task::spawn_local(async move {
    player_load_system_font().await;
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
  globals: &'a HashMap<String, &'a Datum>,
) -> Vec<CastMemberRef> {
  let frame_script = movie.score.get_script_in_frame(movie.current_frame);
  let movie_scripts = movie.cast_manager.get_movie_scripts();

  let mut active_script_refs: Vec<CastMemberRef> = vec![];
  for script in movie_scripts {
    active_script_refs.push(script.member_ref.clone());
  }
  if let Some(frame_script) = frame_script {
    active_script_refs.push(CastMemberRef { cast_lib: frame_script.cast_lib.into(), cast_member: frame_script.cast_member.into() });
  }
  for global in globals.values() {
    if let Datum::VarRef(VarRef::Script(script_ref)) = global {
      active_script_refs.push(script_ref.clone());
    }
  }
  return active_script_refs;
}

#[allow(dead_code)]
fn get_active_scripts<'a>(
  movie: &'a Movie, 
  globals: &'a HashMap<String, Datum>
) -> Vec<&'a Script> {
  let frame_script = movie.score.get_script_in_frame(movie.current_frame);
  let mut movie_scripts = movie.cast_manager.get_movie_scripts();

  let mut active_scripts: Vec<&Script> = vec![];
  active_scripts.append(&mut movie_scripts);
  if let Some(frame_script) = frame_script {
    let script = movie.cast_manager
      .get_cast(frame_script.cast_lib as u32)
      .unwrap()
      .get_script_for_member(frame_script.cast_member.into())
      .unwrap();
    active_scripts.push(script);
  }
  for global in globals.values() {
    if let Datum::VarRef(VarRef::Script(script_ref)) = global {
      active_scripts.push(
        movie.cast_manager.get_script_by_ref(script_ref).unwrap()
      );
    }
  }
  return active_scripts;
}

pub fn get_datum(id: DatumRef, datums: &DatumRefMap) -> &Datum {
  if id == VOID_DATUM_REF {
    return &Datum::Void;
  }
  datums.get(&id).unwrap()
}

async fn player_ext_call<'a>(name: String, args: &Vec<DatumRef>, scope_ref: ScopeRef) -> HandlerExecutionResultContext {
  // let formatted_args: Vec<String> = reserve_player_ref(|player| {
  //   args.iter().map(|datum_ref| format_datum(*datum_ref, player)).collect()
  // });
  // console_warn!("ext_call: {name}({})", formatted_args.join(", "));
  match name.as_str() {
    "return" => {
      if let Some(return_value) = args.first() {
        reserve_player_mut(|player| {
          player.scopes.get_mut(scope_ref).unwrap().return_value = *return_value;
        });
      }

      HandlerExecutionResultContext { result: HandlerExecutionResult::Stop }
    }
    _ => {
      let result = player_call_global_handler(&name, args).await;

      match result {
        Ok(result) => {
          reserve_player_mut(|player| {
            player.last_handler_result = result;
            player.scopes.get_mut(scope_ref).unwrap().return_value = result;
          });
          HandlerExecutionResultContext { result: HandlerExecutionResult::Advance }
        }
        Err(err) => {
          HandlerExecutionResultContext { 
            result: HandlerExecutionResult::Error(err), 
          }
        }
      }
    }
  }
}

fn get_ticks() -> u32 {
  let time = Local::now();
  // 60 ticks per second
  let millis = time.timestamp_millis();
  (millis as f32 / (1000.0 / 60.0)) as u32
}

fn player_duplicate_datum(datum: DatumRef) -> DatumRef {
  let datum_type = reserve_player_ref(|player| {
    player.get_datum(datum).type_enum()
  });
  let new_datum = match datum_type {
    DatumType::PropList => {
      let props = reserve_player_mut(|player| {
        player.get_datum(datum).to_map().unwrap().clone()
      });
      let mut new_props = Vec::new();
        for (key, value) in props {
          let new_key = player_duplicate_datum(key);
          let new_value = player_duplicate_datum(value);
          new_props.push((new_key, new_value));
        }
        Datum::PropList(new_props)
    },
    DatumType::List => {
      let (list_type, list, sorted) = reserve_player_ref(|player| {
        let (a, b, c) = player.get_datum(datum).to_list_tuple().unwrap();
        (a.clone(), b.clone(), c)
      });
      let mut new_list = Vec::new();
      for item in list {
        let new_item = player_duplicate_datum(item);
        new_list.push(new_item);
      }
      Datum::List(list_type.clone(), new_list, sorted)
    },
    DatumType::BitmapRef => reserve_player_mut(|player| {
      let bitmap_ref = player.get_datum(datum).to_bitmap_ref().unwrap();
      let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();
      let new_bitmap = bitmap.clone();
      let new_bitmap_ref = player.bitmap_manager.add_bitmap(new_bitmap);
      Datum::BitmapRef(new_bitmap_ref)
    }),
    _ => reserve_player_ref(|player| {
      player.get_datum(datum).clone()
    }),
  };
  let new_datum_ref = player_alloc_datum(new_datum);
  new_datum_ref
}
