use std::path::Path;
use std::sync::Mutex;

use async_std::channel;

use crate::director::file::read_director_file_bytes;
pub use crate::director::static_datum::StaticDatum;
use crate::player::{
    bitmap::bitmap::{get_system_default_palette, Bitmap, PaletteRef},
    events::run_event_loop,
    fire_pending_timeouts,
    reserve_player_mut, reserve_player_ref, run_single_frame,
    DirPlayer, PlayerVMExecutionItem, PLAYER_OPT,
};
pub use crate::player::testing_shared::{TestHarness, SnapshotOutput};
use crate::rendering::render_stage_to_bitmap;

/// Global lock to ensure only one TestPlayer runs at a time.
/// The player uses global mutable statics, so tests must be serialized.
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Native test harness. Wraps the global DirPlayer for in-memory testing.
pub struct TestPlayer {
    _tx: channel::Sender<PlayerVMExecutionItem>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TestPlayer {
    pub fn new() -> Self {
        let lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let (tx, _rx) = channel::unbounded();
        let (event_tx, event_rx) = channel::unbounded();

        unsafe {
            crate::player::PLAYER_TX = Some(tx.clone());
            crate::player::PLAYER_EVENT_TX = Some(event_tx.clone());
            crate::player::xtra::multiuser::MULTIUSER_XTRA_MANAGER_OPT =
                Some(crate::player::xtra::multiuser::MultiuserXtraManager::new());
            crate::player::xtra::xmlparser::XMLPARSER_XTRA_MANAGER_OPT =
                Some(crate::player::xtra::xmlparser::XmlParserXtraManager::new());
            PLAYER_OPT = Some(DirPlayer::new(tx.clone()));
        }

        async_std::task::spawn_local(async move {
            run_event_loop(event_rx).await;
        });

        TestPlayer { _tx: tx, _lock: lock }
    }
}

impl TestHarness for TestPlayer {
    fn asset_path(&self, relative: &str) -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_root = std::path::Path::new(manifest_dir).parent().unwrap();
        workspace_root.join("public").join(relative).to_string_lossy().to_string()
    }
    async fn load_movie(&mut self, path: &str) {
        let abs_path = if Path::new(path).is_absolute() {
            path.to_string()
        } else {
            let manifest_dir = env!("CARGO_MANIFEST_DIR");
            let workspace_root = Path::new(manifest_dir).parent().unwrap();
            workspace_root.join(path).to_string_lossy().to_string()
        };

        let data_bytes =
            std::fs::read(&abs_path).unwrap_or_else(|e| panic!("Failed to read {}: {}", abs_path, e));

        let file_name = Path::new(&abs_path)
            .file_name().unwrap().to_string_lossy().to_string();

        let base_url = format!(
            "file://{}",
            Path::new(&abs_path).parent().unwrap().to_string_lossy()
        );

        let dir_file = read_director_file_bytes(&data_bytes, &file_name, &base_url)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", file_name, e));

        reserve_player_mut(|player| {
            player.is_playing = true;
            player.is_script_paused = false;
        });

        unsafe {
            let player = PLAYER_OPT.as_mut().unwrap();
            player.load_movie_from_dir(dir_file).await;
        }
    }

    async fn step_frame(&mut self) -> bool {
        fire_pending_timeouts().await;
        let (is_playing, _) = run_single_frame().await;
        let delay_ms = reserve_player_ref(|player| {
            let tempo = player.movie.get_effective_tempo();
            if tempo > 0 { 1000 / tempo } else { 33 }
        });
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
        is_playing
    }

    fn snapshot_stage(&self) -> SnapshotOutput {
        reserve_player_mut(|player| {
            let w = player.movie.rect.width() as u16;
            let h = player.movie.rect.height() as u16;
            let mut bitmap = Bitmap::new(w, h, 32, 32, 0, PaletteRef::BuiltIn(get_system_default_palette()));
            render_stage_to_bitmap(player, &mut bitmap, None);
            SnapshotOutput::Rgba {
                width: w as u32,
                height: h as u32,
                data: bitmap.data,
            }
        })
    }
}

impl Drop for TestPlayer {
    fn drop(&mut self) {
        unsafe {
            if let Some(player) = PLAYER_OPT.take() {
                std::mem::forget(player);
            }
            crate::player::PLAYER_TX = None;
            crate::player::PLAYER_EVENT_TX = None;
        }
    }
}

/// Run an async test body using the async-std runtime (single-threaded).
pub fn run_test<F: std::future::Future<Output = ()>>(f: F) {
    async_std::task::block_on(f);
}

// --- Snapshot comparison utilities ---

pub struct StageSnapshot {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl StageSnapshot {
    /// Create from a SnapshotOutput (native only).
    pub fn from_output(output: SnapshotOutput) -> Self {
        match output {
            SnapshotOutput::Rgba { width, height, data } => StageSnapshot { width, height, data },
            _ => panic!("Expected Rgba snapshot on native"),
        }
    }

    pub fn to_png(&self) -> Vec<u8> {
        use image::{ImageBuffer, RgbaImage};
        let img: RgbaImage = ImageBuffer::from_raw(self.width, self.height, self.data.clone())
            .expect("Failed to create image buffer");
        let mut buf: Vec<u8> = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buf);
        image::ImageEncoder::write_image(
            encoder, img.as_raw(), self.width, self.height,
            image::ExtendedColorType::Rgba8,
        ).expect("Failed to encode PNG");
        buf
    }

    /// Compare against a reference file.
    ///
    /// `snapshot_path` is `"suite/test"` (e.g. `"habbo/load"`).
    /// `name` is the snapshot step name (e.g. "preload").
    ///
    /// Files are stored as `snapshots/{suite}/native/{test}/{name}.png`.
    pub fn assert_snapshot(&self, snapshot_path: &str, name: &str, max_diff_ratio: f64) -> Result<(), String> {
        let (suite, test) = snapshot_path.split_once('/')
            .unwrap_or((snapshot_path, "default"));
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let base = Path::new(manifest_dir).join("tests/snapshots");
        let output_dir = base.join("output").join(suite).join("native").join(test);
        let reference_dir = base.join("reference").join(suite).join("native").join(test);

        std::fs::create_dir_all(&output_dir).unwrap();
        std::fs::create_dir_all(&reference_dir).unwrap();

        let file_name = format!("{}.png", name);
        let output_path = output_dir.join(&file_name);
        let reference_path = reference_dir.join(&file_name);

        let actual_png = self.to_png();
        std::fs::write(&output_path, &actual_png).unwrap();

        if std::env::var("SNAPSHOT_UPDATE").unwrap_or_default() == "1" {
            std::fs::write(&reference_path, &actual_png).unwrap();
            eprintln!("Updated reference: {}", reference_path.display());
            return Ok(());
        }

        if reference_path.exists() {
            let reference_data = std::fs::read(&reference_path).unwrap();
            let reference_img = image::load_from_memory(&reference_data)
                .expect("Failed to decode reference PNG");
            let reference_rgba = reference_img.to_rgba8();

            let gw = reference_rgba.width();
            let gh = reference_rgba.height();
            if self.width != gw || self.height != gh {
                return Err(format!(
                    "Snapshot '{}' dimensions differ: actual {}x{} vs reference {}x{}",
                    name, self.width, self.height, gw, gh
                ));
            }

            let reference_raw = reference_rgba.into_raw();
            let pixel_count = (self.width * self.height) as usize;
            let mut diff_pixels = 0usize;
            let mut max_diff: u8 = 0;
            let mut diff_img = vec![0u8; pixel_count * 4];
            for i in 0..pixel_count {
                let off = i * 4;
                let dr = (self.data[off] as i16 - reference_raw[off] as i16).unsigned_abs() as u8;
                let dg = (self.data[off+1] as i16 - reference_raw[off+1] as i16).unsigned_abs() as u8;
                let db = (self.data[off+2] as i16 - reference_raw[off+2] as i16).unsigned_abs() as u8;
                let da = (self.data[off+3] as i16 - reference_raw[off+3] as i16).unsigned_abs() as u8;
                let ch_max = dr.max(dg).max(db).max(da);
                if ch_max > 0 {
                    diff_pixels += 1;
                    max_diff = max_diff.max(ch_max);
                    // Red highlight for changed pixels
                    diff_img[off] = 255;
                    diff_img[off + 1] = 0;
                    diff_img[off + 2] = 0;
                    diff_img[off + 3] = 255;
                } else {
                    // Dimmed reference pixel
                    diff_img[off] = reference_raw[off] >> 2;
                    diff_img[off + 1] = reference_raw[off + 1] >> 2;
                    diff_img[off + 2] = reference_raw[off + 2] >> 2;
                    diff_img[off + 3] = reference_raw[off + 3];
                }
            }

            // Save diff image if there are differences
            if diff_pixels > 0 {
                let diff_dir = base.join("diff").join(suite).join("native").join(test);
                std::fs::create_dir_all(&diff_dir).unwrap();
                let diff_path = diff_dir.join(&file_name);
                let diff_rgba: image::RgbaImage =
                    image::ImageBuffer::from_raw(self.width, self.height, diff_img)
                        .expect("Failed to create diff image");
                diff_rgba.save(&diff_path).unwrap();
            }

            let ratio = diff_pixels as f64 / pixel_count as f64;
            if ratio > max_diff_ratio {
                return Err(format!(
                    "Snapshot '{}' differs from reference: {:.4}% pixels changed \
                     (max channel diff: {}, threshold: {:.4}%)\n  \
                     actual: {}\n  reference: {}",
                    name, ratio * 100.0, max_diff, max_diff_ratio * 100.0,
                    output_path.display(), reference_path.display(),
                ));
            }
        } else {
            eprintln!(
                "No reference for '{}'; actual saved to {}. \
                 Run with SNAPSHOT_UPDATE=1 to create.",
                name, output_path.display(),
            );
        }
        Ok(())
    }
}
