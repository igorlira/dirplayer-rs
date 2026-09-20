use std::sync::Arc;

use async_std::sync::Mutex;
use js_sys::Uint8Array;
use log::debug;
use url::Url;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use percent_encoding::percent_decode_str;

use crate::player::net_manager::NetManagerSharedState;
use crate::player::net_progress::{
    should_hold_nested_dcr, synthetic_mid_file_progress,
};

pub type NetResult = Result<Vec<u8>, i32>;

/// Tracks the last streamStatus phase reported for a net task.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamStatusPhase {
    Connecting,
    InProgress,
    Final,
}

#[derive(Clone, Default)]
pub struct NetTaskState {
    pub result: Option<NetResult>,
    /// Bytes downloaded so far (updated progressively during streaming)
    pub bytes_loaded: u64,
    /// Total bytes expected (from Content-Length header, 0 if unknown)
    pub bytes_total: u64,
    /// Set when Lingo `getStreamStatus` has returned a true mid-file
    /// `InProgress` (`0 < bytesSoFar < bytesTotal`) for this task.
    pub lingo_saw_mid_progress: bool,
}

#[derive(Clone)]
pub struct NetTask {
    pub id: u32,
    pub url: String,
    pub resolved_url: Url,
    pub method: HttpMethod,
    pub post_data: Option<String>,
}

#[derive(Clone)]
pub enum HttpMethod {
    Get,
    Post,
}

impl NetTask {
    pub fn new<'b>(id: u32, url: &str, resolved_url: &Url) -> NetTask {
        return NetTask {
            id: id.clone().to_owned(),
            url: url.clone().to_owned(),
            resolved_url: resolved_url.clone().to_owned(),
            method: HttpMethod::Get,
            post_data: None,
        };
    }

    pub fn new_post(id: u32, url: &str, resolved_url: &Url, post_data: String) -> NetTask {
        NetTask {
            id,
            url: url.to_owned(),
            resolved_url: resolved_url.to_owned(),
            method: HttpMethod::Post,
            post_data: Some(post_data),
        }
    }
}

impl NetTaskState {
    pub fn is_done(&self) -> bool {
        self.result.is_some()
    }
}

pub async fn fetch_net_task(
    task: &NetTask,
    shared_state: Arc<Mutex<NetManagerSharedState>>,
) -> NetResult {
    let resolved_url_str = task.resolved_url.to_string();
    debug!(
        "execute_task #{} url: {} resolved: {}",
        task.id, task.url, resolved_url_str
    );

    // Normal HTTP(S) fetch
    // Note: file:// URLs are handled in preload_net_thing and never reach this function
    let window = web_sys::window().unwrap();

    let mut url_string = task.resolved_url.to_string();
    url_string = percent_decode_str(&url_string)
        .decode_utf8()
        .unwrap()
        .to_string();

    let request = match task.method {
        HttpMethod::Get => web_sys::Request::new_with_str(&url_string.as_str()).unwrap(),
        HttpMethod::Post => {
            let mut opts = web_sys::RequestInit::new();
            opts.method("POST");

            if let Some(post_data) = &task.post_data {
                opts.body(Some(&JsValue::from_str(post_data)));
            }

            // Set Content-Type to form-urlencoded so servers populate
            // $_POST (PHP) / request.form (others). Without this, fetch()
            // defaults string bodies to text/plain, which most server-side
            // form parsers ignore.
            let headers = web_sys::Headers::new().unwrap();
            headers
                .set("Content-Type", "application/x-www-form-urlencoded")
                .unwrap();
            opts.headers(&headers);

            web_sys::Request::new_with_str_and_init(&url_string.as_str(), &opts).unwrap()
        }
    };

    let resp_result = JsFuture::from(window.fetch_with_request(&request)).await;
    let resp_value = match resp_result {
        Ok(v) => v,
        Err(_) => return Err(4),
    };

    assert!(resp_value.is_instance_of::<Response>());
    let resp: Response = resp_value.dyn_into().unwrap();
    if resp.status() != 200 {
        return Err(4);
    }

    // Get Content-Length for bytesTotal
    let content_length: u64 = resp
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    {
        let mut state = shared_state.lock().await;
        state.update_task_progress(task.id, 0, content_length);
    }

    // Try streaming via ReadableStream for progress updates
    let body = resp.body();
    if let Some(body) = body {
        let reader = body.get_reader();
        let reader: web_sys::ReadableStreamDefaultReader = reader.dyn_into().unwrap();
        let mut bytes = Vec::new();

        loop {
            let chunk_result = JsFuture::from(reader.read()).await;
            let chunk = match chunk_result {
                Ok(v) => v,
                Err(_) => return Err(4),
            };

            let done = js_sys::Reflect::get(&chunk, &"done".into())
                .unwrap()
                .as_bool()
                .unwrap_or(true);

            if done {
                // Final progress. If the server didn't advertise Content-Length
                // (e.g. a CORS proxy stripped it), report the actual byte count
                // as the total so getStreamStatus(netID)[#bytestotal] is non-zero
                // and Director's `percentloaded` reaches 100 on completion
                // (DGS's showGameLoadStats divides by #bytestotal — 0 leaves the
                // loading bar stuck and `percentloaded` at 0 forever).
                let mut state = shared_state.lock().await;
                let total = content_length.max(bytes.len() as u64);
                state.update_task_progress(task.id, bytes.len() as u64, total);
                break;
            }

            let value = js_sys::Reflect::get(&chunk, &"value".into()).unwrap();
            let array = Uint8Array::new(&value);
            bytes.extend_from_slice(&array.to_vec());

            // Update progress
            {
                let mut state = shared_state.lock().await;
                state.update_task_progress(task.id, bytes.len() as u64, content_length);
            }
        }

        maybe_hold_dcr_for_preloader(
            &task.url,
            &shared_state,
            task.id,
            bytes.len() as u64,
        )
        .await;
        Ok(bytes)
    } else {
        // Fallback: no body stream, read all at once
        let blob = JsFuture::from(resp.array_buffer().unwrap()).await.unwrap();
        let blob_buffer = Uint8Array::new(&blob);
        let bytes = blob_buffer.to_vec();

        {
            let mut state = shared_state.lock().await;
            let total = content_length.max(bytes.len() as u64);
            state.update_task_progress(task.id, bytes.len() as u64, total);
        }

        maybe_hold_dcr_for_preloader(
            &task.url,
            &shared_state,
            task.id,
            bytes.len() as u64,
        )
        .await;
        Ok(bytes)
    }
}

/// Neopets DGS shows a Flash preloader whose guest-login prompt is only
/// (re)painted while the nested game `.dcr` is still streaming — the Director
/// loader sits at load-state 34 calling `showGameLoadStats` → Flash
/// `showLoadingStatus` each frame, which refills the `pre_main` field *after*
/// the translation's `onLoad` blanks it. That handler no-ops when
/// `bytesSoFar == 0` or percent == 100, then `gameLoaded()` (`netDone`) jumps
/// straight to state 35/350 which only polls Flash `playGame`.
///
/// When the whole `.dcr` arrives in one chunk (browser cache, or a fetch that
/// never streamed), those paint frames never happen and the guest gate waits
/// forever on a Play button that still says "Loading Game". Typical internet
/// connections of the era took many frames to download a game `.dcr`, so a
/// mid-file `getStreamStatus` was the usual case. Finishing before any poll
/// is a modern cache/fetch artifact, not a Director feature.
///
/// Keep `netDone` false until Lingo has seen one true mid-file
/// `getStreamStatus` (or ~50ms, so Matematik-style `netDone` waits are not
/// stalled). Skip the initial movie load (`!is_playing` / `goto_wait`) and
/// movies with no Flash sprite. Do not key off tempo: the async fetch can
/// complete while `current_frame_tempo` is still the default 30 even after
/// `puppetTempo(999)`.
async fn maybe_hold_dcr_for_preloader(
    url: &str,
    shared_state: &Arc<Mutex<NetManagerSharedState>>,
    task_id: u32,
    bytes_len: u64,
) {
    let (goto_wait_active, is_playing, flash_sprites) =
        crate::player::reserve_player_ref(|p| {
            (
                p.goto_wait_active,
                p.is_playing,
                p.flash_sprite_loaded.len(),
            )
        });
    let flash_active = web_sys::window()
        .and_then(|w| {
            js_sys::Reflect::get(
                &w,
                &wasm_bindgen::JsValue::from_str("__dirplayerActiveFlashCount"),
            )
            .ok()
        })
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if !should_hold_nested_dcr(
        url,
        goto_wait_active,
        is_playing,
        flash_active > 0.0 || flash_sprites > 0,
    ) {
        return;
    }

    let saw_mid = {
        let state = shared_state.lock().await;
        state
            .task_states
            .get(&task_id)
            .map(|s| s.lingo_saw_mid_progress)
            .unwrap_or(false)
    };
    if !saw_mid {
        // DGS `showGameLoadStats` returns immediately at 0% and 100%. If the
        // download never opened a mid-file window (one-chunk / cache — a modern
        // artifact), report one now so the next getStreamStatus actually calls
        // showLoadingStatus.
        {
            let mut state = shared_state.lock().await;
            let (loaded, reported_total) = state
                .task_states
                .get(&task_id)
                .map(|s| (s.bytes_loaded, s.bytes_total))
                .unwrap_or((0, 0));
            if let Some((mid, total)) =
                synthetic_mid_file_progress(loaded, reported_total, bytes_len)
            {
                state.update_task_progress(task_id, mid, total);
            }
        }
        let _ = async_std::future::timeout(
            std::time::Duration::from_millis(50),
            async {
                loop {
                    let saw = {
                        let state = shared_state.lock().await;
                        state
                            .task_states
                            .get(&task_id)
                            .map(|s| s.lingo_saw_mid_progress)
                            .unwrap_or(false)
                    };
                    if saw {
                        return;
                    }
                    let _ = async_std::future::timeout(
                        std::time::Duration::from_millis(5),
                        std::future::pending::<()>(),
                    )
                    .await;
                }
            },
        )
        .await;
    }
    {
        let mut state = shared_state.lock().await;
        state.update_task_progress(task_id, bytes_len, bytes_len.max(1));
    }
}
