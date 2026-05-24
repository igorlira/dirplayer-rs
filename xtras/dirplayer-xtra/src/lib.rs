//! SDK for writing external dirplayer Xtra plugins.
//!
//! # Quick-start
//!
//! ```rust,ignore
//! use dirplayer_xtra::{Datum, XtraPlugin};
//! use dirplayer_xtra_macros::{xtra_handlers, xtra_plugin};
//!
//! #[xtra_plugin("EchoXtra")]
//! struct EchoPlugin;
//!
//! impl XtraPlugin for EchoPlugin {
//!     type Instance = EchoInstance;
//!     fn create(&mut self, _args: &[Datum]) -> Result<EchoInstance, String> {
//!         Ok(EchoInstance)
//!     }
//! }
//!
//! pub struct EchoInstance;
//!
//! #[xtra_handlers]
//! impl EchoInstance {
//!     fn echo(&mut self, args: &[Datum]) -> Result<Datum, String> {
//!         Ok(args.first().cloned().unwrap_or(Datum::Void))
//!     }
//! }
//! ```

pub mod datum;
pub mod host_env;

pub use datum::{args_from_json, datum_from_json, datum_to_json, Datum, XtraRefValue};
pub use dirplayer_xtra_macros::{xtra_handlers, xtra_plugin, xtra_static_handlers};

/// Trait that plugin structs must implement when using `#[xtra_plugin]`.
pub trait XtraPlugin {
    /// The per-instance state type created by `create`.
    type Instance: 'static;

    /// Create a new plugin instance.  Called by the host when Lingo executes
    /// `new(xtra("MyXtra"))`.
    fn create(&mut self, args: &[Datum]) -> Result<Self::Instance, String>;
}
