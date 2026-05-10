mod config;
mod conditions;
mod snapshots;

pub use config::{TestConfig, MovieConfig, TestSection};
pub use conditions::{
    SpriteQuery, SpriteCheck, StepCondition,
    SpriteConditionBuilder, DatumConditionBuilder,
    StepUntilBuilder,
    sprite, datum,
};
pub use snapshots::{SnapshotOutput, SnapshotContext, emit_snapshot};

use crate::director::static_datum::StaticDatum;
use crate::player::{
    commands::{run_player_command, PlayerVMCommand},
    datum_ref::DatumRef,
    eval::eval_lingo_command,
    reserve_player_mut, reserve_player_ref, run_movie_init_sequence,
    ScriptError,
};

const DEFAULT_TIMEOUT_SECS: f64 = 30.0;

/// Get current time in milliseconds (works on both native and wasm).
pub fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    { js_sys::Date::now() }
    #[cfg(not(target_arch = "wasm32"))]
    { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0 }
}

/// Platform-specific operations implemented by each test harness.
pub trait TestHarness {
    /// Resolve a relative asset path (e.g. "dcr_woodpecker/habbo.dcr") to
    /// the platform-appropriate location.
    fn asset_path(&self, relative: &str) -> String;

    /// Load a movie from a path (native) or URL (browser).
    async fn load_movie(&mut self, locator: &str);

    /// Step one frame with platform-appropriate yielding/sleeping.
    /// Returns false if the movie stopped playing.
    async fn step_frame(&mut self) -> bool;

    /// Capture a snapshot of the current stage.
    fn snapshot_stage(&self) -> SnapshotOutput;

    // --- Shared defaults ---

    async fn init_movie(&mut self) {
        run_movie_init_sequence().await;
    }

    async fn step_frames(&mut self, n: usize) {
        for _ in 0..n {
            if !self.step_frame().await {
                break;
            }
        }
    }

    async fn eval(&self, command: &str) -> Result<DatumRef, ScriptError> {
        eval_lingo_command(command.to_string()).await
    }

    async fn eval_datum(&self, command: &str) -> Result<StaticDatum, ScriptError> {
        let result = self.eval(command).await?;
        Ok(StaticDatum::from(&result))
    }

    fn current_frame(&self) -> u32 {
        reserve_player_ref(|player| player.movie.current_frame)
    }

    fn is_playing(&self) -> bool {
        reserve_player_ref(|player| player.is_playing)
    }

    fn get_global_ref(&self, name: &str) -> Option<DatumRef> {
        reserve_player_ref(|player| player.globals.get(name).cloned())
    }

    /// Resolve a sprite query to a sprite number.
    fn find_sprite(&self, query: &SpriteQuery) -> Option<usize> {
        reserve_player_ref(|player| {
            for channel in &player.movie.score.channels {
                let sprite = &channel.sprite;
                match query {
                    SpriteQuery::Number(n) => {
                        if sprite.number == *n {
                            return Some(sprite.number);
                        }
                    }
                    SpriteQuery::MemberName(name) => {
                        if let Some(member_ref) = &sprite.member {
                            if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                                if member.name.eq_ignore_ascii_case(name) {
                                    return Some(sprite.number);
                                }
                            }
                        }
                    }
                    SpriteQuery::MemberPrefix(prefix) => {
                        let prefix_lower = prefix.to_ascii_lowercase();
                        if let Some(member_ref) = &sprite.member {
                            if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                                if member.name.to_ascii_lowercase().starts_with(&prefix_lower) {
                                    return Some(sprite.number);
                                }
                            }
                        }
                    }
                }
            }
            None::<usize>
        })
    }

    async fn sprite_visibility(&self, sprite_num: usize) -> f64 {
        let (stage_w, stage_h) = reserve_player_ref(|player| {
            (player.movie.rect.width(), player.movie.rect.height())
        });
        let sprite_rect = self.eval_datum(&format!("sprite({}).rect", sprite_num)).await.unwrap_or(StaticDatum::Void);
        match sprite_rect {
            StaticDatum::IntRect(left, top, right, bottom) => {
                let sprite_area = (right - left) as f64 * (bottom - top) as f64;
                if sprite_area <= 0.0 { return 0.0; }
                let ix_left = left.max(0);
                let ix_top = top.max(0);
                let ix_right = right.min(stage_w);
                let ix_bottom = bottom.min(stage_h);
                let visible_area = (ix_right - ix_left).max(0) as f64
                    * (ix_bottom - ix_top).max(0) as f64;
                visible_area / sprite_area
            }
            _ => 0.0,
        }
    }

    /// Step frames until a condition is met. Returns a builder that can be
    /// `.await`ed directly (uses default 30s timeout) or customized:
    ///
    /// ```ignore
    /// player.step_until(sprite().member("Logo").visible(1.0)).await?;
    /// player.step_until(sprite().member("Logo").visible(1.0)).timeout(5.0).await?;
    /// player.step_until(datum("gReady").is_truthy()).await?;
    /// ```
    fn step_until(&mut self, condition: StepCondition) -> StepUntilBuilder<'_, Self> {
        StepUntilBuilder::new(self, condition)
    }

    // --- Input simulation ---

    async fn click(&mut self, x: i32, y: i32) {
        reserve_player_mut(|player| {
            player.mouse_loc = (x, y);
            player.movie.mouse_down = true;
        });
        let _ = run_player_command(PlayerVMCommand::MouseDown((x, y))).await;
        self.step_frame().await;
        reserve_player_mut(|player| {
            player.mouse_loc = (x, y);
            player.movie.mouse_down = false;
        });
        let _ = run_player_command(PlayerVMCommand::MouseUp((x, y))).await;
    }

    async fn mouse_down(&mut self, x: i32, y: i32) {
        reserve_player_mut(|player| {
            player.mouse_loc = (x, y);
            player.movie.mouse_down = true;
        });
        let _ = run_player_command(PlayerVMCommand::MouseDown((x, y))).await;
    }

    async fn mouse_up(&mut self, x: i32, y: i32) {
        reserve_player_mut(|player| {
            player.mouse_loc = (x, y);
            player.movie.mouse_down = false;
        });
        let _ = run_player_command(PlayerVMCommand::MouseUp((x, y))).await;
    }

    async fn mouse_move(&mut self, x: i32, y: i32) {
        reserve_player_mut(|player| {
            player.mouse_loc = (x, y);
        });
        let _ = run_player_command(PlayerVMCommand::MouseMove((x, y))).await;
    }

    async fn key_down(&mut self, key: &str, code: u16) {
        reserve_player_mut(|player| {
            player.keyboard_manager.key_down(key.to_string(), code);
        });
        let _ = run_player_command(PlayerVMCommand::KeyDown(key.to_string(), code)).await;
    }

    async fn key_up(&mut self, key: &str, code: u16) {
        reserve_player_mut(|player| {
            player.keyboard_manager.key_up(key, code);
        });
        let _ = run_player_command(PlayerVMCommand::KeyUp(key.to_string(), code)).await;
    }

    async fn key_press(&mut self, key: &str, code: u16) {
        self.key_down(key, code).await;
        self.step_frame().await;
        self.key_up(key, code).await;
    }

    async fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.key_press(&ch.to_string(), ch as u16).await;
        }
    }

    /// Get a sprite's rect as (left, top, right, bottom).
    async fn sprite_rect(&self, sprite_num: usize) -> Result<(i32, i32, i32, i32), String> {
        let rect = self.eval_datum(&format!("sprite({}).rect", sprite_num)).await?;
        match rect {
            StaticDatum::IntRect(l, t, r, b) => Ok((l, t, r, b)),
            _ => Err(format!("sprite({}).rect returned {:?}, expected IntRect", sprite_num, rect)),
        }
    }

    /// Snapshot the stage cropped to a sprite's bounding rect.
    /// The result includes other sprites that overlap the same area.
    ///
    /// ```ignore
    /// let snap = player.snapshot_sprite(sprite().member("avatar")).await?;
    /// snapshots.verify("avatar", snap)?;
    /// ```
    async fn snapshot_sprite(&self, query: impl Into<SpriteQuery>) -> Result<SnapshotOutput, String> {
        let query = query.into();
        let sprite_num = self.find_sprite(&query)
            .ok_or_else(|| format!("No sprite with {} found", query))?;
        let (l, t, r, b) = self.sprite_rect(sprite_num).await?;
        Ok(self.snapshot_stage().crop(l, t, r, b))
    }

    /// Render a single sprite in isolation (transparent background, no other
    /// sprites) and return the result cropped to its bounding rect.
    ///
    /// ```ignore
    /// let snap = player.snapshot_sprite_isolated(sprite().member("avatar")).await?;
    /// snapshots.verify("avatar_only", snap)?;
    /// ```
    async fn snapshot_sprite_isolated(&self, query: impl Into<SpriteQuery>) -> Result<SnapshotOutput, String> {
        // Default: fall back to the crop approach. Browser harness overrides
        // this with a true isolated render.
        self.snapshot_sprite(query).await
    }

    /// Click the center of a sprite matched by query.
    ///
    /// ```ignore
    /// player.click_sprite(sprite().member("login_button")).await?;
    /// player.click_sprite(sprite().member_prefix("nav_")).await?;
    /// player.click_sprite(sprite().number(5)).await?;
    /// ```
    async fn click_sprite(&mut self, query: impl Into<SpriteQuery>) -> Result<(), String> {
        let query = query.into();
        let sprite_num = self.find_sprite(&query)
            .ok_or_else(|| format!("No sprite with {} found", query))?;
        let (l, t, r, b) = self.sprite_rect(sprite_num).await?;
        self.click((l + r) / 2, (t + b) / 2).await;
        Ok(())
    }

    /// Click at a specific offset (dx, dy) from the top-left of a matched sprite.
    ///
    /// ```ignore
    /// player.click_sprite_at(sprite().member("roomlist"), 100, 9).await?;
    /// ```
    async fn click_sprite_at(&mut self, query: impl Into<SpriteQuery>, dx: i32, dy: i32) -> Result<(), String> {
        let query = query.into();
        let sprite_num = self.find_sprite(&query)
            .ok_or_else(|| format!("No sprite with {} found", query))?;
        let (l, t, _, _) = self.sprite_rect(sprite_num).await?;
        self.click(l + dx, t + dy).await;
        Ok(())
    }
}

/// Compile a block only on native (non-wasm) targets.
#[macro_export]
macro_rules! native_only {
    ($($tt:tt)*) => {
        #[cfg(not(target_arch = "wasm32"))]
        { $($tt)* }
    };
}

/// Compile a block only on browser (wasm) targets.
#[macro_export]
macro_rules! browser_only {
    ($($tt:tt)*) => {
        #[cfg(target_arch = "wasm32")]
        { $($tt)* }
    };
}

/// Defines a cross-platform e2e test that runs on both native and browser.
#[macro_export]
macro_rules! hybrid_e2e_test {
    ($name:ident, |$player:ident| $body:expr) => {
        vm_rust::native_e2e_test!($name, |$player| $body);
        vm_rust::browser_e2e_test!($name, |$player| $body);
    };
}

/// Defines a native-only e2e test (skipped on wasm).
#[macro_export]
macro_rules! native_e2e_test {
    ($name:ident, |$player:ident| $body:expr) => {
        #[cfg(not(target_arch = "wasm32"))]
        #[test]
        fn $name() {
            vm_rust::player::testing::run_test(async {
                let mut $player = vm_rust::player::testing::TestPlayer::new();
                let result: Result<(), String> = $body.await;
                result.unwrap();
            });
        }
    };
}

/// Defines a browser-only e2e test (skipped on native).
///
/// Installs a panic hook that converts panics into JS exceptions so the
/// test runner sees a failure instead of hanging on an unresolved Promise.
#[macro_export]
macro_rules! browser_e2e_test {
    ($name:ident, |$player:ident| $body:expr) => {
        #[cfg(target_arch = "wasm32")]
        #[wasm_bindgen::prelude::wasm_bindgen]
        pub async fn $name() -> Result<(), wasm_bindgen::JsValue> {
            // Install a panic hook that throws a JS error instead of trapping.
            std::panic::set_hook(Box::new(|info| {
                let msg = info.to_string();
                // Log to console for visibility
                web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&msg));
                // Store on window so the test runner can detect it
                let window = web_sys::window().unwrap();
                let _ = js_sys::Reflect::set(
                    &window,
                    &wasm_bindgen::JsValue::from_str("__testPanic"),
                    &wasm_bindgen::JsValue::from_str(&msg),
                );
            }));

            let mut $player = vm_rust::player::testing_browser::BrowserTestPlayer::new().await;
            let result: Result<(), String> = $body.await;
            result.map_err(|e| wasm_bindgen::JsValue::from_str(&e))
        }
    };
}
