use std::path::Path;
use std::sync::Mutex;

use async_std::channel;

use crate::director::file::read_director_file_bytes;
pub use crate::director::static_datum::StaticDatum;

/// Global lock to ensure only one TestPlayer runs at a time.
/// The player uses global mutable statics, so tests must be serialized.
static TEST_LOCK: Mutex<()> = Mutex::new(());
use crate::player::{
    bitmap::bitmap::{get_system_default_palette, Bitmap, PaletteRef},
    cast_lib::CastMemberRef,
    datum_ref::DatumRef,
    eval::eval_lingo_command,
    reserve_player_mut, reserve_player_ref, run_movie_init_sequence, run_single_frame,
    DirPlayer, PlayerVMExecutionItem, ScriptError, PLAYER_OPT,
};
use crate::rendering::render_stage_to_bitmap;

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
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TestPlayer {
    /// Create a new test player, initializing the global PLAYER_OPT.
    /// Acquires a global lock to ensure only one test runs at a time.
    pub fn new() -> Self {
        let lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let (tx, _rx) = channel::unbounded();

        // Set up global player state
        unsafe {
            crate::player::PLAYER_TX = Some(tx.clone());
            PLAYER_OPT = Some(DirPlayer::new(tx.clone()));
        }

        TestPlayer { _tx: tx, _lock: lock }
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

    /// Run one complete frame cycle (scripts + advance), matching the real
    /// `run_frame_loop` logic. Returns false if the movie stopped playing.
    pub async fn step_frame(&mut self) -> bool {
        let (is_playing, _) = run_single_frame().await;
        // Sleep for one frame period so wall-clock time advances.
        // Movies use `the ticks` / `the milliSeconds` for animations, and
        // without this delay those values would never change between frames.
        let delay_ms = reserve_player_ref(|player| {
            let tempo = player.movie.get_effective_tempo();
            if tempo > 0 { 1000 / tempo } else { 33 }
        });
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
        is_playing
    }

    /// Run `n` complete frame cycles.
    pub async fn step_frames(&mut self, n: usize) {
        for _ in 0..n {
            if !self.step_frame().await {
                break;
            }
        }
    }

    /// Step frames until a Lingo expression evaluates to the expected value.
    pub async fn step_until_datum(
        &mut self,
        max_frames: usize,
        expr: &str,
        expected: &StaticDatum,
    ) {
        let description = format!("{} == {:?}", expr, expected);
        for _ in 0..max_frames {
            if self.eval_datum(expr).await == *expected {
                return;
            }
            if !self.step_frame().await {
                panic!("Movie stopped playing while waiting for: {}", description);
            }
        }
        let actual = self.eval_datum(expr).await;
        panic!(
            "Condition not met after {} frames: {}\n  actual: {:?}",
            max_frames, description, actual
        );
    }

    /// Step frames until a sprite with the given member name is at least
    /// `min_visibility` (0.0–1.0) visible on the stage.
    pub async fn step_until_sprite_visible(
        &mut self,
        max_frames: usize,
        member_name: &str,
        min_visibility: f64,
    ) {
        let description = format!("sprite with member '{}' >= {:.0}% visible", member_name, min_visibility * 100.0);
        for _ in 0..max_frames {
            if let Some(sprite_num) = self.find_sprite_by_member_name(member_name) {
                if self.sprite_visibility(sprite_num).await >= min_visibility {
                    return;
                }
            }
            if !self.step_frame().await {
                panic!("Movie stopped playing while waiting for: {}", description);
            }
        }
        if let Some(sprite_num) = self.find_sprite_by_member_name(member_name) {
            let vis = self.sprite_visibility(sprite_num).await;
            panic!(
                "Timed out after {} frames: {} (sprite {} found at {:.1}% visibility)",
                max_frames, description, sprite_num, vis * 100.0
            );
        } else {
            panic!(
                "Timed out after {} frames: {} (no sprite with that member found)",
                max_frames, description
            );
        }
    }

    /// Find a sprite whose member has the given name.
    /// Returns the sprite number, or None if not found.
    pub fn find_sprite_by_member_name(&self, name: &str) -> Option<usize> {
        reserve_player_ref(|player| {
            for channel in &player.movie.score.channels {
                let sprite = &channel.sprite;
                if let Some(member_ref) = &sprite.member {
                    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                        if member.name.eq_ignore_ascii_case(name) {
                            return Some(sprite.number);
                        }
                    }
                }
            }
            None::<usize>
        })
    }

    /// Check what fraction of a sprite's rect is within the stage bounds (0.0 to 1.0).
    pub async fn sprite_visibility(&self, sprite_num: usize) -> f64 {
        let (stage_w, stage_h) = reserve_player_ref(|player| {
            (player.movie.rect.width(), player.movie.rect.height())
        });
        let sprite_rect = self.eval_datum(&format!("sprite({}).rect", sprite_num)).await;
        match sprite_rect {
            StaticDatum::IntRect(left, top, right, bottom) => {
                let sprite_w = (right - left) as f64;
                let sprite_h = (bottom - top) as f64;
                let sprite_area = sprite_w * sprite_h;
                if sprite_area <= 0.0 {
                    return 0.0;
                }
                let ix_left = left.max(0);
                let ix_top = top.max(0);
                let ix_right = right.min(stage_w);
                let ix_bottom = bottom.min(stage_h);
                let visible_w = (ix_right - ix_left).max(0) as f64;
                let visible_h = (ix_bottom - ix_top).max(0) as f64;
                (visible_w * visible_h) / sprite_area
            }
            _ => 0.0,
        }
    }

    /// Evaluate a Lingo expression and return the raw DatumRef.
    pub async fn eval(&self, command: &str) -> Result<DatumRef, ScriptError> {
        eval_lingo_command(command.to_string()).await
    }

    /// Evaluate a Lingo expression and return a `StaticDatum` for comparison.
    /// Panics if the expression errors — use `eval()` directly to handle errors.
    pub async fn eval_datum(&self, command: &str) -> StaticDatum {
        let result = self.eval(command).await
            .unwrap_or_else(|e| panic!("eval '{}' failed: {}", command, e.message));
        StaticDatum::from(result)
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


    /// Render the current stage to an in-memory RGBA bitmap.
    /// Returns (width, height, rgba_data).
    pub fn snapshot_stage(&self) -> StageSnapshot {
        reserve_player_mut(|player| {
            let w = player.movie.rect.width() as u16;
            let h = player.movie.rect.height() as u16;
            let mut bitmap = Bitmap::new(w, h, 32, 32, 0, PaletteRef::BuiltIn(get_system_default_palette()));
            render_stage_to_bitmap(player, &mut bitmap, None);
            StageSnapshot {
                width: w as u32,
                height: h as u32,
                data: bitmap.data,
            }
        })
    }

    /// Render a specific cast member as a preview bitmap.
    /// Returns None if the member is not previewable.
    pub fn snapshot_member(&self, member_ref: &CastMemberRef) -> Option<StageSnapshot> {
        reserve_player_mut(|player| {
            let bitmap = crate::rendering::render_preview_bitmap(player, member_ref, None)?;
            Some(StageSnapshot {
                width: bitmap.width as u32,
                height: bitmap.height as u32,
                data: bitmap.data,
            })
        })
    }
}

/// An RGBA pixel snapshot of the stage or a member.
pub struct StageSnapshot {
    pub width: u32,
    pub height: u32,
    /// Raw RGBA pixel data, length = width * height * 4.
    pub data: Vec<u8>,
}

impl StageSnapshot {
    /// Encode as PNG bytes (useful for golden-file comparison).
    pub fn to_png(&self) -> Vec<u8> {
        use image::{ImageBuffer, RgbaImage};
        let img: RgbaImage = ImageBuffer::from_raw(self.width, self.height, self.data.clone())
            .expect("Failed to create image buffer");
        let mut buf: Vec<u8> = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("Failed to encode PNG");
        buf
    }

    /// Save this snapshot and compare against a golden file.
    ///
    /// The snapshot name is derived from the current test name + the given suffix,
    /// e.g. calling `assert_snapshot("loaded", 0.0)` from test `e2e::habbo_v7::test_loading`
    /// produces `e2e__habbo_v7__test_loading__loaded.png`.
    ///
    /// - Always writes the actual PNG to `tests/snapshots/output/{name}.png`
    /// - If `SNAPSHOT_UPDATE=1`, also overwrites the golden file
    /// - If a golden exists, compares and panics if diff exceeds `max_diff_ratio`
    /// - If no golden exists, prints a warning and passes (first run)
    pub fn assert_snapshot(&self, suffix: &str, max_diff_ratio: f64) {
        let test_name = std::thread::current()
            .name()
            .unwrap_or("unknown")
            .replace("::", "__");
        let name = format!("{}__{}", test_name, suffix);

        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let output_dir = Path::new(manifest_dir).join("tests/snapshots/output");
        let golden_dir = Path::new(manifest_dir).join("tests/snapshots/golden");

        std::fs::create_dir_all(&output_dir).unwrap();
        std::fs::create_dir_all(&golden_dir).unwrap();

        let file_name = format!("{}.png", name);
        let output_path = output_dir.join(&file_name);
        let golden_path = golden_dir.join(&file_name);

        let actual_png = self.to_png();

        // Always save actual output (CI artifact)
        std::fs::write(&output_path, &actual_png).unwrap();

        // If SNAPSHOT_UPDATE=1, overwrite golden
        if std::env::var("SNAPSHOT_UPDATE").unwrap_or_default() == "1" {
            std::fs::write(&golden_path, &actual_png).unwrap();
            eprintln!("Updated golden: {}", golden_path.display());
            return;
        }

        // Compare against golden if it exists
        if golden_path.exists() {
            let golden_data = std::fs::read(&golden_path).unwrap();
            let golden_img = image::load_from_memory(&golden_data)
                .expect("Failed to decode golden PNG");
            let golden_rgba = golden_img.to_rgba8();

            let golden_snapshot = StageSnapshot {
                width: golden_rgba.width(),
                height: golden_rgba.height(),
                data: golden_rgba.into_raw(),
            };

            let diff = self.diff(&golden_snapshot);
            let ratio = diff.diff_ratio();

            if ratio > max_diff_ratio {
                panic!(
                    "Snapshot '{}' differs from golden: {:.4}% pixels changed \
                     (max channel diff: {}, threshold: {:.4}%)\n  \
                     actual: {}\n  golden: {}",
                    name,
                    ratio * 100.0,
                    diff.max_channel_diff,
                    max_diff_ratio * 100.0,
                    output_path.display(),
                    golden_path.display(),
                );
            }
        } else {
            eprintln!(
                "No golden for '{}'; actual saved to {}. \
                 Run with SNAPSHOT_UPDATE=1 to create.",
                name,
                output_path.display(),
            );
        }
    }

    /// Compare with another snapshot pixel-by-pixel.
    pub fn diff(&self, other: &StageSnapshot) -> SnapshotDiff {
        assert_eq!(self.width, other.width, "Snapshot widths differ");
        assert_eq!(self.height, other.height, "Snapshot heights differ");

        let pixel_count = (self.width * self.height) as usize;
        let mut diff_pixels = 0usize;
        let mut max_diff: u8 = 0;

        for i in 0..pixel_count {
            let off = i * 4;
            let dr = (self.data[off] as i16 - other.data[off] as i16).unsigned_abs() as u8;
            let dg = (self.data[off + 1] as i16 - other.data[off + 1] as i16).unsigned_abs() as u8;
            let db = (self.data[off + 2] as i16 - other.data[off + 2] as i16).unsigned_abs() as u8;
            let da = (self.data[off + 3] as i16 - other.data[off + 3] as i16).unsigned_abs() as u8;
            let ch_max = dr.max(dg).max(db).max(da);
            if ch_max > 0 {
                diff_pixels += 1;
                max_diff = max_diff.max(ch_max);
            }
        }

        SnapshotDiff {
            total_pixels: pixel_count,
            diff_pixels,
            max_channel_diff: max_diff,
        }
    }
}

pub struct SnapshotDiff {
    pub total_pixels: usize,
    pub diff_pixels: usize,
    pub max_channel_diff: u8,
}

impl SnapshotDiff {
    pub fn is_identical(&self) -> bool {
        self.diff_pixels == 0
    }

    pub fn diff_ratio(&self) -> f64 {
        self.diff_pixels as f64 / self.total_pixels as f64
    }
}

impl Drop for TestPlayer {
    fn drop(&mut self) {
        // Reset global state. We take the player out and leak it rather than
        // dropping it, because `step_frames` may have spawned async tasks
        // that still hold references to player data. Dropping while those
        // tasks are alive causes a SIGABRT.
        unsafe {
            if let Some(player) = PLAYER_OPT.take() {
                std::mem::forget(player);
            }
            crate::player::PLAYER_TX = None;
        }
    }
}

/// Run an async test body using the async-std runtime (single-threaded).
pub fn run_test<F: std::future::Future<Output = ()>>(f: F) {
    async_std::task::block_on(f);
}

