use fxhash::FxHashMap;
use log::{debug, warn};

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, ScriptError},
};

/// localStorage-backed persistence for the virtual filesystem, so files a
/// movie writes (settings, logs, e.g. a measured movie's settings file with its
/// language choice) survive a page reload. Keys are the lowercased BASENAME
/// so "C:\dir\settings.txt", "http://host/dir/settings.txt" and "settings.txt"
/// all refer to the same stored file. Values are base64 (bytes are CP1252,
/// not valid UTF-16 storage material). All failures are silently ignored —
/// persistence is best-effort and must never take the movie down.
/// The save games this browser holds, newest first.
///
/// A Director projector puts up the OS file dialog here. A browser cannot,
/// and blocking dialogs are the wrong shape for a web page anyway, so the
/// two dialog handlers below answer from the persistence layer instead:
/// saving reuses the name the movie suggests (the player's own name), and
/// loading picks the most recent save. That makes "Gem spil" and "Hent spil"
/// work end to end, with a save per player name.
fn list_persisted_saves() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let count = storage.length().unwrap_or(0);
        for i in 0..count {
            if let Ok(Some(key)) = storage.key(i) {
                if let Some(base) = key.strip_prefix("dirplayer.fileio.") {
                    if base.ends_with(".mim") {
                        names.push(base.to_string());
                    }
                }
            }
        }
    }
    // Newest first: the save index below records the order files were written.
    let order = save_order();
    names.sort_by_key(|n| std::cmp::Reverse(order.iter().position(|o| o == n).map(|p| order.len() - p).unwrap_or(0)));
    names
}

/// Write order for saved games, so "load" can offer the newest one.
/// Kept as its own small list next to the files themselves.
fn save_order() -> Vec<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("dirplayer.fileio.saveorder").ok().flatten())
        .map(|v| v.split(char::from_u32(10).unwrap()).filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default()
}

fn record_save_order(file_name: &str) {
    let base = file_name
        .rsplit(|c| c == char::from_u32(92).unwrap() || c == '/')
        .next()
        .unwrap_or(file_name)
        .to_lowercase();
    if !base.ends_with(".mim") {
        return;
    }
    let mut order = save_order();
    order.retain(|o| o != &base);
    order.push(base);
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let joined = order.join(&String::from(char::from_u32(10).unwrap()));
        let _ = storage.set_item("dirplayer.fileio.saveorder", &joined);
    }
}

fn persist_storage_key(file_name: &str) -> String {
    let base = file_name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(file_name)
        .to_lowercase();
    format!("dirplayer.fileio.{}", base)
}

fn persist_file(file_name: &str, data: &[u8]) {
    use base64::Engine;
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let _ = storage.set_item(&persist_storage_key(file_name), &encoded);
    }
    // Saved games get their write order recorded so the load handler can
    // offer the newest one in place of a file dialog.
    record_save_order(file_name);
}

fn load_persisted_file(file_name: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten())?;
    let encoded = storage.get_item(&persist_storage_key(file_name)).ok()??;
    base64::engine::general_purpose::STANDARD.decode(encoded).ok()
}

fn remove_persisted_file(file_name: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.remove_item(&persist_storage_key(file_name));
    }
}

/// Blocking fetch for the SYNC openFile path (Lingo command syntax
/// `openFile(fileObj, name, mode)` dispatches through the sync handler
/// chain, which cannot await). Sync XHR is deprecated but supported on the
/// main thread, and only runs for the rare bootstrap read of a
/// server-shipped file that is in neither the VFS nor localStorage.
/// The x-user-defined charset trick keeps the transfer byte-exact.
fn sync_fetch_bytes(relative_name: &str) -> Option<Vec<u8>> {
    let url: Option<String> = reserve_player_mut(|player| {
        if relative_name.contains("://") {
            return Some(relative_name.to_string());
        }
        player
            .net_manager
            .base_path
            .as_ref()
            .and_then(|b| b.join(relative_name).ok())
            .map(|u| u.to_string())
    });
    let url = url?;
    let xhr = web_sys::XmlHttpRequest::new().ok()?;
    xhr.open_with_async("GET", &url, false).ok()?;
    xhr.override_mime_type("text/plain; charset=x-user-defined").ok()?;
    xhr.send().ok()?;
    if xhr.status().ok()? != 200 {
        return None;
    }
    let text = xhr.response_text().ok()??;
    Some(text.chars().map(|c| (c as u32 & 0xFF) as u8).collect())
}

/// Resolve a file path from the Lingo world (which may use movie_path_override)
/// to a relative filename that can be fetched from the real net_manager.base_path.
/// Returns (resolved_relative_name, real_base_url) or None if no override applies.
fn resolve_override_path(file_path: &str) -> Option<(String, String)> {
    reserve_player_mut(|player| {
        let override_base = &player.movie.base_path;
        let real_base = player.net_manager.base_path.as_ref().map(|u| u.to_string());

        if override_base.is_empty() || real_base.is_none() {
            return None;
        }
        let real_base = real_base.unwrap();

        // Normalize both paths for comparison (backslash → forward slash, case-insensitive on Windows)
        let norm_file = file_path.replace('\\', "/");
        let norm_override = override_base.replace('\\', "/");

        // Check if the file path starts with the override base path
        let norm_file_lower = norm_file.to_lowercase();
        let norm_override_lower = norm_override.to_lowercase();
        let prefix = if norm_override_lower.ends_with('/') {
            norm_override_lower.clone()
        } else {
            format!("{}/", norm_override_lower)
        };

        if norm_file_lower.starts_with(&prefix) {
            let relative = &norm_file[prefix.len()..];
            Some((relative.to_string(), real_base))
        } else {
            // Also try just the filename
            let file_name = norm_file.rsplit('/').next().unwrap_or(&norm_file);
            Some((file_name.to_string(), real_base))
        }
    })
}

/// FileIO Xtra instance — virtual in-memory file with read/write cursor.
pub struct FileIoXtraInstance {
    /// Current file name (set by fileName or openFile/createFile)
    pub file_name: String,
    /// In-memory file content
    pub data: Vec<u8>,
    /// Current read/write position
    pub position: usize,
    /// Whether the file is currently open
    pub is_open: bool,
    /// Last error code (0 = no error)
    pub last_error: i32,
    /// Filter mask for displayOpen/displaySave dialogs
    pub filter_mask: String,
    /// Newline conversion mode: 0=none, 1=platform
    pub newline_conversion: i32,
}

impl FileIoXtraInstance {
    pub fn new() -> Self {
        FileIoXtraInstance {
            file_name: String::new(),
            data: Vec::new(),
            position: 0,
            is_open: false,
            last_error: 0,
            filter_mask: String::new(),
            newline_conversion: 0,
        }
    }

    fn read_until(&mut self, delimiter: Option<u8>, skip_whitespace: bool) -> String {
        if !self.is_open || self.position >= self.data.len() {
            return String::new();
        }
        let start = if skip_whitespace {
            let mut s = self.position;
            while s < self.data.len() && (self.data[s] == b' ' || self.data[s] == b'\t') {
                s += 1;
            }
            s
        } else {
            self.position
        };
        let mut end = start;
        while end < self.data.len() {
            let b = self.data[end];
            if let Some(delim) = delimiter {
                if b == delim {
                    break;
                }
            }
            // Always stop on newlines for readLine/readToken/readWord.
            // readLine passes delimiter=None and STILL must stop at the line
            // end — gating this on delimiter.is_some() made readLine return
            // the whole rest of the file, which broke every line-parsing
            // loop (e.g. a measured movie's user-info reader).
            if b == b'\r' || b == b'\n' {
                break;
            }
            end += 1;
        }
        // UTF-8 strict first, Win-1252 fallback. See io::encoding.
        let result = crate::io::encoding::decode_text_auto(&self.data[start..end]);
        // Advance past delimiter/newline
        self.position = end;
        if self.position < self.data.len() {
            let b = self.data[self.position];
            if b == b'\r' || b == b'\n' || (delimiter.is_some() && Some(b) == delimiter) {
                self.position += 1;
                // Handle \r\n pair
                if b == b'\r' && self.position < self.data.len() && self.data[self.position] == b'\n' {
                    self.position += 1;
                }
            }
        }
        result
    }
}

pub struct FileIoXtraManager {
    pub instances: FxHashMap<u32, FileIoXtraInstance>,
    pub instance_counter: u32,
    /// Simple virtual filesystem: file_name -> data
    pub virtual_fs: FxHashMap<String, Vec<u8>>,
}

impl FileIoXtraManager {
    pub fn new() -> Self {
        FileIoXtraManager {
            instances: FxHashMap::default(),
            instance_counter: 0,
            virtual_fs: FxHashMap::default(),
        }
    }

    pub fn create_instance(&mut self, _args: &Vec<DatumRef>) -> u32 {
        self.instance_counter += 1;
        self.instances
            .insert(self.instance_counter, FileIoXtraInstance::new());
        self.instance_counter
    }

    pub fn has_instance_async_handler(name: &str) -> bool {
        matches!(name.to_lowercase().as_str(), "displayopen" | "displaysave" | "openfile")
    }

    pub async fn call_instance_async_handler(
        handler_name: &str,
        instance_id: u32,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.to_lowercase().as_str() {
            "openfile" => {
                // Async openFile: fetch the file and wait for completion
                let (file_name, mode) = reserve_player_mut(|player| {
                    let name = player.get_datum(&args[0]).string_value()?;
                    let mode = if args.len() > 1 {
                        player.get_datum(&args[1]).int_value()?
                    } else {
                        1
                    };
                    Ok((name, mode))
                })?;

                let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                instance.file_name = file_name.clone();
                instance.position = 0;
                instance.last_error = 0;

                // Check virtual FS first
                if let Some(data) = manager.virtual_fs.get(&file_name) {
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.data = data.clone();
                    instance.is_open = true;
                    return reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::Void))
                    });
                }

                // Resolve path and fetch
                let fetch_result = resolve_override_path(&file_name);
                let relative_name = if let Some((rel, _)) = &fetch_result {
                    rel.clone()
                } else {
                    // Use just the filename for URLs or unresolvable paths
                    file_name.rsplit(['\\', '/']).next().unwrap_or(&file_name).to_string()
                };

                // Check virtual FS with relative name
                let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
                if let Some(data) = manager.virtual_fs.get(&relative_name) {
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.data = data.clone();
                    instance.is_open = true;
                    return reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::Void))
                    });
                }

                // Check localStorage persistence (written by an earlier
                // session's closeFile) before hitting the network — a file
                // the movie saved wins over the pristine served copy.
                if let Some(data) = load_persisted_file(&file_name) {
                    debug!(
                        "FileIO.openFile: '{}' restored from localStorage ({} bytes)",
                        file_name, data.len()
                    );
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.data = data;
                    instance.is_open = true;
                    return reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::Void))
                    });
                }

                // Fetch via net_manager and await completion
                let task_id = reserve_player_mut(|player| {
                    player.net_manager.preload_net_thing(relative_name.clone())
                });

                reserve_player_mut(|player| {
                    if !player.net_manager.is_task_done(Some(task_id)) {
                        // Need to await - drop the player lock first
                    }
                });

                // Await the fetch outside of reserve_player_mut
                {
                    let player = unsafe { crate::player::player_mut() };
                    if !player.net_manager.is_task_done(Some(task_id)) {
                        player.net_manager.await_task(task_id).await;
                    }
                    let result = player.net_manager.get_task_result(Some(task_id));
                    let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    match result {
                        Some(Ok(bytes)) => {
                            debug!(
                                "FileIO.openFile: loaded '{}' ({} bytes)",
                                relative_name, bytes.len()
                            );
                            instance.data = bytes;
                            instance.is_open = true;
                        }
                        _ => {
                            warn!(
                                "FileIO.openFile: failed to load '{}'",
                                relative_name
                            );
                            instance.data = Vec::new();
                            instance.is_open = true;
                            if mode == 1 {
                                instance.last_error = -43;
                            }
                        }
                    }
                }

                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            "displaysave" => {
                // A projector puts up the OS save dialog here. A browser
                // cannot, and a blocking dialog is the wrong shape for a web
                // page, so answer with the name the movie already suggests
                // (in the measured movie the player name plus an extension). Saving then works end to end
                // through the persistence layer, one save per player name.
                // Args are (title, defaultName): the xtra instance itself is
                // the receiver, not an argument - same convention as openFile,
                // whose file name is args[0].
                let suggested = args
                    .get(1)
                    .and_then(|a| reserve_player_mut(|player| player.get_datum(a).string_value().ok()))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "untitled".to_string());
                debug!("FileIO.displaySave(): using suggested name {}", suggested);
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(suggested))))
            }
            "displayopen" => {
                // Offer the most recent save. EMPTY reads as "cancel" to the
                // game, which is the right answer before anything is saved.
                let newest = list_persisted_saves().into_iter().next().unwrap_or_default();
                debug!("FileIO.displayOpen(): newest save = {:?}", newest);
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(newest))))
            }
            _ => Err(ScriptError::new(format!(
                "No async handler {} found for FileIO xtra instance #{}",
                handler_name, instance_id
            ))),
        }
    }

    pub fn call_instance_handler(
        handler_name: &str,
        instance_id: u32,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
        let handler = handler_name.to_lowercase();

        match handler.as_str() {
            // openfile also exists as an async handler (method-syntax dispatch
            // checks has_instance_async_handler and awaits it). Lingo COMMAND
            // syntax — `openFile(fileObj, name, mode)`, which Director-era
            // movies use heavily — routes through the sync global-handler
            // fallback (handlers/manager.rs) that cannot await, and used to
            // error out here, aborting e.g. a measured movie's
            // user-info writer before any write. The sync implementation covers
            // the same sources in the same order, with a blocking XHR standing
            // in for the async fetch.
            "openfile" => {
                let (file_name, mode) = reserve_player_mut(|player| {
                    let name = player.get_datum(&args[0]).string_value()?;
                    let mode = if args.len() > 1 {
                        player.get_datum(&args[1]).int_value()?
                    } else {
                        1
                    };
                    Ok((name, mode))
                })?;

                let instance = manager.instances.get_mut(&instance_id).unwrap();
                instance.file_name = file_name.clone();
                instance.position = 0;
                instance.last_error = 0;

                // 1) Virtual FS by the exact name the movie passed
                if let Some(data) = manager.virtual_fs.get(&file_name) {
                    let data = data.clone();
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.data = data;
                    instance.is_open = true;
                    return Ok(DatumRef::Void);
                }

                // Resolve a relative name the same way the async path does
                let fetch_result = resolve_override_path(&file_name);
                let relative_name = if let Some((rel, _)) = &fetch_result {
                    rel.clone()
                } else {
                    file_name.rsplit(['\\', '/']).next().unwrap_or(&file_name).to_string()
                };

                // 2) Virtual FS by relative name
                let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
                if let Some(data) = manager.virtual_fs.get(&relative_name) {
                    let data = data.clone();
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.data = data;
                    instance.is_open = true;
                    return Ok(DatumRef::Void);
                }

                // 3) localStorage persistence from an earlier session
                if let Some(data) = load_persisted_file(&file_name) {
                    debug!(
                        "FileIO.openFile(sync): '{}' restored from localStorage ({} bytes)",
                        file_name, data.len()
                    );
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.data = data;
                    instance.is_open = true;
                    return Ok(DatumRef::Void);
                }

                // 4) Blocking fetch of a server-shipped file (bootstrap reads)
                let fetched = sync_fetch_bytes(&relative_name);
                let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                match fetched {
                    Some(bytes) => {
                        debug!(
                            "FileIO.openFile(sync): loaded '{}' ({} bytes)",
                            relative_name, bytes.len()
                        );
                        instance.data = bytes;
                        instance.is_open = true;
                    }
                    None => {
                        warn!("FileIO.openFile(sync): '{}' not found", relative_name);
                        instance.data = Vec::new();
                        instance.is_open = true;
                        if mode == 1 {
                            instance.last_error = -43;
                        }
                    }
                }
                Ok(DatumRef::Void)
            }
            "createfile" => {
                let file_name = reserve_player_mut(|player| {
                    player.get_datum(&args[0]).string_value()
                })?;
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                instance.file_name = file_name.clone();
                instance.data = Vec::new();
                instance.position = 0;
                instance.is_open = true;
                instance.last_error = 0;

                // Native createFile leaves an EMPTY file on disk. Register it
                // in the virtual FS (and persistence) immediately, so the
                // delete() -> createFile() -> openFile() rewrite idiom (e.g.
                // a measured movie's user-info writer) reopens the empty
                // file instead of falling through to a network fetch of the
                // pristine served copy and rewriting over stale bytes.
                manager.virtual_fs.insert(file_name.clone(), Vec::new());
                persist_file(&file_name, &[]);

                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            "closefile" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                if instance.is_open && !instance.file_name.is_empty() {
                    // Persist to virtual filesystem AND localStorage (reload-proof)
                    persist_file(&instance.file_name, &instance.data);
                    manager.virtual_fs.insert(
                        instance.file_name.clone(),
                        instance.data.clone(),
                    );
                    // Re-borrow instance after virtual_fs insert
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.is_open = false;
                }
                Ok(DatumRef::Void)
            }
            "delete" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                if !instance.file_name.is_empty() {
                    manager.virtual_fs.remove(&instance.file_name.clone());
                    remove_persisted_file(&instance.file_name.clone());
                }
                Ok(DatumRef::Void)
            }

            // -- Read operations --
            "readfile" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                let result = if instance.is_open {
                    crate::io::encoding::decode_text_auto(&instance.data[instance.position..])
                } else {
                    instance.last_error = -1;
                    String::new()
                };
                instance.position = instance.data.len();
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(result)))
                })
            }
            "readline" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                let line = instance.read_until(None, false);
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(line)))
                })
            }
            "readchar" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                let ch = if instance.is_open && instance.position < instance.data.len() {
                    let c = instance.data[instance.position] as char;
                    instance.position += 1;
                    c.to_string()
                } else {
                    String::new()
                };
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(ch)))
                })
            }
            "readword" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                let word = instance.read_until(Some(b' '), true);
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(word)))
                })
            }
            "readtoken" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                // readToken reads until the next delimiter specified by args
                let (skip_str, break_str) = reserve_player_mut(|player| {
                    let s = if args.len() > 0 { player.get_datum(&args[0]).string_value().unwrap_or_default() } else { " \t".to_string() };
                    let b = if args.len() > 1 { player.get_datum(&args[1]).string_value().unwrap_or_default() } else { "\r\n".to_string() };
                    Ok((s, b))
                })?;
                // Skip leading skip chars
                while instance.position < instance.data.len() {
                    let ch = instance.data[instance.position] as char;
                    if skip_str.contains(ch) {
                        instance.position += 1;
                    } else {
                        break;
                    }
                }
                // Read until break char
                let start = instance.position;
                while instance.position < instance.data.len() {
                    let ch = instance.data[instance.position] as char;
                    if break_str.contains(ch) || skip_str.contains(ch) {
                        break;
                    }
                    instance.position += 1;
                }
                let token = crate::io::encoding::decode_text_auto(&instance.data[start..instance.position]);
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(token)))
                })
            }

            // -- Write operations --
            "writestring" => {
                let text = reserve_player_mut(|player| {
                    player.get_datum(&args[0]).string_value()
                })?;
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                if instance.is_open {
                    let bytes = text.as_bytes();
                    // Insert at position (overwrite or extend)
                    if instance.position >= instance.data.len() {
                        instance.data.extend_from_slice(bytes);
                    } else {
                        let end = (instance.position + bytes.len()).min(instance.data.len());
                        let overwrite_len = end - instance.position;
                        instance.data[instance.position..end].copy_from_slice(&bytes[..overwrite_len]);
                        if bytes.len() > overwrite_len {
                            instance.data.extend_from_slice(&bytes[overwrite_len..]);
                        }
                    }
                    instance.position += bytes.len();
                    instance.last_error = 0;
                    // Persist eagerly: not every movie bothers with closeFile,
                    // and the write must survive a reload either way.
                    persist_file(&instance.file_name, &instance.data);
                } else {
                    instance.last_error = -1;
                }
                Ok(DatumRef::Void)
            }
            "writechar" => {
                let ch = reserve_player_mut(|player| {
                    player.get_datum(&args[0]).string_value()
                })?;
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                if instance.is_open && !ch.is_empty() {
                    let byte = ch.as_bytes()[0];
                    if instance.position >= instance.data.len() {
                        instance.data.push(byte);
                    } else {
                        instance.data[instance.position] = byte;
                    }
                    instance.position += 1;
                }
                Ok(DatumRef::Void)
            }
            "writereturn" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                if instance.is_open {
                    if instance.position >= instance.data.len() {
                        instance.data.push(b'\r');
                    } else {
                        instance.data.insert(instance.position, b'\r');
                    }
                    instance.position += 1;
                }
                Ok(DatumRef::Void)
            }

            // -- Position/length --
            "getlength" => {
                let instance = manager.instances.get(&instance_id).unwrap();
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(instance.data.len() as i32)))
                })
            }
            "getposition" => {
                let instance = manager.instances.get(&instance_id).unwrap();
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(instance.position as i32)))
                })
            }
            "setposition" => {
                let pos = reserve_player_mut(|player| {
                    player.get_datum(&args[0]).int_value()
                })?;
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                instance.position = (pos as usize).min(instance.data.len());
                Ok(DatumRef::Void)
            }

            // -- Properties --
            "filename" => {
                if !args.is_empty() {
                    // setter
                    let name = reserve_player_mut(|player| {
                        player.get_datum(&args[0]).string_value()
                    })?;
                    let instance = manager.instances.get_mut(&instance_id).unwrap();
                    instance.file_name = name;
                    Ok(DatumRef::Void)
                } else {
                    // getter
                    let instance = manager.instances.get(&instance_id).unwrap();
                    let name = instance.file_name.clone();
                    reserve_player_mut(|player| {
                        Ok(player.alloc_datum(Datum::String(name)))
                    })
                }
            }
            "status" => {
                let instance = manager.instances.get(&instance_id).unwrap();
                let status = instance.last_error;
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(status)))
                })
            }
            "error" => {
                let instance = manager.instances.get(&instance_id).unwrap();
                let msg = match instance.last_error {
                    0 => "OK",
                    -43 => "File not found",
                    -1 => "File not open",
                    _ => "Unknown error",
                };
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(msg.to_string())))
                })
            }
            "version" => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String("1.5".to_string())))
                })
            }
            "setfiltermask" => {
                let mask = reserve_player_mut(|player| {
                    player.get_datum(&args[0]).string_value()
                })?;
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                instance.filter_mask = mask;
                Ok(DatumRef::Void)
            }
            "setnewlineconversion" => {
                let mode = reserve_player_mut(|player| {
                    player.get_datum(&args[0]).int_value()
                })?;
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                instance.newline_conversion = mode;
                Ok(DatumRef::Void)
            }
            "getosdirectory" => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String("/".to_string())))
                })
            }
            "getfinderinfo" | "setfinderinfo" => {
                // Finder info is Mac-specific, return empty/no-op
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(String::new())))
                })
            }

            // -- Dialog stubs (sync fallback) --
            "displaysave" => {
                // A projector puts up the OS save dialog here. A browser
                // cannot, and a blocking dialog is the wrong shape for a web
                // page, so answer with the name the movie already suggests
                // (in the measured movie the player name plus an extension). Saving then works end to end
                // through the persistence layer, one save per player name.
                // Args are (title, defaultName): the xtra instance itself is
                // the receiver, not an argument - same convention as openFile,
                // whose file name is args[0].
                let suggested = args
                    .get(1)
                    .and_then(|a| reserve_player_mut(|player| player.get_datum(a).string_value().ok()))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "untitled".to_string());
                debug!("FileIO.displaySave(): using suggested name {}", suggested);
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(suggested))))
            }
            "displayopen" => {
                // Offer the most recent save. EMPTY reads as "cancel" to the
                // game, which is the right answer before anything is saved.
                let newest = list_persisted_saves().into_iter().next().unwrap_or_default();
                debug!("FileIO.displayOpen(): newest save = {:?}", newest);
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(newest))))
            }

            // -- put interface --
            "interface" => {
                let interface_str = [
                    "-- xtra FileIO",
                    "new object me",
                    "createFile string fileName -- creates file",
                    "openFile string fileName, int mode -- opens file (1=read,2=write,0=rw)",
                    "closeFile object me -- close file",
                    "readFile object me -- read entire file",
                    "readLine object me -- read a line",
                    "readChar object me -- read one character",
                    "readWord object me -- read a word",
                    "readToken string skipChars, string breakChars -- read a token",
                    "writeString string text -- write text",
                    "writeChar string ch -- write one character",
                    "writeReturn object me -- write a carriage return",
                    "fileName object me -- get or set file name",
                    "getLength object me -- get file length",
                    "getPosition object me -- get cursor position",
                    "setPosition int pos -- set cursor position",
                    "delete object me -- delete the file",
                    "status object me -- get error code",
                    "error object me -- get error message",
                    "version object me -- get xtra version",
                    "setFilterMask string mask -- set file dialog filter",
                    "setNewlineConversion int mode -- set newline conversion",
                    "getOSDirectory -- get OS directory path",
                    "displayOpen -- show open file dialog",
                    "displaySave string title, string name -- show save file dialog",
                ].join("\n");
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(interface_str)))
                })
            }

            _ => Err(ScriptError::new(format!(
                "No handler {} found for FileIO xtra instance #{}",
                handler_name, instance_id
            ))),
        }
    }
}

pub static mut FILEIO_XTRA_MANAGER_OPT: Option<FileIoXtraManager> = None;

pub fn borrow_fileio_manager_mut<T>(
    callback: impl FnOnce(&mut FileIoXtraManager) -> T,
) -> T {
    let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
    callback(manager)
}
