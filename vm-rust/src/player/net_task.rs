use itertools::Itertools;
use js_sys::Uint8Array;
use url::Url;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use crate::utils::log_i;

pub type NetResult = Result<Vec<u8>, i32>;

#[derive(Clone)]
pub struct NetTaskState {
  pub result: Option<NetResult>
}

#[derive(Clone)]
pub struct NetTask {
  pub id: u32,
  pub url: String,
  pub resolved_url: Url,
}

impl NetTask {
  pub fn new<'b>(id: u32, url: &String, resolved_url: &Url) -> NetTask {
    return NetTask {
      id: id.clone().to_owned(),
      url: url.clone().to_owned(),
      resolved_url: resolved_url.clone().to_owned(),
    };
  }
}

impl NetTaskState {
  pub fn is_done(&self) -> bool {
    self.result.is_some()
  }
}

pub async fn fetch_net_task(task: &NetTask) -> NetResult {
  log_i(format_args!("execute_task #{} url: {} resolved: {}", task.id, task.url, task.resolved_url.to_string()).to_string().as_str());

  let task_result: NetResult;
  let window = web_sys::window().unwrap();
  let resp_result = JsFuture::from(window.fetch_with_str(&task.resolved_url.to_string())).await;
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
