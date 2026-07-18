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

pub type NetResult = Result<Vec<u8>, i32>;

/// Tracks the last streamStatus phase reported for a net task.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StreamStatusPhase {
    Connecting,
    InProgress,
    Final,
}

#[derive(Clone)]
pub struct NetTaskState {
    pub result: Option<NetResult>,
    /// Bytes downloaded so far (updated progressively during streaming)
    pub bytes_loaded: u64,
    /// Total bytes expected (from Content-Length header, 0 if unknown)
    pub bytes_total: u64,
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

        maybe_hold_dcr_for_preloader(&task.url).await;
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

        maybe_hold_dcr_for_preloader(&task.url).await;
        Ok(bytes)
    }
}

/// Neopets DGS shows a Flash preloader whose guest-login prompt is only
/// (re)painted while the nested game `.dcr` is still streaming — the Director
/// loader sits at load-state 34 calling the SWF's `showLoadingProcess` each
/// frame, which refills the `pre_main` field *after* the translation's `onLoad`
/// blanks it. If the `.dcr` completes too soon after that blank, the prompt
/// never repaints and there are no links to click (the movie only advanced when
/// a debugger pause stretched that window). Hold a *playing* movie's `.dcr` task
/// "in progress" for a few seconds so those extra frames happen. The task stays
/// unresolved, so `netDone` stays false AND the frame loop's net-yield keeps the
/// offscreen Ruffle instance ticking meanwhile. Only applies once the movie is
/// playing, so it never delays the initial movie load.
async fn maybe_hold_dcr_for_preloader(url: &str) {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    if !path.ends_with(".dcr") {
        return;
    }
    if !crate::player::reserve_player_ref(|p| p.is_playing) {
        return;
    }
    let _ = async_std::future::timeout(
        std::time::Duration::from_millis(3000),
        std::future::pending::<()>(),
    )
    .await;
}
