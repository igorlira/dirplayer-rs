use std::{collections::HashMap, path::Path, sync::Arc};

use async_std::sync::Mutex;
use manual_future::{ManualFuture, ManualFutureCompleter};
use percent_encoding::percent_decode_str;
use url::Url;

use super::net_task::{fetch_net_task, NetResult, NetTask, NetTaskState};

pub struct NetManager {
    pub base_path: Option<Url>,
    pub tasks: HashMap<u32, NetTask>,
    pub task_states: HashMap<u32, NetTaskState>,
    pub shared_state: Arc<Mutex<NetManagerSharedState>>,
}

pub struct NetManagerSharedState {
    pub task_states: HashMap<u32, NetTaskState>,
    pub task_completers: HashMap<u32, Vec<ManualFutureCompleter<()>>>,
}

impl NetManagerSharedState {
    pub fn new() -> NetManagerSharedState {
        return NetManagerSharedState {
            task_states: HashMap::new(),
            task_completers: HashMap::new(),
        };
    }

    pub fn update_task_progress(&mut self, task_id: u32, bytes_loaded: u64, bytes_total: u64) {
        if let Some(state) = self.task_states.get_mut(&task_id) {
            state.bytes_loaded = bytes_loaded;
            state.bytes_total = bytes_total;
        }
    }

    pub async fn fulfill_task(&mut self, id: u32, result: NetResult) {
        let (bytes_loaded, bytes_total) = self.task_states.get(&id)
            .map(|s| (s.bytes_loaded, s.bytes_total))
            .unwrap_or((0, 0));
        let final_bytes = match &result {
            Ok(bytes) => bytes.len() as u64,
            Err(_) => bytes_loaded,
        };
        let new_state = NetTaskState {
            result: Some(result),
            bytes_loaded: final_bytes,
            bytes_total: if bytes_total > 0 { bytes_total } else { final_bytes },
        };
        self.task_states.insert(id, new_state);

        let completers_for_task = self.task_completers.get_mut(&id);
        if let Some(completers) = completers_for_task {
            while let Some(completer) = completers.pop() {
                completer.complete(()).await;
            }
        }
    }

    pub fn add_completer(&mut self, task_id: u32, completer: ManualFutureCompleter<()>) {
        if let Some(completers_for_task) = self.task_completers.get_mut(&task_id) {
            completers_for_task.push(completer);
        } else {
            self.task_completers.insert(task_id, Vec::from([completer]));
        }
    }

    pub fn update_task_state(&mut self, task_id: u32, state: NetTaskState) {
        self.task_states.insert(task_id, state);
    }
}

impl NetManager {
    pub fn set_base_path(&mut self, base_path: Url) {
        let sanitized_path = if !base_path.path().ends_with("/") {
            Url::parse(format!("{}/", base_path.to_string()).as_str()).unwrap()
        } else {
            base_path
        };
        self.base_path = Some(sanitized_path);
    }

    pub fn find_task_by_url(&self, url: &str) -> Option<u32> {
        // Try original URL first, then resolved URL
        find_task_with_url(&self.tasks, &url.to_string())
            .or_else(|| {
                let resolved = normalize_task_url(&url.to_string(), self.base_path.as_ref());
                find_task_with_resolved_url(&self.tasks, &resolved)
            })
            .map(|task| task.id)
    }

    pub fn get_task_state(&self, task_id: Option<u32>) -> Option<NetTaskState> {
        let shared_state = self.shared_state.try_lock().unwrap();
        let task_states = &shared_state.task_states;
        let task_id = task_id.unwrap_or(task_states.len() as u32);
        return task_states.get(&task_id).map(|x| x.clone());
    }

    pub fn is_task_done(&self, task_id: Option<u32>) -> bool {
        return self
            .get_task_state(task_id)
            .map_or(false, |x| x.result.is_some());
    }

    pub fn get_task_result(&self, task_id: Option<u32>) -> Option<NetResult> {
        return self.get_task_state(task_id).and_then(|x| x.result);
    }

    pub fn get_task(&self, task_id: u32) -> Option<&NetTask> {
        return self.tasks.get(&task_id);
    }

    pub fn get_last_task_id(&self) -> Option<u32> {
        if self.tasks.is_empty() {
            None
        } else {
            Some(self.tasks.len() as u32)
        }
    }

    pub fn create_task_future(&mut self, task_id: u32) -> ManualFuture<()> {
        let state = self.get_task_state(Some(task_id));
        if state.is_some() && state.unwrap().result.is_some() {
            return ManualFuture::new_completed(());
        } else {
            let (future, completer) = ManualFuture::<()>::new();
            {
                let mut shared_state = self.shared_state.try_lock().unwrap();
                shared_state.add_completer(task_id, completer);
            }
            return future;
        }
    }

    #[allow(dead_code)]
    pub fn add_task_completer(&mut self, task_id: u32, completer: ManualFutureCompleter<()>) {
        let mut shared_state = self.shared_state.try_lock().unwrap();
        shared_state.add_completer(task_id, completer);
    }

    pub async fn await_task(&mut self, task_id: u32) {
        let state = self.get_task_state(Some(task_id));
        if state.is_some() && state.unwrap().result.is_some() {
            return;
        } else {
            let future = self.create_task_future(task_id);
            future.await;
        }
    }

    pub fn preload_net_thing(&mut self, url: String) -> u32 {
        // Normalize the URL by decoding percent-encoded characters
        let url = percent_decode_str(&url)
            .decode_utf8()
            .map(|s| s.to_string())
            .unwrap_or(url);

        // Check if the task already exists (by original URL) and return it if found
        if let Some(existing_task) = find_task_with_url(&self.tasks, &url) {
            return existing_task.id;
        }

        // If not, construct the task outside of the borrowing scope
        let net_task = {
            let id = self.tasks.len() + 1;
            NetTask::new(
                id as u32,
                &url,
                &normalize_task_url(&url, self.base_path.as_ref()),
            )
        };

        // Also check by resolved URL to catch relative vs absolute URL duplicates
        if let Some(existing_task) = find_task_with_resolved_url(&self.tasks, &net_task.resolved_url) {
            return existing_task.id;
        }
        let task_id = net_task.id;
        let resolved_url_str = net_task.resolved_url.to_string();
        let is_file_url = resolved_url_str.starts_with("file://");

        // Set task initial state
        {
            let mut shared_shared = self.shared_state.try_lock().unwrap();
            shared_shared.update_task_state(task_id, NetTaskState { result: None, bytes_loaded: 0, bytes_total: 0 });
        }

        // Push the task
        self.tasks.insert(task_id, net_task.clone());

        // For file:// URLs, don't execute the fetch task - wait for JS to provide data
        if is_file_url {
            #[cfg(target_arch = "wasm32")]
            {
                // Emit event to request file data from Electron
                let window = web_sys::window().unwrap();
                let event_init = web_sys::CustomEventInit::new();
                let detail = js_sys::Object::new();
                js_sys::Reflect::set(&detail, &"taskId".into(), &task_id.into()).unwrap();
                js_sys::Reflect::set(&detail, &"url".into(), &resolved_url_str.into()).unwrap();
                event_init.set_detail(&detail);

                let event =
                    web_sys::CustomEvent::new_with_event_init_dict("dirplayer:netRequest", &event_init)
                        .unwrap();
                window.dispatch_event(&event).unwrap();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                // In native (test) mode, read the file directly from disk
                let file_path = resolved_url_str.strip_prefix("file://").unwrap_or(&resolved_url_str);
                let result: super::net_task::NetResult = match std::fs::read(file_path) {
                    Ok(bytes) => Ok(bytes),
                    Err(_) => Err(-1),
                };
                let shared_state_arc = Arc::clone(&self.shared_state);
                async_std::task::spawn_local(async move {
                    let mut shared_state = shared_state_arc.lock().await;
                    shared_state.fulfill_task(task_id, result).await;
                });
            }
        } else {
            // Execute normal HTTP fetch
            let shared_state_arc = Arc::clone(&self.shared_state);
            async_std::task::spawn_local(async move {
                Self::execute_task(task_id.clone(), net_task, shared_state_arc).await;
            });
        }

        task_id
    }

    async fn execute_task(
        id: u32,
        task: NetTask,
        shared_state_arc: Arc<Mutex<NetManagerSharedState>>,
    ) {
        let result = fetch_net_task(&task, Arc::clone(&shared_state_arc)).await;
        let mut shared_state = shared_state_arc.lock().await;
        shared_state.fulfill_task(id, result).await;
    }

    // pub fn get_base_path(&self) -> String {
    //   (&self.base_path.as_ref().map_or("".to_owned(), |x| x.to_string())).to_owned()
    // }

    pub fn post_net_text(&mut self, url: String, post_data: String) -> u32 {
        // For POST, we should create a new task each time

        let net_task = {
            let id = self.tasks.len() + 1;
            NetTask::new_post(
                id as u32,
                &url,
                &normalize_task_url(&url, self.base_path.as_ref()),
                post_data,
            )
        };
        let task_id = net_task.id;

        // Set task initial state
        {
            let mut shared_shared = self.shared_state.try_lock().unwrap();
            shared_shared.update_task_state(task_id, NetTaskState { result: None, bytes_loaded: 0, bytes_total: 0 });
        }

        // Push the task and execute it
        self.tasks.insert(task_id, net_task.clone());

        let shared_state_arc = Arc::clone(&self.shared_state);
        async_std::task::spawn_local(async move {
            Self::execute_task(task_id.clone(), net_task, shared_state_arc).await;
        });

        task_id
    }
}

fn normalize_task_url(url: &str, base_path: Option<&Url>) -> Url {
    let slash_norm = url.replace("\\", "/");
    let parsed_path = Path::new(slash_norm.as_str());
    let parsed_url = Url::parse(&slash_norm);

    if let Ok(parsed_url) = parsed_url {
        if parsed_url.has_host() {
            return parsed_url;
        }
    }

    if parsed_path.is_absolute() {
        return Url::parse(format!("file:///{slash_norm}").as_str()).unwrap();
    } else if let Some(base_path) = base_path {
        return base_path.join(url).unwrap();
    } else {
        return Url::parse(&slash_norm).unwrap();
    }
}

pub fn find_task_with_url<'a>(
    tasks: &'a HashMap<u32, NetTask>,
    url: &str,
) -> Option<&'a NetTask> {
    let decoded_url = percent_decode_str(url)
        .decode_utf8()
        .unwrap_or_else(|_| url.into());
    tasks
        .iter()
        .find(|(_, x)| x.url == decoded_url)
        .map(|x| x.1)
}

pub fn find_task_with_resolved_url<'a>(
    tasks: &'a HashMap<u32, NetTask>,
    resolved_url: &Url,
) -> Option<&'a NetTask> {
    tasks
        .iter()
        .find(|(_, x)| x.resolved_url == *resolved_url)
        .map(|x| x.1)
}

#[allow(dead_code)]
pub fn find_task_with_id(tasks: &HashMap<u32, NetTask>, id: u32) -> Option<&NetTask> {
    tasks.get(&id)
}
