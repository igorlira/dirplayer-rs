use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use crate::director::file::read_director_file_bytes;
use crate::player::{
    commands::PlayerVMCommand,
    reserve_player_mut, reserve_player_ref,
    PLAYER_OPT,
};
use crate::player::testing_shared::{TestHarness, SnapshotOutput};

/// Browser test harness that lets the real player runtime drive the frame loop.
/// Instead of calling `run_single_frame` directly, we let `init_player()`'s
/// loops run normally and interact via polling and input events.
pub struct BrowserTestPlayer {}

impl BrowserTestPlayer {
    pub fn new() -> Self {
        // init_player() already ran via #[wasm_bindgen(start)], but we
        // skipped it with __dirplayerTestMode. We need to re-init with
        // fresh state for each test.
        unsafe {
            if let Some(old) = PLAYER_OPT.take() {
                std::mem::forget(old);
            }
        }
        // Run full init_player which sets up channels, command loop,
        // event loop — the complete runtime.
        crate::player::init_player();

        // Also init xtra managers
        unsafe {
            crate::player::xtra::multiuser::MULTIUSER_XTRA_MANAGER_OPT =
                Some(crate::player::xtra::multiuser::MultiuserXtraManager::new());
            crate::player::xtra::xmlparser::XMLPARSER_XTRA_MANAGER_OPT =
                Some(crate::player::xtra::xmlparser::XmlParserXtraManager::new());
        }

        BrowserTestPlayer {}
    }

    async fn fetch_bytes(url: &str) -> Vec<u8> {
        let mut opts = RequestInit::new();
        opts.method("GET");
        let request = Request::new_with_str_and_init(url, &opts)
            .unwrap_or_else(|e| panic!("Failed to create request for {}: {:?}", url, e));
        let window = web_sys::window().unwrap();
        let resp_value = JsFuture::from(window.fetch_with_request(&request)).await
            .unwrap_or_else(|e| panic!("Fetch failed for {}: {:?}", url, e));
        let resp: Response = resp_value.dyn_into().unwrap();
        if !resp.ok() {
            panic!("HTTP {} fetching {}", resp.status(), url);
        }
        let buffer = JsFuture::from(resp.array_buffer().unwrap()).await.unwrap();
        js_sys::Uint8Array::new(&buffer).to_vec()
    }

    /// Wait for the next animation frame.
    async fn next_frame() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window().unwrap()
                .request_animation_frame(&resolve)
                .unwrap();
        });
        let _ = JsFuture::from(promise).await;
    }

    /// Sleep for the given number of milliseconds.
    async fn sleep_ms(ms: u32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window().unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32)
                .unwrap();
        });
        let _ = JsFuture::from(promise).await;
    }

    /// Ensure a renderer exists, creating one if needed.
    fn ensure_renderer() {
        use crate::rendering::RENDERER_LOCK;
        RENDERER_LOCK.with(|lock| {
            if lock.borrow().is_some() { return; }
            crate::rendering::player_create_canvas().unwrap();
        });
    }
}

impl TestHarness for BrowserTestPlayer {
    fn asset_path(&self, relative: &str) -> String {
        format!("/assets/{}", relative)
    }

    async fn init_movie(&mut self) {
        // Trigger play() which spawns the real init sequence and frame loop.
        // Then yield frames until the movie has initialized.
        reserve_player_mut(|player| {
            player.is_playing = false; // Reset so play() doesn't early-return
        });
        unsafe {
            let player = PLAYER_OPT.as_mut().unwrap();
            player.play();
        }
        // Wait a few frames for the init sequence to run
        for _ in 0..10 {
            Self::next_frame().await;
        }
    }

    async fn load_movie(&mut self, url: &str) {
        let full_url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            let origin = web_sys::window().unwrap().location().origin().unwrap();
            format!("{}{}", origin, url)
        };

        let data = Self::fetch_bytes(&full_url).await;
        let file_name = full_url.rsplit('/').next().unwrap_or("movie.dcr");
        let base_url = &full_url[..full_url.rfind('/').map(|i| i + 1).unwrap_or(0)];

        let dir_file = read_director_file_bytes(&data, file_name, base_url)
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
        // Yield to the browser — the real frame loop, command loop,
        // and event loop all run during this yield.
        Self::next_frame().await;
        reserve_player_ref(|player| player.is_playing)
    }

    // Override input methods to dispatch through the command channel
    // so they're processed by the command loop at the right time,
    // avoiding concurrent access with the frame loop.

    async fn click(&mut self, x: i32, y: i32) {
        use crate::player::commands::player_dispatch;
        reserve_player_mut(|player| {
            player.mouse_loc = (x, y);
            player.movie.mouse_down = true;
        });
        player_dispatch(PlayerVMCommand::MouseDown((x, y)));
        self.step_frame().await;
        reserve_player_mut(|player| {
            player.mouse_loc = (x, y);
            player.movie.mouse_down = false;
        });
        player_dispatch(PlayerVMCommand::MouseUp((x, y)));
    }

    async fn key_down(&mut self, key: &str, code: u16) {
        use crate::player::commands::player_dispatch;
        reserve_player_mut(|player| {
            player.keyboard_manager.key_down(key.to_string(), code);
        });
        player_dispatch(PlayerVMCommand::KeyDown(key.to_string(), code));
    }

    async fn key_up(&mut self, key: &str, code: u16) {
        use crate::player::commands::player_dispatch;
        reserve_player_mut(|player| {
            player.keyboard_manager.key_up(key, code);
        });
        player_dispatch(PlayerVMCommand::KeyUp(key.to_string(), code));
    }

    fn snapshot_stage(&self) -> SnapshotOutput {
        use crate::rendering::{RENDERER_LOCK, with_renderer_mut};
        use crate::rendering_gpu::Renderer;

        Self::ensure_renderer();

        with_renderer_mut(|renderer_lock| {
            if let Some(renderer) = renderer_lock {
                reserve_player_mut(|player| renderer.draw_frame(player));
            }
        });

        let data_url = RENDERER_LOCK.with(|lock| {
            let renderer = lock.borrow();
            let renderer = renderer.as_ref().expect("Renderer should be initialized");
            let canvas = renderer.canvas();
            canvas.to_data_url_with_type("image/png").expect("toDataURL failed")
        });

        let base64 = data_url.strip_prefix("data:image/png;base64,")
            .unwrap_or(&data_url)
            .to_string();
        SnapshotOutput::Base64Png(base64)
    }
}
