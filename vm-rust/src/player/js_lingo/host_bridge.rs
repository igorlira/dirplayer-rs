// Host bridge — the abstraction the JS interpreter uses to reach into
// Director's world (sprites, members, the * properties, trace logging,
// audio, etc.).
//
// We keep this an interface rather than direct calls into `DirPlayer` so
// the interpreter can be tested in isolation. The real bridge lives in
// `player::js_lingo_loader` (next phase) and forwards to the existing
// Lingo builtin handlers, so `sprite(1).locH` from JS goes through the
// same code as `sprite(1).locH` from Lingo.
//
// `JsHostBridge` is intentionally minimal: every Director verb the
// interpreter exposes maps to one method here. Adding a verb is a 3-step
// process: (a) add a method to this trait, (b) implement it in the
// player-bound bridge, (c) install a Native that calls it in
// `JsRuntime::install_director_globals`.

use std::any::Any;

use super::value::{JsError, JsValue};

/// One callable from JS into Director. The bridge stays trait-shaped so a
/// stub can drive tests without dragging in the entire player crate.
pub trait JsHostBridge: Any {
    /// `trace(...)` — Director's debug log. Multiple args are joined with
    /// spaces (the Lingo convention).
    fn trace(&mut self, args: &[JsValue]);

    /// `sprite(channel)` — return a proxy whose property access reads/writes
    /// the live sprite. May return `Undefined` if the channel is empty or
    /// the bridge wants to refuse.
    fn sprite(&mut self, channel: i32) -> JsValue;

    /// `member(name_or_number, cast?)` — return a member proxy. The first
    /// argument is either a name (string) or a number; the optional second
    /// argument selects a cast lib.
    fn member(&mut self, args: &[JsValue]) -> JsValue;

    /// `castLib(name_or_number)` — cast-library proxy.
    fn cast_lib(&mut self, args: &[JsValue]) -> JsValue { let _ = args; JsValue::Undefined }

    /// `go(frame, movie?)` — navigate the score.
    fn go(&mut self, args: &[JsValue]) -> Result<JsValue, JsError> { let _ = args; Ok(JsValue::Undefined) }

    /// `puppetSprite(channel, enabled)` — toggle sprite puppetry.
    fn puppet_sprite(&mut self, args: &[JsValue]) -> Result<JsValue, JsError> { let _ = args; Ok(JsValue::Undefined) }

    /// `updateStage()` — force a redraw.
    fn update_stage(&mut self) -> Result<JsValue, JsError> { Ok(JsValue::Undefined) }

    /// Generic Lingo-verb fallback. The interpreter looks up unknown global
    /// names by routing through here so movies that use less-common verbs
    /// (like `put`, `quit`, `pass`, `nothing`) keep working.
    fn call_global(&mut self, name: &str, args: &[JsValue]) -> Result<JsValue, JsError> {
        let _ = (name, args);
        Err(JsError::new(format!("Director global not exposed to JS: {}", name)))
    }
}

/// Default no-op bridge — useful for unit tests and as a fallback when no
/// player context is attached. `trace` lines go to `log::info!`.
pub struct StubBridge;
impl JsHostBridge for StubBridge {
    fn trace(&mut self, args: &[JsValue]) {
        let s = args.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
        log::info!("[js-trace] {}", s);
    }
    fn sprite(&mut self, _channel: i32) -> JsValue { JsValue::Undefined }
    fn member(&mut self, _args: &[JsValue]) -> JsValue { JsValue::Undefined }
}

/// Recording bridge — capture every call so tests can assert on what JS did.
#[derive(Default)]
pub struct RecordingBridge {
    pub traces: Vec<String>,
    pub sprite_calls: Vec<i32>,
    pub member_calls: Vec<Vec<String>>,
    pub go_calls: Vec<Vec<String>>,
    pub update_stage_calls: usize,
}

impl JsHostBridge for RecordingBridge {
    fn trace(&mut self, args: &[JsValue]) {
        self.traces.push(args.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" "));
    }
    fn sprite(&mut self, channel: i32) -> JsValue {
        self.sprite_calls.push(channel);
        // Return a fresh JsObject the JS can write into — lets `.locH = 10`
        // succeed without erroring. The next layer up will wire real proxies.
        JsValue::Object(std::rc::Rc::new(std::cell::RefCell::new(super::value::JsObject {
            class_name: "_sprite",
            ..super::value::JsObject::new()
        })))
    }
    fn member(&mut self, args: &[JsValue]) -> JsValue {
        self.member_calls.push(args.iter().map(|v| v.to_string()).collect());
        JsValue::Object(std::rc::Rc::new(std::cell::RefCell::new(super::value::JsObject {
            class_name: "_member",
            ..super::value::JsObject::new()
        })))
    }
    fn go(&mut self, args: &[JsValue]) -> Result<JsValue, JsError> {
        self.go_calls.push(args.iter().map(|v| v.to_string()).collect());
        Ok(JsValue::Undefined)
    }
    fn update_stage(&mut self) -> Result<JsValue, JsError> {
        self.update_stage_calls += 1;
        Ok(JsValue::Undefined)
    }
}
