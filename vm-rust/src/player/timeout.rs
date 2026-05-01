use std::collections::HashMap;

use crate::{director::lingo::datum::TimeoutRef, js_api::JsApi};

use super::DatumRef;

pub struct TimeoutManager {
    pub timeouts: HashMap<TimeoutRef, Timeout>,
}

pub struct Timeout {
    pub name: TimeoutRef,
    pub period: u32,
    pub handler: String,
    pub target_ref: DatumRef,
    pub is_scheduled: bool,
    /// Wall-clock timestamp (ms) when this timeout should next fire.
    pub next_fire_ms: f64,
}

impl TimeoutManager {
    pub fn new() -> TimeoutManager {
        TimeoutManager {
            timeouts: HashMap::new(),
        }
    }

    pub fn add_timeout(&mut self, timeout: Timeout) {
        // Fully cancel the old timeout if one exists with the same name —
        // both flag it as un-scheduled AND dispatch clear_timeout to the JS
        // side so the underlying setInterval stops. Previously only the
        // is_scheduled flag was flipped, leaking the JS interval. Each
        // re-add without an explicit forget()/clear stacked another active
        // setInterval for the same name; CS's CD-request window hits this
        // because Lingo's `timeout("CDplayer").forget()` (capital CD) doesn't
        // match the parser-stored `"cdplayer"` (lowercase) and the close
        // doesn't clean up — every reopen accumulates another parallel
        // interval, making the countdown appear N× faster after N opens.
        if let Some(old) = self.timeouts.get_mut(&timeout.name) {
            old.cancel();
        }
        self.timeouts.insert(timeout.name.to_owned(), timeout);
    }

    #[allow(dead_code)]
    pub fn forget_timeout(&mut self, timeout_name: &TimeoutRef) {
        let timeout = &mut self.timeouts.remove(timeout_name);
        if let Some(timeout) = timeout {
            timeout.cancel();
        }
    }

    #[allow(dead_code)]
    pub fn get_timeout(&self, timeout_name: &TimeoutRef) -> Option<&Timeout> {
        self.timeouts.get(timeout_name)
    }

    pub fn get_timeout_mut(&mut self, timeout_name: &TimeoutRef) -> Option<&mut Timeout> {
        self.timeouts.get_mut(timeout_name)
    }

    pub fn clear(&mut self) {
        for (_, timeout) in self.timeouts.iter_mut() {
            timeout.cancel();
        }
        self.timeouts.clear();
    }
}

impl Timeout {
    pub fn cancel(&mut self) {
        if self.is_scheduled {
            JsApi::dispatch_clear_timeout(&self.name);
            self.is_scheduled = false;
        }
    }

    pub fn schedule(&mut self) {
        self.cancel();

        let timeout_name = self.name.to_owned();
        JsApi::dispatch_schedule_timeout(&timeout_name, self.period);
        self.is_scheduled = true;
        self.next_fire_ms = crate::player::testing_shared::now_ms() + self.period as f64;
    }
}
