// SPDX-License-Identifier: GPL-3.0-only
//
//! Plugin-author-facing trait definitions.
//!
//! The two traits here match the WIT contract on the plugin side:
//!
//! - [`XtraPlugin`] is implemented on a zero-sized "plugin handle" struct
//!   and provides the xtra name + per-instance constructor + static handler
//!   dispatch.
//! - [`XtraInstance`] is implemented on the type used as `XtraPlugin::Instance`
//!   and provides instance-method dispatch (`call_handler`) and destruction.
//!
//! The `#[xtra_plugin]` and `#[xtra_handlers]` proc macros (Phase 2) generate
//! the C-ABI exports needed by the host. Until then, plugin authors must
//! declare the C-ABI by hand using the helpers in the `abi` module.

use alloc::string::String;

use crate::datum::Datum;

/// Result type used throughout plugin code. `Err(String)` becomes a
/// `WireFrame::Error` on the host side and turns into a Lingo
/// `ScriptError` for the calling movie.
pub type XtraResult<T> = Result<T, String>;

/// Plugin-level contract: name, lifecycle, and static (no-instance) handlers.
///
/// One implementor per WASM plugin. The host instantiates this implicitly —
/// you don't construct it yourself; the proc macros (or hand-rolled C-ABI)
/// dispatch through static methods.
pub trait XtraPlugin {
    /// The per-instance state type. Created by [`Self::create_instance`]
    /// and stored in an internal registry keyed by [`InstanceId`].
    type Instance: XtraInstance;

    /// Director-side name of this xtra (case-insensitive match). For
    /// example BobbaXtra returns `"BobbaXtra"`. The host always matches
    /// case-insensitively, so capitalization is for readability only.
    fn xtra_name() -> &'static str;

    /// Construct a new instance, optionally consuming constructor args.
    /// Called when a movie executes `new(xtra "MyXtra", ...)`.
    fn create_instance(args: &[Datum]) -> XtraResult<Self::Instance>;

    /// Returns `true` if `handler_name` (already lowercased by the host)
    /// is a static handler this plugin exposes. Default: `false`.
    fn has_static_handler(_handler_name: &str) -> bool {
        false
    }

    /// Invoke a static (no-instance) handler. Only called when
    /// [`Self::has_static_handler`] returned `true` for the same name.
    /// Default implementation returns an error — override when
    /// [`Self::has_static_handler`] can return `true`.
    fn call_static_handler(handler_name: &str, _args: &[Datum]) -> XtraResult<Datum> {
        Err(alloc::format!(
            "{}: no static handler {}",
            Self::xtra_name(),
            handler_name
        ))
    }

    /// Returns `true` if `handler_name` must be dispatched asynchronously.
    /// Async support is reserved for a later spec version — for now this
    /// always returns `false` and the SDK has no async dispatch path.
    fn has_async_handler(_handler_name: &str) -> bool {
        false
    }
}

/// Per-instance contract: handler dispatch + destruction.
///
/// Plugin authors usually let the `#[xtra_handlers]` macro derive this
/// from a normal `impl MyInstance { fn foo(&mut self, ...) -> ... }`.
/// When writing by hand, match on the lowercased `handler_name` yourself.
pub trait XtraInstance {
    /// Invoke a non-static handler on this instance. `handler_name` is
    /// already lowercased by the host. Unknown handler names should
    /// return `Err`.
    fn call_handler(&mut self, handler_name: &str, args: &[Datum]) -> XtraResult<Datum>;

    /// Optional cleanup hook. Called when the host destroys the instance
    /// (movie calls `forget(instance)` or the player resets). Default is
    /// a no-op; override to release external resources (open handles,
    /// timers, etc.).
    fn destroy(&mut self) {}
}
