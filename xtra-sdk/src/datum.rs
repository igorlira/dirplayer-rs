// SPDX-License-Identifier: GPL-3.0-only
//
//! `Datum` — the universal Director value type passed between host and plugin.
//!
//! This enum matches the `datum` variant in `dirplayer-xtra.wit` exactly.
//! Each variant maps one-to-one to its WIT counterpart; the wire encoding
//! (postcard via serde derive) is produced by the `wire` module.

use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// An instance handle handed out by a plugin's `create-instance` call.
/// The host uses this opaque integer to address a specific instance on
/// subsequent `call-handler` / `destroy-instance` calls.
pub type InstanceId = u32;

/// The universal Director value type, matching the `datum` WIT variant.
///
/// String and symbol payloads are UTF-8 in this representation; the host
/// converts to/from Director's Latin-1 at the WASM boundary so plugins
/// never see Latin-1 directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Datum {
    Void,
    Int(i32),
    Float(f64),
    String(String),
    Bool(bool),
    Symbol(String),
    List(Vec<Datum>),
    PropList(Vec<(Datum, Datum)>),
    /// `(locH, locV)`
    Point(f64, f64),
    /// `(left, top, right, bottom)`
    Rect(f64, f64, f64, f64),
    /// An opaque reference to an instance of another xtra:
    /// `(xtra_name, instance_id)`.
    XtraRef(String, InstanceId),
}

impl Datum {
    /// Returns `true` for `Datum::Void`. Director uses `voidp(x)` as a
    /// "is this nil?" check and this is the SDK's equivalent.
    #[inline]
    pub fn is_void(&self) -> bool {
        matches!(self, Datum::Void)
    }

    /// Convenience: try to interpret this datum as an integer. Returns
    /// `None` for incompatible variants. Symmetric to Director's loose
    /// numeric coercion is intentionally NOT performed here — plugins
    /// that want it should opt in explicitly.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Datum::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Convenience: try to interpret this datum as a string slice.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Datum::String(s) | Datum::Symbol(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Convenience: try to interpret this datum as a bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Datum::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Look up a key in a `PropList`. Director's prop-list lookup is
    /// case-insensitive when the key is a string or symbol — this helper
    /// matches that behavior. Returns `None` if `self` is not a PropList
    /// or the key is absent.
    pub fn prop_get(&self, key: &str) -> Option<&Datum> {
        match self {
            Datum::PropList(items) => {
                for (k, v) in items {
                    if let Some(k_str) = k.as_str() {
                        if k_str.eq_ignore_ascii_case(key) {
                            return Some(v);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}

