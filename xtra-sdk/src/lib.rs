// SPDX-License-Identifier: GPL-3.0-only
//
//! SDK for authoring external Director Xtras as dynamically loaded WASM
//! plugins for dirplayer-rs.
//!
//! See `dirplayer-xtra.wit` (in this crate's root) for the canonical
//! interface contract. The WIT package name stays `dirplayer:xtra`
//! because it's the wire-level protocol identifier; the Rust crate is
//! the friendlier `xtra-sdk`.
//!
//! # Quickstart
//!
//! ```ignore
//! use xtra_sdk::{Datum, XtraPlugin, XtraInstance, XtraResult, export_plugin};
//!
//! pub struct MyPlugin;
//! pub struct MyInstance;
//!
//! impl XtraPlugin for MyPlugin {
//!     type Instance = MyInstance;
//!     fn xtra_name() -> &'static str { "MyXtra" }
//!     fn create_instance(_: &[Datum]) -> XtraResult<MyInstance> {
//!         Ok(MyInstance)
//!     }
//! }
//!
//! impl XtraInstance for MyInstance {
//!     fn call_handler(&mut self, name: &str, _: &[Datum]) -> XtraResult<Datum> {
//!         match name {
//!             "ping" => Ok(Datum::String("pong".into())),
//!             other => Err(format!("unknown handler: {}", other)),
//!         }
//!     }
//! }
//!
//! export_plugin!(MyPlugin);
//! ```
//!
//! `export_plugin!` emits the per-plugin instance registry and all eight
//! C-ABI `__xtra_*` exports plus the two `__plugin_alloc` /
//! `__plugin_dealloc` exports. Plugin authors never touch the C-ABI by hand.

#![no_std]

extern crate alloc;

pub mod abi;
pub mod datum;
pub mod host_env;
pub mod plugin;
pub mod wire;

pub use datum::{Datum, InstanceId};
pub use plugin::{XtraInstance, XtraPlugin, XtraResult};

/// The canonical WIT contract this SDK implements. Plugins written in
/// languages other than Rust should generate bindings from this string
/// (identical to `dirplayer-xtra.wit` at the crate root).
pub const WIT_CONTRACT: &str = include_str!("../dirplayer-xtra.wit");

/// Emit the C-ABI exports that turn a `XtraPlugin` impl into a host-loadable
/// WASM plugin. Invoke exactly once in your `lib.rs` after defining the
/// `XtraPlugin` impl on a unit struct.
///
/// ```ignore
/// impl XtraPlugin for MyPlugin { /* ... */ }
/// impl XtraInstance for MyInstance { /* ... */ }
/// xtra_sdk::export_plugin!(MyPlugin);
/// ```
///
/// The macro generates:
/// - A `static mut __XTRA_SDK_REGISTRY` of the plugin's instance type.
/// - `__plugin_alloc` / `__plugin_dealloc` exports (the host's allocation
///   surface for returning variable-length data into plugin memory).
/// - All eight `__xtra_*` exports matching the WIT contract.
///
/// Re-entrancy note: the registry is accessed through `static mut` because
/// WASM is single-threaded. **Do not recursively re-enter the plugin's own
/// instance handlers from inside another handler** — that would mutably
/// borrow the registry twice. Calls *out* to other xtras via
/// `host_env::call_xtra_handler` are safe (they don't touch this registry).
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        // Instance registry. WASM is single-threaded → `static mut` is sound.
        static mut __XTRA_SDK_REGISTRY: $crate::abi::InstanceRegistry<
            <$plugin as $crate::XtraPlugin>::Instance,
        > = $crate::abi::InstanceRegistry::new();

        #[inline]
        #[allow(non_snake_case)]
        fn __xtra_sdk_registry() -> &'static mut $crate::abi::InstanceRegistry<
            <$plugin as $crate::XtraPlugin>::Instance,
        > {
            unsafe { &mut *core::ptr::addr_of_mut!(__XTRA_SDK_REGISTRY) }
        }

        // ── Buffer surface used by the host to return variable-length data ──

        #[unsafe(no_mangle)]
        pub extern "C" fn __plugin_alloc(size: u32) -> u32 {
            $crate::abi::plugin_alloc(size)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn __plugin_dealloc(ptr: u32, size: u32) {
            $crate::abi::plugin_dealloc(ptr, size)
        }

        // ── The WIT `xtra-plugin` interface ────────────────────────────────

        #[unsafe(no_mangle)]
        pub extern "C" fn __xtra_name() -> u64 {
            $crate::abi::dispatch_xtra_name::<$plugin>()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn __xtra_create_instance(args_ptr: u32, args_len: u32) -> u64 {
            $crate::abi::dispatch_create_instance::<$plugin>(
                args_ptr,
                args_len,
                __xtra_sdk_registry(),
            )
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn __xtra_destroy_instance(id: u32) {
            $crate::abi::dispatch_destroy_instance::<$plugin>(
                id,
                __xtra_sdk_registry(),
            )
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn __xtra_call_handler(
            id: u32,
            name_ptr: u32,
            name_len: u32,
            args_ptr: u32,
            args_len: u32,
        ) -> u64 {
            $crate::abi::dispatch_call_handler::<$plugin>(
                id,
                name_ptr,
                name_len,
                args_ptr,
                args_len,
                __xtra_sdk_registry(),
            )
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn __xtra_has_static_handler(name_ptr: u32, name_len: u32) -> u32 {
            $crate::abi::dispatch_has_static_handler::<$plugin>(name_ptr, name_len)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn __xtra_call_static_handler(
            name_ptr: u32,
            name_len: u32,
            args_ptr: u32,
            args_len: u32,
        ) -> u64 {
            $crate::abi::dispatch_call_static_handler::<$plugin>(
                name_ptr, name_len, args_ptr, args_len,
            )
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn __xtra_has_async_handler(name_ptr: u32, name_len: u32) -> u32 {
            $crate::abi::dispatch_has_async_handler::<$plugin>(name_ptr, name_len)
        }
    };
}
