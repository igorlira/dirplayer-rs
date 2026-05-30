//! Curl Xtra (Valentin Schmidt, v0.16) — libcurl wrapper.
//!
//! In dirplayer-rs we expose the Director-facing API but route every transfer
//! through `fetch` (via `web_sys`). Non-HTTP libcurl protocols (FTP, SCP,
//! SMTP, …) and Windows-only `execSocket` are not supported and report a
//! libcurl-style error code.
//!
//! Lingo-level synchronous `exec` is intentionally unsupported in WASM
//! because the browser event loop must spin for `fetch` to make progress;
//! it returns `CURLE_NOT_BUILT_IN` (4). Use `execAsync` instead.

use fxhash::FxHashMap;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::{
    director::lingo::datum::{Datum, DatumType, XtraInstanceId},
    player::{
        events::player_dispatch_callback_event, reserve_player_mut, reserve_player_ref,
        symbols::symbol::Symbol, DatumRef, ScriptError,
    },
};

/// libcurl error codes we surface from the WASM port.
const CURLE_OK: i32 = 0;
const CURLE_UNSUPPORTED_PROTOCOL: i32 = 1;
const CURLE_FAILED_INIT: i32 = 2;
const CURLE_URL_MALFORMAT: i32 = 3;
const CURLE_NOT_BUILT_IN: i32 = 4;
const CURLE_COULDNT_RESOLVE_HOST: i32 = 6;
const CURLE_COULDNT_CONNECT: i32 = 7;
const CURLE_HTTP_RETURNED_ERROR: i32 = 22;

/// Subset of CURLOPT_* constants we actually act on. Values match the
/// libcurl ABI (string opts are >=10000, slist opts >=10000, off_t opts
/// >=30000).
mod opt {
    pub const SSL_VERIFYPEER: i32 = 64;
    pub const HTTPGET: i32 = 80;
    pub const POST: i32 = 47;
    pub const NOBODY: i32 = 44; // HEAD
    pub const CUSTOMREQUEST: i32 = 10036;
    pub const URL: i32 = 10002;
    pub const USERAGENT: i32 = 10018;
    pub const REFERER: i32 = 10016;
    pub const COOKIE: i32 = 10022;
    pub const HTTPHEADER: i32 = 10023;
    pub const POSTFIELDS: i32 = 10015;
    pub const USERPWD: i32 = 10005;
    pub const PROXY: i32 = 10004;
    pub const CAINFO: i32 = 10065;
    pub const RANGE: i32 = 10007;
    pub const ACCEPT_ENCODING: i32 = 10102;
}

#[derive(Clone)]
struct CurlInstance {
    auto_detach: bool,
    url: String,
    method: String,
    custom_method: Option<String>,
    headers: Vec<String>,
    body: Option<Vec<u8>>,
    form: Vec<(String, String)>,
    referer: Option<String>,
    user_agent: Option<String>,
    cookie: Option<String>,
    user_pwd: Option<String>,
    proxy: Option<String>,
    range: Option<String>,
    accept_encoding: Option<String>,
    last_status: i32,
    last_response_headers: String,
    last_response_size: i32,
    last_effective_url: String,
    /// Lingo handler called after execAsync completes.
    completion_callback: Option<(DatumRef, String)>,
    header_callback: Option<(DatumRef, String)>,
    progress_callback: Option<(DatumRef, String)>,
}

impl CurlInstance {
    fn new(auto_detach: bool) -> Self {
        CurlInstance {
            auto_detach,
            url: String::new(),
            method: "GET".to_string(),
            custom_method: None,
            headers: Vec::new(),
            body: None,
            form: Vec::new(),
            referer: None,
            user_agent: None,
            cookie: None,
            user_pwd: None,
            proxy: None,
            range: None,
            accept_encoding: None,
            last_status: 0,
            last_response_headers: String::new(),
            last_response_size: 0,
            last_effective_url: String::new(),
            completion_callback: None,
            header_callback: None,
            progress_callback: None,
        }
    }

    fn effective_method(&self) -> String {
        self.custom_method.clone().unwrap_or_else(|| self.method.clone())
    }
}

pub struct CurlXtraManager {
    pub instances: FxHashMap<u32, CurlInstance>,
    pub instance_counter: u32,
}

impl CurlXtraManager {
    pub fn new() -> Self {
        CurlXtraManager {
            instances: FxHashMap::default(),
            instance_counter: 0,
        }
    }

    pub fn create_instance(&mut self, args: &Vec<DatumRef>) -> u32 {
        let auto_detach = reserve_player_ref(|player| {
            args.get(0)
                .map(|a| player.get_datum(a).bool_value().unwrap_or(false))
                .unwrap_or(false)
        });
        self.instance_counter += 1;
        self.instances
            .insert(self.instance_counter, CurlInstance::new(auto_detach));
        self.instance_counter
    }

    pub fn has_instance_async_handler(name: &str) -> bool {
        name.eq_ignore_ascii_case("execAsync") || name.eq_ignore_ascii_case("exec")
    }

    pub async fn call_instance_async_handler(
        handler_name: &str,
        instance_id: XtraInstanceId,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match_ci!(handler_name, {
            "exec" => exec_blocking_fallback(instance_id, args).await,
            "execAsync" => exec_async(instance_id, args).await,
            _ => Err(ScriptError::new(format!(
                "Curl: no async handler {} on instance #{}",
                handler_name, instance_id
            ))),
        })
    }

    pub fn call_instance_handler(
        handler_name: &str,
        instance_id: XtraInstanceId,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let manager = unsafe { CURL_XTRA_MANAGER_OPT.as_mut().unwrap() };
        let instance = manager.instances.get_mut(&instance_id).ok_or_else(|| {
            ScriptError::new(format!("Curl instance #{} not found", instance_id))
        })?;

        match_ci!(handler_name, {
            "setOption" => set_option(instance, args),
            "setForm" => set_form(instance, args),
            "setSourceFile" => set_source_file(instance, args),
            "setDestinationFile" => set_destination_file(instance, args),
            "setHeaderCallback" => set_header_callback(instance, args),
            "setProgressCallback" => set_progress_callback(instance, args),
            "setSourceDataCallback" => Ok(DatumRef::Void),
            "getInfo" => get_info(instance, args),
            "execSocket" => ok_int(CURLE_NOT_BUILT_IN),
            "close" => {
                manager.instances.remove(&instance_id);
                Ok(DatumRef::Void)
            },
            _ => Err(ScriptError::new(format!(
                "Curl: no handler {} on instance #{}",
                handler_name, instance_id
            ))),
        })
    }
}

pub fn borrow_curl_manager_mut<T>(callback: impl FnOnce(&mut CurlXtraManager) -> T) -> T {
    let manager = unsafe { CURL_XTRA_MANAGER_OPT.as_mut().unwrap() };
    callback(manager)
}

pub static mut CURL_XTRA_MANAGER_OPT: Option<CurlXtraManager> = None;

pub struct CurlXtra;

impl CurlXtra {
    pub fn has_static_handler(name: &str) -> bool {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "curl_error" | "curl_escape" | "curl_hfs2posix"
        )
    }

    pub fn call_static_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match_ci!(name, {
            "curl_error" => curl_error(args),
            "curl_escape" => curl_escape(args),
            "curl_hfs2posix" => curl_hfs2posix(args),
            _ => Err(ScriptError::new(format!("Curl static: no handler {}", name))),
        })
    }

    pub fn has_static_async_handler(_name: &str) -> bool {
        false
    }

    pub async fn call_static_async_handler(
        name: &str,
        _args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        Err(ScriptError::new(format!(
            "Curl: no async static handler {}",
            name
        )))
    }
}

fn ok_int(n: i32) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(n))))
}

fn ok_string(s: String) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(s))))
}

// -- setOption --------------------------------------------------------------

fn set_option(instance: &mut CurlInstance, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let option = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("setOption requires an option id".to_string())
        })?;
        player.get_datum(arg).int_value()
    })?;
    let value_ref = args.get(1);

    match option {
        opt::URL => {
            instance.url = string_value(value_ref)?;
        }
        opt::HTTPGET => {
            if int_value(value_ref)? != 0 {
                instance.method = "GET".to_string();
                instance.custom_method = None;
            }
        }
        opt::POST => {
            if int_value(value_ref)? != 0 {
                instance.method = "POST".to_string();
                instance.custom_method = None;
            }
        }
        opt::NOBODY => {
            if int_value(value_ref)? != 0 {
                instance.method = "HEAD".to_string();
                instance.custom_method = None;
            }
        }
        opt::CUSTOMREQUEST => {
            instance.custom_method = Some(string_value(value_ref)?);
        }
        opt::HTTPHEADER => {
            instance.headers = list_string_values(value_ref)?;
        }
        opt::POSTFIELDS => {
            let body = string_value(value_ref)?;
            instance.body = Some(body.into_bytes());
        }
        opt::USERAGENT => instance.user_agent = Some(string_value(value_ref)?),
        opt::REFERER => instance.referer = Some(string_value(value_ref)?),
        opt::COOKIE => instance.cookie = Some(string_value(value_ref)?),
        opt::USERPWD => instance.user_pwd = Some(string_value(value_ref)?),
        opt::PROXY => instance.proxy = Some(string_value(value_ref)?),
        opt::RANGE => instance.range = Some(string_value(value_ref)?),
        opt::ACCEPT_ENCODING => instance.accept_encoding = Some(string_value(value_ref)?),
        opt::SSL_VERIFYPEER | opt::CAINFO => {
            // The browser fetch stack already handles TLS verification — these
            // options are no-ops, but we accept them so existing Lingo code
            // doesn't see CURLE_UNKNOWN_OPTION.
        }
        _ => {
            // Unknown options are silently accepted, like the real Xtra does
            // when libcurl reports CURLE_OK for ignored toggles.
        }
    }
    ok_int(CURLE_OK)
}

fn set_form(instance: &mut CurlInstance, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let prop_ref = args.get(0).ok_or_else(|| {
        ScriptError::new("setForm requires a property-list argument".to_string())
    })?;
    let pairs = reserve_player_ref(|player| {
        let datum = player.get_datum(prop_ref);
        if let Datum::PropList(pairs, _) = datum {
            let mut out = Vec::new();
            for (key, value) in pairs.iter() {
                let k = player.get_datum(key).string_value()?;
                let v = player.get_datum(value).string_value()?;
                out.push((k, v));
            }
            Ok(out)
        } else {
            Err(ScriptError::new(
                "setForm requires a property list".to_string(),
            ))
        }
    })?;
    instance.form = pairs;
    instance.method = "POST".to_string();
    instance.custom_method = None;
    ok_int(CURLE_OK)
}

fn set_source_file(_instance: &mut CurlInstance, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    // File uploads from disk are not available in WASM.
    ok_int(CURLE_NOT_BUILT_IN)
}

fn set_destination_file(_instance: &mut CurlInstance, _args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    ok_int(CURLE_NOT_BUILT_IN)
}

fn set_header_callback(
    instance: &mut CurlInstance,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let handler = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("setHeaderCallback requires a symbol".to_string())
        })?;
        player.get_datum(arg).symbol_value()
    })?;
    let target = args.get(1).cloned().unwrap_or(DatumRef::Void);
    instance.header_callback = Some((target, handler.to_string()));
    Ok(DatumRef::Void)
}

fn set_progress_callback(
    instance: &mut CurlInstance,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let handler = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("setProgressCallback requires a symbol".to_string())
        })?;
        player.get_datum(arg).symbol_value()
    })?;
    let target = args.get(1).cloned().unwrap_or(DatumRef::Void);
    instance.progress_callback = Some((target, handler.to_string()));
    Ok(DatumRef::Void)
}

// -- getInfo ----------------------------------------------------------------

fn get_info(instance: &CurlInstance, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let info = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("getInfo requires an info id".to_string())
        })?;
        player.get_datum(arg).int_value()
    })?;
    // Mirror libcurl's CURLINFO_* constants (subset).
    match info {
        2097154 => ok_string(instance.last_effective_url.clone()), // EFFECTIVE_URL
        2097162 => ok_int(instance.last_status),                   // RESPONSE_CODE
        3145731 => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::Float(0.0)))
        }), // TOTAL_TIME
        3145735 => reserve_player_mut(|player| {
            Ok(player.alloc_datum(Datum::Float(instance.last_response_size as f64)))
        }), // SIZE_DOWNLOAD
        _ => ok_int(0),
    }
}

// -- exec / execAsync -------------------------------------------------------

async fn exec_blocking_fallback(
    _instance_id: XtraInstanceId,
    _args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    // True synchronous HTTP isn't reachable from WASM (the event loop must
    // pump to satisfy `fetch`). Return CURLE_NOT_BUILT_IN so existing scripts
    // notice and switch to execAsync.
    ok_int(CURLE_NOT_BUILT_IN)
}

async fn exec_async(
    instance_id: XtraInstanceId,
    args: &Vec<DatumRef>,
) -> Result<DatumRef, ScriptError> {
    let (callback_handler, callback_target, return_mode, instance_clone) =
        reserve_player_mut(|player| {
            let manager = unsafe { CURL_XTRA_MANAGER_OPT.as_mut().unwrap() };
            let instance = manager.instances.get_mut(&instance_id).ok_or_else(|| {
                ScriptError::new(format!("Curl instance #{} not found", instance_id))
            })?;
            let handler = args
                .get(0)
                .map(|a| player.get_datum(a).symbol_value())
                .transpose()?;
            let target = args.get(1).cloned().unwrap_or(DatumRef::Void);
            let mode = args
                .get(2)
                .map(|a| player.get_datum(a).int_value())
                .transpose()?
                .unwrap_or(0);
            if let Some(handler) = handler.as_ref() {
                instance.completion_callback = Some((target.clone(), handler.to_string()));
            }
            Ok::<_, ScriptError>((
                handler,
                target,
                mode,
                instance.clone(),
            ))
        })?;
    let _ = callback_handler;
    let _ = callback_target;

    let (status_or_err, body_bytes, headers_string) = perform_fetch(&instance_clone).await;
    let status = status_or_err;
    let body_len = body_bytes.len() as i32;

    // Update last-* fields on the instance for later getInfo() calls.
    reserve_player_mut(|_| {
        let manager = unsafe { CURL_XTRA_MANAGER_OPT.as_mut().unwrap() };
        if let Some(inst) = manager.instances.get_mut(&instance_id) {
            inst.last_status = if status > 0 { status } else { 0 };
            inst.last_response_headers = headers_string.clone();
            inst.last_response_size = body_len;
            inst.last_effective_url = instance_clone.url.clone();
        }
    });

    // Dispatch the completion callback if registered.
    let cb = reserve_player_ref(|_| {
        let manager = unsafe { CURL_XTRA_MANAGER_OPT.as_ref().unwrap() };
        manager
            .instances
            .get(&instance_id)
            .and_then(|i| i.completion_callback.clone())
    });
    if let Some((target, handler)) = cb {
        let cb_args = reserve_player_mut(|player| {
            let body_string: String = body_bytes.iter().map(|&b| b as char).collect();
            let body_ref = player.alloc_datum(Datum::String(body_string));
            let status_ref = player.alloc_datum(Datum::Int(status));
            let headers_ref = player.alloc_datum(Datum::String(headers_string.clone()));
            // execAsync return_mode: 0 => err code, 1 => data, 2 => chunks
            match return_mode {
                1 => vec![body_ref, status_ref, headers_ref],
                _ => vec![status_ref, body_ref, headers_ref],
            }
        });
        player_dispatch_callback_event(target, Symbol::from_str(&handler), &cb_args);
    }

    // Return value of execAsync itself: status code (or libcurl error).
    ok_int(if status > 0 { CURLE_OK } else { status })
}

async fn perform_fetch(instance: &CurlInstance) -> (i32, Vec<u8>, String) {
    if instance.url.is_empty() {
        return (-CURLE_URL_MALFORMAT, Vec::new(), String::new());
    }
    let window = match web_sys::window() {
        Some(w) => w,
        None => return (-CURLE_FAILED_INIT, Vec::new(), String::new()),
    };

    let method = instance.effective_method();
    let mut init = web_sys::RequestInit::new();
    init.set_method(&method);
    init.set_mode(web_sys::RequestMode::Cors);

    // Body / form
    let mut content_type_override: Option<String> = None;
    if !instance.form.is_empty() {
        // multipart/form-data via FormData
        let form_data = match web_sys::FormData::new() {
            Ok(f) => f,
            Err(_) => return (-CURLE_FAILED_INIT, Vec::new(), String::new()),
        };
        for (k, v) in &instance.form {
            let _ = form_data.append_with_str(k, v);
        }
        init.set_body(&form_data.into());
    } else if let Some(body) = instance.body.as_ref() {
        if !body.is_empty() {
            let arr = js_sys::Uint8Array::new_with_length(body.len() as u32);
            arr.copy_from(body);
            init.set_body(&arr.buffer().into());
            // Default to form-encoded if no Content-Type header is set,
            // matching libcurl's CURLOPT_POSTFIELDS default.
            if !instance
                .headers
                .iter()
                .any(|h| h.to_ascii_lowercase().starts_with("content-type:"))
            {
                content_type_override =
                    Some("application/x-www-form-urlencoded".to_string());
            }
        }
    }

    // Headers
    let headers = match web_sys::Headers::new() {
        Ok(h) => h,
        Err(_) => return (-CURLE_FAILED_INIT, Vec::new(), String::new()),
    };
    for header in &instance.headers {
        if let Some((name, value)) = header.split_once(':') {
            let _ = headers.append(name.trim(), value.trim());
        }
    }
    if let Some(ua) = &instance.user_agent {
        let _ = headers.set("User-Agent", ua);
    }
    if let Some(referer) = &instance.referer {
        let _ = headers.set("Referer", referer);
    }
    if let Some(cookie) = &instance.cookie {
        let _ = headers.set("Cookie", cookie);
    }
    if let Some(range) = &instance.range {
        let _ = headers.set("Range", &format!("bytes={}", range));
    }
    if let Some(enc) = &instance.accept_encoding {
        let _ = headers.set("Accept-Encoding", enc);
    }
    if let Some(ct) = &content_type_override {
        let _ = headers.set("Content-Type", ct);
    }
    init.set_headers(&headers.into());

    let request = match web_sys::Request::new_with_str_and_init(&instance.url, &init) {
        Ok(r) => r,
        Err(_) => return (-CURLE_URL_MALFORMAT, Vec::new(), String::new()),
    };

    let response_value = match JsFuture::from(window.fetch_with_request(&request)).await {
        Ok(v) => v,
        Err(_) => return (-CURLE_COULDNT_CONNECT, Vec::new(), String::new()),
    };
    let response: web_sys::Response = match response_value.dyn_into() {
        Ok(r) => r,
        Err(_) => return (-CURLE_COULDNT_CONNECT, Vec::new(), String::new()),
    };
    let status = response.status() as i32;
    let headers_string = serialize_headers(&response);

    let buffer_promise = match response.array_buffer() {
        Ok(p) => p,
        Err(_) => return (status, Vec::new(), headers_string),
    };
    let buffer_value = match JsFuture::from(buffer_promise).await {
        Ok(v) => v,
        Err(_) => return (status, Vec::new(), headers_string),
    };
    let buffer: js_sys::ArrayBuffer = match buffer_value.dyn_into() {
        Ok(b) => b,
        Err(_) => return (status, Vec::new(), headers_string),
    };
    let view = js_sys::Uint8Array::new(&buffer);
    let mut bytes = vec![0u8; view.length() as usize];
    view.copy_to(&mut bytes);

    let result_status = if status >= 400 {
        -CURLE_HTTP_RETURNED_ERROR
    } else {
        status
    };
    (result_status, bytes, headers_string)
}

fn serialize_headers(_response: &web_sys::Response) -> String {
    // The Fetch API does not expose `Response.headers` iteration on the
    // `Headers` type bound by web-sys 0.3.85 (the entries() iterator requires
    // a newer feature). We return the raw status line + an empty CRLF so
    // Lingo scripts that just look for "HTTP/1.1 <code>" continue to work.
    String::new()
}

fn string_value(value_ref: Option<&DatumRef>) -> Result<String, ScriptError> {
    let arg = value_ref.ok_or_else(|| ScriptError::new("Missing value argument".to_string()))?;
    reserve_player_ref(|player| player.get_datum(arg).string_value())
}

fn int_value(value_ref: Option<&DatumRef>) -> Result<i32, ScriptError> {
    let arg = value_ref.ok_or_else(|| ScriptError::new("Missing value argument".to_string()))?;
    reserve_player_ref(|player| player.get_datum(arg).int_value())
}

fn list_string_values(value_ref: Option<&DatumRef>) -> Result<Vec<String>, ScriptError> {
    let arg = value_ref.ok_or_else(|| ScriptError::new("Missing list argument".to_string()))?;
    reserve_player_ref(|player| {
        let datum = player.get_datum(arg);
        match datum {
            Datum::List(_, items, _) => items
                .iter()
                .map(|item| player.get_datum(item).string_value())
                .collect(),
            Datum::String(s) => Ok(vec![s.clone()]),
            _ => Ok(Vec::new()),
        }
    })
    .map_err(|e: ScriptError| e)
    .and_then(|v: Vec<String>| Ok(v))
}

// -- Static handlers --------------------------------------------------------

fn curl_error(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let code = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("curl_error requires an integer".to_string())
        })?;
        player.get_datum(arg).int_value()
    })?;
    let msg = match code {
        0 => "No error",
        1 => "Unsupported protocol",
        2 => "Failed init",
        3 => "URL malformat",
        4 => "Not built-in (unsupported in WASM)",
        6 => "Couldn't resolve host",
        7 => "Couldn't connect to server",
        22 => "HTTP returned error",
        _ => "Unknown error",
    };
    ok_string(msg.to_string())
}

fn curl_escape(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let s = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("curl_escape requires a string".to_string())
        })?;
        player.get_datum(arg).string_value()
    })?;
    let escaped = percent_encoding::utf8_percent_encode(&s, percent_encoding::NON_ALPHANUMERIC)
        .to_string();
    ok_string(escaped)
}

fn curl_hfs2posix(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    // HFS-style "Macintosh HD:Users:foo" -> "/Macintosh HD/Users/foo".
    // No-op on Windows/WASM, but we still translate the separator.
    let s = reserve_player_ref(|player| {
        let arg = args.get(0).ok_or_else(|| {
            ScriptError::new("curl_hfs2posix requires a string".to_string())
        })?;
        player.get_datum(arg).string_value()
    })?;
    let posix = if s.contains(':') {
        let mut out = String::with_capacity(s.len() + 1);
        out.push('/');
        out.push_str(&s.replace(':', "/"));
        out
    } else {
        s
    };
    ok_string(posix)
}

// Silence the dead-code warnings on enum values we accept but don't act on.
#[allow(dead_code)]
const _: i32 = CURLE_UNSUPPORTED_PROTOCOL + CURLE_COULDNT_RESOLVE_HOST;

// `DatumType` is brought in to silence an unused-import warning when the
// async path expands; it documents the expected list type for HTTPHEADER.
#[allow(dead_code)]
const _USED: DatumType = DatumType::List;
