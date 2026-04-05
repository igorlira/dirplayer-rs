/// Platform-agnostic snapshot output.
#[derive(Clone)]
pub enum SnapshotOutput {
    /// Raw RGBA pixel data (native — used for reference-file comparison).
    Rgba { width: u32, height: u32, data: Vec<u8> },
    /// Base64-encoded PNG string (browser — from canvas.toDataURL).
    Base64Png(String),
}

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
        emit_snapshot(&snapshot_path, name, &output, self.max_diff_ratio);

        // Compare against reference files on native
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::player::testing::StageSnapshot::from_output(output)
                .assert_snapshot(&snapshot_path, name, self.max_diff_ratio)?;
        }

        Ok(())
    }
}

/// Emit a snapshot result. On browser, sends to the JS snapshot collector.
/// On native, this is a no-op (native tests handle snapshots via StageSnapshot).
pub fn emit_snapshot(suite: &str, name: &str, output: &SnapshotOutput, max_diff_ratio: f64) {
    #[cfg(target_arch = "wasm32")]
    {
        if let SnapshotOutput::Base64Png(base64) = output {
            let window = web_sys::window().unwrap();
            let save_fn = js_sys::Reflect::get(&window, &"__saveSnapshot".into())
                .expect("__saveSnapshot not found on window");
            let save_fn: js_sys::Function = save_fn.into();
            let args = js_sys::Array::new();
            args.push(&wasm_bindgen::JsValue::from_str(suite));
            args.push(&wasm_bindgen::JsValue::from_str(name));
            args.push(&wasm_bindgen::JsValue::from_str(base64));
            args.push(&wasm_bindgen::JsValue::from_f64(max_diff_ratio));
            save_fn.apply(&wasm_bindgen::JsValue::NULL, &args).unwrap();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = (suite, name, output, max_diff_ratio); }
}
