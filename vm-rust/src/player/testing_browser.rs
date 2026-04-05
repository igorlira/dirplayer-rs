use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

use crate::director::file::read_director_file_bytes;
use async_std::channel;
use crate::player::{
    events::PlayerVMEvent,
    fire_pending_timeouts,
    reserve_player_mut, run_single_frame,
    PLAYER_OPT,
};
use crate::player::testing_shared::{TestHarness, SnapshotOutput};

/// Browser-based test harness. Works on top of the global player
/// initialized by `#[wasm_bindgen(start)]`.
pub struct BrowserTestPlayer {
    event_rx: channel::Receiver<PlayerVMEvent>,
}

impl BrowserTestPlayer {
    pub fn new() -> Self {
        // Create fresh channels and a fresh player for each test.
        let (tx, _rx) = channel::unbounded();
        let (event_tx, event_rx) = channel::unbounded();
        unsafe {
            // Replace with a fresh player, discarding old state.
            // Leak the old player to avoid SIGABRT from dangling async tasks.
            if let Some(old) = PLAYER_OPT.take() {
                std::mem::forget(old);
            }
            crate::player::PLAYER_TX = None;
            crate::player::PLAYER_EVENT_TX = Some(event_tx);
            PLAYER_OPT = Some(crate::player::DirPlayer::new(tx));
        }
        BrowserTestPlayer { event_rx }
    }

    /// Drain and process any pending events from the event channel.
    async fn drain_events(&self) {
        use crate::player::{
            events::{player_invoke_global_event, player_invoke_targeted_event},
            handlers::datum_handlers::player_call_datum_handler,
        };
        while let Ok(event) = self.event_rx.try_recv() {
            let _ = match event {
                PlayerVMEvent::Global(name, args) => {
                    player_invoke_global_event(&name, &args).await
                }
                PlayerVMEvent::Targeted(name, args, instances) => {
                    player_invoke_targeted_event(&name, &args, instances.as_ref()).await
                }
                PlayerVMEvent::Callback(receiver, name, args) => {
                    player_call_datum_handler(&receiver, &name, &args).await
                }
            };
        }
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

    /// Sleep for the given number of milliseconds, yielding to the browser
    /// event loop so pending fetches and spawned tasks can complete.
    async fn sleep_ms(ms: u32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window().unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms as i32)
                .unwrap();
        });
        let _ = JsFuture::from(promise).await;
    }

    /// Ensure a renderer exists, creating one if needed.
    /// Expects `#stage_canvas_container` to already exist in the DOM.
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
        fire_pending_timeouts().await;
        let (is_playing, _) = run_single_frame().await;
        // Sleep for one frame period so wall-clock time matches the movie's
        // tempo. This also yields to the browser event loop for fetch
        // completions and WebSocket messages.
        let delay_ms = reserve_player_mut(|player| {
            let tempo = player.movie.get_effective_tempo();
            if tempo > 0 { 1000 / tempo } else { 33 }
        });
        Self::sleep_ms(delay_ms).await;
        // Process any events that arrived during the sleep (WebSocket
        // messages, callbacks dispatched by Lingo scripts, etc.)
        self.drain_events().await;
        is_playing
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
