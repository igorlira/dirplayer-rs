use itertools::Itertools;
use js_sys::Uint8Array;
use url::Url;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use percent_encoding::percent_decode_str;

use crate::utils::log_i;

pub type NetResult = Result<Vec<u8>, i32>;

#[derive(Clone)]
pub struct NetTaskState {
    pub result: Option<NetResult>,
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
    pub fn new<'b>(id: u32, url: &String, resolved_url: &Url) -> NetTask {
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

pub async fn fetch_net_task(task: &NetTask) -> NetResult {
    let resolved_url_str = task.resolved_url.to_string();
    log_i(
        format_args!(
            "execute_task #{} url: {} resolved: {}",
            task.id, task.url, resolved_url_str
        )
        .to_string()
        .as_str(),
    );

    // Normal HTTP(S) fetch
    // Note: file:// URLs are handled in preload_net_thing and never reach this function
    let task_result: NetResult;
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

            // Set content type for form data
            web_sys::Request::new_with_str_and_init(&url_string.as_str(), &opts).unwrap()
        }
    };

    let resp_result = JsFuture::from(window.fetch_with_request(&request)).await;
    if let Ok(resp_value) = resp_result {
        assert!(resp_value.is_instance_of::<Response>());
        let resp: Response = resp_value.dyn_into().unwrap();
        if resp.status() == 200 {
            let blob = JsFuture::from(resp.array_buffer().unwrap()).await.unwrap();
            let blob_buffer: Uint8Array = js_sys::Uint8Array::new(&blob);

            task_result = Ok(blob_buffer.to_vec().iter().map(|x| *x as u8).collect_vec());
        } else {
            task_result = Err(4); // TODO: Error code
        }
    } else {
        task_result = Err(4); // TODO: Error code
    }

    return task_result;
}
