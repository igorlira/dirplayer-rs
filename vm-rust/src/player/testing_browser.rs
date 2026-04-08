use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::player::{
    commands::PlayerVMCommand,
    reserve_player_mut, reserve_player_ref,
    PLAYER_OPT,
};
use crate::player::testing_shared::{TestHarness, SnapshotOutput, SpriteQuery};

/// Browser test harness that lets the real player runtime drive the frame loop.
/// Instead of calling `run_single_frame` directly, we let `init_player()`'s
/// loops run normally and interact via polling and input events.
pub struct BrowserTestPlayer {}

impl BrowserTestPlayer {
    pub async fn new() -> Self {
        // Stop the current movie and clear all timeouts before resetting.
        unsafe {
            if let Some(player) = PLAYER_OPT.as_mut() {
                player.stop();
                player.timeout_manager.clear();
            }
        }
        crate::js_api::JsApi::dispatch_clear_timeouts();

        unsafe {
            if let Some(old) = PLAYER_OPT.take() {
                std::mem::forget(old);
            }
            // Increment generation so old frame/command loops from previous
            // tests detect staleness and exit.
            crate::player::PLAYER_GENERATION += 1;

            // Create fresh channels to disconnect any old command/event loops
            // from init_player(). This prevents them from holding the semaphore
            // or interfering with our inline init_movie().
            let (tx, rx) = async_std::channel::unbounded();
            let (event_tx, event_rx) = async_std::channel::unbounded();
            crate::player::PLAYER_TX = Some(tx.clone());
            crate::player::PLAYER_EVENT_TX = Some(event_tx);
            PLAYER_OPT = Some(crate::player::DirPlayer::new(tx));
            crate::player::xtra::multiuser::MULTIUSER_XTRA_MANAGER_OPT =
                Some(crate::player::xtra::multiuser::MultiuserXtraManager::new());
            crate::player::xtra::xmlparser::XMLPARSER_XTRA_MANAGER_OPT =
                Some(crate::player::xtra::xmlparser::XmlParserXtraManager::new());
            // Spawn fresh command and event loops for the new channels
            async_std::task::spawn_local(async move {
                crate::player::commands::run_command_loop(rx).await;
            });
            async_std::task::spawn_local(async move {
                crate::player::events::run_event_loop(event_rx).await;
            });
        }

        // Init logger (normally done by init_player which we skip in test mode)
        let _ = console_log::init_with_level(log::Level::Warn);

        // Load the system font (required for text rendering)
        crate::player::font::player_load_system_font("/assets/charmap-system.png").await;

        BrowserTestPlayer {}
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
        reserve_player_mut(|player| {
            player.is_playing = false;
        });
        unsafe {
            let player = PLAYER_OPT.as_mut().unwrap();
            player.play();
        }
    }

    async fn load_movie(&mut self, url: &str) {
        let full_url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            let origin = web_sys::window().unwrap().location().origin().unwrap();
            format!("{}{}", origin, url)
        };

        unsafe {
            let player = PLAYER_OPT.as_mut().unwrap();
            player.load_movie_from_file(&full_url).await;
        }

        // Initialize the renderer now that the stage size is known
        Self::ensure_renderer();
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

        Self::canvas_to_snapshot()
    }

    async fn snapshot_sprite_isolated(&self, query: impl Into<SpriteQuery>) -> Result<SnapshotOutput, String> {
        use crate::rendering::with_renderer_mut;
        use crate::rendering_gpu::Renderer;

        Self::ensure_renderer();

        let query = query.into();
        let sprite_num = self.find_sprite(&query)
            .ok_or_else(|| format!("No sprite with {} found", query))?;
        let (l, t, r, b) = self.sprite_rect(sprite_num).await?;

        with_renderer_mut(|renderer_lock| {
            if let Some(renderer) = renderer_lock {
                reserve_player_mut(|player| {
                    renderer.draw_sprite_isolated(player, sprite_num as i16);
                });
            }
        });

        let full = Self::canvas_to_snapshot();
        // Restore the full frame so subsequent renders aren't broken
        with_renderer_mut(|renderer_lock| {
            if let Some(renderer) = renderer_lock {
                reserve_player_mut(|player| renderer.draw_frame(player));
            }
        });

        Ok(full.crop(l, t, r, b))
    }
}

impl BrowserTestPlayer {
    /// Capture the current canvas contents as a base64 PNG snapshot.
    fn canvas_to_snapshot() -> SnapshotOutput {
        use crate::rendering::RENDERER_LOCK;
        use crate::rendering_gpu::Renderer;

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
