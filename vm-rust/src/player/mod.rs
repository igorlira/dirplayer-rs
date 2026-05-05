/// Case-insensitive match for Lingo property lookups.
/// Lingo is case-insensitive, so `the markerlist` and `the markerList` must both work.
macro_rules! match_ci {
    ($val:expr, { $($($pat:literal)|+ => $body:expr),*, _ => $default:expr $(,)? }) => {
        $(if $( $val.eq_ignore_ascii_case($pat) )||+ { $body } else)*
        { $default }
    };
}

pub mod allocator;
pub mod bitmap;
pub mod bytecode;
pub mod cast_lib;
pub mod cast_manager;
pub mod ci_string;
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
pub mod mcp;
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
pub mod score_keyframes;
pub mod stream_status;
pub mod virtual_scripts;
pub mod console;
pub mod testing_shared;
#[cfg(not(target_arch = "wasm32"))]
pub mod testing;
#[cfg(target_arch = "wasm32")]
pub mod testing_browser;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::{Arc, OnceLock},
    time::Duration,
    pin::Pin,
    future::Future,
};

use allocator::{
    DatumAllocator, DatumAllocatorTrait, ResetableAllocator, ScriptInstanceAllocatorTrait,
};
use async_std::{
    channel::{self, Sender},
    future::{self, timeout},
    sync::Mutex,
    task::spawn_local,
};
use cast_manager::CastPreloadReason;
use cast_member::CastMemberType;
use datum_ref::DatumRef;
use fxhash::FxHashMap;
use handlers::datum_handlers::script_instance::ScriptInstanceUtils;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::wasm_bindgen;
use log::{debug, error, warn};
use manual_future::{ManualFuture, ManualFutureCompleter};
use net_manager::NetManager;
use scope::ScopeResult;
use score::{get_score_sprite_mut, ScoreRef};
use script::script_get_prop_opt;
use script_ref::ScriptInstanceRef;
use sprite::Sprite;
use xtra::fileio::{FileIoXtraManager, FILEIO_XTRA_MANAGER_OPT};
use xtra::multiuser::{MultiuserXtraManager, MULTIUSER_XTRA_MANAGER_OPT};
use xtra::xmlparser::{XmlParserXtraManager, XMLPARSER_XTRA_MANAGER_OPT};
use rand::SeedableRng;

use crate::{
    director::{
        chunks::handler::{Bytecode, HandlerDef},
        enums::ScriptType,
        file::{read_director_file_bytes, DirectorFile},
        lingo::{
            constants::{get_anim2_prop_name, get_anim_prop_name},
            datum::{datum_bool, Datum, DatumType, VarRef},
        },
    },
    console_warn,
    js_api::JsApi,
    player::{
        bytecode::handler_manager::{player_execute_bytecode, BytecodeHandlerContext},
        datum_formatting::format_datum,
        geometry::IntRect,
        profiling::get_profiler_report,
        scope::Scope,
        events::{player_dispatch_event_beginsprite, player_invoke_event_to_instances,
        dispatch_event_to_all_behaviors, player_invoke_frame_and_movie_scripts, dispatch_system_event_to_timeouts,
        player_invoke_targeted_event},
    },
    rendering::with_renderer_mut,
    utils::{get_base_url, get_elapsed_ticks},
};
use url::Url;

use self::{
    bitmap::manager::BitmapRef,
    bytecode::handler_manager::StaticBytecodeHandlerManager,
    cast_lib::CastMemberRef,
    cast_manager::CastManager,
    commands::{run_command_loop, PlayerVMCommand},
    debug::{Breakpoint, BreakpointContext, BreakpointManager, StepMode},
    events::{
        player_dispatch_global_event, player_invoke_global_event,
        player_wait_available, run_event_loop, PlayerVMEvent,
    },
    font::FontManager,
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
use crate::player::handlers::movie::MovieHandlers;
use crate::player::handlers::datum_handlers::player_call_datum_handler;

fn trace_output(player: &DirPlayer, message: &str) {
    use crate::js_api::JsApi;
    
    player.console.write_line(message);
    let trace_log_file = &player.movie.trace_log_file;
    if trace_log_file.is_empty() {
        JsApi::dispatch_debug_message(message);
    } else {
        // Append to file via FileIO virtual filesystem
        let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut() };
        if let Some(mgr) = manager {
            let entry = mgr.virtual_fs.entry(trace_log_file.to_string()).or_insert_with(Vec::new);
            entry.extend_from_slice(message.as_bytes());
            entry.push(b'\n');
        }
        // Emit file-append event for Electron/local file writing
        dispatch_file_write_event(trace_log_file, message);
    }
}

pub fn dispatch_file_write_event(file_path: &str, content: &str) {
    let window = web_sys::window();
    if let Some(window) = window {
        let event_init = web_sys::CustomEventInit::new();
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &"filePath".into(), &file_path.into());
        let _ = js_sys::Reflect::set(&detail, &"content".into(), &content.into());
        let _ = js_sys::Reflect::set(&detail, &"append".into(), &true.into());
        event_init.set_detail(&detail);
        if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("dirplayer:fileWrite", &event_init) {
            let _ = window.dispatch_event(&event);
        }
    }
}

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
    pub step_mode: StepMode,
    pub step_scope_depth: u32,
    pub break_on_error: bool,
    pub stage_size: (u32, u32),
    pub bitmap_manager: bitmap::manager::BitmapManager,
    pub cursor: CursorRef,
    pub start_time: chrono::DateTime<chrono::Local>,
    pub timeout_manager: TimeoutManager,
    pub title: String,
    pub bg_color: ColorRef,
    pub stage_draw_rect: Option<[f64; 4]>,
    pub center_stage: bool,
    pub keyboard_focus_sprite: i16,
    pub text_selection_start: u16,
    pub text_selection_end: u16,
    /// Mirror of the OS clipboard's last-known plain text. Populated by the
    /// frontend's copy/cut listeners and by `the clipBoard = ...` assignment;
    /// read by `the clipBoard`. Not the source of truth — `paste` reads
    /// directly from the OS via the JS gesture event.
    pub clipboard_mirror: String,
    /// IME composition state. When `Some(start)`, an in-progress composition
    /// is active in the focused editable member: `start` is the byte offset
    /// where the provisional composition begins. `end` is the current end of
    /// the provisional run (start + composition_text.len()). When None,
    /// no composition is in flight.
    pub ime_composition: Option<(i32, i32)>,
    pub mouse_loc: (i32, i32),
    pub wants_pointer_lock: bool,
    pub cursor_is_hidden: bool,
    /// Track parent DatumRef for chained property access (transform.position.z = value)
    /// (vector DatumRef, parent transform DatumRef, sub-property name)
    pub transform_sub_refs: Vec<(DatumRef, DatumRef, String)>,
    pub last_mouse_down_time: i64,
    pub is_double_click: bool,
    pub mouse_down_sprite: i16,
    pub drag_offset: (i32, i32),
    pub trails_bitmap: Option<bitmap::bitmap::Bitmap>,
    pub click_on_sprite: i16,
    pub subscribed_member_refs: Vec<CastMemberRef>, // TODO move to debug module
    pub is_subscribed_to_channel_names: bool,       // TODO move to debug module
    pub font_manager: FontManager,
    pub keyboard_manager: KeyboardManager,
    pub float_precision: u8,
    pub last_handler_result: DatumRef,
    pub hovered_sprite: Option<i16>,
    pub picking_mode: bool,
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
    pub enable_stream_status_handler: bool,
    /// Tracks the last streamStatus phase reported per net task.
    pub stream_status_reported: HashMap<u32, net_task::StreamStatusPhase>,
    pub is_in_frame_update: bool,
    pub is_dispatching_events: bool, // Prevents re-entrant event dispatch
    pub is_in_send_all_sprites: bool, // Prevents re-entrant sendAllSprites calls
    pub system_start_time: chrono::DateTime<chrono::Local>, // For ticks & milliSeconds (system uptime)
    pub handler_stack_depth: usize,
    pub in_frame_script: bool,
    /// Flash frame buffers: maps (cast_lib, cast_member) to BitmapRef in bitmap_manager
    pub flash_frame_buffers: HashMap<(i32, i32), bitmap::manager::BitmapRef>,
    /// Cached rendered 3D scene bitmaps (populated during sprite rendering, read by world.image)
    pub w3d_frame_buffers: HashMap<(i32, i32), bitmap::manager::BitmapRef>,
    pub in_enter_frame: bool,
    pub in_prepare_frame: bool,
    pub in_step_frame: bool,
    pub in_event_dispatch: bool,
    pub command_handler_yielding: bool, // Pauses frame loop when a command handler (keyDown) needs updateStage to yield
    pub in_mouse_command: bool, // Pauses frame loop during mouse handlers; updateStage renders without yielding
    pub nothing_call_count: u32,    // Consecutive nothing() calls, reset after yield
    pub last_nothing_yield_ms: f64, // Timestamp of last nothing() yield for time-throttled rendering
    pub current_frame_tempo: u32,  // Cached tempo for the current frame
    pub has_player_frame_changed: bool,
    pub stage_dirty: bool, // Set when any sprite property changes; cleared after render
    pub preview_dirty: bool, // Set when preview member/settings change; cleared after preview render
    pub has_frame_changed_in_go: bool,
    pub go_same_frame: bool,
    pub go_direction: u8,
    pub is_getting_property_descriptions: bool,
    pub is_initializing_behavior_props: bool,
    pub last_initialized_frame: Option<u32>,
    /// Current score context for sprite property access.
    /// When a filmloop sprite's behavior runs, this is set to the filmloop's ScoreRef
    /// so that sprite(n) accesses the filmloop's sprites, not the main stage.
    pub current_score_context: ScoreRef,
    pub debug_datum_refs: Vec<DatumRef>,
    pub eval_scope_index: Option<u32>,
    pub delay_until: Option<chrono::DateTime<chrono::Local>>,
    /// Pending gotoNetMovie operation: (task_id, frame_destination).
    /// Overwritten by subsequent gotoNetMovie/go-to-movie calls (cancels previous).
    pub pending_goto_net_movie: Option<(u32, MovieFrameTarget)>,
    /// True while a net movie transition is in progress.
    /// Prevents the event loop from dispatching external events during the transition.
    pub is_in_transition: bool,
    pub actor_list_generation: u64,
    pub behavior_channel_cache_generation: u64,
    pub active_stage_filmloop_cache_generation: u64,
    pub rng: rand::rngs::SmallRng,
    /// Cache of allocated scriptInstanceList datums per sprite.
    /// Ensures that `sprite.scriptInstanceList.add(x)` modifies the live list
    /// rather than a copy. Keyed by sprite number.
    pub script_instance_list_cache: FxHashMap<i16, DatumRef>,
    /// Reverse lookup from cached scriptInstanceList datum id to sprite number.
    pub script_instance_list_cache_owner: FxHashMap<usize, i16>,
    /// Mutation generation per cached scriptInstanceList.
    pub script_instance_list_generation: FxHashMap<i16, u64>,
    /// Parsed script instance ids keyed by sprite and cache generation.
    pub script_instance_list_ids_cache: FxHashMap<i16, (u64, Vec<ScriptInstanceRef>)>,
    /// Cached stage channels with active behaviors for the current frame.
    pub active_stage_behavior_channels_cache: Option<(u32, u64, Vec<usize>)>,
    /// Cached visible filmloop members currently present on the stage for a frame.
    pub active_stage_filmloop_members_cache: Option<(u32, u64, Vec<CastMemberRef>)>,
    /// Set by `sprite_get_prop` when a property returns a pre-allocated DatumRef
    /// (e.g. cached scriptInstanceList). Callers should check this before
    /// allocating a new DatumRef, to ensure mutations share the same arena entry.
    pub last_sprite_prop_ref: Option<DatumRef>,
    pub virtual_scripts: FxHashMap<CastMemberRef, Rc<dyn virtual_scripts::VirtualScriptHandler>>,
    /// Optional fake movie path override. When set, `the moviePath` and `the movieName`
    /// return values derived from this path, while actual file fetching uses the real URL.
    /// URLs the script builds from `the moviePath` (e.g. `postNetText(the moviePath & "x.aspx")`)
    /// are rewritten back to the real base in net handlers.
    pub movie_path_override: Option<String>,
    /// Like `movie_path_override` but **purely informational** — sets
    /// `the moviePath` / `the movieName` for the script to read, and
    /// nothing more. Net handlers do NOT rewrite URLs constructed from
    /// it; the script-emitted URL is fetched as-is. Use this when the
    /// JS-side fetch interceptor already handles host routing (e.g.
    /// `flashPlayerManager.ts` rewriting `maidmarian.com` to a local
    /// CORS proxy) and you want the URL the script builds to land at
    /// the proxy unchanged. Mutually exclusive with the rewrite path —
    /// when both are set, this label wins for `the moviePath`.
    pub movie_path_label: Option<String>,
    pub console: console::ConsoleBuffer,
}

/// Target frame for a movie transition (gotoNetMovie or go movie).
#[derive(Clone)]
pub enum MovieFrameTarget {
    /// No specific frame — start at frame 1
    Default,
    /// Jump to a labeled frame (from URL #fragment or string arg)
    Label(String),
    /// Jump to a specific frame number
    Frame(u32),
}

impl DirPlayer {
    pub fn new<'a>(tx: Sender<PlayerVMExecutionItem>) -> DirPlayer {
        let sound_manager = SoundManager::new(8).expect("Sound manager failed to initialize"); // 8 sound channels (Director standard)
        let now = chrono::Local::now();

        let mut result = DirPlayer {
            movie: Movie {
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
                mouse_down: false,
                click_loc: (0,0),
                frame_script_instance: None,
                frame_script_member: None,
                sound_device: String::new(),
            },
            net_manager: NetManager {
                base_path: None,
                override_base_path: None,
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
            step_mode: StepMode::None,
            step_scope_depth: 0,
            break_on_error: true,
            stage_size: (100, 100),
            bitmap_manager: bitmap::manager::BitmapManager::new(),
            cursor: CursorRef::System(0),
            start_time: now, // supposed to be time at which computer started, but we don't have access from browser. this is sufficient for calculating elapsed time.
            timeout_manager: TimeoutManager::new(),
            title: "".to_string(),
            bg_color: ColorRef::Rgb(0, 0, 0),
            stage_draw_rect: None,
            center_stage: true,
            keyboard_focus_sprite: -1, // Setting keyboardFocusSprite to -1 returns keyboard focus control to the Score, and setting it to 0 disables keyboard entry into any editable sprite.
            mouse_loc: (0, 0),
            wants_pointer_lock: false,
            cursor_is_hidden: false,
            transform_sub_refs: Vec::new(),
            last_mouse_down_time: 0,
            is_double_click: false,
            mouse_down_sprite: 0,
            drag_offset: (0, 0),
            trails_bitmap: None,
            subscribed_member_refs: vec![],
            is_subscribed_to_channel_names: false,
            font_manager: FontManager::new(),
            keyboard_manager: KeyboardManager::new(),
            text_selection_start: 0,
            text_selection_end: 0,
            clipboard_mirror: String::new(),
            ime_composition: None,
            float_precision: 4,
            last_handler_result: DatumRef::Void,
            hovered_sprite: None,
            picking_mode: false,
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
            click_on_sprite: 0,
            enable_stream_status_handler: false,
            stream_status_reported: HashMap::new(),
            is_in_frame_update: false,
            is_dispatching_events: false,
            is_in_send_all_sprites: false,
            system_start_time: now - chrono::Duration::days(8), // Simulated system start
            handler_stack_depth: 0,
            in_frame_script: false,
            flash_frame_buffers: HashMap::new(),
            w3d_frame_buffers: HashMap::new(),
            in_enter_frame: false,
            in_prepare_frame: false,
            in_step_frame: false,
            in_event_dispatch: false,
            command_handler_yielding: false,
            in_mouse_command: false,
            nothing_call_count: 0,
            last_nothing_yield_ms: 0.0,
            current_frame_tempo: 30,  // Default to 30 fps
            has_player_frame_changed: false,
            stage_dirty: true,
            preview_dirty: true,
            has_frame_changed_in_go: false,
            go_same_frame: false,
            go_direction: 0,
            is_getting_property_descriptions: false,
            is_initializing_behavior_props: false,
            last_initialized_frame: None,
            current_score_context: ScoreRef::Stage,
            debug_datum_refs: vec![],
            eval_scope_index: None,
            delay_until: None,
            pending_goto_net_movie: None,
            is_in_transition: false,
            actor_list_generation: 0,
            behavior_channel_cache_generation: 0,
            active_stage_filmloop_cache_generation: 0,
            script_instance_list_cache: FxHashMap::default(),
            script_instance_list_cache_owner: FxHashMap::default(),
            script_instance_list_generation: FxHashMap::default(),
            script_instance_list_ids_cache: FxHashMap::default(),
            active_stage_behavior_channels_cache: None,
            active_stage_filmloop_members_cache: None,
            last_sprite_prop_ref: None,
            virtual_scripts: FxHashMap::default(),
            movie_path_override: None,
            movie_path_label: None,
            console: console::ConsoleBuffer::new(),
            rng: rand::rngs::SmallRng::seed_from_u64(0),
        };

        result.reset();
        result
    }

    /// Pre-dispatch Flash members for all active sprites so they start loading
    /// before Lingo scripts try to access them.
    pub fn pre_dispatch_flash_members(&mut self) {
        for channel in &self.movie.score.channels {
            if let Some(member_ref) = &channel.sprite.member {
                let flash_key = (member_ref.cast_lib, member_ref.cast_member);
                if self.flash_frame_buffers.contains_key(&flash_key) {
                    continue;
                }
                if let Some(member) = self.movie.cast_manager.find_member_by_ref(member_ref) {
                    if let CastMemberType::Flash(flash_member) = &member.member_type {
                        if crate::rendering::has_swf_signature(&flash_member.data) {
                            let data = flash_member.data.clone();
                            let w = channel.sprite.width.max(1) as u32;
                            let h = channel.sprite.height.max(1) as u32;
                            debug!(
                                "[Flash] Pre-dispatching {}:{} ({}x{}, {} bytes)",
                                member_ref.cast_lib, member_ref.cast_member,
                                w, h, data.len()
                            );
                            JsApi::dispatch_flash_member_loaded(
                                member_ref.cast_lib,
                                member_ref.cast_member,
                                &data,
                                w,
                                h,
                            );
                            self.flash_frame_buffers.insert(flash_key, 0);
                        }
                    }
                }
            }
        }
    }

    pub async fn load_movie_from_file(&mut self, path: &str) {
        let task_id = self.net_manager.preload_net_thing(path.to_owned());
        self.net_manager.await_task(task_id).await;
        let task = self.net_manager.get_task(task_id)
            .expect(&format!("Network task not found for '{}'", path));
        let data_bytes = self
            .net_manager
            .get_task_result(Some(task_id))
            .expect(&format!("No response received for '{}'", path))
            .expect(&format!("Network request failed for '{}'", path));

        let file_name = task.resolved_url
            .path_segments()
            .and_then(|segments| segments.last())
            .unwrap_or("untitled.dcr");

        let movie_file = read_director_file_bytes(
            &data_bytes,
            &file_name,
            &get_base_url(&task.resolved_url).to_string(),
        )
        .expect(&format!("Failed to parse movie file '{}'", path));
        self.load_movie_from_dir(movie_file).await;
    }

    pub(crate) async fn load_movie_from_dir(&mut self, dir: DirectorFile) {
        self.movie
            .load_from_file(
                dir,
                &mut self.net_manager,
                &mut self.bitmap_manager,
                &mut self.dir_cache,
            )
            .await;

        // Apply fake movie path override if set (moviePath/movieName use
        // this, but net_manager.base_path stays real for actual file
        // fetching).
        //
        // Three sources, listed by precedence:
        //   1. `movie_path_label` (set_movie_path_label JS API) —
        //      label-only, no URL rewrite.
        //   2. external_params["_moviePath"] — same semantics as #1; just
        //      a more declarative way to set it (drop a key in the
        //      externalParams the host passes to dirplayer).
        //   3. `movie_path_override` (set_movie_path_override JS API) —
        //      rewrite-mode: registers `net_manager.override_base_path`
        //      so URLs the script builds get translated back to the real
        //      base before they're fetched.
        //
        // The first two are "label-only"; the third is the legacy
        // rewrite path. Only the rewrite path registers an
        // override_base_path with the net manager.
        let label_value: Option<String> = self
            .movie_path_label
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| {
                self.external_params
                    .get("_moviePath")
                    .filter(|s| !s.is_empty())
                    .cloned()
            });
        let path_to_apply: Option<(String, bool /* is_label */)> = label_value
            .map(|s| (s, true))
            .or_else(|| {
                self.movie_path_override
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .map(|s| (s.clone(), false))
            });
        if let Some((path, is_label)) = path_to_apply {
            if let Ok(url) = Url::parse(&path) {
                self.movie.base_path = get_base_url(&url).to_string();
                if let Some(name) = url.path_segments().and_then(|s| s.last()) {
                    self.movie.file_name = name.to_string();
                }
            } else {
                // Treat as a plain path
                if let Some(pos) = path.rfind('/') {
                    self.movie.base_path = path[..pos].to_string();
                    self.movie.file_name = path[pos + 1..].to_string();
                } else {
                    self.movie.file_name = path.clone();
                }
            }
            if !is_label {
                self.net_manager.override_base_path = Some(self.movie.base_path.clone());
            }
        }

        self.bg_color = self.movie.stage_color_ref.clone();

        // Load all fonts from cast members into the font manager
        log::debug!("Loading fonts from cast members...");
        self.movie
            .cast_manager
            .load_fonts_into_manager(&mut self.font_manager);

        with_renderer_mut(|renderer_opt| {
            if let Some(renderer) = renderer_opt {
                use crate::rendering_gpu::Renderer;
                renderer.set_size(
                    self.movie.rect.width() as u32,
                    self.movie.rect.height() as u32,
                );
            }
        });

        JsApi::dispatch_movie_loaded(self.movie.file.as_ref().unwrap());

        // Register built-in virtual scripts
        virtual_scripts::register_virtual_scripts(self);

        self.begin_all_sprites();
        JsApi::dispatch_frame_changed(self.movie.current_frame);
    }

    pub fn play(&mut self) {
        if self.is_playing {
            return;
        }
        self.is_playing = true;
        self.is_script_paused = false;

        use crate::js_api::safe_string;
        debug!("Loading Movie: {} (version: {})", safe_string(&self.movie.file_name), self.movie.dir_version);

        async_std::task::spawn_local(async move {
            run_movie_init_sequence().await;
            run_frame_loop().await;
        });
    }

    pub fn begin_all_sprites(&mut self) {
        self.movie.score.begin_sprites(ScoreRef::Stage, self.movie.current_frame);
        
        // Cache the tempo for this frame
        self.current_frame_tempo = self.movie.get_effective_tempo();
        
        // If the player isn't playing yet (i.e., during initial load),
        // reset the entered flags so that beginSprite will be called again
        // when the movie actually starts playing with the fixed spriteNum
        if !self.is_playing {
            for channel in &mut self.movie.score.channels {
                channel.sprite.entered = false;
                channel.sprite.script_instance_list.clear();
            }
            self.clear_script_instance_list_caches();
        }

        self.invalidate_active_stage_filmloop_cache();
        let active_filmloops = self.active_stage_filmloop_member_refs();
        for member_ref in active_filmloops {
            let film_loop = match self
                .movie
                .cast_manager
                .find_mut_member_by_ref(&member_ref)
                .and_then(|m| m.member_type.as_film_loop_mut())
            {
                Some(fl) => fl,
                None => continue,
            };
            // Use filmloop's own current_frame instead of movie's current_frame
            let current_frame = film_loop.current_frame;
            film_loop
                .score
                .begin_sprites(ScoreRef::FilmLoop(member_ref.clone()), current_frame);
            film_loop.score.apply_tween_modifiers(current_frame);
        }

        self.invalidate_behavior_channel_cache();
    }

    pub async fn end_all_sprites(&mut self) -> Vec<(ScoreRef, u32)> {
        let next_frame = self.get_next_frame();
        let mut all_ended_sprite_nums: Vec<(ScoreRef, u32)> = vec![];
        let ended_sprite_nums = self
            .movie
            .score
            .end_sprites(ScoreRef::Stage, self.movie.current_frame, next_frame).await;
        all_ended_sprite_nums.extend(ended_sprite_nums.iter().map(|&x| (ScoreRef::Stage, x)));

        let active_filmloops = self.active_stage_filmloop_member_refs();
        for member_ref in active_filmloops {
            let score_ref = ScoreRef::FilmLoop(member_ref.clone());
            let film_loop = match self
                .movie
                .cast_manager
                .find_mut_member_by_ref(&member_ref)
                .and_then(|m| m.member_type.as_film_loop_mut())
            {
                Some(fl) => fl,
                None => continue,
            };

            let filmloop_current_frame = film_loop.current_frame;
            let filmloop_next_frame = filmloop_current_frame + 1;

            let ended_sprite_nums = film_loop
                .score
                .end_sprites(score_ref.clone(), filmloop_current_frame, filmloop_next_frame).await;
            all_ended_sprite_nums.extend(
                ended_sprite_nums
                    .iter()
                    .map(|&x| (score_ref.clone(), x)),
            );
        }
        // for sprite_num in ended_sprite_nums.iter() {
        //   let sprite = self.movie.score.get_sprite_mut(*sprite_num as i16);
        //   sprite.exited = true;
        // }
        self.invalidate_behavior_channel_cache();
        self.invalidate_active_stage_filmloop_cache();
        all_ended_sprite_nums
    }

    /// Get all active filmloop scores with their member references.
    /// Returns a vector of tuples containing (member_ref, current_frame).
    /// Only includes filmloops that are currently visible on stage.
    /// Deduplicates filmloops - each unique filmloop is only returned once even if used in multiple sprites.
    pub fn get_active_filmloop_scores(&mut self) -> Vec<(CastMemberRef, u32)> {
        let member_refs = self.active_stage_filmloop_member_refs();
        let mut active_filmloops = Vec::with_capacity(member_refs.len());

        for member_ref in member_refs {
            if let Some(member) = self.movie.cast_manager.find_member_by_ref(&member_ref) {
                if let cast_member::CastMemberType::FilmLoop(film_loop) = &member.member_type {
                    active_filmloops.push((member_ref, film_loop.current_frame));
                }
            }
        }

        active_filmloops
    }

    pub fn pause_script(&mut self) {
        self.is_script_paused = true;
    }

    pub fn resume_script(&mut self) {
        self.is_script_paused = false;
    }

    pub fn resume_breakpoint(&mut self) {
        self.step_mode = StepMode::None;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            spawn_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_into(&mut self) {
        self.step_mode = StepMode::Into;
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            spawn_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_over(&mut self) {
        self.step_mode = StepMode::Over;
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            spawn_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_out(&mut self) {
        self.step_mode = StepMode::Out;
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            spawn_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_over_line(&mut self, skip_bytecode_indices: Vec<usize>) {
        self.step_mode = StepMode::OverLine { skip_bytecode_indices };
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            spawn_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_into_line(&mut self, skip_bytecode_indices: Vec<usize>) {
        self.step_mode = StepMode::IntoLine { skip_bytecode_indices };
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            spawn_local(breakpoint.completer.complete(()));
        }
    }

    #[inline]
    pub fn get_datum(&self, id: &DatumRef) -> &Datum {
        self.allocator.get_datum(id)
    }

    pub fn get_datum_mut(&mut self, id: &DatumRef) -> &mut Datum {
        self.allocator.get_datum_mut(id)
    }

    /// Resolve a datum to a BitmapRef.  Accepts:
    ///  - `Datum::BitmapRef(n)` → direct handle
    ///  - `Datum::Int(n)` where n > 0 → treated as a member slot number;
    ///    the member is looked up and its bitmap image_ref is returned.
    pub fn resolve_bitmap_ref(&self, datum: &Datum) -> Result<BitmapRef, ScriptError> {
        match datum {
            Datum::BitmapRef(br) => Ok(*br),
            Datum::Int(n) if *n > 0 => {
                let member_ref = handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers
                    ::member_ref_from_slot_number(*n as u32);
                if let Some(member) = self.movie.cast_manager.find_member_by_ref(&member_ref) {
                    if let Some(bitmap_member) = member.member_type.as_bitmap() {
                        Ok(bitmap_member.image_ref)
                    } else {
                        Err(ScriptError::new(format!(
                            "Cannot convert Int({}) to bitmap ref: member {}:{} is not a bitmap",
                            n, member_ref.cast_lib, member_ref.cast_member
                        )))
                    }
                } else {
                    Err(ScriptError::new(format!(
                        "Cannot convert Int({}) to bitmap ref: no member found for slot number",
                        n
                    )))
                }
            }
            _ => datum.to_bitmap_ref().map(|br| *br),
        }
    }

    pub fn get_fps(&self) -> u32 {
        if self.movie.puppet_tempo > 0 {
            self.movie.puppet_tempo
        } else {
            self.movie.frame_rate as u32
        }
    }

    pub fn get_hydrated_globals(&self) -> FxHashMap<&str, &Datum> {
        self.globals
            .iter()
            .map(|(k, v)| (k.as_str(), self.get_datum(v)))
            .collect()
    }

    #[allow(dead_code)]
    pub fn get_global(&self, name: &str) -> Option<&Datum> {
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
            let next = self.movie.current_frame + 1;
            // Loop back to frame 1 when past the last frame
            if let Some(frame_count) = self.movie.score.frame_count {
                if next > frame_count {
                    return 1;
                }
            }
            return next;
        }
    }

    pub fn advance_frame(&mut self) {
        if !self.is_playing {
            return;
        }
        self.stage_dirty = true;

        let prev_frame = self.movie.current_frame;
        let next_frame = self.get_next_frame();

        // Always advance logic (scripts, behaviors)
        self.next_frame = None;
        self.movie.current_frame = next_frame;

        // NOTE: Filmloop frames are advanced solely by update_filmloop_frames() in the main loop.
        // Do NOT call advance_filmloop_frames() here - it would cause double advancement since
        // advance_frame() is also called from the `go` handler, and update_filmloop_frames()
        // runs separately in the main loop.

        // Only dispatch and render if updateLock is off
        if !self.movie.update_lock && prev_frame != self.movie.current_frame {
            JsApi::dispatch_frame_changed(self.movie.current_frame);
            self.has_player_frame_changed = true;
        }
    }

    pub fn stop(&mut self) {
        // TODO dispatch stop movie
        self.is_playing = false;
        self.next_frame = None;
        //scopes.clear();
        // currentBreakpoint?.completer.completeError(CancelledException());
        // currentBreakpoint = null;
        //self.timeout_manager.clear();
        //notifyListeners();

        warn!("Profiler report: {}", get_profiler_report());
    }

    pub fn reset(&mut self) {
        self.stop();

        // Clear all references before resetting the allocator
        // This ensures all DatumRef and ScriptInstanceRef objects are dropped properly
        debug!("Clearing scopes");
        self.scopes.clear();
        debug!("Clearing globals");
        self.globals.clear();
        debug!("Clearing timeout manager");
        self.timeout_manager.clear();
        debug!("Clearing debug datum refs");
        self.debug_datum_refs.clear();
        // netManager.clear();
        debug!("Resetting score");
        self.movie.score.reset();
        self.clear_script_instance_list_caches();
        self.invalidate_active_stage_filmloop_cache();
        self.movie.current_frame = 1;
        // TODO cancel breakpoints
        self.current_breakpoint = None;
        self.scope_count = 0;
        self.pending_goto_net_movie = None;

        debug!("Resetting allocator");
        // Now it's safe to reset the allocator
        self.allocator.reset();

        self.initialize_globals();

        // Initialize scopes (call stack)
        for i in 0..MAX_STACK_SIZE {
            self.scopes.push(Scope::default(i));
        }

        JsApi::dispatch_frame_changed(self.movie.current_frame);
        JsApi::dispatch_scope_list(self);
        JsApi::dispatch_script_error_cleared();
        JsApi::dispatch_global_list(self);
    }

    pub fn initialize_globals(&mut self) {
        // Initialize the actorList as a global variable
        let actor_list_datum = self.alloc_datum(Datum::List(DatumType::List, VecDeque::new(), false));
        self.globals.insert("actorList".to_string(), actor_list_datum);
        self.actor_list_generation = 0;

        // Mathematical constant
        let pi_datum = self.alloc_datum(Datum::Float(std::f64::consts::PI));
        self.globals.insert("PI".to_string(), pi_datum);
        
        // Special values
        let void_datum = self.alloc_datum(Datum::Void);
        self.globals.insert("VOID".to_string(), void_datum);
        
        let empty_datum = self.alloc_datum(Datum::String("".to_string()));
        self.globals.insert("EMPTY".to_string(), empty_datum);
        
        // String constants
        let return_datum = self.alloc_datum(Datum::String("\r".to_string()));
        self.globals.insert("RETURN".to_string(), return_datum);

        let enter_datum = self.alloc_datum(Datum::String("\x03".to_string()));
        self.globals.insert("ENTER".to_string(), enter_datum);
        
        let quote_datum = self.alloc_datum(Datum::String("\"".to_string()));
        self.globals.insert("QUOTE".to_string(), quote_datum);
        
        let tab_datum = self.alloc_datum(Datum::String("\t".to_string()));
        self.globals.insert("TAB".to_string(), tab_datum);
        
        // Backspace character
        let backspace_datum = self.alloc_datum(Datum::String("\x08".to_string()));
        self.globals.insert("BACKSPACE".to_string(), backspace_datum);
        
        // Boolean constants (these are typically handled as keywords, but can be globals)
        let true_datum = self.alloc_datum(Datum::Int(1));
        self.globals.insert("TRUE".to_string(), true_datum);
        
        let false_datum = self.alloc_datum(Datum::Int(0));
        self.globals.insert("FALSE".to_string(), false_datum);
    }

    #[inline]
    pub fn alloc_datum(&mut self, datum: Datum) -> DatumRef {
        return self.allocator.alloc_datum(datum).unwrap();
    }

    /// Sync cached scriptInstanceList back to sprite's Vec for a given sprite.
    /// Call this before reading script_instance_list on sprites that may have been
    /// modified via .add() on the cached Datum::List.
    pub fn sync_script_instance_list(&mut self, sprite_id: i16) {
        if self.script_instance_list_cache.contains_key(&sprite_id) {
            let existing_ids = self
                .movie
                .score
                .get_sprite(sprite_id)
                .map(|sprite| sprite.script_instance_list.clone())
                .unwrap_or_default();
            let synced_ids = self.get_sprite_script_instance_ids(sprite_id, &existing_ids);
            let sprite = self.movie.score.get_sprite_mut(sprite_id);
            sprite.script_instance_list = synced_ids;
        }
    }

    /// Sync ALL cached scriptInstanceLists back to sprite Vecs.
    pub fn sync_all_script_instance_lists(&mut self) {
        let sprite_ids: Vec<i16> = self.script_instance_list_cache.keys().cloned().collect();
        for sprite_id in sprite_ids {
            self.sync_script_instance_list(sprite_id);
        }
    }

    pub fn cache_script_instance_list(
        &mut self,
        sprite_id: i16,
        list_ref: DatumRef,
        initial_ids: Vec<ScriptInstanceRef>,
    ) {
        self.remove_script_instance_list_cache(sprite_id);
        let generation = *self.script_instance_list_generation.entry(sprite_id).or_insert(0);
        self.script_instance_list_cache_owner
            .insert(list_ref.unwrap(), sprite_id);
        self.script_instance_list_cache
            .insert(sprite_id, list_ref);
        self.script_instance_list_ids_cache
            .insert(sprite_id, (generation, initial_ids));
    }

    pub fn remove_script_instance_list_cache(&mut self, sprite_id: i16) {
        if let Some(cached_ref) = self.script_instance_list_cache.remove(&sprite_id) {
            self.script_instance_list_cache_owner.remove(&cached_ref.unwrap());
        }
        self.script_instance_list_generation.remove(&sprite_id);
        self.script_instance_list_ids_cache.remove(&sprite_id);
    }

    pub fn clear_script_instance_list_caches(&mut self) {
        self.script_instance_list_cache.clear();
        self.script_instance_list_cache_owner.clear();
        self.script_instance_list_generation.clear();
        self.script_instance_list_ids_cache.clear();
        self.invalidate_behavior_channel_cache();
    }

    pub fn note_script_instance_list_mutation(&mut self, datum_ref: &DatumRef) {
        if let Some(sprite_id) = self
            .script_instance_list_cache_owner
            .get(&datum_ref.unwrap())
            .copied()
        {
            let generation = self.script_instance_list_generation.entry(sprite_id).or_insert(0);
            *generation = generation.wrapping_add(1);
            self.script_instance_list_ids_cache.remove(&sprite_id);
            self.refresh_stage_behavior_channel_cache_entry(sprite_id);
        }
    }

    pub fn invalidate_behavior_channel_cache(&mut self) {
        self.behavior_channel_cache_generation =
            self.behavior_channel_cache_generation.wrapping_add(1);
        self.active_stage_behavior_channels_cache = None;
    }

    pub fn invalidate_active_stage_filmloop_cache(&mut self) {
        self.active_stage_filmloop_cache_generation =
            self.active_stage_filmloop_cache_generation.wrapping_add(1);
        self.active_stage_filmloop_members_cache = None;
    }

    pub fn get_sprite_script_instance_ids(
        &mut self,
        sprite_id: i16,
        fallback: &[ScriptInstanceRef],
    ) -> Vec<ScriptInstanceRef> {
        let Some(cached_ref) = self.script_instance_list_cache.get(&sprite_id).cloned() else {
            return fallback.to_vec();
        };

        let generation = *self.script_instance_list_generation.get(&sprite_id).unwrap_or(&0);
        if let Some((cached_generation, ids)) = self.script_instance_list_ids_cache.get(&sprite_id)
        {
            if *cached_generation == generation {
                return ids.clone();
            }
        }

        let ids = match self.get_datum(&cached_ref) {
            Datum::List(_, item_refs, _) => item_refs
                .iter()
                .filter_map(|item_ref| match self.get_datum(item_ref) {
                    Datum::ScriptInstanceRef(id) => Some(id.clone()),
                    _ => None,
                })
                .collect(),
            _ => fallback.to_vec(),
        };

        self.script_instance_list_ids_cache
            .insert(sprite_id, (generation, ids.clone()));
        ids
    }

    pub fn sprite_has_script_instance_ids(
        &self,
        sprite_id: i16,
        fallback: &[ScriptInstanceRef],
    ) -> bool {
        if !fallback.is_empty() {
            return true;
        }

        self.script_instance_list_cache
            .get(&sprite_id)
            .is_some_and(|cached_ref| {
                matches!(self.get_datum(cached_ref), Datum::List(_, items, _) if !items.is_empty())
            })
    }

    pub fn refresh_stage_behavior_channel_cache_entry(&mut self, sprite_id: i16) {
        let channel_number = sprite_id as usize;
        let should_include = self
            .movie
            .score
            .channels
            .get(channel_number)
            .is_some_and(|channel| {
                (channel.sprite.entered || channel.sprite.puppet)
                    && self.sprite_has_script_instance_ids(
                        sprite_id,
                        &channel.sprite.script_instance_list,
                    )
            });

        let Some((_, _, channels)) = self.active_stage_behavior_channels_cache.as_mut() else {
            return;
        };

        match channels.binary_search(&channel_number) {
            Ok(index) if !should_include => {
                channels.remove(index);
            }
            Err(index) if should_include => {
                channels.insert(index, channel_number);
            }
            _ => {}
        }
    }

    pub fn active_stage_behavior_channels(&mut self) -> Vec<usize> {
        let frame_num = self.movie.current_frame;
        let generation = self.behavior_channel_cache_generation;

        if let Some((cached_frame, cached_generation, channels)) =
            &self.active_stage_behavior_channels_cache
        {
            if *cached_frame == frame_num && *cached_generation == generation {
                return channels.clone();
            }
        }

        let channels: Vec<usize> = self
            .movie
            .score
            .channels
            .iter()
            .filter(|channel| channel.sprite.entered || channel.sprite.puppet)
            .filter(|channel| {
                self.sprite_has_script_instance_ids(
                    channel.number as i16,
                    &channel.sprite.script_instance_list,
                )
            })
            .map(|channel| channel.number)
            .collect();

        self.active_stage_behavior_channels_cache =
            Some((frame_num, generation, channels.clone()));
        channels
    }

    pub fn active_stage_filmloop_member_refs(&mut self) -> Vec<CastMemberRef> {
        let frame_num = self.movie.current_frame;
        let generation = self.active_stage_filmloop_cache_generation;
        if let Some((cached_frame, cached_generation, member_refs)) =
            &self.active_stage_filmloop_members_cache
        {
            if *cached_frame == frame_num && *cached_generation == generation {
                return member_refs.clone();
            }
        }

        let mut channel_numbers = self.movie.score.active_channel_numbers_for_frame(frame_num);
        let mut seen_channels = channel_numbers
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        for channel in self.movie.score.channels.iter() {
            if channel.number != 0
                && channel.sprite.puppet
                && channel.sprite.visible
                && seen_channels.insert(channel.number)
            {
                channel_numbers.push(channel.number);
            }
        }

        let mut member_refs = Vec::new();
        let mut seen_members = std::collections::HashSet::new();
        for channel_number in channel_numbers {
            let Some(channel) = self.movie.score.channels.get(channel_number) else {
                continue;
            };

            if channel.number == 0 || !channel.sprite.visible {
                continue;
            }
            let Some(member_ref) = channel.sprite.member.as_ref() else {
                continue;
            };

            if !seen_members.insert(member_ref.clone()) {
                continue;
            }

            let is_filmloop = self
                .movie
                .cast_manager
                .find_member_by_ref(member_ref)
                .is_some_and(|member| {
                    member.member_type.member_type_id() == cast_member::CastMemberTypeId::FilmLoop
                });
            if is_filmloop {
                member_refs.push(member_ref.clone());
            }
        }

        self.active_stage_filmloop_members_cache =
            Some((frame_num, generation, member_refs.clone()));
        member_refs
    }

    pub fn active_stage_script_instance_ids(&mut self) -> Vec<ScriptInstanceRef> {
        let mut channel_numbers = self.active_stage_behavior_channels();
        let frame_channel_is_active = self
            .movie
            .score
            .active_channel_numbers_for_frame(self.movie.current_frame)
            .contains(&0);
        if frame_channel_is_active && !channel_numbers.contains(&0) {
            channel_numbers.push(0);
        }

        let mut receivers = Vec::new();
        for channel_number in channel_numbers {
            let Some(fallback) = self
                .movie
                .score
                .channels
                .get(channel_number)
                .map(|channel| channel.sprite.script_instance_list.clone())
            else {
                continue;
            };
            receivers.extend(
                self.get_sprite_script_instance_ids(channel_number as i16, fallback.as_slice()),
            );
        }
        receivers
    }

    pub fn note_actor_list_mutation(&mut self, datum_ref: &DatumRef) {
        if self.globals.get("actorList").is_some_and(|actor_list_ref| actor_list_ref == datum_ref) {
            self.actor_list_generation = self.actor_list_generation.wrapping_add(1);
        }
    }

    pub fn actor_list_stepframe_snapshot(&self) -> (VecDeque<DatumRef>, HashSet<usize>, u64) {
        let actor_list_ref = self
            .globals
            .get("actorList")
            .cloned()
            .unwrap_or(DatumRef::Void);
        match self.get_datum(&actor_list_ref) {
            Datum::List(_, items, _) => {
                let snapshot = items.clone();
                let active_ids = items.iter().map(|actor_ref| actor_ref.unwrap()).collect();
                (snapshot, active_ids, self.actor_list_generation)
            }
            _ => (VecDeque::new(), HashSet::new(), self.actor_list_generation),
        }
    }

    pub fn actor_list_active_ids(&self) -> (HashSet<usize>, u64) {
        let actor_list_ref = self
            .globals
            .get("actorList")
            .cloned()
            .unwrap_or(DatumRef::Void);
        match self.get_datum(&actor_list_ref) {
            Datum::List(_, items, _) => (
                items.iter().map(|actor_ref| actor_ref.unwrap()).collect(),
                self.actor_list_generation,
            ),
            _ => (HashSet::new(), self.actor_list_generation),
        }
    }

    fn datum_leak_scan(&self) -> String {
        use std::collections::{HashMap, HashSet};
        use crate::player::datum_formatting::format_datum;

        let snapshot_id = self.allocator.snapshot_max_id;
        let is_new = |id: usize| -> bool { snapshot_id > 0 && id > snapshot_id };

        let ref_id = |r: &DatumRef| -> Option<usize> {
            if let DatumRef::Ref(id, ..) = r { Some(*id) } else { None }
        };

        // Step 1: Count ALL new datums by type
        let mut type_counts_new: HashMap<String, usize> = HashMap::new();
        let mut new_datum_ids: HashSet<usize> = HashSet::new();
        for (id, entry) in self.allocator.datums.iter() {
            if is_new(id) {
                let type_name = entry.datum.type_str().to_string();
                *type_counts_new.entry(type_name).or_insert(0) += 1;
                new_datum_ids.insert(id);
            }
        }

        // Step 2: Find OLD containers that hold NEW datums (= GROWING containers)
        let mut growing_containers: Vec<(String, usize)> = Vec::new();
        let mut accounted: HashSet<usize> = HashSet::new();

        for (cid, entry) in self.allocator.datums.iter() {
            if is_new(cid) { continue; } // only look at old containers
            let rc = unsafe { *entry.ref_count.get() };
            match &entry.datum {
                Datum::List(list_type, items, _) => {
                    let new_items: Vec<usize> = items.iter()
                        .filter_map(|r| ref_id(r).filter(|id| new_datum_ids.contains(id)))
                        .collect();
                    if !new_items.is_empty() {
                        // Show what the new items ARE (types)
                        let mut child_types: HashMap<String, usize> = HashMap::new();
                        for &nid in &new_items {
                            if let Some(e) = self.allocator.datums.get(nid) {
                                *child_types.entry(e.datum.type_str().to_string()).or_insert(0) += 1;
                            }
                        }
                        let types_str: Vec<String> = child_types.iter()
                            .map(|(t, c)| format!("{}x{}", c, t)).collect();
                        growing_containers.push((
                            format!("OLD {:?}List #{} (len={},rc={}) +{} new [{}]",
                                list_type, cid, items.len(), rc, new_items.len(), types_str.join(",")),
                            new_items.len()
                        ));
                        for nid in new_items { accounted.insert(nid); }
                    }
                }
                Datum::PropList(pairs, _) => {
                    let mut new_count = 0;
                    for (k, v) in pairs {
                        if ref_id(v).map_or(false, |id| new_datum_ids.contains(&id)) { new_count += 1; accounted.insert(ref_id(v).unwrap()); }
                        if ref_id(k).map_or(false, |id| new_datum_ids.contains(&id)) { new_count += 1; accounted.insert(ref_id(k).unwrap()); }
                    }
                    if new_count > 0 {
                        let keys_preview: Vec<String> = pairs.iter().take(3)
                            .map(|(k, _)| format_datum(k, self)).collect();
                        growing_containers.push((
                            format!("OLD PropList #{} (len={},rc={}) +{} new keys=[{}...]",
                                cid, pairs.len(), rc, new_count, keys_preview.join(",")),
                            new_count
                        ));
                    }
                }
                Datum::Point(..) | Datum::Rect(..) => {
                    // Inline storage - no DatumRef children to track
                }
                _ => {}
            }
        }

        // Step 3: Find NEW compound datums containing NEW children (new sub-trees)
        let mut new_subtree_types: HashMap<String, usize> = HashMap::new();
        for (cid, entry) in self.allocator.datums.iter() {
            if !is_new(cid) { continue; }
            let rc = unsafe { *entry.ref_count.get() };
            let new_child_count = match &entry.datum {
                Datum::List(_, items, _) => {
                    let c: usize = items.iter().filter(|r| ref_id(r).map_or(false, |id| new_datum_ids.contains(&id))).count();
                    if c > 0 { for r in items { if let Some(id) = ref_id(r).filter(|id| new_datum_ids.contains(id)) { accounted.insert(id); } } }
                    c
                }
                Datum::PropList(pairs, _) => {
                    let mut c = 0;
                    for (k, v) in pairs {
                        if ref_id(v).map_or(false, |id| new_datum_ids.contains(&id)) { c += 1; accounted.insert(ref_id(v).unwrap()); }
                        if ref_id(k).map_or(false, |id| new_datum_ids.contains(&id)) { c += 1; accounted.insert(ref_id(k).unwrap()); }
                    }
                    c
                }
                Datum::Point(..) | Datum::Rect(..) => {
                    // Inline storage - no DatumRef children to track
                    0
                }
                _ => 0,
            };
            if new_child_count > 0 {
                *new_subtree_types.entry(format!("NEW {}(rc={})", entry.datum.type_str(), rc)).or_insert(0) += 1;
            }
        }

        // Step 4: Check script instance properties for new datums
        let mut si_new: HashMap<String, usize> = HashMap::new();
        for (si_id, entry) in self.allocator.script_instances.iter() {
            for (prop_name, r) in &entry.script_instance.properties {
                if let Some(id) = ref_id(r).filter(|id| new_datum_ids.contains(id)) {
                    let sn = self.movie.cast_manager.get_script_by_ref(&entry.script_instance.script)
                        .map(|s| s.name.clone()).unwrap_or_else(|| format!("si#{}", si_id));
                    let dtype = self.allocator.datums.get(id).map(|e| e.datum.type_str()).unwrap_or("?");
                    *si_new.entry(format!("si({}).{} [{}]", sn, prop_name.as_str(), dtype)).or_insert(0) += 1;
                    accounted.insert(id);
                }
            }
        }

        // Step 5: Check globals and scopes
        let mut other_roots: HashMap<String, usize> = HashMap::new();
        for (name, r) in &self.globals {
            if let Some(id) = ref_id(r).filter(|id| new_datum_ids.contains(id)) {
                *other_roots.entry(format!("global.{}", name)).or_insert(0) += 1;
                accounted.insert(id);
            }
        }
        for i in 0..self.scopes.len() {
            let scope = &self.scopes[i];
            let pfx = if i >= self.scope_count as usize { "STALE" } else { "active" };
            for r in &scope.stack {
                if let Some(id) = ref_id(r).filter(|id| new_datum_ids.contains(id)) {
                    *other_roots.entry(format!("{}[{}].stack", pfx, i)).or_insert(0) += 1;
                    accounted.insert(id);
                }
            }
            for (_, r) in &scope.locals {
                if let Some(id) = ref_id(r).filter(|id| new_datum_ids.contains(id)) {
                    *other_roots.entry(format!("{}[{}].locals", pfx, i)).or_insert(0) += 1;
                    accounted.insert(id);
                }
            }
            if let Some(id) = ref_id(&scope.return_value).filter(|id| new_datum_ids.contains(id)) {
                *other_roots.entry(format!("{}[{}].retval", pfx, i)).or_insert(0) += 1;
                accounted.insert(id);
            }
        }
        if let Some(id) = ref_id(&self.last_handler_result).filter(|id| new_datum_ids.contains(id)) {
            *other_roots.entry("last_handler_result".to_string()).or_insert(0) += 1;
            accounted.insert(id);
        }

        // Build report
        let mut result = format!("=== Datum Leak Scan v3 ===\nSnapshot: {}, Total new: {}\n\n", snapshot_id, new_datum_ids.len());

        result.push_str("New datums by type:\n");
        let mut ns: Vec<_> = type_counts_new.into_iter().collect();
        ns.sort_by(|a, b| b.1.cmp(&a.1));
        for (t, c) in &ns { result.push_str(&format!("  {}: {}\n", t, c)); }

        result.push_str(&format!("\nOLD containers with new items (GROWING):\n"));
        growing_containers.sort_by(|a, b| b.1.cmp(&a.1));
        for (desc, _) in growing_containers.iter().take(25) {
            result.push_str(&format!("  {}\n", desc));
        }

        if !new_subtree_types.is_empty() {
            result.push_str(&format!("\nNew compound sub-trees:\n"));
            let mut nst: Vec<_> = new_subtree_types.into_iter().collect();
            nst.sort_by(|a, b| b.1.cmp(&a.1));
            for (t, c) in nst.iter().take(15) { result.push_str(&format!("  {} x{}\n", t, c)); }
        }

        if !si_new.is_empty() {
            result.push_str(&format!("\nScript props with new datums:\n"));
            let mut si: Vec<_> = si_new.into_iter().collect();
            si.sort_by(|a, b| b.1.cmp(&a.1));
            for (k, c) in si.iter().take(15) { result.push_str(&format!("  {}: {}\n", k, c)); }
        }

        if !other_roots.is_empty() {
            result.push_str(&format!("\nOther roots with new datums:\n"));
            let mut or: Vec<_> = other_roots.into_iter().collect();
            or.sort_by(|a, b| b.1.cmp(&a.1));
            for (k, c) in or.iter().take(10) { result.push_str(&format!("  {}: {}\n", k, c)); }
        }

        let unacc = new_datum_ids.len() - accounted.len();
        result.push_str(&format!("\nAccounted: {}, Unaccounted: {}\n", accounted.len(), unacc));

        // Step 6: Inspect top growing containers - show content samples and ownership chain
        // Collect top 3 growing container IDs from the sorted list
        let top_container_ids: Vec<usize> = growing_containers.iter().take(3)
            .filter_map(|(desc, _)| {
                // Parse the datum ID from the description string "OLD ...List #XXXX ..."
                desc.find('#').and_then(|start| {
                    let after_hash = &desc[start + 1..];
                    after_hash.split(|c: char| !c.is_ascii_digit()).next()
                        .and_then(|s| s.parse::<usize>().ok())
                })
            })
            .collect();

        for &cid in &top_container_ids {
            if let Some(entry) = self.allocator.datums.get(cid) {
                result.push_str(&format!("\n--- Inspect #{} ---\n", cid));

                // Show content sample
                match &entry.datum {
                    Datum::List(_, items, _) => {
                        result.push_str(&format!("List len={}\n", items.len()));
                        // First 5 items
                        result.push_str("First 5: ");
                        for (i, r) in items.iter().enumerate().take(5) {
                            if i > 0 { result.push_str(", "); }
                            result.push_str(&format_datum(r, self));
                        }
                        result.push('\n');
                        // Last 5 items
                        let start = if items.len() > 5 { items.len() - 5 } else { 0 };
                        result.push_str("Last 5: ");
                        for (i, r) in items.iter().skip(start).enumerate() {
                            if i > 0 { result.push_str(", "); }
                            result.push_str(&format_datum(r, self));
                        }
                        result.push('\n');
                    }
                    _ => {
                        result.push_str(&format!("Type: {}\n", entry.datum.type_str()));
                    }
                }

                // Trace ownership: who holds a reference to this datum?
                result.push_str("Stored in: ");
                let mut found_owner = false;

                // Check script instance properties
                for (si_id, si_entry) in self.allocator.script_instances.iter() {
                    for (prop_name, r) in &si_entry.script_instance.properties {
                        if ref_id(r) == Some(cid) {
                            let sn = self.movie.cast_manager.get_script_by_ref(&si_entry.script_instance.script)
                                .map(|s| s.name.clone()).unwrap_or_else(|| format!("si#{}", si_id));
                            result.push_str(&format!("si({}).{}", sn, prop_name.as_str()));
                            found_owner = true;
                        }
                    }
                }

                // Check globals
                if !found_owner {
                    for (name, r) in &self.globals {
                        if ref_id(r) == Some(cid) {
                            result.push_str(&format!("global.{}", name));
                            found_owner = true;
                        }
                    }
                }

                // Check inside compound datums (one level up)
                if !found_owner {
                    for (pid, pentry) in self.allocator.datums.iter() {
                        if pid == cid { continue; }
                        let contains = match &pentry.datum {
                            Datum::List(_, items, _) => items.iter().any(|r| ref_id(r) == Some(cid)),
                            Datum::PropList(pairs, _) => pairs.iter().any(|(k, v)|
                                ref_id(k) == Some(cid) || ref_id(v) == Some(cid)),
                            _ => false,
                        };
                        if contains {
                            let prc = unsafe { *pentry.ref_count.get() };
                            result.push_str(&format!("inside {} #{} (rc={})", pentry.datum.type_str(), pid, prc));

                            // Trace one more level: who holds the parent?
                            for (si_id, si_entry) in self.allocator.script_instances.iter() {
                                for (prop_name, r) in &si_entry.script_instance.properties {
                                    if ref_id(r) == Some(pid) {
                                        let sn = self.movie.cast_manager.get_script_by_ref(&si_entry.script_instance.script)
                                            .map(|s| s.name.clone()).unwrap_or_else(|| format!("si#{}", si_id));
                                        result.push_str(&format!(" ← si({}).{}", sn, prop_name.as_str()));
                                    }
                                }
                            }
                            for (name, r) in &self.globals {
                                if ref_id(r) == Some(pid) {
                                    result.push_str(&format!(" ← global.{}", name));
                                }
                            }

                            found_owner = true;
                            break;
                        }
                    }
                }

                if !found_owner {
                    result.push_str("(unknown - not found in script props, globals, or compounds)");
                }
                result.push('\n');
            }
        }

        result
    }

    fn get_movie_prop(&mut self, prop: &str) -> Result<DatumRef, ScriptError> {
        match_ci!(prop, {
            "datumStats" => {
                let stats = self.allocator.datum_type_stats();
                web_sys::console::log_1(&stats.clone().into());
                Ok(self.alloc_datum(Datum::String(stats)))
            },
            "datumSnapshot" => {
                self.allocator.take_datum_snapshot();
                debug!("Datum snapshot taken. Use 'put the datumStats' to see new datums since snapshot.");
                Ok(DatumRef::Void)
            },
            "datumLeakScan" => {
                let stats = self.datum_leak_scan();
                web_sys::console::log_1(&stats.clone().into());
                Ok(self.alloc_datum(Datum::String(stats)))
            },
            "systemDate" => {
                let date_id = self.allocator.get_free_script_instance_id();
                let date_obj = crate::player::handlers::datum_handlers::date::DateObject::new(date_id);
                self.date_objects.insert(date_id, date_obj);
                Ok(self.alloc_datum(Datum::DateRef(date_id)))
            },
            "stage" => Ok(self.alloc_datum(Datum::Stage)),
            "time" => Ok(self.alloc_datum(Datum::String(
                chrono::Local::now().format("%H:%M %p").to_string(),
            ))),
            "milliSeconds" => Ok(self.alloc_datum(Datum::Int(
                chrono::Local::now()
                    .signed_duration_since(self.system_start_time)
                    .num_milliseconds() as i32,
            ))),
            "keyboardFocusSprite" => {
                Ok(self.alloc_datum(Datum::Int(self.keyboard_focus_sprite as i32)))
            },
            "selection" => {
                // Returns the currently selected text in the focused editable
                // Field/Text member, or "" if none.
                let s = if self.keyboard_focus_sprite >= 0 {
                    let sprite_id = self.keyboard_focus_sprite as i16;
                    let sprite = self.movie.score.get_sprite(sprite_id);
                    let member = sprite
                        .and_then(|s| s.member.as_ref())
                        .and_then(|m| self.movie.cast_manager.find_member_by_ref(m));
                    match member.map(|m| &m.member_type) {
                        Some(crate::player::cast_member::CastMemberType::Field(f)) if f.editable => {
                            let len = f.text.len() as i32;
                            let lo = f.sel_start.min(f.sel_end).clamp(0, len);
                            let hi = f.sel_start.max(f.sel_end).clamp(0, len);
                            f.text[lo as usize..hi as usize].to_string()
                        }
                        Some(crate::player::cast_member::CastMemberType::Text(t))
                            if t.info.as_ref().map_or(false, |i| i.editable) =>
                        {
                            let len = t.text.len() as i32;
                            let lo = t.sel_start.min(t.sel_end).clamp(0, len);
                            let hi = t.sel_start.max(t.sel_end).clamp(0, len);
                            t.text[lo as usize..hi as usize].to_string()
                        }
                        _ => String::new(),
                    }
                } else {
                    String::new()
                };
                Ok(self.alloc_datum(Datum::String(s)))
            },
            "clipBoard" => {
                Ok(self.alloc_datum(Datum::String(self.clipboard_mirror.clone())))
            },
            "frameTempo" => {
                // Get tempo from current frame in score, or use default frame_rate
                let frame_tempo = self.movie.score.get_frame_tempo(self.movie.current_frame)
                    .unwrap_or(self.movie.frame_rate as u32);
                Ok(self.alloc_datum(Datum::Int(frame_tempo as i32)))
            },
            "mouseLoc" => {
                Ok(self.alloc_datum(Datum::Point([self.mouse_loc.0 as f64, self.mouse_loc.1 as f64], 0)))
            },
            "mouseH" => Ok(self.alloc_datum(Datum::Int(self.mouse_loc.0 as i32))),
            "mouseV" => Ok(self.alloc_datum(Datum::Int(self.mouse_loc.1 as i32))),
            "stillDown" => Ok(self.alloc_datum(datum_bool(self.movie.mouse_down))),
            "rollover" => {
                let sprite = get_sprite_at(self, self.mouse_loc.0, self.mouse_loc.1, false);
                Ok(self.alloc_datum(Datum::Int(sprite.unwrap_or(0) as i32)))
            },
            "keyCode" => Ok(self.alloc_datum(Datum::Int(self.keyboard_manager.key_code() as i32))),
            "shiftDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_shift_down()))),
            "optionDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_alt_down()))),
            "commandDown" => {
                Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_command_down())))
            },
            "controlDown" => {
                Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_control_down())))
            },
            "altDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_alt_down()))),
            "key" => Ok(self.alloc_datum(Datum::String(self.keyboard_manager.key()))),
            "keyPressed" => Ok(self.alloc_datum(Datum::String(self.keyboard_manager.key_pressed()))),
            "floatPrecision" => Ok(self.alloc_datum(Datum::Int(self.float_precision as i32))),
            "doubleClick" => Ok(self.alloc_datum(datum_bool(self.is_double_click))),
            "ticks" => Ok(self.alloc_datum(Datum::Int(get_elapsed_ticks(self.system_start_time)))),
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
            },
            "currentSpriteNum" => {
                // TODO: this can also be called by a static script
                let script_instance_ref = self
                    .scopes
                    .get(self.current_scope_ref())
                    .and_then(|scope| scope.receiver.clone());

                if let Some(script_instance_ref) = script_instance_ref {
                    // Try to get spriteNum from the script instance
                    if let Some(datum_ref) = script_get_prop_opt(self, &script_instance_ref, &"spriteNum".to_owned()) {
                        let datum = self.get_datum(&datum_ref);
                        // Check if it's Void - if so, return 0 as default
                        if !matches!(datum, Datum::Void) {
                            if let Ok(sprite_num) = datum.int_value() {
                                return Ok(self.alloc_datum(Datum::Int(sprite_num)));
                            }
                        }
                    }
                }

                // Default: return 0 when no sprite context is available
                Ok(self.alloc_datum(Datum::Int(0)))
            },
            "actorList" => {
                // Return the reference to the global actorList, not a clone of its contents
                Ok(self
                    .globals
                    .get("actorList")
                    .unwrap_or(&DatumRef::Void)
                    .clone())
            },
            "clickOn" => Ok(self.alloc_datum(Datum::Int(self.click_on_sprite as i32))),
            "environment" | "environmentPropList" => {
                // Build the environment property list
                let props = VecDeque::from(vec![
                    (
                        self.alloc_datum(Datum::Symbol("shockMachine".to_string())),
                        self.alloc_datum(Datum::Int(0))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("shockMachineVersion".to_string())),
                        self.alloc_datum(Datum::String("".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("platform".to_string())),
                        self.alloc_datum(Datum::String("Windows,32".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("runMode".to_string())),
                        self.alloc_datum(Datum::String(
                            self.external_params.get("_runMode")
                                .cloned()
                                .unwrap_or_else(|| "Plugin".to_string())
                        ))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("colorDepth".to_string())),
                        self.alloc_datum(Datum::Int(32))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("internetConnected".to_string())),
                        self.alloc_datum(Datum::Symbol("online".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("uiLanguage".to_string())),
                        self.alloc_datum(Datum::String("English".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("osLanguage".to_string())),
                        self.alloc_datum(Datum::String("English".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("productBuildVersion".to_string())),
                        self.alloc_datum(Datum::String("188".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("productVersion".to_string())),
                        self.alloc_datum(Datum::String("10.1".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("osVersion".to_string())),
                        self.alloc_datum(Datum::String("Windows XP,5,1,148,2,Service Pack 3".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("directXVersion".to_string())),
                        self.alloc_datum(Datum::String("9.0.0".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("licenseType".to_string())),
                        self.alloc_datum(Datum::String("Full".to_string()))
                    ),
                    (
                        self.alloc_datum(Datum::Symbol("trialTime".to_string())),
                        self.alloc_datum(Datum::Int(0))
                    ),
                ]);
                Ok(self.alloc_datum(Datum::PropList(props, false)))
            },
            "clickLoc" => {
                Ok(self.alloc_datum(Datum::Point([self.movie.click_loc.0 as f64, self.movie.click_loc.1 as f64], 0)))
            },
            "markerList" => {
                let labels: Vec<_> = self.movie.score.frame_labels
                    .iter()
                    .map(|fl| (fl.label.clone(), fl.frame_num))
                    .collect();
                let props: VecDeque<(DatumRef, DatumRef)> = labels
                    .into_iter()
                    .map(|(label, frame_num)| {
                        let label = self.alloc_datum(Datum::String(label));
                        let frame_num = self.alloc_datum(Datum::Int(frame_num));
                        (label, frame_num)
                    })
                    .collect();
                Ok(self.alloc_datum(Datum::PropList(props, false)))
            },
            "xtraList" => {
                let xtra_names = xtra::manager::get_registered_xtra_names();
                let xtra_list: VecDeque<DatumRef> = xtra_names
                    .iter()
                    .map(|name| {
                        let name_key = self.alloc_datum(Datum::Symbol("name".to_string()));
                        let name_val = self.alloc_datum(Datum::String(name.to_string()));
                        self.alloc_datum(Datum::PropList(VecDeque::from(vec![(name_key, name_val)]), false))
                    })
                    .collect();
                Ok(self.alloc_datum(Datum::List(crate::director::lingo::datum::DatumType::List, xtra_list, false)))
            },
            "runMode" => {
                let mode = self.external_params.get("_runMode")
                    .cloned()
                    .unwrap_or_else(|| "Plugin".to_string());
                Ok(self.alloc_datum(Datum::String(mode)))
            },
            // `the moviePath` precedence (read at every Lingo access so
            // it stays correct even if external_params change post-load):
            //   1. external_params["_moviePath"]  — declarative label
            //   2. movie_path_label               — JS API label
            //   3. self.movie.base_path           — actual loaded path
            //      (handled by Movie::get_prop in the fallthrough below)
            // Director's `the moviePath` is the directory part of the
            // movie's location — NOT the full file URL. So if the label
            // looks like a full URL (`http://host/dir/movie.dcr`), strip
            // the filename and return `http://host/dir/`. This matches
            // the load-time logic in `load_movie_from_dir`. Label sources
            // do NOT trigger URL rewriting in net handlers — that's
            // reserved for `movie_path_override`. See `set_movie_path_label`.
            "moviePath" => {
                let label = self.external_params
                    .get("_moviePath")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .or_else(|| {
                        self.movie_path_label
                            .as_ref()
                            .filter(|s| !s.is_empty())
                            .cloned()
                    });
                if let Some(path) = label {
                    let base_path = if let Ok(url) = Url::parse(&path) {
                        get_base_url(&url).to_string()
                    } else if let Some(pos) = path.rfind('/') {
                        path[..=pos].to_string()
                    } else if let Some(pos) = path.rfind('\\') {
                        path[..=pos].to_string()
                    } else {
                        // Single-segment path: nothing to strip — append
                        // a separator so concatenation with movieName
                        // produces a well-formed path.
                        let mut s = path.clone();
                        if !s.ends_with('/') && !s.ends_with('\\') {
                            s.push('/');
                        }
                        s
                    };
                    Ok(self.alloc_datum(Datum::String(base_path)))
                } else {
                    let datum = self.movie.get_prop(prop)?;
                    Ok(self.alloc_datum(datum))
                }
            },
            // `the movieName` mirrors `the moviePath` — when a label is
            // active, return the FILENAME portion (last path segment).
            // Scripts use `the moviePath & the movieName` to reconstruct
            // the full URL for security/whitelist checks. Without this
            // companion intercept, `movieName` would still come from the
            // actually-loaded file, breaking the concatenation.
            "movieName" => {
                let label = self.external_params
                    .get("_moviePath")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .or_else(|| {
                        self.movie_path_label
                            .as_ref()
                            .filter(|s| !s.is_empty())
                            .cloned()
                    });
                if let Some(path) = label {
                    let file_name = if let Ok(url) = Url::parse(&path) {
                        url.path_segments()
                            .and_then(|s| s.last())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .unwrap_or_default()
                    } else if let Some(pos) = path.rfind(|c| c == '/' || c == '\\') {
                        path[pos + 1..].to_string()
                    } else {
                        path.clone()
                    };
                    Ok(self.alloc_datum(Datum::String(file_name)))
                } else {
                    let datum = self.movie.get_prop(prop)?;
                    Ok(self.alloc_datum(datum))
                }
            },
            _ => {
                let datum = self.movie.get_prop(prop)?;
                Ok(self.alloc_datum(datum))
            }
        })
    }

    fn get_player_prop(&mut self, prop: &str) -> Result<DatumRef, ScriptError> {
        match prop {
            "traceScript" => Ok(self.alloc_datum(datum_bool(false))), // TODO
            "productVersion" => Ok(self.alloc_datum(Datum::String("10.1".to_string()))), // TODO
            "runMode" => {
                let mode = self.external_params.get("_runMode")
                    .cloned()
                    .unwrap_or_else(|| "Plugin".to_string());
                Ok(self.alloc_datum(Datum::String(mode)))
            }
            // Key state properties (also accessible via _key.optionDown etc.)
            "optionDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_alt_down()))),
            "commandDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_command_down()))),
            "controlDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_control_down()))),
            "shiftDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_shift_down()))),
            "altDown" => Ok(self.alloc_datum(datum_bool(self.keyboard_manager.is_alt_down()))),
            "keyCode" => Ok(self.alloc_datum(Datum::Int(self.keyboard_manager.key_code() as i32))),
            "key" => Ok(self.alloc_datum(Datum::String(self.keyboard_manager.key()))),
            _ => Err(ScriptError::new(format!("Unknown player prop {}", prop))),
        }
    }

    fn get_mouse_prop(&mut self, prop: &str) -> Result<DatumRef, ScriptError> {
        match prop {
            "doubleClick" => Ok(self.alloc_datum(datum_bool(self.is_double_click))),
            "mouseLoc" => {
                Ok(self.alloc_datum(Datum::Point([self.mouse_loc.0 as f64, self.mouse_loc.1 as f64], 0)))
            }
            _ => Err(ScriptError::new(format!("Unknown _mouse prop {}", prop))),
        }
    }

    fn set_mouse_prop(&mut self, prop: &str, value_ref: &DatumRef) -> Result<(), ScriptError> {
        match prop {
            "mouseLoc" => {
                let value = self.get_datum(value_ref).clone();
                match value {
                    Datum::Point(vals, _flags) => {
                        let x = vals[0] as i32;
                        let y = vals[1] as i32;
                        self.mouse_loc = (x, y);
                        // Game is programmatically warping the mouse — this is the FPS mouselook pattern.
                        // Only request pointer lock if cursor is hidden AND 3D content is active.
                        if self.cursor_is_hidden && !self.w3d_frame_buffers.is_empty() {
                            self.wants_pointer_lock = true;
                        }
                        Ok(())
                    }
                    _ => Err(ScriptError::new("mouseLoc requires a point value".to_string())),
                }
            }
            _ => Err(ScriptError::new(format!("Cannot set _mouse prop {}", prop))),
        }
    }

    fn set_player_prop(&mut self, prop: &str, value: &DatumRef) -> Result<(), ScriptError> {
        match prop {
            "itemDelimiter" => {
                let value = self.get_datum(value);
                self.movie.item_delimiter = (value.string_value()?).chars().next().unwrap();
                Ok(())
            }
            "traceScript" => {
                // Accepted, no-op
                Ok(())
            }
            "debugPlaybackEnabled" => {
                let v = self.get_datum(value).int_value()? != 0;
                self.movie.debug_playback_enabled = v;
                Ok(())
            }
            _ => Err(ScriptError::new(format!("Cannot set player prop {}", prop))),
        }
    }

    fn get_anim_prop(&self, prop_id: u16) -> Result<Datum, ScriptError> {
        let prop_name = get_anim_prop_name(prop_id);
        match prop_name {
            "colorDepth" => Ok(Datum::Int(32)),
            "fullColorPermit" => Ok(Datum::Int(1)), // Full color mode is permitted
            "timer" => Ok(Datum::Int(get_elapsed_ticks(self.start_time))),
            "timeoutLength" | "timeoutKeyDown" | "timeoutMouse" | "timeoutPlay" => Ok(Datum::Int(0)),
            "timeoutLapsed" => Ok(Datum::Int(0)),
            "soundEnabled" => Ok(Datum::Int(1)),
            "soundLevel" => Ok(Datum::Int(7)), // max volume
            "beepOn" | "fixStageSize" => Ok(Datum::Int(0)),
            "centerStage" => Ok(datum_bool(self.center_stage)),
            "exitLock" => Ok(datum_bool(self.movie.exit_lock)),
            "key" => Ok(Datum::String(self.keyboard_manager.key())),
            "keyPressed" => Ok(Datum::String(self.keyboard_manager.key_pressed())),
            "keyCode" => Ok(Datum::Int(self.keyboard_manager.key_code() as i32)),
            "stageColor" => Ok(Datum::Int(0)),
            "doubleClick" => Ok(datum_bool(self.is_double_click)),
            "lastClick" | "lastEvent" | "lastKey" | "lastRoll" => {
                Ok(Datum::Int(get_elapsed_ticks(self.start_time)))
            }
            "multiSound" => Ok(Datum::Int(1)),
            "pauseState" => Ok(datum_bool(self.is_script_paused)),
            "selStart" => Ok(Datum::Int(self.text_selection_start as i32)),
            "selEnd" => Ok(Datum::Int(self.text_selection_end as i32)),
            "switchColorDepth" | "imageDirect" | "colorQD" | "quickTimePresent"
            | "videoForWindowsPresent" | "netPresent" | "safePlayer"
            | "soundKeepDevice" | "soundMixMedia" | "preLoadRAM"
            | "buttonStyle" | "checkBoxAccess" | "checkboxType" => Ok(Datum::Int(0)),
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
        match_ci!(prop, {
            "keyboardFocusSprite" => {
                // TODO switch focus
                self.keyboard_focus_sprite = value.int_value()? as i16;
                Ok(())
            },
            "selStart" => {
                self.text_selection_start = value.int_value()? as u16;
                Ok(())
            },
            "selEnd" => {
                self.text_selection_end = value.int_value()? as u16;
                Ok(())
            },
            "clipBoard" => {
                self.clipboard_mirror = value.string_value()?;
                Ok(())
            },
            "floatPrecision" => {
                self.float_precision = value.int_value()? as u8;
                Ok(())
            },
            "centerStage" => {
                self.center_stage = value.int_value()? != 0;
                crate::player::stage::apply_stage_draw_rect(self);
                let (w, h) = crate::player::stage::stage_canvas_dims(self);
                crate::js_api::JsApi::dispatch_stage_size_changed(w, h, self.center_stage);
                Ok(())
            },
            "actorList" => {
                // Setting actorList - update the global variable
                match value {
                    Datum::List(list_type, list_items, sorted) => {
                        let new_actor_list =
                            self.alloc_datum(Datum::List(list_type, list_items, sorted));
                        self.globals.insert("actorList".to_string(), new_actor_list);
                        self.actor_list_generation = self.actor_list_generation.wrapping_add(1);
                        Ok(())
                    }
                    _ => Err(ScriptError::new("actorList must be a list".to_string())),
                }
            },
            _ => self.movie.set_prop(prop, value, &self.allocator)
        })
    }

    fn on_script_error(&mut self, err: &ScriptError) {
        // abort is flow control (exits handler chain), not a real error
        if err.code == ScriptErrorCode::Abort {
            return;
        }
        web_sys::console::error_1(&format!("[!!] play failed with error: {}", err.message).into());
        warn!("[!!] play failed with error: {}", err.message);
        self.stop();

        // Dispatch debug update with full call stack (scopes are preserved on error)
        JsApi::dispatch_debug_update(self);
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
            // Try to get some context about what's on the stack
            let mut stack_trace = String::from("Stack overflow detected - this is likely due to infinite recursion in the movie's Lingo scripts.\nRecent scope stack:\n");
            let start = if self.scope_count > 10 { self.scope_count - 10 } else { 0 };
            for i in start..self.scope_count {
                if let Some(scope) = self.scopes.get(i as ScopeRef) {
                    // Try to get the handler name from the script
                    let handler_info = if let Some(script) = self.movie.cast_manager.get_script_by_ref(&scope.script_ref) {
                        // Find handler name by looking through the handlers map
                        let handler_name = script.handlers.iter()
                            .find(|(_, h)| h.name_id == scope.handler_name_id)
                            .map(|(name, _)| name.as_str().to_owned())
                            .unwrap_or_else(|| format!("handler_name_id#{}", scope.handler_name_id));
                        format!("{}::{}", script.name, handler_name)
                    } else {
                        format!("unknown_script::handler_name_id#{}", scope.handler_name_id)
                    };
                    stack_trace.push_str(&format!("  Scope {}: {} (bytecode_index={})\n", i, handler_info, scope.bytecode_index));
                }
            }
            stack_trace.push_str("\nThis usually indicates a bug in the Director movie's scripts (e.g., a handler calling itself infinitely).\n");
            stack_trace.push_str("Note: If this is happening during frame events, it may be a re-entrant call issue.\n");
            web_sys::console::error_1(&stack_trace.into());
            panic!("Stack overflow - infinite recursion in Lingo scripts");
        }
        let scope_ref = self.scope_count;
        let scope = self.scopes.get_mut(scope_ref as ScopeRef).unwrap();
        let prev_gen = scope.generation;
        scope.reset();
        scope.generation = prev_gen + 1;
        self.scope_count += 1;
        scope_ref as ScopeRef
    }

    pub fn pop_scope(&mut self) {
        // Guard against underflow. A stale handler resuming from an async JS
        // callback after a test reset (Ruffle init, MP3 decode, fetch) can
        // decrement scope_count on a freshly-reset player. A wrapping subtract
        // would drive `scope_count` to u32::MAX-N and poison every later
        // `scope_count - 1` index computation.
        if self.scope_count > 0 {
            self.scope_count -= 1;
        } else {
            warn!("pop_scope called with scope_count=0 (stale handler?)");
        }
    }

    pub fn current_scope_ref(&self) -> ScopeRef {
        self.scope_count.saturating_sub(1) as ScopeRef
    }

    // Lingo: sound(channelNum)
    pub fn get_sound_channel(&mut self, channel_num: i32) -> Result<DatumRef, ScriptError> {
        // Store the 1-based channel number (conversion to 0-based happens in get_channel_index)
        Ok(self.alloc_datum(Datum::SoundChannel(channel_num as u16)))
    }

    // Lingo: puppetSound channelNum, memberRef
    pub fn puppet_sound(
        &mut self,
        channel_num: i32,
        member_ref: DatumRef,
    ) -> Result<(), ScriptError> {
        let sound_channel = self.get_sound_channel(channel_num)?;
        
        // Get the loop setting from the cast member
        let loop_count = {
            let member_datum = self.get_datum(&member_ref);
            if let Datum::CastMember(cast_member_ref) = member_datum {
                if let Some(cast_member) = self.movie.cast_manager.find_member_by_ref(cast_member_ref) {
                    if let CastMemberType::Sound(sound_member) = &cast_member.member_type {
                        if sound_member.info.loop_enabled {
                            0  // Loop forever
                        } else {
                            1  // Play once
                        }
                    } else {
                        1
                    }
                } else {
                    1
                }
            } else {
                1
            }
        };
        
        // Set the loop count on the channel BEFORE playing
        let channel_rc = self.sound_manager
            .get_channel_mut((channel_num - 1) as usize)
            .ok_or_else(|| ScriptError::new(format!("Invalid sound channel {}", channel_num)))?;
        
        {
            let mut channel = channel_rc.borrow_mut();
            channel.loop_count = loop_count;
            channel.loops_remaining = loop_count;
        }
        
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

    pub fn is_yield_safe(&self) -> bool {
        !self.is_in_frame_update
        && !self.in_frame_script
        && !self.in_enter_frame
        && !self.in_prepare_frame
        && !self.in_event_dispatch
    }

    /// Process filmloop frame changes and sprite updates
    /// Returns list of (member_ref, old_frame, new_frame) for filmloops that changed
    pub async fn update_filmloop_frames(&mut self) -> Vec<(CastMemberRef, u32, u32)> {
        let mut changed_filmloops = Vec::new();

        // Collect active filmloop refs with their current and next frames
        let active_filmloops: Vec<(CastMemberRef, u32, u32)> = self
            .active_stage_filmloop_member_refs()
            .into_iter()
            .filter_map(|member_ref| {
                self.movie
                    .cast_manager
                    .find_member_by_ref(&member_ref)
                    .and_then(|m| {
                        if let CastMemberType::FilmLoop(film_loop) = &m.member_type {
                            let current = film_loop.current_frame;

                            let frame_count = film_loop.score.frame_count.unwrap_or(1).max(1);

                            let next = current + 1;
                            let should_loop = (film_loop.info.loops & 0x20) == 0;
                            
                            let new_frame = if next > frame_count {
                                if should_loop { 1 } else { frame_count }
                            } else {
                                next
                            };
                            
                            Some((member_ref, current, new_frame))
                        } else {
                            None
                        }
                    })
            })
            .collect();
        
        // Process each filmloop
        for (member_ref, old_frame, new_frame) in active_filmloops {
            // Skip if frame didn't change
            if old_frame == new_frame {
                continue;
            }

            // End sprites that are leaving
            let score_ref = ScoreRef::FilmLoop(member_ref.clone());
            let ended_sprites = if let Some(member) = self.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                if let CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                    film_loop.score.end_sprites(score_ref.clone(), old_frame, new_frame).await
                } else {
                    vec![]
                }
            } else {
                vec![]
            };
            
            if let Some(member) = self.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                if let CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                    for sprite_num in ended_sprites {
                        let sprite = film_loop.score.get_sprite_mut(sprite_num as i16);
                        sprite.exited = true;
                    }
                    film_loop.current_frame = new_frame;
                    film_loop.score.begin_sprites(score_ref.clone(), new_frame);
                    film_loop.score.apply_tween_modifiers(new_frame);
                }
            }
            
            changed_filmloops.push((member_ref, old_frame, new_frame));
        }

        if !changed_filmloops.is_empty() {
            self.invalidate_behavior_channel_cache();
        }

        changed_filmloops
    }

    /// Just advance frame counters - actual sprite management happens in update_filmloop_frames
    fn advance_filmloop_frames(&mut self) {
        let active_filmloop_refs = self.active_stage_filmloop_member_refs();

        for member_ref in active_filmloop_refs {
            if let Some(member) = self.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                if let CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                    let frame_count = film_loop.score.frame_count.unwrap_or(1).max(1);

                    let old_frame = film_loop.current_frame;
                    film_loop.current_frame += 1;

                    if film_loop.current_frame > frame_count {
                        let should_loop = (film_loop.info.loops & 0x20) == 0;

                        if should_loop {
                            film_loop.current_frame = 1;
                        } else {
                            film_loop.current_frame = frame_count;
                        }
                    }
                }
            }
        }
    }
}

pub fn player_alloc_datum(datum: Datum) -> DatumRef {
    // let mut player_opt = PLAYER_LOCK.try_write().unwrap();
    unsafe {
        let player = PLAYER_OPT.as_mut().unwrap();
        player.alloc_datum(datum)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ScriptErrorCode {
    HandlerNotFound,
    Generic,
    Abort,
}

#[derive(Debug, Clone)]
pub struct ScriptError {
    pub code: ScriptErrorCode,
    pub message: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<ScriptError> for String {
    fn from(e: ScriptError) -> String {
        e.message
    }
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

pub async fn player_call_global_handler(
    handler_name: &str,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let receiver_handler = unsafe {
        // let player_opt = PLAYER_LOCK.try_read().unwrap();
        let player = PLAYER_OPT.as_mut().unwrap();

        let mut receiver_handler = None;

        // "new" invocations should always go through the built-in handler
        if handler_name != "new" {
            // Director appears to support customFunc(firstArg, ..) invocations
            // where firstArg is a script or script instance
            receiver_handler = ScriptInstanceUtils::get_handler_from_first_arg(&args, handler_name);

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

            if receiver_handler.is_none() {
                receiver_handler = player
                    .active_stage_script_instance_ids()
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
    }

    // Check virtual scripts for global handler calls
    let virtual_result = reserve_player_mut(|player| {
        virtual_scripts::VirtualScriptRegistry::try_call_any_global_handler(player, handler_name, args)
    });
    match virtual_result {
        Ok(Some(result)) => return Ok(result),
        Err(e) => return Err(e),
        Ok(None) => {}
    }

    if BuiltInHandlerManager::has_async_handler(handler_name) {
        return Box::pin(BuiltInHandlerManager::call_async_handler(
            handler_name,
            args,
        ))
        .await;
    } else {
        return BuiltInHandlerManager::call_handler(handler_name, args);
    }
}

#[inline(always)]
pub fn reserve_player_ref<T, F>(callback: F) -> T
where
    F: FnOnce(&DirPlayer) -> T,
{
    unsafe {
        let player = PLAYER_OPT.as_ref().unwrap_unchecked();
        callback(player)
    }
}

#[inline(always)]
pub fn reserve_player_mut<T, F>(callback: F) -> T
where
    F: FnOnce(&mut DirPlayer) -> T,
{
    unsafe {
        let player = PLAYER_OPT.as_mut().unwrap_unchecked();
        callback(player)
    }
}

/// Direct reference access without closure overhead.
/// Caller must ensure no mutable references exist.
#[inline(always)]
pub unsafe fn player_ref() -> &'static DirPlayer {
    PLAYER_OPT.as_ref().unwrap_unchecked()
}

/// Direct mutable reference access without closure overhead.
/// Caller must ensure no other references exist.
#[inline(always)]
pub unsafe fn player_mut() -> &'static mut DirPlayer {
    PLAYER_OPT.as_mut().unwrap_unchecked()
}

fn reserve_player_mut_async<F, R>(callback: F) -> impl Future<Output = R>
where
    F: for<'a> FnOnce(&'a mut DirPlayer) -> Pin<Box<dyn Future<Output = R> + 'a>>,
{
    async move {
        unsafe {
            let player = PLAYER_OPT.as_mut().unwrap();
            callback(player).await
        }
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum ScriptReceiver {
    Script(CastMemberRef),
    ScriptInstance(ScriptInstanceRef),
    ScriptText(String),
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

    // Check for virtual script handler before running bytecode
    let virtual_result = reserve_player_mut(|player| {
        virtual_scripts::VirtualScriptRegistry::try_call_handler(player, script_member_ref, receiver.as_ref(), handler_name, arg_list)
    });
    match virtual_result {
        Ok(Some(return_value)) => {
            return Ok(ScopeResult {
                return_value,
                passed: false,
            });
        }
        Ok(None) => {}
        Err(e) => return Err(e),
    }

    // Check if this is a frame script handler
    let is_frame_script = reserve_player_ref(|player| {
        let frame_script = player.movie.score.get_script_in_frame(player.movie.current_frame);
        if let Some(fs) = frame_script {
            let frame_script_ref = CastMemberRef {
                cast_lib: fs.cast_lib.into(),
                cast_member: fs.cast_member.into(),
            };
            script_member_ref == &frame_script_ref
        } else {
            false
        }
    });

    reserve_player_mut(|player| {
        player.handler_stack_depth += 1;
        if is_frame_script {
            player.in_frame_script = true;
        }
    });

    let (scope_ref, handler_ptr, script_ptr, names_ptr) = reserve_player_mut(|player| {
        let (script_ptr, handler_ptr, handler_name_id, script_type, names_ptr) = {
            let script_rc = player
                .movie
                .cast_manager
                .get_script_by_ref(&script_member_ref)
                .unwrap();
            let script = script_rc.as_ref();
            let script_ptr = script as *const Script;
            let names_ptr = player
                .movie
                .cast_manager
                .get_cast(script.member_ref.cast_lib as u32)
                .unwrap()
                .lctx
                .as_ref()
                .map(|lctx| &lctx.names as *const Vec<String>)
                .unwrap();
            let handler = script.get_own_handler(&handler_name);

            if let Some(handler_rc) = handler {
                let handler_name_id = handler_rc.name_id;
                let handler_ptr: *const HandlerDef = handler_rc.as_ref();
                Ok((script_ptr, handler_ptr, handler_name_id, script.script_type, names_ptr))
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

        Ok((scope_ref, handler_ptr, script_ptr, names_ptr))
    })?;

    let ctx = BytecodeHandlerContext {
        scope_ref,
        handler_def_ptr: handler_ptr,
        script_ptr,
        names_ptr,
    };

    // Trace handler entry if traceScript is enabled
    reserve_player_ref(|player| {
        if player.movie.trace_script {
            let trace_file = player.movie.trace_log_file.clone();
            let (cast_lib, cast_member) = (
                script_member_ref.cast_lib,
                script_member_ref.cast_member
            );
            let msg = format!(
                "== Script: (member {} of castLib {}) Handler: {}",
                cast_member, cast_lib, handler_name
            );
            trace_output(player, &msg);
            
            // ADD THIS BLOCK HERE - Clear expression tracker for new handler
            use crate::player::bytecode::handler_manager::EXPRESSION_TRACKER;
            EXPRESSION_TRACKER.with(|tracker| {
                tracker.borrow_mut().clear();
            });
        }
    });

    let mut should_return = false;
    let scope_generation = reserve_player_ref(|player| {
        player.scopes.get(scope_ref).unwrap().generation
    });

    loop {
        // Check if scope was reused (generation changed = scope was popped and re-pushed)
        let current_gen = reserve_player_ref(|player| {
            player.scopes.get(scope_ref).unwrap().generation
        });
        if current_gen != scope_generation {
            break;
        }

        // Single player access to read bytecode_index and debugger state
        let (bytecode_index, debugger_active) = reserve_player_ref(|player| {
            let bi = player.scopes.get(scope_ref).unwrap().bytecode_index;
            let debugging = !player.breakpoint_manager.breakpoints.is_empty()
                || !matches!(player.step_mode, StepMode::None);
            (bi, debugging)
        });

        // Only check breakpoints and step mode if the debugger is actually active
        if debugger_active {
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

            let should_step_break = reserve_player_ref(|player| {
                match &player.step_mode {
                    StepMode::None => false,
                    StepMode::Into => true,
                    StepMode::IntoLine { skip_bytecode_indices } => {
                        !skip_bytecode_indices.contains(&bytecode_index)
                    }
                    StepMode::Over => player.scope_count <= player.step_scope_depth,
                    StepMode::OverLine { skip_bytecode_indices } => {
                        player.scope_count <= player.step_scope_depth
                            && !skip_bytecode_indices.contains(&bytecode_index)
                    }
                    StepMode::Out => player.scope_count < player.step_scope_depth,
                }
            });

            if should_step_break {
                let breakpoint = Breakpoint {
                    script_name: unsafe { (&*script_ptr).name.clone() },
                    handler_name: handler_name.clone(),
                    bytecode_index,
                };
                player_trigger_breakpoint(
                    breakpoint,
                    script_member_ref.to_owned(),
                    handler_ref.to_owned(),
                    bytecode_index,
                )
                .await;
            }
        }

        let result = match player_execute_bytecode(&ctx).await {
            Ok(result) => result,
            Err(err) => {
                // abort is flow control, not a real error - skip break-on-error
                if err.code != ScriptErrorCode::Abort {
                    let should_break = reserve_player_ref(|player| player.break_on_error);
                    if should_break {
                        let (current_script_ref, current_bytecode_idx) = reserve_player_ref(|player| {
                            let scope = player.scopes.get(scope_ref).unwrap();
                            (scope.script_ref.clone(), scope.bytecode_index)
                        });
                        player_trigger_error_pause(
                            err.clone(),
                            current_script_ref,
                            (script_member_ref.clone(), handler_name.clone()),
                            current_bytecode_idx,
                        ).await;
                    }
                }
                // Cleanup on error
                reserve_player_mut(|player| {
                    player.handler_stack_depth = player.handler_stack_depth.saturating_sub(1);
                    if is_frame_script {
                        player.in_frame_script = false;
                    }
                    player.pop_scope();
                });
                return Err(err);
            }
        };

        // Check if scope was reused after an async yield point
        let post_gen = reserve_player_ref(|player| {
            player.scopes.get(scope_ref).unwrap().generation
        });
        if post_gen != scope_generation {
            break;
        }

        match result {
            HandlerExecutionResult::Advance => {
                let handler = unsafe { &*ctx.handler_def_ptr };
                reserve_player_mut(|player| {
                    let scope = player.scopes.get_mut(scope_ref).unwrap();
                    if scope.bytecode_index + 1 >= handler.bytecode_array.len() {
                        should_return = true;
                    } else {
                        scope.bytecode_index += 1;
                    }
                });
            }
            HandlerExecutionResult::Stop => {
                should_return = true;
            }
            HandlerExecutionResult::Error(err) => {
                // abort is flow control, not a real error - skip break-on-error
                if err.code != ScriptErrorCode::Abort {
                    let should_break = reserve_player_ref(|player| player.break_on_error);
                    if should_break {
                        let (current_script_ref, current_bytecode_idx) = reserve_player_ref(|player| {
                            let scope = player.scopes.get(scope_ref).unwrap();
                            (scope.script_ref.clone(), scope.bytecode_index)
                        });
                        player_trigger_error_pause(
                            err.clone(),
                            current_script_ref,
                            (script_member_ref.clone(), handler_name.clone()),
                            current_bytecode_idx,
                        ).await;
                    }
                }
                // Cleanup on error
                reserve_player_mut(|player| {
                    player.handler_stack_depth = player.handler_stack_depth.saturating_sub(1);
                    if is_frame_script {
                        player.in_frame_script = false;
                    }
                    player.pop_scope();
                });
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
        // Trace handler exit
        if player.movie.trace_script {
            trace_output(player, "--> end");
        }

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

    // Cleanup after successful execution
    reserve_player_mut(|player| {
        player.handler_stack_depth = player.handler_stack_depth.saturating_sub(1);
        if is_frame_script {
            player.in_frame_script = false;
        }
    });

    return Ok(scope);
}

/// Dispatch stopMovie events and end all sprites. Used by both `run_frame_loop`
/// (when `is_playing` becomes false) and `transition_to_net_movie`.
async fn stop_movie_sequence() {
    dispatch_system_event_to_timeouts(&"stopMovie".to_string(), &vec![]).await;

    reserve_player_mut(|player| {
        player.timeout_manager.clear();
    });

    player_wait_available().await;

    if let Err(err) = player_invoke_global_event(&"stopMovie".to_string(), &vec![]).await {
        if err.code != ScriptErrorCode::Abort {
            reserve_player_mut(|player| player.on_script_error(&err));
        }
    }

    player_wait_available().await;

    let ended_sprite_nums = reserve_player_mut_async(|player| {
        Box::pin(async move { player.end_all_sprites().await })
    }).await;

    player_wait_available().await;

    reserve_player_mut(|player| {
        for (score_source, sprite_num) in ended_sprite_nums.iter() {
            if let Some(sprite) = get_score_sprite_mut(
                &mut player.movie, score_source, *sprite_num as i16
            ) {
                sprite.exited = true;
            }
        }
    });

    player_wait_available().await;
}

/// Run the movie initialization sequence: prepareMovie, beginSprite, behavior init,
/// stepFrame, prepareFrame, startMovie, enterFrame, exitFrame.
/// Shared by `play()` and `transition_to_net_movie`.
async fn run_movie_init_sequence() {
    // prepareMovie
    debug!(">>> Dispatching prepareMovie");
    dispatch_system_event_to_timeouts(&"prepareMovie".to_string(), &vec![]).await;

    if let Err(err) = player_invoke_global_event(&"prepareMovie".to_string(), &vec![]).await {
        web_sys::console::error_1(&format!("prepareMovie FAILED: {}", err.message).into());
        if err.code != ScriptErrorCode::Abort {
            reserve_player_mut(|player| player.on_script_error(&err));
        }
        return;
    }
    debug!(">>> prepareMovie completed successfully");

    // Log bPreloadCasts state after prepareMovie
    reserve_player_ref(|player| {
        let val = player.globals.get("bPreloadCasts");
        let desc = match val {
            Some(r) => format!("{}", player.get_datum(r).type_str()),
            None => "NOT SET".to_string(),
        };
        debug!(">>> bPreloadCasts after prepareMovie: {}", desc);
    });

    player_wait_available().await;

    // Dispatch streamStatus for any resources already loaded by the time prepareMovie
    // enables the handler (movie file, external casts, etc.)
    stream_status::dispatch_pending_stream_status().await;

    // Initialize sprites
    reserve_player_mut(|player| {
        player.movie.frame_script_instance = None;
        player.begin_all_sprites();
        player.movie.score.apply_tween_modifiers(player.movie.current_frame);
    });

    player_wait_available().await;

    // Collect behaviors that need initialization
    let behaviors_to_init: Vec<(ScriptInstanceRef, u32)> = reserve_player_mut(|player| {
        let mut behaviors = Vec::new();
        for channel_number in player.active_stage_behavior_channels() {
            let Some((sprite_num, fallback)) = player
                .movie
                .score
                .channels
                .get(channel_number)
                .map(|channel| {
                    (
                        channel.sprite.number as u32,
                        channel.sprite.script_instance_list.clone(),
                    )
                })
            else {
                continue;
            };

            for behavior_ref in player.get_sprite_script_instance_ids(
                sprite_num as i16,
                fallback.as_slice(),
            ) {
                if player
                    .allocator
                    .get_script_instance_entry(behavior_ref.id())
                    .is_some_and(|entry| !entry.script_instance.begin_sprite_called)
                {
                    behaviors.push((behavior_ref, sprite_num));
                }
            }
        }
        behaviors
    });

    for (behavior_ref, sprite_num) in &behaviors_to_init {
        if let Err(err) = Score::initialize_behavior_defaults_async(behavior_ref.clone(), *sprite_num).await {
            log::warn!("Failed to initialize behavior defaults: {}", err.message);
        }
    }

    player_wait_available().await;

    reserve_player_mut(|player| {
        player.is_in_frame_update = true;
    });

    let begin_sprite_nums = player_dispatch_event_beginsprite(
        &"beginSprite".to_string(),
        &vec![]
    ).await;

    player_wait_available().await;

    reserve_player_mut(|player| {
        for sprite_list in begin_sprite_nums.iter() {
            for (score_source, sprite_num) in sprite_list.iter() {
                if let Some(sprite) = get_score_sprite_mut(
                    &mut player.movie,
                    score_source,
                    *sprite_num as i16,
                ) {
                    for script_ref in &sprite.script_instance_list {
                        if let Some(entry) =
                            player.allocator.get_script_instance_entry_mut(script_ref.id())
                        {
                            entry.script_instance.begin_sprite_called = true;
                        }
                    }
                }
            }
        }
    });

    // Dispatch beginSprite to remaining behaviors not handled above
    let remaining_behaviors: Vec<ScriptInstanceRef> = reserve_player_mut(|player| {
        behaviors_to_init.iter()
            .filter(|(behavior_ref, _)| {
                player.allocator.get_script_instance_entry(behavior_ref.id())
                    .map_or(false, |entry| !entry.script_instance.begin_sprite_called)
            })
            .map(|(behavior_ref, _)| behavior_ref.clone())
            .collect()
    });

    for behavior_ref in &remaining_behaviors {
        let receivers = vec![behavior_ref.clone()];
        let _ = player_invoke_targeted_event(
            &"beginSprite".to_string(),
            &vec![],
            Some(&receivers),
        ).await;
    }

    if !remaining_behaviors.is_empty() {
        reserve_player_mut(|player| {
            for behavior_ref in &remaining_behaviors {
                if let Some(entry) =
                    player.allocator.get_script_instance_entry_mut(behavior_ref.id())
                {
                    entry.script_instance.begin_sprite_called = true;
                }
            }
        });
    }

    player_wait_available().await;

    // stepFrame to actorList — gate on in_step_frame to prevent re-entry.
    let step_frame_entered = reserve_player_mut(|player| {
        if player.in_step_frame { return true; }
        player.in_step_frame = true;
        false
    });
    if !step_frame_entered {
        let (actor_list_snapshot, mut active_actor_ids, mut actor_list_generation) =
            reserve_player_ref(|player| player.actor_list_stepframe_snapshot());

        for (idx, actor_ref) in actor_list_snapshot.iter().enumerate() {
            let still_active = active_actor_ids.contains(&actor_ref.unwrap());

            if still_active {
                let result =
                    player_call_datum_handler(&actor_ref, &"stepFrame".to_string(), &vec![]).await;

                if let Err(err) = result {
                    if err.code == ScriptErrorCode::Abort {
                        reserve_player_mut(|player| {
                            player.is_in_frame_update = false;
                            player.in_step_frame = false;
                        });
                        return;
                    }
                    error!("⚠ stepFrame[{}] error: {}", idx, err.message);
                    reserve_player_mut(|player| {
                        player.on_script_error(&err);
                        player.is_in_frame_update = false;
                        player.in_step_frame = false;
                    });
                    return;
                }

                let refreshed_active_ids = reserve_player_ref(|player| {
                    if player.actor_list_generation != actor_list_generation {
                        Some(player.actor_list_active_ids())
                    } else {
                        None
                    }
                });

                if let Some((next_active_actor_ids, next_actor_list_generation)) = refreshed_active_ids
                {
                    active_actor_ids = next_active_actor_ids;
                    actor_list_generation = next_actor_list_generation;
                }
            }
        }
        reserve_player_mut(|player| { player.in_step_frame = false; });
    }

    reserve_player_mut(|player| {
        player.in_prepare_frame = true;
    });

    dispatch_system_event_to_timeouts(&"prepareFrame".to_string(), &vec![]).await;
    let _ = dispatch_event_to_all_behaviors(&"prepareFrame".to_string(), &vec![]).await;

    reserve_player_mut(|player| {
        player.in_prepare_frame = false;
    });

    // startMovie
    dispatch_system_event_to_timeouts(&"startMovie".to_string(), &vec![]).await;

    if let Err(err) = player_invoke_global_event(&"startMovie".to_string(), &vec![]).await {
        if err.code != ScriptErrorCode::Abort {
            reserve_player_mut(|player| player.on_script_error(&err));
        }
        return;
    }

    player_wait_available().await;

    // enterFrame
    reserve_player_mut(|player| {
        player.in_enter_frame = true;
    });

    let _ = dispatch_event_to_all_behaviors(&"enterFrame".to_string(), &vec![]).await;

    reserve_player_mut(|player| {
        player.in_enter_frame = false;
    });

    player_wait_available().await;

    // exitFrame
    dispatch_system_event_to_timeouts(&"exitFrame".to_string(), &vec![]).await;
    let _ = dispatch_event_to_all_behaviors(&"exitFrame".to_string(), &vec![]).await;

    player_wait_available().await;

    reserve_player_mut(|player| {
        player.is_in_frame_update = false;
    });
}

/// Perform the movie transition for gotoNetMovie.
/// Called from within the frame loop when the pending fetch is complete.
async fn transition_to_net_movie(task_id: u32, target: MovieFrameTarget) {
    // 1. Parse the fetched movie data
    let dir_file = reserve_player_mut(|player| {
        let task = player.net_manager.get_task(task_id);
        let data_result = player.net_manager.get_task_result(Some(task_id));

        match (task, data_result) {
            (Some(task), Some(Ok(data_bytes))) => {
                let file_name = task.resolved_url
                    .path_segments()
                    .and_then(|segments| segments.last())
                    .unwrap_or("untitled.dcr")
                    .to_string();
                let base_url = get_base_url(&task.resolved_url).to_string();
                read_director_file_bytes(&data_bytes, &file_name, &base_url).ok()
            }
            _ => None,
        }
    });

    let dir_file = match dir_file {
        Some(f) => f,
        None => {
            log::warn!("gotoNetMovie: failed to parse movie data for task {}", task_id);
            reserve_player_mut(|player| { player.pending_goto_net_movie = None; });
            return;
        }
    };

    // 2. Shutdown current movie
    // Block the event loop for the entire transition: set is_playing = false and
    // is_in_transition = true. Direct calls to player_invoke_global_event (for
    // stopMovie, prepareMovie, beginSprite, etc.) are unaffected because they
    // execute inline from the frame loop task, not through the event loop channel.
    reserve_player_mut(|player| {
        player.is_playing = false;
        player.is_in_transition = true;
    });

    stop_movie_sequence().await;

    // 3. Load the new movie data (preserving globals and allocator)
    reserve_player_mut(|player| {
        player.movie.score.reset();
        player.clear_script_instance_list_caches();
        player.movie.frame_script_instance = None;
        player.movie.frame_script_member = None;
        player.movie.current_frame = 1;
        player.last_initialized_frame = None;

        for scope in player.scopes.iter_mut() {
            scope.reset();
        }
        player.scope_count = 0;

        // Temporarily set is_playing = false so that begin_all_sprites (called inside
        // load_movie_from_dir) resets entered flags and clears script instance lists,
        // matching the behavior of the initial movie load path.
        player.is_playing = false;
    });

    reserve_player_mut_async(|player| {
        Box::pin(async move {
            player.load_movie_from_dir(dir_file).await;
        })
    }).await;

    // Apply frame target if specified, restore is_playing before init sequence
    reserve_player_mut(|player| {
        match &target {
            MovieFrameTarget::Label(label) => {
                let target_frame = player.movie.score.frame_labels
                    .iter()
                    .find(|fl| fl.label.eq_ignore_ascii_case(label))
                    .map(|fl| fl.frame_num as u32);
                if let Some(frame) = target_frame {
                    player.movie.current_frame = frame;
                }
            }
            MovieFrameTarget::Frame(frame) => {
                player.movie.current_frame = *frame;
            }
            MovieFrameTarget::Default => {}
        }
        player.pending_goto_net_movie = None;
        player.is_playing = true;
    });

    // 4. Run new movie initialization sequence
    run_movie_init_sequence().await;

    reserve_player_mut(|player| {
        player.is_in_transition = false;
    });
}

// JS bridge name uses the `dirplayer_` prefix so this fork's globals don't
// collide with stock Ruffle / other libraries on the same page.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "dirplayer_isFlashLoading", catch)]
    fn is_flash_loading() -> Result<bool, wasm_bindgen::JsValue>;
}
/// Execute one complete frame cycle: run frame scripts, then advance to the next frame.
/// Returns (is_playing, is_script_paused) so callers can check if the movie is still running.
///
/// This contains the core per-frame logic extracted from `run_frame_loop`, minus the
/// timing/delay logic and gotoNetMovie handling which are loop-level concerns.
/// Fire all scheduled timeouts immediately. In the real player, timeouts are
/// driven by JS `setInterval`, but in tests there is no JS event loop (native)
/// or frames run too fast for intervals to fire (browser). Call this each frame
/// in test harnesses before `run_single_frame`.
/// Fire scheduled timeouts whose period has elapsed according to wall-clock time.
/// Each timeout that fires is rescheduled to `now + period`. The time is captured
/// once at the start so handlers that take time don't cause cascading re-fires.
pub async fn fire_pending_timeouts() {
    let now = testing_shared::now_ms();
    let pending_timeouts: Vec<(DatumRef, String, String)> = reserve_player_mut(|player| {
        let mut ready = Vec::new();
        for t in player.timeout_manager.timeouts.values_mut() {
            if t.is_scheduled {
                let remaining = t.next_fire_ms - now;
                if remaining <= 0.0 {
                    t.next_fire_ms = now + t.period as f64;
                    ready.push((t.target_ref.clone(), t.handler.clone(), t.name.clone()));
                }
            }
        }
        ready
    });
    for (target_ref, handler_name, timeout_name) in pending_timeouts {
        let ref_datum = player_alloc_datum(Datum::TimeoutRef(timeout_name.clone()));
        let args = vec![ref_datum];
        let result = if target_ref != DatumRef::Void {
            player_call_datum_handler(&target_ref, &handler_name, &args).await
        } else {
            player_invoke_global_event(&handler_name, &args).await
        };
        if let Err(err) = result {
            if err.code != ScriptErrorCode::HandlerNotFound {
                warn!("Timeout '{}' handler '{}' error: {}", timeout_name, handler_name, err.message);
            }
        }
    }
}

pub async fn run_single_frame() -> (bool, bool) {
    let (mut is_playing, mut is_script_paused) = reserve_player_ref(|player| {
        (player.is_playing, player.is_script_paused)
    });
    if !is_playing {
        return (false, is_script_paused);
    }

    // Dispatch streamStatus for any net tasks that completed since last check
    stream_status::dispatch_pending_stream_status().await;

    // --- Phase 1: Execute frame scripts ---
    if !is_script_paused {
        player_wait_available().await;

        let skip_frame = reserve_player_ref(|player| player.command_handler_yielding || player.in_mouse_command);
        if !skip_frame {
            let update_result = MovieHandlers::execute_frame_update().await;

            reserve_player_mut(|player| {
                if let Err(err) = update_result {
                    if err.code != ScriptErrorCode::Abort {
                        player.on_script_error(&err);
                    }
                }
            });
        }
    }

    // --- Phase 2: Advance to next frame ---
    let mut prev_frame = 0;
    let mut new_frame = 0;
    reserve_player_mut(|player| {
        is_playing = player.is_playing;
        is_script_paused = player.is_script_paused;
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
        return (false, is_script_paused);
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

    if is_script_paused {
        return (is_playing, is_script_paused);
    }

    // Check if delay() is in effect
    let is_delayed = reserve_player_mut(|player| {
        if let Some(until) = player.delay_until {
            if chrono::Local::now() < until {
                true
            } else {
                player.delay_until = None;
                false
            }
        } else {
            false
        }
    });

    let skip_frame = reserve_player_ref(|player| player.command_handler_yielding || player.in_mouse_command);
    if skip_frame || is_delayed {
        return (is_playing, is_script_paused);
    }

    let (has_player_frame_changed, has_frame_changed_in_go, go_direction) =
        reserve_player_ref(|player| {
            (
                player.has_player_frame_changed,
                player.has_frame_changed_in_go,
                player.go_direction
            )
        });

    player_wait_available().await;

    // Relay exitFrame to timeout targets
    dispatch_system_event_to_timeouts(&"exitFrame".to_string(), &vec![]).await;

    let mut stayed_on_same_frame = false;

    // Clear stale go_same_frame from previous frame's processing
    reserve_player_mut(|player| {
        player.go_same_frame = false;
    });

    if has_player_frame_changed {
        player_wait_available().await;

        if has_frame_changed_in_go && go_direction == 1 { // backwards
            dispatch_event_to_all_behaviors(&"exitFrame".to_string(), &vec![]).await;
        } else {
            if let Err(err) = player_invoke_frame_and_movie_scripts(&"exitFrame".to_string(), &vec![]).await {
                if err.code != ScriptErrorCode::Abort {
                    reserve_player_mut(|player| player.on_script_error(&err));
                }
            }
        }

        player_wait_available().await;

        if has_frame_changed_in_go {
            reserve_player_mut(|player| {
                player.has_frame_changed_in_go = false;

                if player.go_direction > 0 {
                    player.go_direction = 0;
                }
            });
        }

        // go() already performed end_all_sprites + advance_frame + begin_all_sprites,
        // so we only clear the flag here.
        (is_playing, is_script_paused) = reserve_player_mut(|player| {
            player.has_player_frame_changed = false;
            player.go_same_frame = false;
            player.has_frame_changed_in_go = false;
            (player.is_playing, player.is_script_paused)
        });
    } else {
        player_wait_available().await;

        dispatch_event_to_all_behaviors(&"exitFrame".to_string(), &vec![]).await;

        player_wait_available().await;

        let (has_frame_changed, go_same_frame) = reserve_player_ref(|player|
            (player.has_frame_changed_in_go, player.go_same_frame));

        if go_same_frame {
            // go(the frame) — stay on current frame, no advancement
            reserve_player_mut(|player| {
                player.go_same_frame = false;
            });
            stayed_on_same_frame = true;
        } else if has_frame_changed {
            // go(differentFrame) — go() already did end/begin/advance
            reserve_player_mut(|player| {
                player.has_frame_changed_in_go = false;
                player.has_player_frame_changed = false;
            });
            stayed_on_same_frame = true;
        } else {
            // No go() called — normal frame advancement
            let ended_sprite_nums = reserve_player_mut_async(|player| {
                Box::pin(async move {
                    player.end_all_sprites().await
                })
            }).await;
            player_wait_available().await;
            reserve_player_mut(|player| {
                for (score_source, sprite_num) in ended_sprite_nums.iter() {
                    if let Some(sprite) =
                        get_score_sprite_mut(&mut player.movie, score_source, *sprite_num as i16)
                    {
                        sprite.exited = true;
                    }
                }
            });

            (is_playing, is_script_paused) = reserve_player_mut(|player| {
                player.advance_frame();

                player.has_player_frame_changed = false;
                (player.is_playing, player.is_script_paused)
            });
        }

        player_wait_available().await;
    }

    player_wait_available().await;

    reserve_player_mut(|player| {
        if !stayed_on_same_frame {
            player.movie.frame_script_instance = None;
            player.begin_all_sprites();
        }

        player.movie.score.apply_tween_modifiers(player.movie.current_frame);
    });

    player_wait_available().await;

    let changed_filmloops = reserve_player_mut_async(|player| {
        Box::pin(async move {
            player.update_filmloop_frames().await
        })
    }).await;

    if !changed_filmloops.is_empty() {
        reserve_player_mut(|player| {
            for (member_ref, _, new_frame) in changed_filmloops {
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                    if let CastMemberType::FilmLoop(film_loop) = &mut member.member_type {
                        film_loop.score.apply_tween_modifiers(new_frame);
                    }
                }
            }
            // Filmloop frame changes require a redraw
            player.stage_dirty = true;
        });
    }

    player_wait_available().await;

    // Collect behaviors that need initialization
    let behaviors_to_init: Vec<(ScriptInstanceRef, u32)> = reserve_player_mut(|player| {
        let mut behaviors = Vec::new();
        for channel_number in player.active_stage_behavior_channels() {
            let Some((sprite_num, fallback)) = player
                .movie
                .score
                .channels
                .get(channel_number)
                .map(|channel| {
                    (
                        channel.sprite.number as u32,
                        channel.sprite.script_instance_list.clone(),
                    )
                })
            else {
                continue;
            };

            for behavior_ref in player.get_sprite_script_instance_ids(
                sprite_num as i16,
                fallback.as_slice(),
            ) {
                if player
                    .allocator
                    .get_script_instance_entry(behavior_ref.id())
                    .is_some_and(|entry| !entry.script_instance.begin_sprite_called)
                {
                    behaviors.push((behavior_ref, sprite_num));
                }
            }
        }
        behaviors
    });

    // Initialize behavior default properties
    for (behavior_ref, sprite_num) in &behaviors_to_init {
        if let Err(err) = Score::initialize_behavior_defaults_async(behavior_ref.clone(), *sprite_num).await {
            console_warn!("Failed to initialize behavior defaults: {}", err.message);
        }
    }

    player_wait_available().await;

    reserve_player_mut(|player| {
        player.is_in_frame_update = true;
    });

    let begin_sprite_nums = player_dispatch_event_beginsprite(
        &"beginSprite".to_string(),
        &vec![]
    ).await;

    player_wait_available().await;

    reserve_player_mut(|player| {
        for sprite_list in begin_sprite_nums.iter() {
            for (score_source, sprite_num) in sprite_list.iter() {
                if let Some(sprite) = get_score_sprite_mut(
                    &mut player.movie,
                    score_source,
                    *sprite_num as i16,
                ) {
                    for script_ref in &sprite.script_instance_list {
                        if let Some(entry) =
                            player.allocator.get_script_instance_entry_mut(script_ref.id())
                        {
                            entry.script_instance.begin_sprite_called = true;
                        }
                    }
                }
            }
        }
    });

    // Dispatch beginSprite to any remaining behaviors that weren't handled
    // by player_dispatch_event_beginsprite (e.g., puppet sprites not in the
    // score's sprite_spans, or sprites without entered=true).
    let remaining_behaviors: Vec<ScriptInstanceRef> = reserve_player_mut(|player| {
        behaviors_to_init.iter()
            .filter(|(behavior_ref, _)| {
                player.allocator.get_script_instance_entry(behavior_ref.id())
                    .map_or(false, |entry| !entry.script_instance.begin_sprite_called)
            })
            .map(|(behavior_ref, _)| behavior_ref.clone())
            .collect()
    });

    for behavior_ref in &remaining_behaviors {
        let receivers = vec![behavior_ref.clone()];
        let _ = player_invoke_event_to_instances(
            &"beginSprite".to_string(),
            &vec![],
            &receivers,
        ).await;
    }

    // Mark all remaining behaviors as having had beginSprite called
    if !remaining_behaviors.is_empty() {
        reserve_player_mut(|player| {
            for behavior_ref in &remaining_behaviors {
                if let Some(entry) =
                    player.allocator.get_script_instance_entry_mut(behavior_ref.id())
                {
                    entry.script_instance.begin_sprite_called = true;
                }
            }
        });
    }

    reserve_player_mut(|player| {
        player.is_in_frame_update = false;
    });

    (is_playing, is_script_paused)
}

pub async fn run_frame_loop() {
    unsafe {
        let player = PLAYER_OPT.as_ref().unwrap();
        if !player.is_playing {
            return;
        }
    }

    let generation = unsafe { PLAYER_GENERATION };
    let mut is_playing = true;
    while is_playing {
        // Exit if the player was reset (e.g. between tests)
        if unsafe { PLAYER_GENERATION } != generation {
            return;
        }
        // Check for pending gotoNetMovie completion
        let goto_transition = reserve_player_ref(|player| {
            if let Some((task_id, ref target)) = player.pending_goto_net_movie {
                if player.net_manager.is_task_done(Some(task_id)) {
                    Some((task_id, target.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        });

        if let Some((task_id, target)) = goto_transition {
            transition_to_net_movie(task_id, target).await;
            (is_playing, _) = reserve_player_ref(|player| {
                (player.is_playing, player.is_script_paused)
            });
            continue;
        }

        // Pre-dispatch Flash members for sprites on the current frame so they start
        // loading before Lingo scripts try to access them.
        reserve_player_mut(|player| {
            player.pre_dispatch_flash_members();
        });

        // Wait for pending Flash/Ruffle instances to finish loading BEFORE running scripts.
        if is_flash_loading().unwrap_or(false) {
            debug!("[Flash] Waiting for Ruffle instance to finish loading...");
            for _ in 0..150 {
                timeout(Duration::from_millis(100), future::pending::<()>()).await.unwrap_err();
                if !is_flash_loading().unwrap_or(false) {
                    break;
                }
            }
            debug!("[Flash] Ruffle instance ready, resuming frame loop.");
        }

        // Run one frame cycle (scripts + advance)
        let (playing, _) = run_single_frame().await;
        is_playing = playing;

        if !is_playing {
            stop_movie_sequence().await;
            return;
        }

        // Also check after frame execution: if scripts tried to access a Flash
        // instance that doesn't exist yet, wait for Ruffle to finish loading.
        if is_flash_loading().unwrap_or(false) {
            debug!("[Flash] Scripts accessed unready Flash instance, waiting...");
            for _ in 0..150 {
                timeout(Duration::from_millis(100), future::pending::<()>()).await.unwrap_err();
                if !is_flash_loading().unwrap_or(false) {
                    break;
                }
            }
            debug!("[Flash] Ruffle instance ready, resuming frame loop.");
        }

        // Get the target frame delay based on cached tempo for current frame
        let target_delay_ms = reserve_player_ref(|player| {
            let tempo = player.current_frame_tempo;
            if tempo == 0 {
                1000.0 / 30.0  // Default to 30fps if tempo is 0
            } else {
                1000.0 / tempo as f64
            }
        });

        // Wait for the frame delay using the tempo-based timing
        timeout(
            Duration::from_millis(target_delay_ms.ceil() as u64),
            future::pending::<()>(),
        )
        .await
        .unwrap_err();
        player_wait_available().await;
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
        error: None,
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

pub async fn player_trigger_error_pause(
    err: ScriptError,
    script_ref: CastMemberRef,
    handler_ref: ScriptHandlerRef,
    bytecode_index: usize,
) {
    let (future, completer) = ManualFuture::new();
    let script_name = reserve_player_ref(|player| {
        player
            .movie
            .cast_manager
            .get_script_by_ref(&script_ref)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    });
    let breakpoint = Breakpoint {
        script_name,
        handler_name: handler_ref.1.clone(),
        bytecode_index,
    };
    let breakpoint_ctx = BreakpointContext {
        breakpoint,
        script_ref,
        handler_ref,
        bytecode_index,
        completer,
        error: Some(err.clone()),
    };
    reserve_player_mut(|player| {
        player.current_breakpoint = Some(breakpoint_ctx);
        player.pause_script();
        JsApi::dispatch_scope_list(player);
        JsApi::dispatch_script_error(player, &err);
    });
    future.await;
    reserve_player_mut(|player| {
        player.resume_script();
    });
}

pub async fn player_is_playing() -> bool {
    unsafe { PLAYER_OPT.as_ref().unwrap().is_playing }
}

pub(crate) static mut PLAYER_TX: Option<Sender<PlayerVMExecutionItem>> = None;
static mut PLAYER_EVENT_TX: Option<Sender<PlayerVMEvent>> = None;
pub static mut PLAYER_OPT: Option<DirPlayer> = None;

/// Generation counter incremented each time the player is reset (e.g. between
/// tests). Long-running spawned tasks (frame loop, command loop) capture the
/// generation at spawn time and exit when it becomes stale.
pub(crate) static mut PLAYER_GENERATION: u64 = 0;

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
        FILEIO_XTRA_MANAGER_OPT = Some(FileIoXtraManager::new());
        MULTIUSER_XTRA_MANAGER_OPT = Some(MultiuserXtraManager::new());
        XMLPARSER_XTRA_MANAGER_OPT = Some(XmlParserXtraManager::new());
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
    globals: &'a FxHashMap<&str, &'a Datum>,
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
    // Track handler depth
    reserve_player_mut(|player| {
        player.handler_stack_depth += 1;
    });
    // let formatted_args: Vec<String> = reserve_player_ref(|player| {
    //   args.iter().map(|datum_ref| format_datum(*datum_ref, player)).collect()
    // });
    // warn!("ext_call: {name}({})", formatted_args.join(", "));
    let result = match name.as_str() {
        "return" => {
            let return_value = if let Some(return_value) = args.first() {
                reserve_player_mut(|player| {
                    player.scopes.get_mut(scope_ref).unwrap().return_value = return_value.clone();
                });
                return_value.clone()
            } else {
                // Lingo's bare `return` yields Void. We must explicitly reset
                // scope.return_value because a preceding extcall (e.g.
                // `voidp(...)`) would have written its own result there, and
                // without this reset a bare `return` leaks that stale value
                // to the caller. See Coke Studios popup-positioning bug,
                // where `me.closeWindow()` was leaking `voidp`'s Int(1).
                reserve_player_mut(|player| {
                    player.scopes.get_mut(scope_ref).unwrap().return_value = DatumRef::Void;
                });
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
    };

    // Always decrement handler depth before returning
    reserve_player_mut(|player| {
        player.handler_stack_depth = player.handler_stack_depth.saturating_sub(1);
    });
    
    result
}

fn player_duplicate_datum(datum: &DatumRef) -> DatumRef {
    let datum_type = reserve_player_ref(|player| player.get_datum(datum).type_enum());
    let new_datum = match datum_type {
        DatumType::PropList => {
            let (props, sorted) = reserve_player_mut(|player| {
                let (props, sorted) = player.get_datum(datum).to_map_tuple().unwrap();
                (props.clone(), sorted)
            });
            let mut new_props = VecDeque::new();
            for (key, value) in props {
                let new_key = player_duplicate_datum(&key);
                let new_value = player_duplicate_datum(&value);
                new_props.push_back((new_key, new_value));
            }
            Datum::PropList(new_props, sorted)
        }
        DatumType::List => {
            let (list_type, list, sorted) = reserve_player_ref(|player| {
                let (a, b, c) = player.get_datum(datum).to_list_tuple().unwrap();
                (a.clone(), b.clone(), c)
            });
            let mut new_list = VecDeque::new();
            for item in list {
                let new_item = player_duplicate_datum(&item);
                new_list.push_back(new_item);
            }
            Datum::List(list_type.clone(), new_list, sorted)
        }
        DatumType::BitmapRef => reserve_player_mut(|player| {
            let bitmap_ref = player.get_datum(datum).to_bitmap_ref().unwrap();
            let bitmap = player.bitmap_manager.get_bitmap(*bitmap_ref).unwrap();
            let new_bitmap = bitmap.clone();
            // `duplicate(...)` on a Datum::BitmapRef produces an unowned copy.
            // It is freed once the wrapping DatumRef goes away (or persists
            // for as long as something holds it via refcount).
            let new_bitmap_ref = player.bitmap_manager.add_ephemeral_bitmap(new_bitmap);
            Datum::BitmapRef(new_bitmap_ref)
        }),
        _ => reserve_player_ref(|player| player.get_datum(datum).clone()),
    };
    let new_datum_ref = player_alloc_datum(new_datum);
    new_datum_ref
}
