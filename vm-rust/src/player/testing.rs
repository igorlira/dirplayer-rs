use std::path::Path;

use async_std::channel;

use crate::director::file::read_director_file_bytes;
use crate::player::{
    datum_ref::DatumRef,
    events::{
        dispatch_event_to_all_behaviors, player_dispatch_event_beginsprite,
    },
    eval::eval_lingo_command,
    handlers::movie::MovieHandlers,
    reserve_player_mut, reserve_player_ref, run_movie_init_sequence, DirPlayer,
    PlayerVMExecutionItem, ScriptError, PLAYER_OPT,
};

/// A test harness that wraps the global DirPlayer for in-memory movie testing.
///
/// # Usage
/// ```ignore
/// let mut harness = TestPlayer::new();
/// harness.load_movie("path/to/movie.dcr").await;
/// harness.init_movie().await;
/// harness.step_frames(10).await;
/// assert_eq!(harness.current_frame(), 11);
/// ```
pub struct TestPlayer {
    _tx: channel::Sender<PlayerVMExecutionItem>,
}

impl TestPlayer {
    /// Create a new test player, initializing the global PLAYER_OPT.
    pub fn new() -> Self {
        let (tx, _rx) = channel::unbounded();

        // Set up global player state
        unsafe {
            crate::player::PLAYER_TX = Some(tx.clone());
            PLAYER_OPT = Some(DirPlayer::new(tx.clone()));
        }

        TestPlayer { _tx: tx }
    }

    /// Load a Director movie file (.dcr/.dir) from disk.
    pub async fn load_movie(&mut self, path: &str) {
        let abs_path = if Path::new(path).is_absolute() {
            path.to_string()
        } else {
            // Resolve relative to the workspace root (parent of vm-rust)
            let manifest_dir = env!("CARGO_MANIFEST_DIR");
            let workspace_root = Path::new(manifest_dir).parent().unwrap();
            workspace_root.join(path).to_string_lossy().to_string()
        };

        let data_bytes =
            std::fs::read(&abs_path).unwrap_or_else(|e| panic!("Failed to read {}: {}", abs_path, e));

        let file_name = Path::new(&abs_path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let base_url = format!(
            "file://{}",
            Path::new(&abs_path).parent().unwrap().to_string_lossy()
        );

        let dir_file = read_director_file_bytes(&data_bytes, &file_name, &base_url)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", file_name, e));

        // Load the parsed movie into the player
        reserve_player_mut(|player| {
            player.is_playing = true;
            player.is_script_paused = false;
        });

        unsafe {
            let player = PLAYER_OPT.as_mut().unwrap();
            player.load_movie_from_dir(dir_file).await;
        }
    }

    /// Run the movie initialization sequence (prepareMovie, startMovie, etc.).
    pub async fn init_movie(&mut self) {
        run_movie_init_sequence().await;
    }

    /// Execute one frame update (Lingo handlers for the current frame).
    pub async fn execute_frame(&mut self) -> Result<(), ScriptError> {
        MovieHandlers::execute_frame_update().await
    }

    /// Advance to the next frame, dispatching exit/enter events.
    pub async fn advance_frame(&mut self) {
        // exitFrame
        let _ = dispatch_event_to_all_behaviors(&"exitFrame".to_string(), &vec![]).await;

        // End old sprites and advance
        reserve_player_mut(|player| {
            player.advance_frame();
            player.begin_all_sprites();
            player
                .movie
                .score
                .apply_tween_modifiers(player.movie.current_frame);
        });

        // beginSprite for new sprites
        let _ =
            player_dispatch_event_beginsprite(&"beginSprite".to_string(), &vec![]).await;

        // enterFrame
        let _ = dispatch_event_to_all_behaviors(&"enterFrame".to_string(), &vec![]).await;
    }

    /// Execute frame update + advance, repeating `n` times.
    pub async fn step_frames(&mut self, n: usize) {
        for _ in 0..n {
            let _ = self.execute_frame().await;
            self.advance_frame().await;
        }
    }

    /// Evaluate a Lingo expression and return the result.
    pub async fn eval(&self, command: &str) -> Result<DatumRef, ScriptError> {
        eval_lingo_command(command.to_string()).await
    }

    /// Get the current frame number.
    pub fn current_frame(&self) -> u32 {
        reserve_player_ref(|player| player.movie.current_frame)
    }

    /// Get a global variable's value as a string representation.
    pub fn get_global_string(&self, name: &str) -> Option<String> {
        reserve_player_ref(|player| {
            player
                .globals
                .get(name)
                .map(|datum_ref| crate::player::datum_formatting::format_datum(datum_ref, player))
        })
    }

    /// Get a global variable's DatumRef.
    pub fn get_global_ref(&self, name: &str) -> Option<DatumRef> {
        reserve_player_ref(|player| player.globals.get(name).cloned())
    }

    /// Check if the player is currently playing.
    pub fn is_playing(&self) -> bool {
        reserve_player_ref(|player| player.is_playing)
    }
}

impl Drop for TestPlayer {
    fn drop(&mut self) {
        // Clean up global state
        unsafe {
            PLAYER_OPT = None;
            crate::player::PLAYER_TX = None;
        }
    }
}

/// Run an async test body using the async-std runtime (single-threaded).
pub fn run_test<F: std::future::Future<Output = ()>>(f: F) {
    async_std::task::block_on(f);
}

