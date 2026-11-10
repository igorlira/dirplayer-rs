//! Director's Speech built-ins (text-to-speech), implemented on the browser's Web Speech API
//! (`window.speechSynthesis`).
//!
//! Director 8.5+ exposes speech as global functions: voiceInitialize(), voiceCount(), voiceGetAll(),
//! voiceGet(), voiceSet(n), voiceSpeak(text), voicePause(), voiceResume(), voiceStop(), voiceState(),
//! voiceWordPos(), and get/set pairs for pitch, rate and volume. Movies call voiceInitialize() first and
//! fall back to silence when it does not return 1, so on a host without speech synthesis (or outside
//! the browser) every function answers the way Director does on a machine with no speech engine.
//!
//! Pitch, rate and volume are integers whose scale Director left to the platform's speech engine.
//! Here all three are percentages: 100 is the voice's normal pitch and rate and full volume, so a
//! movie that derives its values from the getters (`voiceSetPitch(voiceGetPitch() * 0.8)`) gets the
//! relative change it asked for.
//!
//! The current voice and the pitch/rate/volume settings live on the JS side (`globalThis.__dirplayerSpeech`)
//! so they sit next to the SpeechSynthesis objects they apply to.

use std::collections::VecDeque;

use wasm_bindgen::{JsCast, JsValue};

use crate::director::lingo::datum::{Datum, DatumType};
use crate::player::{reserve_player_mut, symbols::symbol::Symbol, DatumRef, ScriptError};

/// One JS function taking (op, arg) and returning a plain value. Kept as source so the VM needs no
/// extra import from the embedding page.
const SPEECH_JS: &str = r#"
const g = globalThis, syn = g.speechSynthesis;
if (!syn || typeof g.SpeechSynthesisUtterance !== 'function') return op === 'getall' ? [] : (op === 'state' ? 'stopped' : 0);
const st = g.__dirplayerSpeech || (g.__dirplayerSpeech = { voice: 0, pitch: 100, rate: 100, volume: 100, word: 0 });
const voices = () => syn.getVoices() || [];
switch (op) {
  case 'init': voices(); return 1;
  case 'count': return voices().length;
  case 'getall': return voices().map(v => v.name);
  case 'get': return st.voice + 1;
  case 'set': { const n = Math.round(Number(arg)) - 1; if (n >= 0 && n < Math.max(1, voices().length)) st.voice = n; return st.voice + 1; }
  case 'speak': {
    syn.cancel();
    const u = new g.SpeechSynthesisUtterance(String(arg));
    const v = voices()[st.voice]; if (v) u.voice = v;
    u.pitch = Math.min(2, Math.max(0, st.pitch / 100));
    u.rate = Math.min(10, Math.max(0.1, st.rate / 100));
    u.volume = Math.min(1, Math.max(0, st.volume / 100));
    st.word = 0; u.onboundary = () => { st.word += 1; };
    syn.speak(u); return 1;
  }
  case 'stop': syn.cancel(); return 1;
  case 'pause': syn.pause(); return 1;
  case 'resume': syn.resume(); return 1;
  case 'state': return syn.paused ? 'paused' : (syn.speaking || syn.pending ? 'playing' : 'stopped');
  case 'wordpos': return st.word;
  case 'getpitch': return st.pitch;
  case 'setpitch': st.pitch = Math.min(200, Math.max(0, Math.round(Number(arg)) || 0)); return st.pitch;
  case 'getrate': return st.rate;
  case 'setrate': st.rate = Math.min(1000, Math.max(10, Math.round(Number(arg)) || 0)); return st.rate;
  case 'getvolume': return st.volume;
  case 'setvolume': st.volume = Math.min(100, Math.max(0, Math.round(Number(arg)) || 0)); return st.volume;
}
return 0;
"#;

fn js(op: &str, arg: &JsValue) -> JsValue {
    #[cfg(target_arch = "wasm32")]
    {
        thread_local! {
            static F: js_sys::Function = js_sys::Function::new_with_args("op, arg", SPEECH_JS);
        }
        return F.with(|f| f.call2(&JsValue::NULL, &JsValue::from_str(op), arg).unwrap_or(JsValue::from_f64(0.0)));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (op, arg, SPEECH_JS);
        JsValue::from_f64(0.0)
    }
}

fn voice_proplist(player: &mut crate::player::DirPlayer, index: i32, name: &str) -> DatumRef {
    // Director's voice description: [#name, #age, #gender, #index]. Browsers report neither age nor gender.
    let mut pairs: VecDeque<(DatumRef, DatumRef)> = VecDeque::new();
    let mut add = |player: &mut crate::player::DirPlayer, k: &str, v: Datum| {
        let key = player.alloc_datum(Datum::Symbol(Symbol::from_str(k)));
        let val = player.alloc_datum(v);
        pairs.push_back((key, val));
    };
    add(player, "name", Datum::String(name.to_string()));
    add(player, "age", Datum::String("unknown".to_string()));
    add(player, "gender", Datum::String("unknown".to_string()));
    add(player, "index", Datum::Int(index));
    player.alloc_datum(Datum::PropList(pairs, false))
}

fn voice_names() -> Vec<String> {
    let v = js("getall", &JsValue::NULL);
    match v.dyn_into::<js_sys::Array>() {
        Ok(a) => a.iter().map(|x| x.as_string().unwrap_or_default()).collect(),
        Err(_) => vec![],
    }
}

/// Handles `name(args)` when it is one of Director's speech functions; `None` otherwise.
pub fn try_call(name: &str, args: &Vec<DatumRef>) -> Option<Result<DatumRef, ScriptError>> {
    let lname = name.to_ascii_lowercase();
    let op = lname.strip_prefix("voice")?;
    if !matches!(op, "initialize" | "count" | "getall" | "get" | "set" | "speak" | "stop" | "pause" | "resume"
        | "state" | "wordpos" | "getpitch" | "setpitch" | "getrate" | "setrate" | "getvolume" | "setvolume") {
        return None;
    }
    Some(reserve_player_mut(|player| {
        let arg = match args.first() {
            Some(a) => {
                let d = player.get_datum(a);
                match d.int_value() {
                    Ok(i) if !matches!(d, Datum::String(_)) => JsValue::from_f64(i as f64),
                    _ => JsValue::from_str(&d.string_value().unwrap_or_default()),
                }
            }
            None => JsValue::NULL,
        };
        match op {
            "getall" => {
                let items: VecDeque<DatumRef> = voice_names().iter().enumerate()
                    .map(|(i, n)| voice_proplist(player, i as i32 + 1, n)).collect();
                Ok(player.alloc_datum(Datum::List(DatumType::List, items, false)))
            }
            "get" => {
                let idx = js("get", &JsValue::NULL).as_f64().unwrap_or(0.0) as i32;
                let names = voice_names();
                match names.get((idx - 1).max(0) as usize) {
                    Some(n) if idx >= 1 => Ok(voice_proplist(player, idx, n)),
                    _ => Ok(player.alloc_datum(Datum::Int(0))),
                }
            }
            "state" => {
                let s = js("state", &JsValue::NULL).as_string().unwrap_or_else(|| "stopped".to_string());
                Ok(player.alloc_datum(Datum::Symbol(Symbol::from_str(&s))))
            }
            "initialize" => Ok(player.alloc_datum(Datum::Int(js("init", &JsValue::NULL).as_f64().unwrap_or(0.0) as i32))),
            _ => Ok(player.alloc_datum(Datum::Int(js(op, &arg).as_f64().unwrap_or(0.0) as i32))),
        }
    }))
}
