/// Platform-agnostic snapshot output.
#[derive(Clone)]
pub enum SnapshotOutput {
    /// Raw RGBA pixel data (native — used for reference-file comparison).
    Rgba { width: u32, height: u32, data: Vec<u8> },
    /// Base64-encoded PNG string (browser — from canvas.toDataURL).
    Base64Png(String),
}

impl SnapshotOutput {
    /// Crop to a pixel rectangle (left, top, right, bottom). Coordinates are
    /// clamped to the image bounds.
    pub fn crop(self, left: i32, top: i32, right: i32, bottom: i32) -> Self {
        match self {
            SnapshotOutput::Rgba { width, height, data } => {
                let l = (left.max(0) as u32).min(width);
                let t = (top.max(0) as u32).min(height);
                let r = (right.max(0) as u32).min(width);
                let b = (bottom.max(0) as u32).min(height);
                let cw = r.saturating_sub(l);
                let ch = b.saturating_sub(t);
                let mut cropped = Vec::with_capacity((cw * ch * 4) as usize);
                for row in t..b {
                    let start = ((row * width + l) * 4) as usize;
                    let end = start + (cw * 4) as usize;
                    cropped.extend_from_slice(&data[start..end]);
                }
                SnapshotOutput::Rgba { width: cw, height: ch, data: cropped }
            }
            SnapshotOutput::Base64Png(b64) => {
                use base64::Engine;
                let png_bytes = base64::engine::general_purpose::STANDARD
                    .decode(&b64).expect("Invalid base64 in snapshot");
                let img = image::load_from_memory_with_format(&png_bytes, image::ImageFormat::Png)
                    .expect("Failed to decode PNG snapshot");
                let (iw, ih) = (img.width(), img.height());
                let l = (left.max(0) as u32).min(iw);
                let t = (top.max(0) as u32).min(ih);
                let r = (right.max(0) as u32).min(iw);
                let b = (bottom.max(0) as u32).min(ih);
                let cropped = image::imageops::crop_imm(&img, l, t, r - l, b - t).to_image();
                let mut buf = std::io::Cursor::new(Vec::new());
                cropped.write_to(&mut buf, image::ImageFormat::Png)
                    .expect("Failed to encode cropped PNG");
                let new_b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
                SnapshotOutput::Base64Png(new_b64)
            }
        }
    }
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
        self.verify_with_ratio(name, output, self.max_diff_ratio)
    }

    /// Like [`verify`], but overrides the diff tolerance for this snapshot only.
    pub fn verify_with_ratio(&self, name: &str, output: SnapshotOutput, max_diff_ratio: f64) -> Result<(), String> {
        let snapshot_path = format!("{}/{}", self.suite, self.test);

        // Browser: emit to the JS collector; logging happens async via __saveSnapshot callback.
        #[cfg(target_arch = "wasm32")]
        emit_snapshot(&snapshot_path, name, &output, max_diff_ratio);

        // Native: compare synchronously and log the result in one line.
        #[cfg(not(target_arch = "wasm32"))]
        {
            match crate::player::testing::StageSnapshot::from_output(output)
                .assert_snapshot(&snapshot_path, name, max_diff_ratio)
            {
                Ok(Some(ratio)) => super::log_test_action(&format!(
                    "Snapshot: {} — {:.3}% diff", name, ratio * 100.0
                )),
                Ok(None) => super::log_test_action(&format!(
                    "Snapshot: {} — no reference", name
                )),
                Err(e) => return Err(e),
            }
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
