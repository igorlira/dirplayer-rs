use fxhash::FxHashMap;
use itertools::Itertools;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

pub struct ProfilingToken {
    name: String,
    start_time: u64,
    end_time: Option<u64>,
}

impl ProfilingToken {
    pub fn elapsed(&self) -> Option<std::time::Duration> {
        match self.end_time {
            Some(end_time) => Some(Duration::from_millis(end_time - self.start_time)),
            None => None,
        }
    }
}

pub struct PlayerProfiler {
    tokens: FxHashMap<u32, ProfilingToken>,
    total_time_by_name: HashMap<String, Duration>,
    token_id_counter: u32,
}

impl PlayerProfiler {
    pub fn new() -> PlayerProfiler {
        PlayerProfiler {
            tokens: FxHashMap::default(),
            token_id_counter: 0,
            total_time_by_name: HashMap::new(),
        }
    }

    pub fn start(&mut self, name: String) -> u32 {
        let id = self.token_id_counter;
        self.tokens.insert(
            id,
            ProfilingToken {
                name,
                start_time: chrono::Local::now().timestamp_millis() as u64,
                end_time: None,
            },
        );
        self.token_id_counter += 1;
        id
    }

    pub fn end(&mut self, id: u32) {
        let mut token = self.tokens.remove(&id).unwrap();
        token.end_time = Some(chrono::Local::now().timestamp_millis() as u64);
        let elapsed = token.elapsed().unwrap();

        let elapsed_by_this_name = self
            .total_time_by_name
            .get(&token.name)
            .map(|x| x.to_owned())
            .unwrap_or(Duration::from_millis(0));

        self.total_time_by_name
            .insert(token.name.to_owned(), elapsed_by_this_name + elapsed);

        println!("{} took {:?}", token.name, elapsed);
    }

    pub fn report(&self) -> String {
        let mut result = String::new();

        let total_elapsed: Duration = self.total_time_by_name.values().map(|x| x.to_owned()).sum();
        let total_ms = total_elapsed.as_millis();

        let names_sorted_by_elapsed = self
            .total_time_by_name
            .iter()
            .sorted_by(|a, b| b.1.cmp(a.1))
            .rev();
        for (name, elapsed) in names_sorted_by_elapsed {
            let elapsed_percent = (elapsed.as_millis() as f64 / total_ms as f64) * 100.0;

            result.push_str(&format!(
                "{} took {:?} ({:.2}%)\n",
                name, elapsed, elapsed_percent
            ));
        }
        result.push_str(&format!("Total: {:?}\n", total_elapsed));

        return result;
    }
}

fn profiler() -> &'static Mutex<PlayerProfiler> {
    static MAP: OnceLock<Mutex<PlayerProfiler>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(PlayerProfiler::new()))
}

#[allow(dead_code)]
pub fn start_profiling(name: String) -> u32 {
    let mut profiler = profiler().lock().unwrap();
    profiler.start(name)
}

#[allow(dead_code)]
pub fn end_profiling(id: u32) {
    let mut profiler = profiler().lock().unwrap();
    profiler.end(id)
}

pub fn get_profiler_report() -> String {
    let profiler = profiler().lock().unwrap();
    profiler.report()
}

// ---------------------------------------------------------------------------
// Speedscope "evented" profile recorder
//
// Records a properly-nested stream of open/close events for handler calls and
// individual bytecode ops, then serialises them to the speedscope native file
// format (https://www.speedscope.app/file-formats/0.0.1.json). Recording is
// off by default and gated by a cheap atomic load so the hot bytecode loop
// pays effectively nothing when profiling is disabled.
// ---------------------------------------------------------------------------

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fast gate checked by `ProfileScope` on the hot path. Avoids touching the
/// thread-local recorder at all when recording is off.
static RECORDING: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn is_recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// Monotonic clock in fractional milliseconds. Uses `performance.now()` in the
/// browser (microsecond precision) and `Instant` elapsed on native.
#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    use std::time::Instant;
    static BASE: OnceLock<Instant> = OnceLock::new();
    let base = BASE.get_or_init(Instant::now);
    base.elapsed().as_secs_f64() * 1000.0
}

/// Target wall-clock gap between stack samples, in milliseconds (≈ Chrome's
/// default sampling rate). Smaller = finer detail but more samples.
const SAMPLE_INTERVAL_MS: f64 = 1.0;

/// Read the (relatively expensive) JS clock only once every this many stack
/// transitions, then decide whether enough wall-clock has elapsed to take a
/// sample. Keeps `performance.now()` off the per-opcode hot path.
const CLOCK_CHECK_EVERY: u32 = 64;

/// Safety ceiling on stored samples. At the 1 ms target this is ~50 minutes of
/// capture; far beyond it we stop to avoid unbounded memory. (Unlike the old
/// tracing buffer, sample volume is bounded by *time*, not by how much code
/// runs — so a 250 s capture is ~250 k samples regardless of opcode count.)
const MAX_SAMPLES: usize = 3_000_000;

/// A sampling profiler (like Chrome's): instead of recording every handler/op
/// enter and exit, it keeps the *live* call stack and snapshots it on a fixed
/// wall-clock cadence. Memory scales with capture duration, not with how many
/// opcodes execute — so long (minutes-long) captures stay bounded.
struct SpeedscopeRecorder {
    frames: Vec<String>,
    frame_index: HashMap<String, usize>,
    /// Live call stack of frame indices (root first), maintained by the guards.
    stack: Vec<usize>,
    /// One entry per sample: a snapshot of `stack` (root first).
    samples: Vec<Vec<usize>>,
    /// Wall-clock weight (ms) attributed to each sample.
    weights: Vec<f64>,
    start_ms: f64,
    last_sample_ms: f64,
    transitions: u32,
    capped: bool,
}

impl SpeedscopeRecorder {
    fn new() -> Self {
        SpeedscopeRecorder {
            frames: Vec::new(),
            frame_index: HashMap::new(),
            stack: Vec::new(),
            samples: Vec::new(),
            weights: Vec::new(),
            start_ms: 0.0,
            last_sample_ms: 0.0,
            transitions: 0,
            capped: false,
        }
    }

    fn intern(&mut self, name: &str) -> usize {
        if let Some(idx) = self.frame_index.get(name) {
            return *idx;
        }
        let idx = self.frames.len();
        self.frames.push(name.to_owned());
        self.frame_index.insert(name.to_owned(), idx);
        idx
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.frame_index.clear();
        self.stack.clear();
        self.samples.clear();
        self.weights.clear();
        let now = now_ms();
        self.start_ms = now;
        self.last_sample_ms = now;
        self.transitions = 0;
        self.capped = false;
    }

    /// Take a sample of the current stack if enough wall-clock has elapsed.
    /// Clock reads are throttled by `CLOCK_CHECK_EVERY` to stay cheap.
    fn maybe_sample(&mut self) {
        self.transitions = self.transitions.wrapping_add(1);
        if self.transitions < CLOCK_CHECK_EVERY {
            return;
        }
        self.transitions = 0;
        let now = now_ms();
        let elapsed = now - self.last_sample_ms;
        if elapsed < SAMPLE_INTERVAL_MS {
            return;
        }
        if self.samples.len() >= MAX_SAMPLES {
            if !self.capped {
                self.capped = true;
                RECORDING.store(false, Ordering::Relaxed);
                log::warn!(
                    "[dirplayer profiler] sample cap ({}) reached — recording auto-stopped.",
                    MAX_SAMPLES
                );
            }
            return;
        }
        self.samples.push(self.stack.clone());
        self.weights.push(elapsed);
        self.last_sample_ms = now;
    }

    fn open(&mut self, name: &str) {
        let frame = self.intern(name);
        self.stack.push(frame);
        self.maybe_sample();
    }

    fn close(&mut self) {
        // Sample (attributing elapsed time to the stack that includes the
        // frame being closed) before popping it.
        self.maybe_sample();
        self.stack.pop();
    }
}

thread_local! {
    static RECORDER: RefCell<SpeedscopeRecorder> = RefCell::new(SpeedscopeRecorder::new());
}

/// Begin a fresh recording (clears any previous samples).
pub fn start_recording() {
    RECORDER.with(|r| r.borrow_mut().clear());
    RECORDING.store(true, Ordering::Relaxed);
}

/// Stop recording. The buffered samples remain available for export.
pub fn stop_recording() {
    RECORDING.store(false, Ordering::Relaxed);
}

/// Discard all buffered samples.
pub fn clear_recording() {
    RECORDER.with(|r| r.borrow_mut().clear());
}

#[inline]
fn record_open(name: &str) {
    RECORDER.with(|r| r.borrow_mut().open(name));
}

#[inline]
fn record_close() {
    RECORDER.with(|r| r.borrow_mut().close());
}

/// RAII guard: pushes `name` onto the live call stack on creation and pops it
/// when dropped. Because guards drop in reverse (LIFO) order, nesting them
/// mirrors the call stack — and the pop fires on every exit path, including
/// early `return`/`break`. The sampler snapshots this stack on a time cadence.
///
/// Cheap when recording is off: `new` records the gate state once, and a
/// disabled guard does nothing on drop.
pub struct ProfileScope {
    active: bool,
}

impl ProfileScope {
    #[inline]
    pub fn new(name: &'static str) -> Self {
        let active = is_recording();
        if active {
            record_open(name);
        }
        ProfileScope { active }
    }
}

impl Drop for ProfileScope {
    #[inline]
    fn drop(&mut self) {
        if self.active {
            record_close();
        }
    }
}

/// Variant of [`ProfileScope`] for dynamic (owned) frame names, e.g. handler
/// names that aren't `&'static`.
pub struct ProfileScopeOwned {
    active: bool,
}

impl ProfileScopeOwned {
    #[inline]
    pub fn new(name: String) -> Self {
        let active = is_recording();
        if active {
            record_open(&name);
        }
        ProfileScopeOwned { active }
    }
}

impl Drop for ProfileScopeOwned {
    #[inline]
    fn drop(&mut self) {
        if self.active {
            record_close();
        }
    }
}

/// Serialise the buffered samples to a speedscope "sampled" profile JSON string.
pub fn export_speedscope_json() -> String {
    RECORDER.with(|r| {
        let rec = r.borrow();

        let mut frames_json = String::from("[");
        for (i, name) in rec.frames.iter().enumerate() {
            if i > 0 {
                frames_json.push(',');
            }
            frames_json.push_str(&format!("{{\"name\":{}}}", json_escape(name)));
        }
        frames_json.push(']');

        // samples: array of stacks (each an array of frame indices, root first)
        let mut samples_json = String::from("[");
        for (i, stack) in rec.samples.iter().enumerate() {
            if i > 0 {
                samples_json.push(',');
            }
            samples_json.push('[');
            for (j, frame) in stack.iter().enumerate() {
                if j > 0 {
                    samples_json.push(',');
                }
                samples_json.push_str(itoa(*frame).as_str());
            }
            samples_json.push(']');
        }
        samples_json.push(']');

        // weights: one wall-clock duration (ms) per sample
        let mut weights_json = String::from("[");
        let mut total: f64 = 0.0;
        for (i, w) in rec.weights.iter().enumerate() {
            if i > 0 {
                weights_json.push(',');
            }
            weights_json.push_str(&format!("{}", w));
            total += *w;
        }
        weights_json.push(']');

        format!(
            "{{\"$schema\":\"https://www.speedscope.app/file-formats/0.0.1.json\",\
\"exporter\":\"dirplayer\",\"name\":\"dirplayer movie\",\"activeProfileIndex\":0,\
\"shared\":{{\"frames\":{frames}}},\
\"profiles\":[{{\"type\":\"sampled\",\"name\":\"Lingo VM\",\"unit\":\"milliseconds\",\
\"startValue\":0,\"endValue\":{total},\"samples\":{samples},\"weights\":{weights}}}]}}",
            frames = frames_json,
            total = total,
            samples = samples_json,
            weights = weights_json,
        )
    })
}

/// Minimal integer-to-string without pulling in a dependency; avoids the
/// per-call `format!` allocation churn in the hot sample-serialisation loop.
fn itoa(mut n: usize) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    String::from_utf8_lossy(&buf[i..]).into_owned()
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod speedscope_tests {
    use super::*;

    // Single test: `RECORDING` is a process-global atomic, so splitting this
    // across multiple `#[test]` fns (which run on parallel threads) would race
    // on the flag. Keeping it in one test makes the state deterministic.
    #[test]
    fn speedscope_sampler_roundtrip() {
        use std::time::Duration;

        // 1. While disabled, guards record nothing.
        clear_recording();
        {
            let _h = ProfileScope::new("should_not_record");
        }
        assert!(!export_speedscope_json().contains("should_not_record"));

        // 2. Drive real transitions over real wall-clock so the sampler lands
        //    several samples (clock is checked every CLOCK_CHECK_EVERY
        //    transitions, then samples once SAMPLE_INTERVAL_MS has elapsed).
        start_recording();
        let _h = ProfileScope::new("prepareMovie");
        for _ in 0..400 {
            {
                let _it = ProfileScope::new("[iter]");
                let _op = ProfileScope::new("getmovieprop"); // open+close pairs
            }
            std::thread::sleep(Duration::from_micros(100));
        }
        stop_recording();

        let json = export_speedscope_json();

        // Speedscope "sampled" format markers.
        assert!(json.contains("speedscope.app/file-formats/0.0.1.json"));
        assert!(json.contains("\"type\":\"sampled\""));
        assert!(json.contains("\"unit\":\"milliseconds\""));
        assert!(json.contains("\"samples\":"));
        assert!(json.contains("\"weights\":"));
        // Frames we opened are interned.
        for f in ["prepareMovie", "[iter]", "getmovieprop"] {
            assert!(json.contains(f), "missing frame {f} in {json}");
        }
        // The sampler produced at least one stack snapshot over ~40 ms.
        assert!(
            !json.contains("\"samples\":[]"),
            "expected at least one sample, got {json}"
        );

        clear_recording();
    }
}
