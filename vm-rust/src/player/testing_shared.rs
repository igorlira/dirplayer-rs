use crate::director::static_datum::StaticDatum;
use crate::player::{
    commands::{run_player_command, PlayerVMCommand},
    datum_ref::DatumRef,
    eval::eval_lingo_command,
    fire_pending_timeouts,
    reserve_player_mut, reserve_player_ref, run_movie_init_sequence, run_single_frame,
    ScriptError,
};

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
        Ok(StaticDatum::from(result))
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

    fn find_sprite_by_member_name(&self, name: &str) -> Option<usize> {
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

    fn find_sprite_by_member_prefix(&self, prefix: &str) -> Option<usize> {
        let prefix_lower = prefix.to_ascii_lowercase();
        reserve_player_ref(|player| {
            for channel in &player.movie.score.channels {
                let sprite = &channel.sprite;
                if let Some(member_ref) = &sprite.member {
                    if let Some(member) = player.movie.cast_manager.find_member_by_ref(member_ref) {
                        if member.name.to_ascii_lowercase().starts_with(&prefix_lower) {
                            return Some(sprite.number);
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

    async fn step_until_sprite_visible(
        &mut self,
        timeout_secs: f64,
        member_name: &str,
        min_visibility: f64,
    ) -> Result<(), String> {
        let deadline_ms = now_ms() + timeout_secs * 1000.0;
        let mut frames = 0usize;
        while now_ms() < deadline_ms {
            if let Some(sprite_num) = self.find_sprite_by_member_name(member_name) {
                if self.sprite_visibility(sprite_num).await >= min_visibility {
                    return Ok(());
                }
            }
            if !self.step_frame().await {
                return Err(format!("Movie stopped while waiting for '{}'", member_name));
            }
            frames += 1;
        }
        if let Some(sprite_num) = self.find_sprite_by_member_name(member_name) {
            let vis = self.sprite_visibility(sprite_num).await;
            Err(format!(
                "'{}' at {:.1}% visibility after {:.1}s / {} frames (need {:.0}%)",
                member_name, vis * 100.0, timeout_secs, frames, min_visibility * 100.0
            ))
        } else {
            Err(format!("'{}' not found after {:.1}s / {} frames", member_name, timeout_secs, frames))
        }
    }

    async fn step_until_datum(
        &mut self,
        timeout_secs: f64,
        expr: &str,
        expected: &StaticDatum,
    ) -> Result<(), String> {
        let deadline_ms = now_ms() + timeout_secs * 1000.0;
        let mut frames = 0usize;
        while now_ms() < deadline_ms {
            if let Ok(val) = self.eval_datum(expr).await {
                if val == *expected {
                    return Ok(());
                }
            }
            if !self.step_frame().await {
                return Err(format!("Movie stopped while waiting for {} == {:?}", expr, expected));
            }
            frames += 1;
        }
        let actual = self.eval_datum(expr).await.unwrap_or(StaticDatum::Void);
        Err(format!(
            "{} == {:?} not met after {:.1}s / {} frames (actual: {:?})",
            expr, expected, timeout_secs, frames, actual
        ))
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

    /// Click the center of a sprite.
    async fn click_sprite(&mut self, sprite_num: usize) -> Result<(), String> {
        let (l, t, r, b) = self.sprite_rect(sprite_num).await?;
        self.click((l + r) / 2, (t + b) / 2).await;
        Ok(())
    }

    /// Click at a specific offset (dx, dy) from the sprite's top-left corner.
    async fn click_sprite_at(&mut self, sprite_num: usize, dx: i32, dy: i32) -> Result<(), String> {
        let (l, t, _, _) = self.sprite_rect(sprite_num).await?;
        self.click(l + dx, t + dy).await;
        Ok(())
    }

    async fn click_member(&mut self, member_name: &str) -> Result<(), String> {
        let sprite_num = self.find_sprite_by_member_name(member_name)
            .ok_or_else(|| format!("No sprite with member '{}' found", member_name))?;
        self.click_sprite(sprite_num).await
    }

    /// Click at a specific offset from the top-left of the sprite with the given member name.
    async fn click_member_at(&mut self, member_name: &str, dx: i32, dy: i32) -> Result<(), String> {
        let sprite_num = self.find_sprite_by_member_name(member_name)
            .ok_or_else(|| format!("No sprite with member '{}' found", member_name))?;
        self.click_sprite_at(sprite_num, dx, dy).await
    }

    async fn click_member_prefix(&mut self, prefix: &str) -> Result<(), String> {
        let sprite_num = self.find_sprite_by_member_prefix(prefix)
            .ok_or_else(|| format!("No sprite with member starting with '{}' found", prefix))?;
        self.click_sprite(sprite_num).await
    }

    /// Click at a specific offset from the top-left of the sprite whose member name starts with the given prefix.
    async fn click_member_prefix_at(&mut self, prefix: &str, dx: i32, dy: i32) -> Result<(), String> {
        let sprite_num = self.find_sprite_by_member_prefix(prefix)
            .ok_or_else(|| format!("No sprite with member starting with '{}' found", prefix))?;
        self.click_sprite_at(sprite_num, dx, dy).await
    }
}

/// Emit a snapshot result. On browser, sends to the JS snapshot collector.
/// On native, this is a no-op (native tests handle snapshots via StageSnapshot).
pub fn emit_snapshot(suite: &str, name: &str, output: &SnapshotOutput) {
    #[cfg(target_arch = "wasm32")]
    {
        if let SnapshotOutput::Base64Png(base64) = output {
            let window = web_sys::window().unwrap();
            let save_fn = js_sys::Reflect::get(&window, &"__saveSnapshot".into())
                .expect("__saveSnapshot not found on window");
            let save_fn: js_sys::Function = save_fn.into();
            save_fn.call3(&wasm_bindgen::JsValue::NULL,
                &wasm_bindgen::JsValue::from_str(suite),
                &wasm_bindgen::JsValue::from_str(name),
                &wasm_bindgen::JsValue::from_str(base64),
            ).unwrap();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = (suite, name, output); }
}

/// Platform-agnostic snapshot output.
#[derive(Clone)]
pub enum SnapshotOutput {
    /// Raw RGBA pixel data (native — used for reference-file comparison).
    Rgba { width: u32, height: u32, data: Vec<u8> },
    /// Base64-encoded PNG string (browser — from canvas.toDataURL).
    Base64Png(String),
}

// --- Shared test scenarios ---
// These are written once and called from both native and browser test entry points.

/// Tracks snapshot paths and diff tolerance for snapshot verification.
///
/// `suite` is the top-level test suite (e.g. "habbo").
/// `test` is the individual test within the suite (e.g. "load", "login").
///
/// Snapshots are stored as: `snapshots/{suite}/{native|browser}/{test}/{name}.png`
pub struct SnapshotContext {
    pub suite: String,
    pub test: String,
    pub max_diff_ratio: f64,
}

impl SnapshotContext {
    pub fn new(suite: &str, test: &str) -> Self {
        SnapshotContext {
            suite: suite.to_string(),
            test: test.to_string(),
            max_diff_ratio: 0.006,
        }
    }

    /// Take a snapshot, emit it to the browser collector, and verify it
    /// against the reference file on native. Returns an error if the diff
    /// exceeds `max_diff_ratio`.
    pub fn verify(&self, name: &str, output: SnapshotOutput) -> Result<(), String> {
        let snapshot_path = format!("{}/{}", self.suite, self.test);

        // Emit to browser snapshot collector (no-op on native)
        emit_snapshot(&snapshot_path, name, &output);

        // Compare against reference files on native
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::player::testing::StageSnapshot::from_output(output)
                .assert_snapshot(&snapshot_path, name, self.max_diff_ratio)?;
        }

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

            let mut $player = vm_rust::player::testing_browser::BrowserTestPlayer::new();
            let result: Result<(), String> = $body.await;
            result.map_err(|e| wasm_bindgen::JsValue::from_str(&e))
        }
    };
}
