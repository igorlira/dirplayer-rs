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
        // setInterval for the same name.
        //
        // Names are inserted *as-is* (case preserved). We deliberately do
        // not canonicalize to lowercase here — Habbo creates many timeouts
        // whose uniqueness depends on case-preserving keys (e.g. "Delay" &&
        // me.getID() && the milliSeconds, where the embedded ID may be a
        // mixed-case symbol). Lowercasing would alias unrelated entries and
        // each new add would cancel an unrelated live timer, which during
        // teardown cascades into the Object Manager / Window Manager
        // recursion and blows the scope stack.
        if let Some(old) = self.timeouts.get_mut(&timeout.name) {
            old.cancel();
        }
        self.timeouts.insert(timeout.name.to_owned(), timeout);
    }

    #[allow(dead_code)]
    pub fn forget_timeout(&mut self, timeout_name: &TimeoutRef) {
        // Exact match wins. This is the common case and matches Habbo's
        // expectation that case-distinct keys stay distinct.
        if let Some(mut timeout) = self.timeouts.remove(timeout_name) {
            timeout.cancel();
            return;
        }
        // Fallback: case-insensitive scan. CS's cdtimer creates
        // `timeout("cdplayer").new(...)` and cancels with
        // `timeout("CDplayer").forget()` (typo); without this the cancel
        // would miss, the 1-second tick keeps firing, and once
        // `pSeconds <= 0` it spams `oStudio.sendCdStop()` every second
        // through the song. Only used when no exact match exists, so it
        // doesn't accidentally cancel a same-named-different-case entry
        // that Habbo legitimately keeps live.
        let key_to_remove = self
            .timeouts
            .keys()
            .find(|k| k.eq_ignore_ascii_case(timeout_name))
            .cloned();
        if let Some(key) = key_to_remove {
            if let Some(mut timeout) = self.timeouts.remove(&key) {
                timeout.cancel();
            }
        }
    }

    #[allow(dead_code)]
    pub fn get_timeout(&self, timeout_name: &TimeoutRef) -> Option<&Timeout> {
        if let Some(t) = self.timeouts.get(timeout_name) {
            return Some(t);
        }
        // Same fallback as forget_timeout: only used when no exact match
        // exists.
        self.timeouts
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(timeout_name))
            .map(|(_, v)| v)
    }

    pub fn get_timeout_mut(&mut self, timeout_name: &TimeoutRef) -> Option<&mut Timeout> {
        // Resolve the actual key first (exact, then case-insensitive
        // fallback), then re-borrow mutably. Avoids the borrow-checker
        // issue with returning a &mut while also holding the iterator.
        let key = if self.timeouts.contains_key(timeout_name) {
            Some(timeout_name.clone())
        } else {
            self.timeouts
                .keys()
                .find(|k| k.eq_ignore_ascii_case(timeout_name))
                .cloned()
        };
        key.and_then(move |k| self.timeouts.get_mut(&k))
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
