use itertools::Itertools;
use url::Url;
use wasm_bindgen::JsValue;

pub fn set_panic_hook() {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

pub fn log_i(value: &str) {
    web_sys::console::log_1(&JsValue::from_str(value))
}

#[macro_export]
macro_rules! console_warn {
  ($($arg:tt)*) => (
    web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&format_args!($($arg)*).to_string().as_str()))
  )
}

#[macro_export]
macro_rules! console_error {
  ($($arg:tt)*) => (
    web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&format_args!($($arg)*).to_string().as_str()))
  )
}

pub fn get_basename_no_extension(path: &str) -> String {
    let segments = path.split("/");
    let file_name = segments.last().unwrap_or_default();
    let dot_segments = file_name.split(".").collect_vec();
    let basename = dot_segments[0..dot_segments.len() - 1].join(".");
    return basename;
}

pub fn get_base_url(url: &Url) -> Url {
    let mut result = url.clone();
    result.set_fragment(None);

    let path_segments = url
        .path_segments()
        .map(|f| f.collect_vec())
        .unwrap_or(Vec::new());
    let has_path_segments = !path_segments.is_empty();
    if !has_path_segments || path_segments.last().unwrap().is_empty() {
        return result;
    } else {
        let path_no_filename = path_segments[0..path_segments.len() - 1].join(",");
        result.set_path(format!("{}/", path_no_filename).as_str());
        return result;
    }
}

pub const PATH_SEPARATOR: &str = "/";

pub trait ToHexString {
    fn to_hex_string(&self) -> String;
}

impl ToHexString for Vec<u8> {
    fn to_hex_string(&self) -> String {
        self.iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<String>>()
            .join(" ")
    }
}
