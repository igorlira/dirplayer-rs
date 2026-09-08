// SPDX-License-Identifier: GPL-3.0-only
//
//! FileXtra4 — stub plugin for dirplayer-rs.
//!
//! A Director MX 2004 movie calls two FileXtra4 handlers:
//!   - fx_FileExists(path)          (Load.dcr)
//!   - fx_FolderGetSpecialPath(id)  (main.dcr)
//!
//! Without a registered FileXtra4, `new(xtra "FileXtra4")` raises a
//! ScriptError, which halts the whole movie (Player::on_script_error calls
//! stop()). This stub keeps the movie alive:
//!   - fx_FileExists reports "not there" so the movie falls back to creating
//!     its user/log files fresh through the built-in FileIO xtra (whose
//!     writes land in dirplayer's in-memory virtual FS).
//!   - fx_FolderGetSpecialPath hands back a fixed fake Windows folder; the
//!     game only uses it to build paths later passed to FileIO, and FileIO
//!     resolves unknown absolute paths by basename anyway.
//!   - Every UNKNOWN handler logs and returns Int(0) instead of erroring,
//!     because an Err here becomes a ScriptError -> movie stop. A wrong but
//!     harmless value beats a dead game; the log line keeps it observable.

use xtra_sdk::plugin::{XtraInstance, XtraPlugin, XtraResult};
use xtra_sdk::{host_env, Datum};

pub struct FileXtra4Plugin;

pub struct FileXtra4Instance;

/// Shared dispatch used by both instance and static calls: Lingo code calls
/// these either on an instance (`vFx.fx_FileExists(p)`) or, in some scripts,
/// as bare globals. Handler names arrive lowercased from the host.
fn dispatch(name: &str, args: &[Datum]) -> XtraResult<Datum> {
    match name {
        "fx_fileexists" | "fx_folderexists" => {
            let path = args.first().and_then(|d| d.as_str()).unwrap_or("");
            host_env::log(&format!("FileXtra4: {}({:?}) -> 0", name, path));
            Ok(Datum::Int(0))
        }
        "fx_foldergetspecialpath" => {
            // Real FileXtra4 maps a CSIDL int to a Windows folder path,
            // without a trailing backslash. Any stable string works here.
            Ok(Datum::String("C:\\Director".into()))
        }
        "fx_getosdirectory" => Ok(Datum::String("C:\\Windows".into())),
        "fx_version" => Ok(Datum::String("FileXtra4 stub 0.1 (dirplayer)".into())),
        other => {
            host_env::log(&format!(
                "FileXtra4 stub: unimplemented handler {} ({} args) -> 0",
                other,
                args.len()
            ));
            Ok(Datum::Int(0))
        }
    }
}

impl XtraPlugin for FileXtra4Plugin {
    type Instance = FileXtra4Instance;

    fn xtra_name() -> &'static str {
        "FileXtra4"
    }

    fn create_instance(_args: &[Datum]) -> XtraResult<FileXtra4Instance> {
        Ok(FileXtra4Instance)
    }

    fn has_static_handler(name: &str) -> bool {
        // Claim every fx_-prefixed global so bare calls also reach us.
        name.starts_with("fx_")
    }

    fn call_static_handler(name: &str, args: &[Datum]) -> XtraResult<Datum> {
        dispatch(name, args)
    }
}

impl XtraInstance for FileXtra4Instance {
    fn call_handler(&mut self, name: &str, args: &[Datum]) -> XtraResult<Datum> {
        dispatch(name, args)
    }

    fn destroy(&mut self) {}
}

xtra_sdk::export_plugin!(FileXtra4Plugin);
