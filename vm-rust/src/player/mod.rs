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
pub mod js_lingo;
pub mod js_lingo_loader;
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
use xtra::curl::{CurlXtraManager, CURL_XTRA_MANAGER_OPT};
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
    /// Host-side composited stage image of each active nested `#movie` sub-player,
    /// keyed by the Linked Movie member. Each host frame the sub-player's stage is
    /// rendered (headless, CPU) and copied here into the HOST's bitmap_manager so
    /// the WebGL2 `Movie` sprite arm can blit it at the sprite rect. One BitmapRef
    /// per member, overwritten in place.
    pub nested_movie_images: HashMap<CastMemberRef, BitmapRef>,
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
    /// Persistent script-owned stage framebuffer for "imaging Lingo" movies
    /// that draw directly into `(the stage).image` (e.g. spectral-wizard).
    /// Created lazily on first `(the stage).image` access and returned as the
    /// same BitmapRef every call, so a cached `theStage = (the stage).image`
    /// keeps accumulating draws. `None` until first access.
    pub stage_image: Option<bitmap::manager::BitmapRef>,
    /// Set true once a draw op (copyPixels/fill/etc.) targets `stage_image`.
    /// Only then does the renderer composite it over the sprite output —
    /// this keeps read-only camera-capture movies (which never draw) on the
    /// old per-call snapshot behavior.
    pub stage_image_dirty: bool,
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
    /// Queue of pending `on cuePassed` events that `SoundChannel::update`
    /// has detected but not yet dispatched. The frame loop drains this
    /// after `tick_sound_manager` (`SoundChannel::update` runs synchronously
    /// from inside that tick, so the actual `player_invoke_frame_and_movie_scripts`
    /// call — which is `async` — has to happen outside the tick).
    /// Tuple: (channel_num_1_based, cue_number_1_based, cue_name).
    pub pending_cue_events: Vec<(i32, i32, String)>,
    /// Anchor for syncing score-frame advance to audio-context time while
    /// sound channel 1 is playing. Set the moment audio actually starts
    /// (`source.start()`), cleared when audio stops. The frame loop uses
    /// this to compute the target frame from `audio_currentTime` and skip
    /// the per-frame `target_delay_ms` wait when the score is behind —
    /// without it, the wall-clock'd frame loop drifts behind audio over
    /// time and visuals (Fugue No.4's Cross sprites tweened by the score)
    /// fall behind the audio-clocked `on cuePassed` driven cursor.
    /// Tuple: (score_frame_at_audio_start, audio_context_time_at_audio_start).
    pub audio_sync_anchor: Option<(u32, f64)>,
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
    /// `tell <target> … end tell` stack. Each entry is the target a `tellcall`
    /// should dispatch to: `Some(nested_player_id)` when the target is a `#movie`
    /// sprite (the loader→game command bridge — DGS `on keyIsDown` does
    /// `tell sprite(spShk) \n sendAllSprites(#keyWasPressed, k) \n end tell`),
    /// or `None` for an unsupported/self target (tellcall runs on this player).
    pub tell_target_stack: Vec<Option<usize>>,
    pub system_start_time: chrono::DateTime<chrono::Local>, // For ticks & milliSeconds (system uptime)
    pub handler_stack_depth: usize,
    pub in_frame_script: bool,
    /// Per-sprite Flash frame buffers. One Ruffle instance per (sprite,
    /// cast_member) gives each Flash sprite an independent playhead, which
    /// is what Director semantics actually require (e.g. storyscramble's 3
    /// story tiles share one Flash member but display different poster
    /// frames simultaneously). Keyed by sprite number; the renderer reads
    /// `flash_frame_buffers[channel_num]` when drawing a Flash sprite.
    pub flash_frame_buffers: HashMap<i16, bitmap::manager::BitmapRef>,
    /// Whether the lazy-load dispatch has fired for a given (sprite,
    /// cast_lib, cast_member). Prevents the renderer from triggering
    /// duplicate `createFlashInstance` calls every frame before the
    /// instance's first pixels arrive.
    pub flash_sprite_loaded: HashSet<(i16, i32, i32)>,
    /// Sprites whose Ruffle instance has been confirmed loaded + AS-initialized
    /// at least once. Flash interop (getVariable/setVariable/callFunction/
    /// setCallback) takes the SYNC fast path for these; only the FIRST access to
    /// a sprite ever goes through the async wait (which then adds it here).
    /// STICKY on purpose — never cleared: once a sprite has had a ready instance
    /// we always take the sync path, which safely returns null/void if the
    /// instance is transiently not ready (a member swap / reload), exactly as
    /// interop behaved before the ready-wait existed. The async path exists only
    /// to make the one-shot startup call (Coke Studios' SESSION_createSession,
    /// which runs before its SWF is dispatched) get a live instance; clearing
    /// this would re-arm the async path mid-game and reload the instance on
    /// ordinary interactions (navigator windows vanishing/rebuilding). Keeps the
    /// hundreds of per-frame interop calls sync with no async-dispatch overhead.
    pub flash_ready_sprites: HashSet<i16>,
    /// Set true whenever a script reads live input state — `keyPressed(...)`,
    /// `the mouseDown`, `the stillDown`. The bytecode loop's busy-wait yield
    /// counts a loop iteration toward yielding ONLY when this was set that
    /// iteration, so it cooperatively yields for input-wait spins (Neopets'
    /// `repeat while keyPressed(" ") end`) but NEVER for compute/AMF loops (Coke
    /// Studios' object-graph conversion), where a yield only adds latency.
    pub input_polled: bool,
    /// Off-screen Flash members rendered into W3D textures (frog01's environment:
    /// `newTexture(#fromCastMember, flashMember)`). Keyed by a synthetic NEGATIVE
    /// sprite number (so it never collides with on-stage positive channels);
    /// maps to the target W3D cast member + texture name. `update_flash_frame`
    /// routes captured frames for these synthetic numbers into the named texture
    /// instead of `flash_frame_buffers`. See `flash_texture_synthetic_id`.
    pub flash_texture_targets: HashMap<i16, (cast_lib::CastMemberRef, String)>,
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
    /// Set by `play movie <the current movie>` (restart). The frame loop, between
    /// frames, re-parses the retained movie bytes and runs the full load+init
    /// (rebuilds the cast → fresh W3D scenes), preserving globals + external params
    /// like Director's `play movie`. (The net loader can't always re-fetch by name
    /// once loaded, so we keep the original bytes.)
    pub pending_restart: bool,
    /// Raw bytes + file_name + base_url of the loaded movie, retained for restart.
    pub movie_reload_data: Option<(Vec<u8>, String, String)>,
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
    /// Runtime overrides for `the scriptText of member`. Director exposes a
    /// member's Lingo source as a settable string on ANY member type; some
    /// movies (e.g. freeT) use it as scratch data storage. We don't compile
    /// Lingo, so we just round-trip the string here keyed by member ref.
    pub script_text_overrides: FxHashMap<CastMemberRef, String>,
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
            movie: Movie::empty(),
            nested_movie_images: HashMap::new(),
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
            stage_image: None,
            stage_image_dirty: false,
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
            pending_cue_events: Vec::new(),
            audio_sync_anchor: None,
            enable_stream_status_handler: false,
            stream_status_reported: HashMap::new(),
            is_in_frame_update: false,
            is_dispatching_events: false,
            is_in_send_all_sprites: false,
            tell_target_stack: Vec::new(),
            system_start_time: now - chrono::Duration::days(8), // Simulated system start
            handler_stack_depth: 0,
            in_frame_script: false,
            flash_frame_buffers: HashMap::new(),
            flash_sprite_loaded: HashSet::new(),
            flash_ready_sprites: HashSet::new(),
            input_polled: false,
            flash_texture_targets: HashMap::new(),
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
            pending_restart: false,
            movie_reload_data: None,
            is_in_transition: false,
            actor_list_generation: 0,
            behavior_channel_cache_generation: 0,
            active_stage_filmloop_cache_generation: 0,
            script_text_overrides: FxHashMap::default(),
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
    /// before Lingo scripts try to access them. Per-sprite: each sprite that
    /// references a Flash member gets its own dedicated Ruffle instance.
    pub fn pre_dispatch_flash_members(&mut self) {
        // When running a nested `#movie` sub-player, dispatch its Flash sprites
        // to Ruffle under a synthetic per-player key so they don't collide with
        // the host's channel keys and their captured frames route back into THIS
        // sub's `flash_frame_buffers` (see NESTED_FLASH_BASE / update_flash_frame).
        let active = unsafe { ACTIVE_PLAYER_ID };
        let js_flash_key = |ch: i16| -> i32 {
            if active == 0 {
                ch as i32
            } else {
                nested_flash_key(active, ch)
            }
        };
        // UNLOAD pass FIRST: tear down any Ruffle instance whose channel no
        // longer holds that exact Flash member — BEFORE the load pass below, so
        // a member swap is deterministically unload(old) → load(new). If the
        // load ran first, `createFlashInstance(new)` sets the new instance at
        // the sprite's key, and the subsequent unload(old) — same key —
        // destroys the freshly-created instance, so the swapped-in member
        // (bogey_nights' bogeyman straw/longarm) never renders (frame stays 0,
        // "stale straw"). Also frees instances when a channel empties
        // (splash `killer`) so we don't leak players.
        let loaded: Vec<(i16, i32, i32)> = self.flash_sprite_loaded.iter().cloned().collect();
        for (ch, cl, cm) in loaded {
            let cur = self
                .movie
                .score
                .get_sprite(ch)
                .and_then(|s| s.member.as_ref())
                .map(|m| (m.cast_lib, m.cast_member));
            let still_present = cur == Some((cl, cm));
            if still_present {
                continue;
            }
            // The channel no longer holds THIS member. But the JS Ruffle instance
            // map is keyed by CHANNEL NUMBER only, while `flash_sprite_loaded` is
            // keyed by (channel, cast_lib, cast_member) — so a flash→flash swap
            // leaves BOTH the old and new member entries in the set for the same
            // channel for a frame. If we blindly `destroyFlashInstance(ch)` for
            // the stale OLD entry, we destroy the NEW member's instance (same JS
            // key) that `createFlashInstance` just put there — bogey_nights'
            // bogeyman swaps longarm↔straw and the straw instance was being
            // killed the instant it registered (permanent "NO INSTANCE",
            // frame reads 0, grab stalls into #retreat).
            //
            // On a flash→flash swap `createFlashInstance` already replaced the
            // single per-channel instance itself (its own leading
            // destroyFlashInstance). So only tear down the JS instance when the
            // channel no longer shows ANY live Flash member; otherwise just drop
            // the stale bookkeeping entry and leave the new instance alone.
            if !self.channel_holds_live_flash(ch) {
                JsApi::dispatch_flash_member_unloaded(js_flash_key(ch));
                self.flash_frame_buffers.remove(&ch);
            }
            self.flash_sprite_loaded.remove(&(ch, cl, cm));
        }

        // LOAD pass: dispatch newly-present Flash members.
        for channel in &self.movie.score.channels {
            let channel_num = channel.number as i16;
            if let Some(member_ref) = &channel.sprite.member {
                let dispatch_key = (channel_num, member_ref.cast_lib, member_ref.cast_member);
                if self.flash_sprite_loaded.contains(&dispatch_key) {
                    // Already loaded: keep the Ruffle render resolution matched
                    // to the sprite's current on-stage size so a Flash sprite
                    // scaled up on stage (bogey_nights' boogyflash/spitflash
                    // splashes GROW; the bogeyman arm swaps member dims) stays
                    // sharp — Ruffle re-renders the vector at the new size
                    // instead of dirplayer upscaling a stale small capture.
                    // setFlashSize no-ops on sub-2px changes so static sprites
                    // (StoryScramble posters) never reflow. Only for on-stage
                    // sprites (positive channel); 3D-texture instances excluded
                    // in the JS twin.
                    let _ = ruffle_set_size(
                        js_flash_key(channel_num),
                        channel.sprite.width.max(1) as i32,
                        channel.sprite.height.max(1) as i32,
                    );
                    continue;
                }
                if let Some(member) = self.movie.cast_manager.find_member_by_ref(member_ref) {
                    if let CastMemberType::Flash(flash_member) = &member.member_type {
                        if crate::rendering::has_swf_signature(&flash_member.data) {
                            let data = flash_member.data.clone();
                            let w = channel.sprite.width.max(1) as u32;
                            let h = channel.sprite.height.max(1) as u32;
                            let paused_at_start = flash_member
                                .flash_info
                                .as_ref()
                                .map(|fi| fi.paused_at_start)
                                .unwrap_or(false);
                            // Re-project the sprite's asserted frame onto the new
                            // instance so shared-member siblings show unique
                            // posters and swaps keep their frame.
                            let asserted_frame = channel.sprite.flash_asserted_frame.unwrap_or(-1);
                            debug!(
                                "[Flash] Pre-dispatching sprite#{} {}:{} ({}x{}, {} bytes, pausedAtStart={}, assertedFrame={})",
                                channel_num, member_ref.cast_lib, member_ref.cast_member,
                                w, h, data.len(), paused_at_start, asserted_frame,
                            );
                            JsApi::dispatch_flash_member_loaded(
                                js_flash_key(channel_num),
                                member_ref.cast_lib,
                                member_ref.cast_member,
                                &data,
                                w,
                                h,
                                paused_at_start,
                                asserted_frame,
                            );
                            self.flash_sprite_loaded.insert(dispatch_key);
                        }
                    }
                }
            }
        }

        // LOOP=false stop-at-end pass. A Flash member with `loop` disabled plays
        // once and halts at its last frame (Director: `the playing of sprite`
        // then becomes FALSE — eds_kart_attack's "Wait for Flash" behavior loops
        // the Director frame until sprite(1).playing = 0). Ruffle, however, loops
        // the root timeline forever when the SWF has no internal stop(). So for
        // loop-disabled Flash sprites, watch the root playhead and, once it
        // reaches the last frame (or wraps past it), gotoAndStop the final frame.
        let loop_check: Vec<(i16, i32)> = self
            .movie
            .score
            .channels
            .iter()
            .filter_map(|channel| {
                let cn = channel.number as i16;
                let member_ref = channel.sprite.member.as_ref()?;
                let member = self.movie.cast_manager.find_member_by_ref(member_ref)?;
                if let CastMemberType::Flash(f) = &member.member_type {
                    let loops = f.flash_info.as_ref().map_or(true, |i| i.loop_enabled);
                    if !loops {
                        let total = crate::player::cast_member::CastMember::parse_swf_frame_count(&f.data)
                            .map(|n| n as i32)
                            .unwrap_or(0);
                        if total > 1 {
                            return Some((cn, total));
                        }
                    }
                }
                None
            })
            .collect();
        for (cn, total) in loop_check {
            // Bridge not installed (test harness / pre-init) → treat as "not
            // playing" and skip; the `catch` keeps the frame loop alive.
            if !ruffle_is_playing(cn as i32).unwrap_or(false) {
                continue;
            }
            let cur = ruffle_get_current_frame(cn as i32).unwrap_or(0);
            if cur < 1 {
                continue;
            }
            let prev = self
                .movie
                .score
                .get_sprite(cn)
                .map(|s| s.flash_prev_frame)
                .unwrap_or(0);
            let wrapped = prev >= 1 && cur < prev;
            if cur >= total || wrapped {
                let _ = ruffle_goto_frame_and_stop(cn as i32, &total.to_string());
            }
            self.movie.score.get_sprite_mut(cn).flash_prev_frame = cur;
        }
    }

    /// True if the given score channel currently holds a Flash (SWF) cast
    /// member — i.e. there should be exactly one live Ruffle instance keyed by
    /// this channel number. Used by the reconcile to distinguish a flash→flash
    /// member swap (keep the instance `createFlashInstance` just made) from a
    /// flash→non-flash/empty change (genuinely tear the instance down).
    fn channel_holds_live_flash(&self, ch: i16) -> bool {
        self.movie
            .score
            .get_sprite(ch)
            .and_then(|s| s.member.as_ref())
            .and_then(|mref| self.movie.cast_manager.find_member_by_ref(mref))
            .map(|member| match &member.member_type {
                CastMemberType::Flash(f) => crate::rendering::has_swf_signature(&f.data),
                _ => false,
            })
            .unwrap_or(false)
    }

    /// Tear down any Flash (Ruffle) instances whose source member lives in the
    /// given cast lib, so the renderer re-creates them from the member's CURRENT
    /// bytes.
    ///
    /// Director keeps a member ref stable across an external-cast swap, but the
    /// bytes behind it change: Storyscramble's `castLib("story").fileName =
    /// nextStory.castFile` reloads cast lib 2 in place, so member 2:1 (the story
    /// SWF the tiles render) is replaced while the sprites still point at 2:1.
    /// The lazy-load gate (`flash_sprite_loaded`) and the captured-frame buffer
    /// would otherwise keep the STALE Ruffle player on screen forever. Clearing
    /// both — and destroying the JS-side player (which also stops its capture
    /// RAF) — makes the next render see `flash_bitmap_ref.is_none()` and
    /// re-dispatch `createFlashInstance` with the new story's SWF.
    pub fn invalidate_flash_for_cast_lib(&mut self, cast_lib: i32) {
        let sprites: Vec<i16> = self
            .flash_sprite_loaded
            .iter()
            .filter(|(_, cl, _)| *cl == cast_lib)
            .map(|(sn, _, _)| *sn)
            .collect();
        if sprites.is_empty() {
            return;
        }
        self.flash_sprite_loaded.retain(|(_, cl, _)| *cl != cast_lib);
        for sn in sprites {
            // Destroy the JS-side Ruffle instance first (cancels its capture
            // RAF so it can't re-insert a frame buffer after we drop it).
            JsApi::dispatch_flash_member_unloaded(sn as i32);
            self.flash_frame_buffers.remove(&sn);
        }
    }

    pub async fn load_movie_from_file(&mut self, path: &str) -> Result<(), String> {
        let task_id = self.net_manager.preload_net_thing(path.to_owned());
        self.net_manager.await_task(task_id).await;
        let task = self.net_manager.get_task(task_id)
            .ok_or_else(|| format!("Network task not found for '{}'", path))?;
        let data_bytes = self
            .net_manager
            .get_task_result(Some(task_id))
            .ok_or_else(|| format!("No response received for '{}'", path))?
            .map_err(|_| format!("Network request failed for '{}'", path))?;

        let file_name = task.resolved_url
            .path_segments()
            .and_then(|segments| segments.last())
            .unwrap_or("untitled.dcr");

        let base_url = get_base_url(&task.resolved_url).to_string();
        let file_name_owned = file_name.to_string();
        let movie_file = read_director_file_bytes(
            &data_bytes,
            &file_name,
            &base_url,
        )
        .map_err(|e| format!("Failed to parse movie file '{}': {}", path, e))?;
        // Retain the raw bytes so a `play movie <current>` restart can re-parse and
        // rebuild the cast (the net loader often can't re-fetch by name once loaded).
        self.movie_reload_data = Some((data_bytes, file_name_owned, base_url));
        self.load_movie_from_dir(movie_file).await;
        Ok(())
    }

    /// Instantiate a full nested `DirPlayer` for a Linked `#movie` member whose
    /// linked bytes are loaded, register it in `NESTED_PLAYERS`, and start it
    /// (load + play + its own command loop) on its own active-player id. The sub
    /// runs the entire engine against itself — its own scripts/score/cast — via
    /// the active-player indirection; the loader keeps running independently.
    /// Synchronous: the async load+play happens in a spawned task bound to the
    /// sub's id (a manual `ACTIVE_PLAYER_ID` set can't span an await here, since
    /// the enclosing task's `WithActivePlayer` wrapper would restore it).
    pub fn spawn_nested_player(&self, member_ref: CastMemberRef) {
        if nested_player_id(&member_ref).is_some() {
            return;
        }
        let (bytes, base_url, file_name) = match self
            .movie
            .cast_manager
            .find_member_by_ref(&member_ref)
            .and_then(|m| {
                if let CastMemberType::Movie(mv) = &m.member_type {
                    mv.bytes.as_ref().map(|b| {
                        let fname = mv
                            .file_name
                            .rsplit(|c| c == '/' || c == '\\')
                            .next()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("nested.dcr")
                            .to_string();
                        (b.clone(), mv.base_url.clone(), fname)
                    })
                } else {
                    None
                }
            }) {
            Some(t) => t,
            None => return,
        };
        let dir = match crate::director::file::read_director_file_bytes(&bytes, &file_name, &base_url)
        {
            Ok(d) => d,
            Err(e) => {
                warn!("[nested-player] parse '{}' failed: {}", file_name, e);
                return;
            }
        };
        let (tx, rx) = async_std::channel::unbounded();
        // The sub gets its OWN event channel (parallel to its command channel) so
        // its events are processed by its own event loop with ACTIVE_PLAYER_ID =
        // its id — otherwise a sub's script-instance ids resolve against the host
        // allocator and panic (see NESTED_EVENT_TX).
        let (event_tx, event_rx) = async_std::channel::unbounded();
        let id = unsafe {
            NESTED_PLAYERS.push(Some(DirPlayer::new(tx)));
            NESTED_PLAYER_KEYS.push(Some(member_ref.clone()));
            NESTED_EVENT_TX.push(Some(event_tx));
            NESTED_PLAYERS.len()
        };
        // The sub never receives the frontend's `SetSystemFontPath` command, so
        // its font_manager has no system font — text whose font isn't a cast
        // member (Verdana etc.) falls back to `get_system_font()` and, finding
        // none, fails to render ("No font found for 'Verdana'"). Inherit the
        // host's system font (a shared `Rc<BitmapFont>`, cheap to clone).
        let host_system_font = self.font_manager.system_font.clone();
        let prev = unsafe { ACTIVE_PLAYER_ID };
        // Bind the load/play task (and everything it spawns, incl. the sub's
        // frame loop via play()) to the sub's id; spawn_player_local captures
        // the id set right here.
        unsafe {
            ACTIVE_PLAYER_ID = id;
        }
        crate::player::spawn_player_local(async move {
            reserve_player_mut_async(|p| Box::pin(p.load_movie_from_dir(dir))).await;
            if host_system_font.is_some() {
                reserve_player_mut(|p| {
                    if p.font_manager.system_font.is_none() {
                        p.font_manager.system_font = host_system_font.clone();
                    }
                });
            }
            reserve_player_mut(|p| p.play());
            crate::player::spawn_player_local(crate::player::commands::run_command_loop(rx));
            // Bound to the sub's id (spawn_player_local captured it above), so
            // reserve_player_* inside the loop resolves to THIS sub.
            crate::player::spawn_player_local(crate::player::events::run_event_loop(event_rx));
            let (rw, rh, ver) = reserve_player_ref(|p| {
                (p.movie.rect.width(), p.movie.rect.height(), p.movie.dir_version)
            });
            debug!(
                "[nested-player] id={} started '{}' dir_version={} rect={}x{}",
                id, file_name, ver, rw, rh
            );
        });
        unsafe {
            ACTIVE_PLAYER_ID = prev;
        }
    }

    /// Scan the host movie's on-stage sprites for Linked Movie (`#movie`)
    /// members whose linked bytes are loaded, and start a nested `DirPlayer` for
    /// any not yet running. Called each frame so assigning a #movie member to a
    /// sprite (DGS `sprite(spShk).member = member(gMember)`) activates playback.
    pub fn activate_nested_players(&self) {
        let to_build: Vec<CastMemberRef> = self
            .movie
            .score
            .channels
            .iter()
            .filter_map(|channel| channel.sprite.member.clone())
            .filter(|m| nested_player_id(m).is_none())
            .filter(|m| {
                matches!(
                    self.movie.cast_manager.find_member_by_ref(m).map(|mem| &mem.member_type),
                    Some(CastMemberType::Movie(mv)) if mv.bytes.is_some()
                )
            })
            .collect();
        for member_ref in to_build {
            self.spawn_nested_player(member_ref);
        }
    }

    /// Render each on-stage nested `#movie` sub-player's stage (headless, CPU)
    /// and copy it into THIS (host) player's `bitmap_manager`, keyed by member,
    /// so the WebGL2 `Movie` sprite arm can blit it. Called on the host each
    /// frame. The sub-player renders from its own score/cast/bitmap_manager under
    /// its active-player id; the resulting stage bitmap is a plain owned `Bitmap`
    /// which is then stored in the host's manager (a separate manager, so the
    /// pixels are copied across — no id collision).
    pub fn render_nested_player_stages(&mut self) {
        use std::collections::HashSet;
        let refs: Vec<CastMemberRef> = self
            .movie
            .score
            .channels
            .iter()
            .filter_map(|ch| ch.sprite.member.clone())
            .filter(|m| nested_player_id(m).is_some())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for member_ref in refs {
            let id = match nested_player_id(&member_ref) {
                Some(i) => i,
                None => continue,
            };
            // Render the sub-player headlessly. Set ACTIVE_PLAYER_ID = id so any
            // internal reserve_player_* inside the renderer resolves to the sub;
            // no await here, so the manual set is safe. `self` (host) and the sub
            // are distinct DirPlayers, so the &mut aliasing is only the usual
            // raw-pointer re-entrancy the engine already relies on.
            let prev = unsafe { ACTIVE_PLAYER_ID };
            let rendered = unsafe {
                ACTIVE_PLAYER_ID = id;
                let out = match NESTED_PLAYERS.get_mut(id - 1).and_then(|o| o.as_mut()) {
                    Some(sub) if {
                        // Guard against a transient bad/huge sub stage rect (seen
                        // when the window is unfocused and the sub is mid-reset):
                        // a large width*height*32 in Bitmap::new aborts with an
                        // allocation error. Movie stages are never > 4096.
                        let (rw, rh) = (sub.movie.rect.width(), sub.movie.rect.height());
                        let ok = rw >= 1 && rh >= 1 && rw <= 4096 && rh <= 4096;
                        if !ok {
                            warn!("[nested] skip render: bad sub rect {}x{}", rw, rh);
                        }
                        ok
                    } => {
                        let w = (sub.movie.rect.width().max(1)) as u16;
                        let h = (sub.movie.rect.height().max(1)) as u16;
                        // Render the sub-movie through the full WebGL2 pipeline
                        // (off-screen FBO → readback) so it matches the host's
                        // fidelity — the CPU rasterizer's text/ink quality is
                        // visibly worse. Fall back to CPU if the live renderer
                        // isn't WebGL2 (e.g. Canvas2D backend or none yet).
                        let webgl_bmp = crate::rendering::with_renderer_mut(|r| match r {
                            Some(crate::rendering_gpu::DynamicRenderer::WebGL2(webgl)) => {
                                Some(webgl.render_player_to_bitmap(sub, id, w as u32, h as u32))
                            }
                            _ => None,
                        });
                        let bmp = webgl_bmp.unwrap_or_else(|| {
                            let mut bmp = crate::player::bitmap::bitmap::Bitmap::new(
                                w,
                                h,
                                32,
                                32,
                                0,
                                crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
                                    crate::player::bitmap::bitmap::get_system_default_palette(),
                                ),
                            );
                            crate::rendering::render_stage_to_bitmap(sub, &mut bmp, None);
                            bmp
                        });
                        Some(bmp)
                    }
                    _ => None,
                };
                ACTIVE_PLAYER_ID = prev;
                out
            };
            // Store into the host's bitmap_manager (ACTIVE is back to host).
            if let Some(bmp) = rendered {
                let (w, h) = (bmp.width, bmp.height);
                let reuse = self.nested_movie_images.get(&member_ref).copied().filter(|r| {
                    matches!(self.bitmap_manager.get_bitmap(*r), Some(b) if b.width == w && b.height == h)
                });
                match reuse {
                    Some(r) => {
                        // `replace_bitmap` bumps the bitmap version so the WebGL
                        // texture cache re-uploads this frame's content. A plain
                        // `*slot = bmp` reset version to 0, so the composite went
                        // STALE after the first frame (only refreshing when the
                        // texture was LRU-evicted) — that was the hover-highlight
                        // "flickers to the state where it should be showing".
                        self.bitmap_manager.replace_bitmap(r, bmp);
                    }
                    None => {
                        let r = self.bitmap_manager.add_bitmap(bmp);
                        self.nested_movie_images.insert(member_ref.clone(), r);
                    }
                }
            }
        }
    }

    pub(crate) async fn load_movie_from_dir(&mut self, dir: DirectorFile) {
        // Pick the platform-correct default system palette before loading. Mac
        // movies (Director 4 titles like thead) default to System-Mac, Windows
        // movies to System-Win; these differ at high indices and decide how
        // indexed bitmaps / shape pattern fills resolve. Read before `dir` moves.
        crate::player::bitmap::bitmap::set_default_system_palette_from_platform(dir.config.platform);
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
                self.external_param_ci("_moviePath")
                    .filter(|s| !s.is_empty())
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

        // A nested `#movie` sub-player is headless — it must never resize the
        // shared WebGL2 renderer (that canvas belongs to the host stage). Only
        // the host player (active id 0) owns the on-screen renderer size.
        if unsafe { ACTIVE_PLAYER_ID } == 0 {
            with_renderer_mut(|renderer_opt| {
                if let Some(renderer) = renderer_opt {
                    use crate::rendering_gpu::Renderer;
                    let (stage_w, stage_h) = crate::player::stage::stage_canvas_dims(self);
                    renderer.set_size(stage_w, stage_h);
                }
            });
        }

        let (stage_w, stage_h) = crate::player::stage::stage_canvas_dims(self);
        crate::js_api::JsApi::dispatch_stage_size_changed(stage_w, stage_h, self.center_stage);

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

        crate::player::spawn_player_local(async move {
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
            crate::player::spawn_player_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_into(&mut self) {
        self.step_mode = StepMode::Into;
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            crate::player::spawn_player_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_over(&mut self) {
        self.step_mode = StepMode::Over;
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            crate::player::spawn_player_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_out(&mut self) {
        self.step_mode = StepMode::Out;
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            crate::player::spawn_player_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_over_line(&mut self, skip_bytecode_indices: Vec<usize>) {
        self.step_mode = StepMode::OverLine { skip_bytecode_indices };
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            crate::player::spawn_player_local(breakpoint.completer.complete(()));
        }
    }

    pub fn step_into_line(&mut self, skip_bytecode_indices: Vec<usize>) {
        self.step_mode = StepMode::IntoLine { skip_bytecode_indices };
        self.step_scope_depth = self.scope_count;
        self.eval_scope_index = None;
        let breakpoint = self.current_breakpoint.take();

        if let Some(breakpoint) = breakpoint {
            crate::player::spawn_player_local(breakpoint.completer.complete(()));
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

        // Leaving a frame drops the "imaging Lingo" stage overlay. An imaging
        // engine (spectral-wizard) draws its game/menu/worldmap into
        // `(the stage).image` on ONE looping frame (`go(the frame)` keeps the
        // same frame → overlay persists). When it actually changes frames —
        // e.g. Game Over does `go("highscore")` to a sprite-based frame — the
        // stale game framebuffer would otherwise keep compositing over the new
        // frame's sprites (a black screen). Clear the dirty flag on a real
        // frame change; the next frame re-dirties it only if it draws into the
        // stage image again.
        if prev_frame != next_frame {
            self.stage_image_dirty = false;
        }

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

        // Silence any sound still playing from the movie we're leaving and tear
        // down its Flash/Ruffle instances (their capture RAF loops + SWF audio),
        // so switching movies doesn't leave old sounds looping or leak players.
        self.sound_manager.stop_all();
        self.flash_frame_buffers.clear();
        JsApi::dispatch_flash_reset_all();
        // JS-Lingo runtimes live in a thread_local map, not on the player, so
        // they survive both this reset and a full player drop — clear them here.
        crate::player::js_lingo_loader::clear_all_runtimes();

        // Cancel any outstanding on-demand xtra loads so leftover oneshot
        // receivers don't leak across movies. Each waiter sees `false`
        // (matches the "load failed" path) and the in-flight bytecode
        // handler that triggered the load surfaces the normal "not
        // found" ScriptError instead of hanging forever.
        debug!("Cancelling pending external-xtra loads");
        crate::player::xtra::external::cancel_all_pending_loads();

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
        self.pending_restart = false;

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
            {
                let sprite = self.movie.score.get_sprite_mut(sprite_id);
                sprite.script_instance_list = synced_ids.clone();
            }
            // Stamp `spriteNum` on every instance so `the currentSpriteNum`
            // resolves for behaviors attached at RUNTIME via
            // `add(the scriptInstanceList of sprite X, new(script(...)))`. That
            // add-to-cache path bypasses the scriptInstanceList *setter* (which
            // stamps spriteNum) and the score begin-sprite path, so without this
            // the instance's spriteNum stayed Void and `the currentSpriteNum`
            // returned 0. Summer Resort's inventory icons attach
            // inventory.select.item this way and read `the currentSpriteNum` in
            // their mouseUp to identify the clicked item — a 0 there made
            // `getPos(page, "")` return 0 and crashed showDescription.
            let sprite_num_ref = self.alloc_datum(Datum::Int(sprite_id as i32));
            for inst in &synced_ids {
                let _ = crate::player::script::script_set_prop(
                    self, inst, "spriteNum", &sprite_num_ref, false,
                );
            }
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

    /// Case-insensitive lookup into `external_params`. The browser lowercases
    /// every HTML `<embed>` attribute name, so a movie's `_runMode` /
    /// `_moviePath` param arrives as `_runmode` / `_moviepath` when delivered
    /// via the Shockwave polyfill's `<embed>` path. Director treats external
    /// parameter names case-insensitively (see `external_param_value`), so the
    /// internal lookups for these special params must too — otherwise an exact
    /// `.get("_runMode")` misses and the movie silently falls back to defaults
    /// (e.g. runMode "Plugin", which trips server-license checks).
    fn external_param_ci(&self, key: &str) -> Option<String> {
        self.external_params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
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
            "globals" => {
                // Director 11.5 Scripting Dictionary — `the globals`:
                // a special property list of every current global variable
                // whose value is not VOID, each keyed by its name (symbol)
                // paired with the value. Supports count/getPropAt/getProp/
                // getAProp via the normal PropList handlers. The list always
                // contains #version (Director's running version), so it is
                // never empty even before any global is declared.
                //
                // dirplayer stores the Lingo language constants (PI, EMPTY,
                // QUOTE, TAB, TRUE, FALSE, …) as globals internally for
                // convenience, but Director does NOT expose those as global
                // variables, so they're excluded here to match the spec.
                // actorList is a genuine global and is kept.
                const CONST_GLOBALS: &[&str] = &[
                    "PI", "VOID", "EMPTY", "RETURN", "ENTER", "QUOTE", "TAB",
                    "BACKSPACE", "TRUE", "FALSE",
                ];
                let entries: Vec<(String, DatumRef)> = self
                    .globals
                    .iter()
                    .filter(|(name, r)| {
                        if CONST_GLOBALS.iter().any(|c| c.eq_ignore_ascii_case(name)) {
                            return false;
                        }
                        // Skip globals whose value is VOID (spec).
                        !matches!(self.get_datum(r), Datum::Void)
                    })
                    .map(|(name, r)| (name.clone(), r.clone()))
                    .collect();
                let mut props: VecDeque<(DatumRef, DatumRef)> = entries
                    .into_iter()
                    .map(|(name, value_ref)| {
                        (self.alloc_datum(Datum::Symbol(name)), value_ref)
                    })
                    .collect();
                let version_key = self.alloc_datum(Datum::Symbol("version".to_string()));
                let version_val = self.alloc_datum(Datum::String("11.0".to_string()));
                props.push_back((version_key, version_val));
                Ok(self.alloc_datum(Datum::PropList(props, false)))
            },
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
            "mouseChar" => {
                let val = compute_mouse_char(self);
                Ok(self.alloc_datum(Datum::Int(val)))
            },
            "mouseLine" => {
                let val = compute_mouse_line(self);
                Ok(self.alloc_datum(Datum::Int(val)))
            },
            "stillDown" => { self.input_polled = true; Ok(self.alloc_datum(datum_bool(self.movie.mouse_down))) },
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
                            self.external_param_ci("_runMode")
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
                        self.alloc_datum(Datum::String("11.0".to_string()))
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
                // Director's `the markerList` is a property list keyed by FRAME
                // NUMBER with the label as the value: `[23: "ts_i04", 232: "Skelly", …]`
                // (not `["Skelly": 232]`). Keep that order so movie code that reads
                // `the markerList` (frame → label) works.
                let labels: Vec<_> = self.movie.score.frame_labels
                    .iter()
                    .map(|fl| (fl.frame_num, fl.label.clone()))
                    .collect();
                let props: VecDeque<(DatumRef, DatumRef)> = labels
                    .into_iter()
                    .map(|(frame_num, label)| {
                        let frame_num = self.alloc_datum(Datum::Int(frame_num));
                        let label = self.alloc_datum(Datum::String(label));
                        (frame_num, label)
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
                let mode = self.external_param_ci("_runMode")
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
                let label = self.external_param_ci("_moviePath")
                    .filter(|s| !s.is_empty())
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
                let label = self.external_param_ci("_moviePath")
                    .filter(|s| !s.is_empty())
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
            "stageleft" | "stagetop" | "stageright" | "stagebottom" => {
                let layout = crate::player::stage::stage_layout(self);
                let value = match_ci!(prop, {
                    "stageleft" => layout.stage_rect[0],
                    "stagetop" => layout.stage_rect[1],
                    "stageright" => layout.stage_rect[2],
                    "stagebottom" => layout.stage_rect[3],
                    _ => unreachable!(),
                });
                Ok(self.alloc_datum(Datum::Int(value as i32)))
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
            "productVersion" => Ok(self.alloc_datum(Datum::String("11.0".to_string()))), // TODO
            "runMode" => {
                let mode = self.external_param_ci("_runMode")
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
            "clickOn" => Ok(self.alloc_datum(Datum::Int(self.click_on_sprite as i32))),
            // `_mouse.clickLoc` — point where the user last clicked,
            // captured at mouseDown (distinct from mouseLoc which tracks
            // current cursor position). Fugue No.4's Narrative_Float
            // mouseWithin reads `getAt(_mouse.clickLoc, 2)`.
            "clickLoc" => Ok(self.alloc_datum(Datum::Point(
                [self.movie.click_loc.0 as f64, self.movie.click_loc.1 as f64], 0,
            ))),
            "mouseH" => Ok(self.alloc_datum(Datum::Int(self.mouse_loc.0 as i32))),
            "mouseV" => Ok(self.alloc_datum(Datum::Int(self.mouse_loc.1 as i32))),
            "mouseChar" => {
                let val = compute_mouse_char(self);
                Ok(self.alloc_datum(Datum::Int(val)))
            }
            "mouseLine" => {
                let val = compute_mouse_line(self);
                Ok(self.alloc_datum(Datum::Int(val)))
            }
            "mouseDown" => { self.input_polled = true; Ok(self.alloc_datum(datum_bool(self.movie.mouse_down))) }
            "mouseUp" => { self.input_polled = true; Ok(self.alloc_datum(datum_bool(!self.movie.mouse_down))) }
            "stillDown" => { self.input_polled = true; Ok(self.alloc_datum(datum_bool(self.movie.mouse_down))) }
            "rightMouseDown" => Ok(self.alloc_datum(datum_bool(self.movie.right_mouse_down))),
            "rightMouseUp" => Ok(self.alloc_datum(datum_bool(!self.movie.right_mouse_down))),
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
            "timer" => {
                // `set the timer = N` resets Director's timer to N ticks (1/60 s);
                // `set the timer = 0` is equivalent to `startTimer`. `the timer`
                // reads elapsed ticks since `start_time`, so back-date start_time
                // by N ticks so it reads N going forward. eds_kart_attack's Kart
                // behavior does `set the timer = 0` in `on wobble`.
                let ticks = value.int_value()?;
                let ms = (ticks as i64) * 1000 / 60;
                self.start_time = chrono::Local::now() - chrono::Duration::milliseconds(ms);
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

    /// Human-readable dump of the current handler call stack (`script::handler`
    /// per scope, outer→inner). Used to surface a nested `#movie` sub-player's
    /// stack in the console — the Dev UI panels are bound to the main player, so
    /// a sub-player's error is otherwise uninspectable there.
    pub fn call_stack_string(&self) -> String {
        let mut trace = String::new();
        for i in 0..self.scope_count {
            if let Some(scope) = self.scopes.get(i as usize) {
                let handler_info = if let Some(script) =
                    self.movie.cast_manager.get_script_by_ref(&scope.script_ref)
                {
                    let handler_name = script
                        .handlers
                        .iter()
                        .find(|(_, h)| h.name_id == scope.handler_name_id)
                        .map(|(name, _)| name.as_str().to_owned())
                        .unwrap_or_else(|| format!("#{}", scope.handler_name_id));
                    format!("{}::{}", script.name, handler_name)
                } else {
                    format!("?::#{}", scope.handler_name_id)
                };
                trace.push_str(&format!(
                    "  {}: {} (pc={})\n",
                    i, handler_info, scope.bytecode_index
                ));
            }
        }
        trace
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

        // handle_play_file applies this loop count to the channel right before
        // it starts the sound. (It used to be set here AND reset to 1 inside
        // handle_play_file, which clobbered looping — now the derived value is
        // threaded through.)
        SoundChannelDatumHandlers::handle_play_file(self, &sound_channel, &member_ref, loop_count)
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
        let player = crate::player::player_mut();
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
        let player = crate::player::player_mut();

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
    }
    // Xtra static async handlers (e.g. Curl's exec/execAsync). These would
    // otherwise be eaten by BuiltInHandlerManager::call_handler and reported
    // as "No built-in handler".
    if xtra::manager::has_xtra_static_async_handler(handler_name) {
        return xtra::manager::call_xtra_static_async_handler(handler_name, args).await;
    }
    BuiltInHandlerManager::call_handler(handler_name, args)
}

/// True if an active movie/static script defines a handler named
/// `handler_name`. Used to decide, for the D4 `name(receiver, ..)` call form
/// (ObjCallV4), whether `name` is a movie handler that should take priority
/// over treating the first arg as a method receiver.
pub fn player_global_handler_exists(player: &DirPlayer, handler_name: &str) -> bool {
    get_active_static_script_refs(&player.movie, &player.get_hydrated_globals())
        .iter()
        .any(|script_ref| {
            player
                .movie
                .cast_manager
                .get_script_by_ref(script_ref)
                .and_then(|x| x.get_own_handler_ref(handler_name))
                .is_some()
        })
}

/// Which player `reserve_player_*` / `player_*` currently resolve to.
/// `0` = the main/host player (`PLAYER_OPT`); `N>0` = `NESTED_PLAYERS[N-1]`, a
/// Linked `#movie` sub-player. A task sets this to run the engine against a
/// specific player; because all ~1300 `reserve_player_*` call sites funnel
/// through the accessors below, switching this one value redirects the whole
/// engine — which is how a nested movie runs its own frame/scripts in the same
/// wasm instance (no second wasm, no cross-realm bridge). Task-scoping this so
/// each async task restores its own id on poll is a later step; today it stays
/// `0`, making these accessors identical to the previous `PLAYER_OPT`-only form.
pub static mut ACTIVE_PLAYER_ID: usize = 0;
/// Registry of Linked `#movie` sub-players, indexed by `ACTIVE_PLAYER_ID - 1`.
pub static mut NESTED_PLAYERS: Vec<Option<DirPlayer>> = Vec::new();
/// Parallel to `NESTED_PLAYERS`: which `#movie` member owns each sub-player,
/// so `activate` can skip already-built members and callers can map member→id.
pub static mut NESTED_PLAYER_KEYS: Vec<Option<CastMemberRef>> = Vec::new();
/// Parallel to `NESTED_PLAYERS`: each sub-player's OWN event channel sender.
/// Events (mouseEnter/mouseWithin/mouseLeave, targeted callbacks) must be
/// processed by the sub's own event loop with `ACTIVE_PLAYER_ID` = its id, so
/// script-instance refs resolve against the sub's allocator. Routing them to the
/// host's `PLAYER_EVENT_TX` (as the old single-global path did) processed a sub's
/// instance ids against the HOST allocator → `get_script_instance` unwrap panic.
pub static mut NESTED_EVENT_TX: Vec<Option<Sender<PlayerVMEvent>>> = Vec::new();

/// The event-channel sender for the currently active player (host id 0 →
/// `PLAYER_EVENT_TX`, else the sub's `NESTED_EVENT_TX` slot). Used by the
/// `player_dispatch_*` event helpers so a sub's events reach the sub's loop.
pub fn active_event_tx() -> Option<Sender<PlayerVMEvent>> {
    unsafe {
        if ACTIVE_PLAYER_ID == 0 {
            PLAYER_EVENT_TX.clone()
        } else {
            NESTED_EVENT_TX
                .get(ACTIVE_PLAYER_ID - 1)
                .and_then(|o| o.clone())
        }
    }
}

/// A nested `#movie` sub-player's Flash sprites dispatch to Ruffle under a
/// synthetic positive sprite key `BASE + player_id*STRIDE + local_channel` so
/// they neither collide with the host's channel keys nor look like the negative
/// off-screen-3D-texture keys (which are capture-count-limited). `update_flash_frame`
/// decodes it back to (player_id, channel) and routes the captured frame into the
/// SUB's `flash_frame_buffers`. STRIDE bounds a sub's channel count; BASE keeps
/// the synthetic keys clear of real channels.
pub const NESTED_FLASH_BASE: i32 = 100_000;
pub const NESTED_FLASH_STRIDE: i32 = 1_000;

/// Encode a nested sub-player's Flash dispatch key (see `NESTED_FLASH_BASE`).
pub fn nested_flash_key(player_id: usize, channel: i16) -> i32 {
    NESTED_FLASH_BASE + (player_id as i32) * NESTED_FLASH_STRIDE + channel as i32
}

/// Decode a synthetic nested Flash key back to `(player_id, channel)`, or `None`
/// if it's an ordinary host channel key.
pub fn decode_nested_flash_key(key: i32) -> Option<(usize, i16)> {
    if key < NESTED_FLASH_BASE {
        return None;
    }
    let rel = key - NESTED_FLASH_BASE;
    Some(((rel / NESTED_FLASH_STRIDE) as usize, (rel % NESTED_FLASH_STRIDE) as i16))
}

/// Active-player id (`>0`) of the sub-player built for `member_ref`, if any.
pub fn nested_player_id(member_ref: &CastMemberRef) -> Option<usize> {
    unsafe {
        NESTED_PLAYER_KEYS
            .iter()
            .position(|k| k.as_ref() == Some(member_ref))
            .map(|i| i + 1)
    }
}

#[inline(always)]
unsafe fn active_player_ptr() -> *mut DirPlayer {
    if ACTIVE_PLAYER_ID == 0 {
        PLAYER_OPT.as_mut().unwrap_unchecked() as *mut DirPlayer
    } else {
        NESTED_PLAYERS
            .get_unchecked_mut(ACTIVE_PLAYER_ID - 1)
            .as_mut()
            .unwrap_unchecked() as *mut DirPlayer
    }
}

pub fn reserve_player_ref<T, F>(callback: F) -> T
where
    F: FnOnce(&DirPlayer) -> T,
{
    unsafe {
        let player = &*active_player_ptr();
        callback(player)
    }
}

#[inline(always)]
pub fn reserve_player_mut<T, F>(callback: F) -> T
where
    F: FnOnce(&mut DirPlayer) -> T,
{
    unsafe {
        let player = &mut *active_player_ptr();
        callback(player)
    }
}

/// Direct reference access without closure overhead.
/// Caller must ensure no mutable references exist.
#[inline(always)]
pub unsafe fn player_ref() -> &'static DirPlayer {
    &*active_player_ptr()
}

/// Direct mutable reference access without closure overhead.
/// Caller must ensure no other references exist.
#[inline(always)]
pub unsafe fn player_mut() -> &'static mut DirPlayer {
    &mut *active_player_ptr()
}

/// Future wrapper that sets `ACTIVE_PLAYER_ID` to a captured id for the duration
/// of every poll, then restores it. This is what makes multi-player safe under
/// the shared async executor: a task spawned for player X always runs against X
/// even when it's polled while another player's frame is on the stack.
struct WithActivePlayer<F> {
    id: usize,
    inner: F,
}
impl<F: std::future::Future> std::future::Future for WithActivePlayer<F> {
    type Output = F::Output;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // SAFETY: standard pin-projection of the single `inner` field; `id` is Copy.
        let this = unsafe { self.get_unchecked_mut() };
        let inner = unsafe { std::pin::Pin::new_unchecked(&mut this.inner) };
        unsafe {
            let prev = ACTIVE_PLAYER_ID;
            ACTIVE_PLAYER_ID = this.id;
            let r = inner.poll(cx);
            ACTIVE_PLAYER_ID = prev;
            r
        }
    }
}

/// Await `fut` with `ACTIVE_PLAYER_ID` pinned to `id` across every poll, so async
/// awaits inside it keep resolving the engine against `id` even after yielding
/// (a plain `ACTIVE_PLAYER_ID = id` before an `.await` is undone by the enclosing
/// task's own `WithActivePlayer` on the next poll). Used by `tellcall` to run a
/// command inside a nested `#movie` sub-player.
pub fn with_active_player<F: std::future::Future>(
    id: usize,
    fut: F,
) -> impl std::future::Future<Output = F::Output> {
    WithActivePlayer { id, inner: fut }
}

/// Deep-copy a datum's VALUE from one player's allocator into another's (for
/// `tellcall` arg/result marshaling across the `#movie` boundary). Simple,
/// self-contained datums (Int/Float/String/Symbol/bool/etc.) copy by value;
/// List/PropList recurse. Player-specific refs (script instances, member refs)
/// are copied verbatim — meaningful only for the value types that cross a tell,
/// which in practice are the simple ones (`sendAllSprites(#sym, k)`).
pub fn marshal_datum(from: &DirPlayer, to: &mut DirPlayer, r: &crate::player::DatumRef) -> crate::player::DatumRef {
    use crate::director::lingo::datum::{Datum, DatumType};
    let value = from.get_datum(r).clone();
    match value {
        Datum::List(t, items, sorted) => {
            let new_items: std::collections::VecDeque<_> =
                items.iter().map(|i| marshal_datum(from, to, i)).collect();
            to.alloc_datum(Datum::List(t, new_items, sorted))
        }
        Datum::PropList(pairs, sorted) => {
            let new_pairs = pairs
                .iter()
                .map(|(k, v)| (marshal_datum(from, to, k), marshal_datum(from, to, v)))
                .collect();
            to.alloc_datum(Datum::PropList(new_pairs, sorted))
        }
        other => {
            let _ = DatumType::Void;
            to.alloc_datum(other)
        }
    }
}

/// Spawn a local task bound to the *currently active* player, so its async work
/// always resolves the engine against the player it was spawned for — even when
/// interleaved with another player's frame. Replaces raw `spawn_local` across
/// the engine; the captured id is `0` (main player) everywhere today.
pub fn spawn_player_local<F>(fut: F)
where
    F: std::future::Future<Output = ()> + 'static,
{
    // `use ... as raw` (no trailing `(`) so the bulk `crate::player::spawn_player_local(`
    // → `spawn_player_local(` rewrite doesn't turn this into infinite recursion.
    use async_std::task::spawn_local as raw;
    let id = unsafe { ACTIVE_PLAYER_ID };
    raw(WithActivePlayer { id, inner: fut });
}

fn reserve_player_mut_async<F, R>(callback: F) -> impl Future<Output = R>
where
    F: for<'a> FnOnce(&'a mut DirPlayer) -> Pin<Box<dyn Future<Output = R> + 'a>>,
{
    async move {
        unsafe {
            let player = crate::player::player_mut();
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

    // JS Lingo handlers: if the script's literal area was an XDR JSScript
    // (recorded at cast-load time), route the call through the interpreter
    // instead of walking Lingo bytecode. `receiver` is Some when the call
    // came in as `script(X).handler(...)` -- Director's calling convention
    // makes `me` an implicit slot-0 arg in that case.
    if let Some(js_result) = js_lingo_loader::try_invoke_js_handler(
        script_member_ref,
        handler_name,
        arg_list,
        receiver.is_some(),
    ) {
        match js_result {
            Ok(return_value) => return Ok(ScopeResult { return_value, passed: false }),
            Err(msg) => return Err(ScriptError::new(format!("JS handler {} threw: {}", handler_name, msg))),
        }
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

        let _ = script_type;
        let is_instance_receiver = receiver.is_some();
        let receiver_arg = if let Some(script_instance_ref) = receiver.as_ref() {
            Some(Datum::ScriptInstanceRef(script_instance_ref.clone()))
        } else {
            // No explicit instance receiver: `me` is the script itself (a
            // ScriptRef). Director binds `me` to the script for
            // `script("x").handler()` calls — for MOVIE scripts too, whose
            // sibling-handler calls rely on it (Neopets DGS `secure` movie
            // script does `me.sub(...)` inside `decrypt_pc927634892`, which
            // errored with `me`=VOID).
            Some(Datum::ScriptRef(handler_ref.0.clone()))
        };

        // Whether the handler declares `me` as its first parameter. The receiver
        // is only PREPENDED as arg0 (filling that param) when it does. Instance
        // receivers (behaviors/parents) always declare `me`, so they always
        // prepend. Movie scripts are the subtle case: `script("x").handler()`
        // calls whose handler declares `me` (DGS `decrypt_pc927634892`) prepend,
        // but a plain global/event handler like `on streamStatus url, state,
        // bytesSoFar, bytesTotal` must NOT — prepending shifted every param by
        // one (bogey_nights got `bytesSoFar` = the "Complete" state string).
        let first_param_is_me = unsafe {
            let handler_def = &*handler_ptr;
            let names = &*names_ptr;
            handler_def
                .argument_name_ids
                .first()
                .and_then(|id| names.get(*id as usize))
                .map_or(false, |n| n.eq_ignore_ascii_case("me"))
        };

        let scope_ref = player.push_scope();
        {
            let scope = player.scopes.get_mut(scope_ref).unwrap();
            scope.script_ref = script_member_ref.clone();
            scope.receiver = receiver;
            scope.handler_name_id = handler_name_id;
        };

        if let Some(receiver_arg) = receiver_arg {
            if !use_raw_arg_list && (is_instance_receiver || first_param_is_me) {
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
    // Cooperative-yield budget for synchronous busy-wait loops. A Lingo
    // `repeat while keyPressed(" ")` (waiting for key release) or
    // `repeat while the mouseDown` runs entirely in bytecode with no `.await`
    // that ever returns Pending, so the WASM never unwinds to the JS event loop
    // — the key-up/mouse-up event can't be processed and the loop spins forever.
    // On each BACKWARD jump (a loop iteration) we check elapsed time and, past a
    // frame's worth, yield a real macrotask so queued input events fire, then
    // resume. Time-budgeted so ordinary fast compute loops are unaffected.
    let mut last_yield_ms = chrono::Local::now().timestamp_millis();
    // Count backward jumps to distinguish a TIGHT busy-wait (thousands of
    // iterations/ms — `repeat while keyPressed`) from a heavy COMPUTE loop (a
    // few iterations, each doing real work — the game's `drawmap`/water
    // `copyPixels` tiling). Only the former should yield; yielding inside compute
    // loops fired ~15×/frame at ~4ms each and tanked the sub to ~3fps.
    let mut backjumps: u32 = 0;

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
            HandlerExecutionResult::Jump => {
                // Busy-wait cooperative yield, scoped to INPUT-polling loops.
                // Added for Neopets' `repeat while keyPressed(" ") end` — an empty
                // tight loop that must yield so the JS event loop can deliver the
                // key-up, else it spins forever. Only count this jump toward the
                // yield when the iteration actually read live input (keyPressed /
                // the mouseDown / the stillDown set `input_polled`); a heavy
                // compute/AMF loop (Coke Studios' object-graph conversion) never
                // sets it, so it never yields — a yield there only adds latency
                // and was the source of the navigator stalls. Reading+clearing a
                // bool is far cheaper than the old per-instruction scope lookup.
                let polled = reserve_player_mut(|player| std::mem::take(&mut player.input_polled));
                if polled {
                    backjumps = backjumps.wrapping_add(1);
                    if backjumps >= 4096 {
                        backjumps = 0;
                        let now = chrono::Local::now().timestamp_millis();
                        if now - last_yield_ms >= 16 {
                            last_yield_ms = now;
                            let _ = timeout(
                                Duration::from_millis(1),
                                future::pending::<()>(),
                            )
                            .await;
                        }
                    }
                }
            }
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

    // Tick W3D #timeMS event registrations (member.registerForEvent).
    // Sits next to prepareFrame so handlers run with the same per-frame
    // semantics — the script-set state (via prepareFrame) is already in
    // place, and the event handler can mutate things before the render
    // pass below.
    crate::player::events::dispatch_w3d_timer_events().await;

    // Advance every W3D member's animation_time once per frame on the
    // shared runtime state — the renderer's local clock and the bone
    // getters used by Lingo must read the same value, otherwise scripts
    // that mirror a bone matrix to another model (e.g. pinning the head
    // to bone[6].worldTransform) see a stale frame and the head freezes
    // while the body animates.
    crate::player::events::tick_w3d_animations().await;

    // Step #particle systems (faucet water, fire, etc.) each frame, independent of
    // animation_playing — emit/age/move particles using the emitter params + model
    // position set by Lingo (see tick_w3d_particles).
    crate::player::events::tick_w3d_particles().await;

    // Native #collision modifier detection: sweep enabled collision models,
    // fire each model's setCollisionCallback handler for overlapping pairs.
    crate::player::events::tick_w3d_collisions().await;

    // After prepareFrame behaviors have run (which is where simulate()
    // typically lives), drain any pending PhysX collision reports and
    // dispatch the script's registered #collisionCallback. AGEIA's xtra
    // auto-fires the handler after each simulate; we approximate that
    // here per frame.
    crate::player::events::dispatch_physx_collision_callbacks().await;

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

/// Restart the current movie in place (`play movie <the current movie>`). Re-parses
/// the retained movie bytes and runs the same load+init flow as a movie transition —
/// rebuilding the cast (fresh W3D scenes, no stale clones) while PRESERVING globals
/// and external params (which belong to the projector/embedding, not the movie file),
/// matching Director's `play movie`. Used because the net loader often can't re-fetch
/// the movie by name once it's been loaded.
async fn restart_current_movie() {
    let reload = reserve_player_ref(|player| player.movie_reload_data.clone());
    let (bytes, file_name, base_url) = match reload {
        Some(x) => x,
        None => {
            // No retained bytes — best-effort soft reset so we don't hang.
            reserve_player_mut(|player| {
                player.pending_restart = false;
                player.reset();
                player.play();
            });
            return;
        }
    };

    let dir_file = match read_director_file_bytes(&bytes, &file_name, &base_url) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("restart: failed to re-parse movie bytes: {}", e);
            reserve_player_mut(|player| player.pending_restart = false);
            return;
        }
    };

    // Shut the current movie down (block the event loop during the transition).
    reserve_player_mut(|player| {
        player.pending_restart = false;
        player.is_playing = false;
        player.is_in_transition = true;
    });
    stop_movie_sequence().await;

    // Rebuild from frame 1, preserving globals + the allocator (like a transition).
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
        player.is_playing = false;
    });

    reserve_player_mut_async(|player| {
        Box::pin(async move {
            player.load_movie_from_dir(dir_file).await;
        })
    }).await;

    reserve_player_mut(|player| {
        player.movie.current_frame = 1;
        player.pending_restart = false;
        player.is_playing = true;
    });

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

    /// Resize a live Ruffle instance so it re-renders the vector sharp at the
    /// sprite's current on-stage size (splashes grow, arm swaps dims).
    ///
    /// `catch`: these four are called from the per-frame loop, so a page that
    /// hasn't installed the Flash bridge (`initFlashBridge`) — e.g. the e2e test
    /// harness, or the dev app before startup wiring runs — would otherwise throw
    /// a `ReferenceError` (`dirplayer_ruffleSetSize is not defined`) that aborts
    /// the whole frame loop. With `catch` the missing global degrades to a no-op.
    #[wasm_bindgen(js_name = "dirplayer_ruffleSetSize", catch)]
    fn ruffle_set_size(sprite_num: i32, w: i32, h: i32) -> Result<(), wasm_bindgen::JsValue>;

    // Used to halt a `loop = false` Flash sprite at end-of-timeline.
    #[wasm_bindgen(js_name = "dirplayer_ruffleIsPlaying", catch)]
    fn ruffle_is_playing(sprite_num: i32) -> Result<bool, wasm_bindgen::JsValue>;
    #[wasm_bindgen(js_name = "dirplayer_ruffleGetCurrentFrame", catch)]
    fn ruffle_get_current_frame(sprite_num: i32) -> Result<i32, wasm_bindgen::JsValue>;
    #[wasm_bindgen(js_name = "dirplayer_ruffleGoToFrameAndStop", catch)]
    fn ruffle_goto_frame_and_stop(sprite_num: i32, frame_or_label: &str) -> Result<(), wasm_bindgen::JsValue>;
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

    // Deferred puppetSprite(N,FALSE) revert: a sprite unpuppeted on a prior tick
    // and not re-puppeted / re-membered since reverts to the Score now (Director
    // defers the revert to the next frame update). Runs every tick so it fires
    // even for single-frame movies — Coke Studios' WallItems unpuppet furniture
    // WITHOUT setting visible=0 and relied on the old immediate wipe to clear it.
    reserve_player_mut(|player| {
        let frame = player.movie.current_frame;
        if player.movie.score.process_pending_unpuppet_reverts(frame) {
            player.movie.score.invalidate_render_channel_cache();
            player.stage_dirty = true;
        }
    });

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
            let player = crate::player::player_mut();
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
            // Forward advance/go: the arriving (or looping-in-place) frame's sprite
            // BEHAVIORS were never sent exitFrame here — only the frame+movie scripts ran —
            // so a sprite's FIRST exitFrame on the frame it lands on was lost (Director fires
            // beginSprite -> enterFrame -> exitFrame in one frame visit; e.g. a RaycastCar's
            // updateWheelModels ran a frame late, leaving its wheels in the hover-ray path).
            // Dispatch the behaviors' exitFrame first (matching the backwards-go branch and
            // the stayed-on-frame else branch below), then the frame+movie script exitFrames.
            dispatch_event_to_all_behaviors(&"exitFrame".to_string(), &vec![]).await;
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
            // go(the frame) — stay on current frame, no advancement.
            // Clear next_frame too: the go() handler sets it to Some(current)
            // when it sets go_same_frame, and if we don't clear it here the
            // stale value leaks into the next tick. On a subsequent tick where
            // no go() is called the main loop would otherwise hit the "normal
            // advance" path and feed the stale next_frame into advance_frame,
            // pinning the playhead to the previously-go'd frame even though
            // the script wanted to fall through.
            reserve_player_mut(|player| {
                player.go_same_frame = false;
                player.next_frame = None;
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
            // begin_all_sprites manages frame_script_instance lifecycle: it preserves
            // the cached instance while the playhead stays within the same script's
            // span (so the script's properties survive across frames) and recreates
            // it only when the active frame script changes or the span is exited.
            player.begin_all_sprites();
        }

        player.movie.score.apply_tween_modifiers(player.movie.current_frame);
    });

    player_wait_available().await;

    // Activate any Linked Movie (`#movie`) sprites that just gained their linked
    // bytes: start a nested DirPlayer running each one on its own active-player
    // id. Only the main loop reaches here (this whole frame loop runs at id 0);
    // sub-players get their own frame loops via spawn_nested_player -> play().
    if unsafe { ACTIVE_PLAYER_ID } == 0 {
        reserve_player_ref(|player| player.activate_nested_players());
        // Composite each on-stage sub-player's stage into the host so the WebGL2
        // `Movie` sprite arm can draw it. Only when sub-players exist.
        if unsafe { !NESTED_PLAYERS.is_empty() } {
            reserve_player_mut(|player| {
                player.render_nested_player_stages();
                player.stage_dirty = true;
            });
        }
    }

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

/// Compute Lingo's `the mouseChar` / `_mouse.mouseChar` — the 1-based
/// character index in the field/text member currently under the mouse
/// pointer. Returns -1 if the pointer is not over a field/text sprite
/// or is in the "gutter" outside the text content. Matches Director
/// 11.5 Scripting Dictionary entry for `mouseChar`.
///
/// Used by Fugue No.4's Narrative member script:
///   if the textStyle of char the mouseChar of member nar = "underline"
/// to detect clicks on inline underlined links.
/// `the mouseLine` — the RETURN-delimited line number (1-based) under the mouse
/// in the field/text sprite it's over, or -1 if not over text. Reuses the
/// char-index hit-test, then counts line breaks before that char. Used by the
/// client movie's question list (clicking a question reads `the mouseLine`).
fn compute_mouse_line(player: &mut DirPlayer) -> i32 {
    let char_index = compute_mouse_char(player);
    if char_index <= 0 {
        return -1;
    }
    let (mx, my) = player.mouse_loc;
    let sprite_num = match score::get_sprite_at(player, mx, my, false) {
        Some(n) => n,
        None => return -1,
    };
    let sprite = match player.movie.score.get_sprite(sprite_num as i16) {
        Some(s) => s,
        None => return -1,
    };
    let member_ref = match sprite.member.as_ref() {
        Some(r) => r,
        None => return -1,
    };
    let member = match player.movie.cast_manager.find_member_by_ref(member_ref) {
        Some(m) => m,
        None => return -1,
    };
    let text = match &member.member_type {
        CastMemberType::Field(f) => f.text.clone(),
        CastMemberType::Text(t) => t.text.clone(),
        _ => return -1,
    };
    let chars: Vec<char> = text.chars().collect();
    let upto = (char_index as usize).saturating_sub(1).min(chars.len());
    let line = chars[..upto].iter().filter(|&&c| c == '\r' || c == '\n').count() + 1;
    line as i32
}

fn compute_mouse_char(player: &mut DirPlayer) -> i32 {
    let (mx, my) = player.mouse_loc;
    let sprite_num = match score::get_sprite_at(player, mx, my, false) {
        Some(n) => n as i16,
        None => return -1,
    };
    compute_char_at(player, sprite_num, mx, my)
}

/// Shared core for `the mouseChar` and the `pointToChar()` builtin. Returns the
/// 1-based character index within `sprite_num`'s text/field member at the stage
/// coordinate `(mx, my)`, or -1 if the sprite holds no text/field member, the
/// point is outside the sprite's rect, or it falls past the end of the text.
/// Matches the Director 11.5 Scripting Dictionary `pointToChar()` contract
/// ("returns -1 if the point is not within the text").
pub fn compute_char_at(player: &mut DirPlayer, sprite_num: i16, mx: i32, my: i32) -> i32 {
    let sprite = match player.movie.score.get_sprite(sprite_num) {
        Some(s) => s,
        None => return -1,
    };
    let member_ref = match sprite.member.as_ref() {
        Some(r) => r,
        None => return -1,
    };
    let member = match player.movie.cast_manager.find_member_by_ref(member_ref) {
        Some(m) => m,
        None => return -1,
    };
    // Pull the wrap width + scroll_top so wrapped/paged fields (Fugue No.4
    // Narrative scrolls in chunks via PageNext/PagePrior to a scroll_top
    // pixel offset) map clicks back to the right character index.
    // Also pull the font name + size so we hit-test using the SAME atlas
    // the renderer drew with — using `get_system_font()` here used to
    // mean clicks were resolved against system Arial widths while the
    // renderer drew with the field's PFR Arial. The two layouts diverged
    // by enough to land Fugue No.4 clicks on the wrong underlined run.
    let (text, line_spacing, top_spacing, wrap_width, scroll_top, word_wrap, font_name, font_size, formatting_runs, is_text) =
        match &member.member_type {
            CastMemberType::Field(f) => (
                f.text.clone(), f.fixed_line_space, f.top_spacing,
                f.width as i32, f.scroll_top as i32, f.word_wrap,
                f.font.clone(), f.font_size,
                f.formatting_runs.clone(), false,
            ),
            CastMemberType::Text(t) => (
                t.text.clone(), t.fixed_line_space, t.top_spacing,
                t.width as i32,
                t.info.as_ref().map(|i| i.scroll_top as i32).unwrap_or(0),
                t.word_wrap,
                t.font.clone(), t.font_size,
                Vec::new(), true,
            ),
            _ => return -1,
        };
    // Pull the cast lib's font_table snapshot before dropping the member
    // borrow — needed to resolve each formatting_run.font_id to a name
    // (Arial / Arial Bold / Arial Italic) for per-run advance lookups.
    let field_font_table: std::collections::HashMap<u16, String> = player
        .movie
        .cast_manager
        .get_cast(member_ref.cast_lib as u32)
        .map(|cl| cl.font_table.clone())
        .unwrap_or_default();
    // Explicitly release the immutable borrow on `player.movie` (held
    // through `member`) before we touch `player.font_manager` /
    // `player.bitmap_manager` mutably below.
    drop(member);
    let rect = score::get_sprite_rect_in_context(player, sprite_num);
    let local_x = mx - rect.0 as i32;
    let local_y = my - rect.1 as i32;
    if local_x < 0 || local_y < 0
        || local_x >= (rect.2 - rect.0) as i32
        || local_y >= (rect.3 - rect.1) as i32
    {
        return -1;
    }
    // Shift the y-coord into the FULL text coordinate space (member-local
    // top of all text, not just the visible page). Without this, paged
    // fields return char indices from the first page even when the user
    // is looking at page 3+.
    let text_y = local_y + scroll_top;
    // Resolve the field's actual font via font_manager. The member borrow
    // was dropped above (we cloned everything into owned locals), so the
    // mutable borrow on font_manager + bitmap_manager is safe. Fall back
    // to the system font only when the named font isn't available.
    let font_arc = player.font_manager.get_font_with_cast_and_bitmap(
        &font_name,
        &player.movie.cast_manager,
        &mut player.bitmap_manager,
        if font_size > 0 { Some(font_size) } else { None },
        None,
    );
    let font_rc = match font_arc.or_else(|| player.font_manager.get_system_font()) {
        Some(f) => f,
        None => return -1,
    };
    let font: crate::player::font::BitmapFont = (*font_rc).clone();
    let min_space_adv = {
        let sz = font.font_size.max(font.char_height) as i32;
        let v = ((sz as f32) * 0.30).round() as i16;
        if v > 0 { Some(v) } else { None }
    };

    // Build per-character advances using the same per-run atlas the
    // renderer draws with. Without this, a field that mixes Arial body
    // text and Arial Bold underlined chunks (Fugue No.4 Narrative) would
    // have hit-test wrap with regular Arial widths everywhere while the
    // renderer wraps with Bold widths for the underlined runs. The
    // accumulated drift makes clicks on visual "Christ's passion" return
    // a char position deep into "the sign of the cross" / "three motives".
    let per_char_advances_vec: Option<Vec<i32>> = if !formatting_runs.is_empty() {
        // Map each char index to which formatting_run covers it.
        // Runs use BYTE positions; convert via char_indices.
        let chars_total = text.chars().count();
        let mut advances: Vec<i32> = Vec::with_capacity(chars_total);
        // Cache loaded variant atlases by canonical name so we don't
        // re-load Arial Bold once per run.
        let mut variant_cache: std::collections::HashMap<String, std::rc::Rc<crate::player::font::BitmapFont>> =
            std::collections::HashMap::new();
        let min_sp = min_space_adv.unwrap_or(0) as i32;
        // Resolve each run's font once (lazy via cache).
        let resolve_font = |player: &mut DirPlayer,
                            cache: &mut std::collections::HashMap<String, std::rc::Rc<crate::player::font::BitmapFont>>,
                            run_font_id: u16|
         -> Option<std::rc::Rc<crate::player::font::BitmapFont>> {
            let resolved_name = field_font_table.get(&run_font_id).cloned()?;
            if let Some(f) = cache.get(&resolved_name) {
                return Some(f.clone());
            }
            let f = player.font_manager.get_font_with_cast_and_bitmap(
                &resolved_name,
                &player.movie.cast_manager,
                &mut player.bitmap_manager,
                if font_size > 0 { Some(font_size) } else { None },
                None,
            )?;
            cache.insert(resolved_name, f.clone());
            Some(f)
        };
        // Walk chars, looking up which run covers each (by BYTE position).
        // Renderer-side scaling: each char's advance is multiplied by
        // `run.font_size / field.font_size` because the renderer loads
        // variant atlases at the FIELD's base size and scales when drawing
        // chars at oversize (24pt "Fugue No. 4" header runs render at 2×
        // the advance of the loaded-at-12 Arial Bold atlas). Without this
        // scaling, hit-test underestimates the width consumed by the
        // header runs, the header wraps differently than the renderer
        // drew it, and every body line below the header is shifted in
        // source-text content.
        let field_base_size = if font_size > 0 { font_size as i32 } else { 12 };
        let mut char_iter = text.char_indices().enumerate();
        // Pre-resolve a "default base" advance per char for fallback.
        while let Some((_ci, (byte_pos, c))) = char_iter.next() {
            // Find the active run for this byte position.
            let active_run = formatting_runs.iter().rev()
                .find(|r| (r.start_position as usize) <= byte_pos);
            let run_font_opt = active_run
                .and_then(|r| resolve_font(player, &mut variant_cache, r.font_id));
            let run_font_for_char = run_font_opt.as_deref().unwrap_or(&font);
            let run_size = active_run
                .map(|r| if r.font_size >= 6 { r.font_size as i32 } else { field_base_size })
                .unwrap_or(field_base_size);
            let raw_atlas = run_font_for_char.get_char_advance(c as u8) as i32;
            // Mirror the renderer's `size_px / base_size` scale factor
            // applied per-char in the PFR multi-span draw loop.
            let raw = (raw_atlas * run_size / field_base_size.max(1)).max(if c == ' ' { 0 } else { 1 });
            let clamped = if c == ' ' { raw.max(min_sp) } else { raw };
            advances.push(clamped);
        }
        Some(advances)
    } else {
        None
    };

    // Per-char effective font_size (for variable-line-height hit-testing).
    // Needed because the Narrative field has a 24pt "Fugue No. 4" header
    // on a wrap-line that also carries 12pt content — the renderer makes
    // that visual line as tall as its max font_size (24px), while a
    // plain hit-test using a fixed line_h (12) drifts a full line below
    // the renderer's actual layout for every click on the body.
    let per_char_font_sizes_vec: Option<Vec<i16>> = if !formatting_runs.is_empty() {
        let chars_total = text.chars().count();
        let mut sizes: Vec<i16> = Vec::with_capacity(chars_total);
        let base_size = if font_size > 0 { font_size as i16 } else { 12 };
        for (byte_pos, _c) in text.char_indices() {
            let active_run = formatting_runs.iter().rev()
                .find(|r| (r.start_position as usize) <= byte_pos);
            let s = active_run
                .map(|r| if r.font_size >= 6 { r.font_size as i16 } else { base_size })
                .unwrap_or(base_size);
            sizes.push(s);
        }
        Some(sizes)
    } else {
        None
    };

    // Variable-line-height hit-test. Walks chars, wraps using per-char
    // advances, finalizes each visual line's height as the max of the
    // per-char font_sizes (matching the renderer's `line.max_size` rule),
    // and finds the char at the target (local_x, text_y).
    let idx = if let (Some(advs), Some(sizes)) = (per_char_advances_vec.as_ref(), per_char_font_sizes_vec.as_ref()) {
        let wrap_max = if word_wrap && wrap_width > 0 { wrap_width as i32 } else { i32::MAX };
        let base_size = if font_size > 0 { font_size as i32 } else { 12 };
        let chars_vec: Vec<char> = text.chars().collect();
        let mut line_start_idx: usize = 0;
        let mut line_w: i32 = 0;
        let mut last_space_idx_after: Option<usize> = None; // idx AFTER the space
        let mut last_space_w_at: i32 = 0;
        let mut line_y: i32 = top_spacing as i32;
        // Visual lines as (start_idx_inclusive, end_idx_exclusive, line_h)
        let mut visual_lines: Vec<(usize, usize, i32)> = Vec::new();
        let mut line_max_size: i32 = base_size;
        let finalize_line = |start: usize, end: usize, max_size: i32, lines: &mut Vec<(usize, usize, i32)>, line_y: &mut i32| {
            let line_h = max_size.max(base_size).max(1);
            lines.push((start, end, line_h));
            *line_y += line_h;
        };
        let mut ci: usize = 0;
        while ci < chars_vec.len() {
            let c = chars_vec[ci];
            let cs = sizes.get(ci).copied().unwrap_or(base_size as i16) as i32;
            line_max_size = line_max_size.max(cs);
            if c == '\r' || c == '\n' {
                finalize_line(line_start_idx, ci, line_max_size, &mut visual_lines, &mut line_y);
                ci += 1;
                line_start_idx = ci;
                line_w = 0;
                line_max_size = base_size;
                last_space_idx_after = None;
                last_space_w_at = 0;
                continue;
            }
            let cw = advs.get(ci).copied().unwrap_or(0);
            if line_w + cw > wrap_max && ci > line_start_idx {
                if let Some(sp) = last_space_idx_after.filter(|&sp| sp > line_start_idx) {
                    finalize_line(line_start_idx, sp, line_max_size, &mut visual_lines, &mut line_y);
                    line_start_idx = sp;
                    line_w -= last_space_w_at;
                    last_space_idx_after = None;
                    last_space_w_at = 0;
                    line_max_size = base_size;
                    // Re-evaluate max for the wrapped tail (chars sp..ci).
                    for k in sp..=ci {
                        let ks = sizes.get(k).copied().unwrap_or(base_size as i16) as i32;
                        line_max_size = line_max_size.max(ks);
                    }
                }
            }
            line_w += cw;
            if c == ' ' {
                last_space_idx_after = Some(ci + 1);
                last_space_w_at = line_w;
            }
            ci += 1;
        }
        if line_start_idx < chars_vec.len() {
            finalize_line(line_start_idx, chars_vec.len(), line_max_size, &mut visual_lines, &mut line_y);
        }
        // Find the visual line containing text_y, then the char in it.
        let mut chosen_idx = chars_vec.len().saturating_sub(1);
        let mut accum_y: i32 = top_spacing as i32;
        for &(start, end, line_h) in &visual_lines {
            if text_y >= accum_y && text_y < accum_y + line_h {
                // Walk x within this line.
                let mut x_accum: i32 = 0;
                let mut found = end.saturating_sub(1);
                for k in start..end {
                    let cw = advs.get(k).copied().unwrap_or(0);
                    if local_x < x_accum + cw {
                        found = k;
                        break;
                    }
                    x_accum += cw;
                    found = k;
                }
                chosen_idx = found;
                break;
            }
            accum_y += line_h;
        }
        chosen_idx
    } else {
        // Text members render via the native (Canvas2D) path, where the line
        // STEP is `fixedLineSpace` itself when set (see
        // render_native_text_to_bitmap: `effective_line_height = fixed_line_space`),
        // NOT `font_size + fixedLineSpace`. Passing line_spacing on top of the
        // font-derived line height (as fields/bitmap path want) double-counts
        // and makes the y→line mapping drift by a full item per few lines —
        // e.g. spectral-wizard's help_menu mapped a bottom-of-list hover to a
        // mid-list link. So for text members override line_height to the native
        // value and zero the extra spacing.
        let (line_height_override, eff_line_spacing) = if is_text {
            let lh: u16 = if line_spacing > 0 {
                line_spacing
            } else if font.font_size > 0 {
                font.font_size
            } else {
                font.char_height
            };
            (Some(lh), 0u16)
        } else {
            (None, line_spacing)
        };
        let params = crate::player::font::DrawTextParams {
            font: &font,
            line_height: line_height_override,
            line_spacing: eff_line_spacing,
            top_spacing,
            char_spacing: 0,
            member_width: if word_wrap && wrap_width > 0 { Some(wrap_width as i16) } else { None },
            min_space_advance: min_space_adv,
            per_char_advances: per_char_advances_vec.as_deref(),
        };
        crate::player::font::get_text_index_at_pos(&text, &params, local_x, text_y)
    };
    let total = text.chars().count();

    if idx >= total { -1 } else { (idx + 1) as i32 }
}

/// Tick the global SoundManager by `delta` seconds — advances fades
/// regardless of whether the frame loop is currently paused (e.g.
/// blocked on a Flash/Ruffle load). Without this, a `sound fadeIn`
/// started just before such a pause leaves the channel at gain=0 for
/// the entire blocked period: the audio source plays silently into a
/// zero-gain node and is gone by the time the frame loop resumes.
fn tick_sound_manager(delta: f64) {
    unsafe {
        // Route to the ACTIVE player's sound manager, not always the host. A
        // nested `#movie` sub-player's frame loop runs under its own active id;
        // ticking PLAYER_OPT (the host) here left the sub's sounds un-updated
        // (fades/loops/cue-points/stop never progressed) — the same
        // de-globalization gap as DatumRef::drop.
        let player_opt = if ACTIVE_PLAYER_ID == 0 {
            PLAYER_OPT.as_mut()
        } else {
            NESTED_PLAYERS.get_mut(ACTIVE_PLAYER_ID - 1).and_then(|o| o.as_mut())
        };
        if let Some(player) = player_opt {
            // Aliasing the player via a raw pointer because
            // SoundManager::update needs `&mut DirPlayer` for
            // bookkeeping but lives inside the same player.
            // SoundManager::update is `&self` and only touches the
            // channels (Rc<RefCell<…>>), so this aliasing is sound.
            let player_ptr = player as *mut DirPlayer;
            let _ = player.sound_manager.update(delta, &mut *player_ptr);
        }
    }
}

pub async fn run_frame_loop() {
    unsafe {
        let player = crate::player::player_ref();
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
        // Restart (`play movie <the current movie>`). Done HERE — between frames,
        // no active bytecode — so it's safe to rebuild the cast. Re-parses the
        // retained movie bytes and runs the full load+init (fresh W3D scenes etc.),
        // preserving globals + external params like Director's `play movie`.
        let do_restart = reserve_player_ref(|player| player.pending_restart);
        if do_restart {
            restart_current_movie().await;
            (is_playing, _) = reserve_player_ref(|player| {
                (player.is_playing, player.is_script_paused)
            });
            continue;
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
                // Tick sound manager during the wait: a 1.6s applause
                // with `sound fadeIn` started right before this pause
                // would otherwise stay at gain=0 for its entire
                // duration (frame loop is blocked, so the fade ramp
                // never runs). 0.1s matches the wait granularity.
                tick_sound_manager(0.1);
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

        // Dispatch the movie-level `idle` event. Director sends `idle`
        // continuously while the playhead waits on a frame, and many movies —
        // especially Director 4 titles like thead's playback Controller —
        // drive ALL of their animation from `on idle` in a movie script
        // (stepping sprite castNums each tick). Without it nothing animates.
        // Goes to the frame script + movie scripts (Director's idle hierarchy);
        // a no-op for movies that define no `idle` handler.
        {
            let active = reserve_player_ref(|player| player.is_playing && !player.is_script_paused);
            if active {
                if let Err(err) = player_invoke_frame_and_movie_scripts("idle", &vec![]).await {
                    warn!("idle dispatch failed: {}", err.message);
                }
            }
        }

        // Tick the sound manager so per-channel fade-in / fade-out
        // progress. Without this, Lingo's `sound fadeIn` sets the
        // channel volume to 0 to start the fade but the ramp never
        // runs — the audio gets scheduled into a gain=0 node and plays
        // silently (storyscramble's `playPayoff` applause uses fadeIn
        // via the soundLoop script). Delta is the per-frame tempo
        // interval in seconds; matches what the SoundChannel::update
        // fade math expects.
        {
            let tempo = reserve_player_ref(|player| player.current_frame_tempo);
            let delta = if tempo == 0 { 1.0 / 30.0 } else { 1.0 / tempo as f64 };
            tick_sound_manager(delta);
        }

        // Drain cue-point events SoundChannel::update detected. The handler
        // dispatch is async — has to happen out here in the frame loop, not
        // inside the synchronous sound tick. Pass each as
        // `on cuePassed channelSymbol, number, name` per Director 11.5 spec
        // (channel arg is a symbol like `#sound1`). Dispatched to frame +
        // movie scripts in order.
        loop {
            let events = reserve_player_mut(|player| std::mem::take(&mut player.pending_cue_events));
            if events.is_empty() {
                break;
            }
            for (channel_num, cue_number, cue_name) in events {
                let args = reserve_player_mut(|player| {
                    let chan_sym = player.alloc_datum(crate::director::lingo::datum::Datum::Symbol(
                        format!("sound{}", channel_num),
                    ));
                    let number_ref = player.alloc_datum(crate::director::lingo::datum::Datum::Int(cue_number));
                    let name_ref = player.alloc_datum(crate::director::lingo::datum::Datum::String(cue_name));
                    vec![chan_sym, number_ref, name_ref]
                });
                if let Err(err) = player_invoke_frame_and_movie_scripts("cuePassed", &args).await {
                    warn!("cuePassed dispatch failed: {}", err.message);
                }
            }
        }

        // Also check after frame execution: if scripts tried to access a Flash
        // instance that doesn't exist yet, wait for Ruffle to finish loading.
        if is_flash_loading().unwrap_or(false) {
            debug!("[Flash] Scripts accessed unready Flash instance, waiting...");
            for _ in 0..150 {
                timeout(Duration::from_millis(100), future::pending::<()>()).await.unwrap_err();
                tick_sound_manager(0.1);
                if !is_flash_loading().unwrap_or(false) {
                    break;
                }
            }
            debug!("[Flash] Ruffle instance ready, resuming frame loop.");
        }

        // Get the target frame delay based on cached tempo for current frame
        let target_delay_ms = reserve_player_ref(|player| {
            let tempo = player.current_frame_tempo;
            let base = if tempo == 0 {
                1000.0 / 30.0  // Default to 30fps if tempo is 0
            } else {
                1000.0 / tempo as f64
            };
            // While an async net fetch is in flight, use a longer per-frame
            // yield (>= ~12 ms) so the browser's fetch/stream gets enough
            // event-loop time to complete. A tight high-tempo loop
            // (DGS puppetTempo(999) ≈ 1 ms/frame) that calls into Flash every
            // frame (showGameLoadStats) otherwise starves the fetch, so
            // gameLoaded()/netDone never turns true and the loader hangs on its
            // loading frame — only a debugger pause (which yields the event
            // loop) unstuck it. The slowdown lasts only while a task loads.
            // Also cover Ruffle-side browser fetches (LoadVars/URLLoader/XML)
            // that dirplayer's net_manager never sees: flashPlayerManager's
            // fetch monkey-patch publishes the count of outstanding requests as
            // `window.__dirplayerPendingNetCount`. The DGS loader spins at load
            // state 81 waiting on the preloader's translation POST (Ruffle-side,
            // 1-3 s) to set `preloaderTranslationSuccess`; without this the tight
            // loop starves that fetch's completion callback and the login links
            // (asfunction hyperlinks in the translated IDS strings) never load.
            let read_window_count = |key: &str| -> f64 {
                web_sys::window()
                    .and_then(|w| {
                        js_sys::Reflect::get(&w, &wasm_bindgen::JsValue::from_str(key)).ok()
                    })
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
            };
            let browser_fetch_pending = read_window_count("__dirplayerPendingNetCount") > 0.0;
            // Offscreen Ruffle instances self-tick via requestAnimationFrame; a
            // tight high-tempo Director loop (DGS puppetTempo(999) guest-gate
            // poll at load-state 350) hogs the main thread and starves those
            // ticks, so a preloader text field whose htmlText was just updated
            // (the login links) never re-renders — the movie only worked with a
            // debugger pause, which yields the loop. Floor the per-frame yield
            // to ~one RAF interval while any Flash sprite is on stage AND the
            // movie is spinning faster than 60fps (base < 16ms), so normal-tempo
            // Flash playback is unaffected but a busy-poll can't starve Ruffle.
            let flash_active = read_window_count("__dirplayerActiveFlashCount") > 0.0;
            if player.net_manager.has_in_progress_tasks() || browser_fetch_pending {
                base.max(25.0)
            } else if flash_active {
                base.max(16.0)
            } else {
                base
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
    unsafe { crate::player::player_ref().is_playing }
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
        CURL_XTRA_MANAGER_OPT = Some(CurlXtraManager::new());
    }

    unsafe {
        PLAYER_OPT = Some(DirPlayer::new(tx));
    }
    // let mut player = //PLAYER_LOCK.try_write().unwrap();
    // *player = Some(DirPlayer::new(tx, allocator_rx, allocator_tx));

    crate::player::spawn_player_local(async move {
        // player_load_system_font().await;
        crate::player::spawn_player_local(async move {
            run_command_loop(rx).await;
        });
        crate::player::spawn_player_local(async move {
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
