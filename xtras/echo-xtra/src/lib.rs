//! EchoXtra — example external Xtra plugin for dirplayer.
//!
//! Demonstrates the full plugin authoring flow:
//!   - Instance handlers (`echoString`, `add`, `greet`)
//!   - Static handlers   (`getVersion`, `ping`)
//!   - Host-env calls    (log, random bytes)
//!
//! # Building
//!
//! ```sh
//! cargo build --target wasm32-unknown-unknown --release
//! ```
//!
//! # Lingo usage (after loading via `loadExternalXtraFromUrl`)
//!
//! ```lingo
//! set xo = new(xtra("EchoXtra"))
//!
//! -- instance handlers
//! put xo.echoString("hello")        -- "hello"
//! put xo.add(3, 4)                  -- 7
//! put xo.greet("World")             -- "Hello, World!"
//!
//! -- static handlers
//! put EchoXtra.getVersion()          -- "1.0.0"
//! put EchoXtra.ping()                -- "pong"
//! ```

use dirplayer_xtra::{host_env, xtra_handlers, xtra_plugin, xtra_static_handlers, Datum, XtraPlugin};

// ── Plugin struct ─────────────────────────────────────────────────────────────

/// The plugin struct is the "class object" — one per xtra, shared across all
/// instances.  Instance state lives in `EchoInstance`.
#[xtra_plugin("EchoXtra")]
pub struct EchoPlugin;

impl XtraPlugin for EchoPlugin {
    type Instance = EchoInstance;

    fn create(&mut self, _args: &[Datum]) -> Result<EchoInstance, String> {
        host_env::log("EchoXtra: new instance created");
        Ok(EchoInstance {
            call_count: 0,
        })
    }
}

// ── Instance struct ───────────────────────────────────────────────────────────

pub struct EchoInstance {
    call_count: u32,
}

#[xtra_handlers]
impl EchoInstance {
    /// Return the first argument unchanged.
    pub fn echo_string(&mut self, args: &[Datum]) -> Result<Datum, String> {
        self.call_count += 1;
        Ok(args.first().cloned().unwrap_or(Datum::Void))
    }

    /// Add two integers together.
    pub fn add(&mut self, args: &[Datum]) -> Result<Datum, String> {
        self.call_count += 1;
        let a = args.get(0).and_then(|d| d.as_int()).unwrap_or(0);
        let b = args.get(1).and_then(|d| d.as_int()).unwrap_or(0);
        Ok(Datum::Int { value: a + b })
    }

    /// Return a greeting string.
    pub fn greet(&mut self, args: &[Datum]) -> Result<Datum, String> {
        self.call_count += 1;
        let name = args
            .first()
            .and_then(|d| d.as_string())
            .unwrap_or("World");
        Ok(Datum::String {
            value: format!("Hello, {}!", name),
        })
    }

    /// Return the number of handler calls made on this instance.
    pub fn get_call_count(&mut self) -> Result<Datum, String> {
        Ok(Datum::Int {
            value: self.call_count as i32,
        })
    }

    /// Return 16 random bytes encoded as a hex string.
    pub fn random_hex(&mut self) -> Result<Datum, String> {
        self.call_count += 1;
        let bytes = host_env::random_fill(16).map_err(|e| e.to_string())?;
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        Ok(Datum::String { value: hex })
    }

    /// Persist a key/value pair via the host's localStorage.
    pub fn store(&mut self, args: &[Datum]) -> Result<Datum, String> {
        self.call_count += 1;
        let key = args.get(0).and_then(|d| d.as_string()).unwrap_or("");
        let val = args.get(1).and_then(|d| d.as_string()).unwrap_or("");
        host_env::storage_set(key, val).map_err(|e| e.to_string())?;
        Ok(Datum::Void)
    }

    /// Read a key from the host's localStorage.
    pub fn load(&mut self, args: &[Datum]) -> Result<Datum, String> {
        self.call_count += 1;
        let key = args.first().and_then(|d| d.as_string()).unwrap_or("");
        match host_env::storage_get(key) {
            Some(val) => Ok(Datum::String { value: val }),
            None => Ok(Datum::Void),
        }
    }
}

// ── Static handlers ───────────────────────────────────────────────────────────

#[xtra_static_handlers]
impl EchoPlugin {
    /// Return the plugin version string.
    pub fn get_version(&mut self) -> Result<Datum, String> {
        Ok(Datum::String {
            value: "1.0.0".to_string(),
        })
    }

    /// Liveness check — always returns the symbol `#pong`.
    pub fn ping(&mut self) -> Result<Datum, String> {
        Ok(Datum::Symbol {
            value: "pong".to_string(),
        })
    }
}
