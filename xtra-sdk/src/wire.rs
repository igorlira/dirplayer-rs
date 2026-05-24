// SPDX-License-Identifier: GPL-3.0-only
//
//! Wire format for `Datum` values crossing the host↔plugin WASM boundary.
//!
//! Today the encoding is postcard (a serde-derived compact binary format).
//! Plugins and host both go through these helpers — never through `postcard`
//! directly — so the encoding is a swappable implementation detail.
//!
//! ## Frame shape
//!
//! Every cross-boundary call passes a single `&[u8]` whose payload is a
//! postcard-encoded `WireFrame`. The frame carries either an `args` list
//! (request) or a single `Datum` (return value). Errors travel as their
//! own variant.

use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

use crate::datum::Datum;

/// A single wire payload. Used in both directions: requests carry `Args`,
/// returns carry `Return` or `Error`.
#[derive(Debug, Serialize, Deserialize)]
pub enum WireFrame {
    Args(Vec<Datum>),
    Return(Datum),
    Error(String),
}

/// Serialize a `Datum` slice as a `WireFrame::Args` payload.
pub fn encode_args(args: &[Datum]) -> Vec<u8> {
    let frame = WireFrame::Args(args.to_vec());
    postcard::to_allocvec(&frame).unwrap_or_default()
}

/// Serialize a single `Datum` as a `WireFrame::Return` payload.
pub fn encode_return(value: &Datum) -> Vec<u8> {
    let frame = WireFrame::Return(value.clone());
    postcard::to_allocvec(&frame).unwrap_or_default()
}

/// Serialize an error message as a `WireFrame::Error` payload.
pub fn encode_error(msg: &str) -> Vec<u8> {
    let frame = WireFrame::Error(String::from(msg));
    postcard::to_allocvec(&frame).unwrap_or_default()
}

/// Parse a wire payload received from the other side.
pub fn decode(bytes: &[u8]) -> Result<WireFrame, postcard::Error> {
    postcard::from_bytes(bytes)
}

/// Convenience: decode a payload that's expected to be an `Args` frame.
/// Returns `Err` if the frame was a `Return` or `Error` instead.
pub fn decode_args(bytes: &[u8]) -> Result<Vec<Datum>, String> {
    match decode(bytes) {
        Ok(WireFrame::Args(a)) => Ok(a),
        Ok(WireFrame::Return(_)) => Err(String::from("wire: expected Args, got Return")),
        Ok(WireFrame::Error(e)) => Err(e),
        Err(e) => Err(alloc::format!("wire decode error: {:?}", e)),
    }
}

/// Convenience: decode a payload that's expected to be a `Return` or
/// `Error` frame. Returns the inner `Datum` on success, the error message
/// on `Error`, or a wire-level error message otherwise.
pub fn decode_return(bytes: &[u8]) -> Result<Datum, String> {
    match decode(bytes) {
        Ok(WireFrame::Return(d)) => Ok(d),
        Ok(WireFrame::Args(_)) => Err(String::from("wire: expected Return, got Args")),
        Ok(WireFrame::Error(e)) => Err(e),
        Err(e) => Err(alloc::format!("wire decode error: {:?}", e)),
    }
}
