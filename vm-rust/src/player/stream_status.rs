use crate::director::lingo::datum::Datum;
use crate::player::events::{player_invoke_global_event, player_wait_available};
use crate::player::net_task::StreamStatusPhase;
use crate::player::reserve_player_mut;

struct StreamEvent {
    url: String,
    state: &'static str,
    bytes_so_far: i32,
    bytes_total: i32,
    error: i32,
}

/// Dispatch streamStatus callbacks for net tasks whose state has changed.
/// Director fires: streamStatus(url, state, bytesSoFar, bytesTotal, error) where
///   state: "Connecting" | "InProgress" | "Complete" | "Error"
///   bytesSoFar: actual bytes downloaded
///   bytesTotal: from Content-Length (0 if unknown)
///   error: numeric error code (0 = no error)
///
/// Each task progresses: Connecting -> InProgress (repeated) -> Complete/Error.
pub async fn dispatch_pending_stream_status() {
    let events: Vec<StreamEvent> = reserve_player_mut(|player| {
        if !player.enable_stream_status_handler {
            return vec![];
        }
        let mut result = vec![];
        let task_ids: Vec<u32> = player.net_manager.tasks.keys().cloned().collect();
        for task_id in task_ids {
            let last_phase = player.stream_status_reported.get(&task_id).copied();
            let url = match player.net_manager.get_task(task_id) {
                Some(t) => t.url.clone(),
                None => continue,
            };
            let task_state = match player.net_manager.get_task_state(Some(task_id)) {
                Some(s) => s,
                None => continue,
            };
            let is_done = task_state.result.is_some();

            // Phase 1: Report "Connecting" if not yet reported
            if last_phase.is_none() {
                player.stream_status_reported.insert(task_id, StreamStatusPhase::Connecting);
                // If already done, skip Connecting and go straight to final
                if !is_done {
                    result.push(StreamEvent {
                        url: url.clone(),
                        state: "Connecting",
                        bytes_so_far: 0,
                        bytes_total: 0,
                        error: 0,
                    });
                }
            }

            // Phase 2: Report "InProgress" while downloading (bytes_loaded > 0 but not done)
            if !is_done && task_state.bytes_loaded > 0
                && last_phase.map_or(true, |p| p < StreamStatusPhase::Final)
            {
                player.stream_status_reported.insert(task_id, StreamStatusPhase::InProgress);
                result.push(StreamEvent {
                    url: url.clone(),
                    state: "InProgress",
                    bytes_so_far: task_state.bytes_loaded as i32,
                    bytes_total: task_state.bytes_total as i32,
                    error: 0,
                });
            }

            // Phase 3: Report "Complete" or "Error" when task is done
            if is_done && last_phase.map_or(true, |p| p < StreamStatusPhase::Final) {
                match &task_state.result {
                    Some(Ok(bytes)) => {
                        let len = bytes.len() as i32;
                        result.push(StreamEvent {
                            url,
                            state: "Complete",
                            bytes_so_far: len,
                            bytes_total: len,
                            error: 0,
                        });
                    }
                    Some(Err(error_code)) => {
                        result.push(StreamEvent {
                            url,
                            state: "Error",
                            bytes_so_far: 0,
                            bytes_total: 0,
                            error: *error_code,
                        });
                    }
                    None => {}
                }
                player.stream_status_reported.insert(task_id, StreamStatusPhase::Final);
            }
        }
        result
    });

    for event in events {
        let args = reserve_player_mut(|player| {
            let url_datum = player.alloc_datum(Datum::String(event.url));
            let state_datum = player.alloc_datum(Datum::String(event.state.to_string()));
            let bytes_so_far_datum = player.alloc_datum(Datum::Int(event.bytes_so_far));
            let bytes_total_datum = player.alloc_datum(Datum::Int(event.bytes_total));
            let error_datum = player.alloc_datum(Datum::Int(event.error));
            vec![url_datum, state_datum, bytes_so_far_datum, bytes_total_datum, error_datum]
        });
        let _ = player_invoke_global_event(&"streamStatus".to_string(), &args).await;
        player_wait_available().await;
    }
}
