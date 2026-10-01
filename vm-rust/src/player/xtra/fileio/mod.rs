use fxhash::FxHashMap;
use log::{debug, warn};

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, ScriptError},
};

pub mod dialog;

/// localStorage-backed persistence for the virtual filesystem, so files a
/// movie writes (settings, logs, e.g. a measured movie's settings file with its
/// language choice) survive a page reload. Keys are the lowercased BASENAME
/// so "C:\dir\settings.txt", "http://host/dir/settings.txt" and "settings.txt"
/// all refer to the same stored file. Values are base64 (bytes are CP1252,
/// not valid UTF-16 storage material). All failures are silently ignored:
/// persistence is best-effort and must never take the movie down.
const PERSIST_PREFIX: &str = "dirplayer.fileio.";
/// Per file: the time of the last write and the name as the movie spelled
/// it, "<ms>\t<name>", for the file dialogs. Its own prefix, so the listing
/// of stored files never sees it.
const META_PREFIX: &str = "dirplayer.fileio-meta.";
/// The write order older builds kept, newline separated. Still read, so
/// files saved before the metadata existed sort sensibly.
const LEGACY_ORDER_KEY: &str = "dirplayer.fileio.saveorder";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

fn base_name(file_name: &str) -> &str {
    file_name.rsplit(['\\', '/']).next().unwrap_or(file_name)
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// The files the virtual filesystem holds (persisted ones and the ones only
/// in memory this session) whose names pass `patterns`, newest first.
fn list_vfs_files(patterns: &[String]) -> Vec<dialog::DialogFile> {
    let mut files: Vec<dialog::DialogFile> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let legacy_order: Vec<String> = local_storage()
        .and_then(|s| s.get_item(LEGACY_ORDER_KEY).ok().flatten())
        .map(|v| v.split('\n').filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();
    if let Some(storage) = local_storage() {
        let count = storage.length().unwrap_or(0);
        let mut keys: Vec<String> = Vec::new();
        for i in 0..count {
            if let Ok(Some(key)) = storage.key(i) {
                keys.push(key);
            }
        }
        for key in keys {
            let Some(base) = key.strip_prefix(PERSIST_PREFIX) else { continue };
            if key == LEGACY_ORDER_KEY || base.is_empty() {
                continue;
            }
            let meta = storage.get_item(&format!("{}{}", META_PREFIX, base)).ok().flatten();
            let (modified, name) = match meta.as_deref().and_then(|m| m.split_once('\t')) {
                Some((ms, name)) => (ms.parse::<f64>().unwrap_or(0.0), name.to_string()),
                None => {
                    // Older save without metadata: place it by the old write
                    // order, below everything that has a real time.
                    let pos = legacy_order.iter().position(|o| o == base).map(|p| p + 1).unwrap_or(0);
                    (pos as f64, base.to_string())
                }
            };
            if !dialog::name_matches(&name, patterns) {
                continue;
            }
            let size = storage
                .get_item(&key)
                .ok()
                .flatten()
                .map(|v| v.len() / 4 * 3)
                .unwrap_or(0);
            seen.push(base.to_string());
            files.push(dialog::DialogFile { name, modified, size });
        }
    }
    let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_ref() };
    if let Some(manager) = manager {
        for (path, data) in manager.virtual_fs.iter() {
            let name = base_name(path);
            let lower = name.to_lowercase();
            if name.is_empty() || seen.contains(&lower) || !dialog::name_matches(name, patterns) {
                continue;
            }
            seen.push(lower);
            files.push(dialog::DialogFile { name: name.to_string(), modified: 0.0, size: data.len() });
        }
    }
    files.sort_by(|a, b| b.modified.partial_cmp(&a.modified).unwrap_or(std::cmp::Ordering::Equal));
    files
}

fn record_file_meta(file_name: &str) {
    let name = base_name(file_name);
    if name.is_empty() {
        return;
    }
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(
            &format!("{}{}", META_PREFIX, name.to_lowercase()),
            &format!("{}\t{}", now_ms() as u64, name),
        );
    }
}

/// Text as a file written by Director holds it: in the Windows code page
/// (CP1252), the encoding the reads below fall back to and the one a
/// projector's files are in, so a file saved here opens in the original
/// program too. Text with characters CP1252 cannot hold stays UTF-8 rather
/// than lose them.
fn encode_file_text(text: &str) -> Vec<u8> {
    let cp1252: Option<Vec<u8>> = text
        .chars()
        .map(crate::io::encoding::char_to_win1252_byte)
        .collect();
    cp1252.unwrap_or_else(|| text.as_bytes().to_vec())
}

fn persist_storage_key(file_name: &str) -> String {
    let base = file_name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(file_name)
        .to_lowercase();
    format!("dirplayer.fileio.{}", base)
}

/// Store a file in the browser so it survives a reload, and tell the page
/// whether that worked. A failed write (storage full, or storage blocked as in
/// some private windows) used to be dropped silently: the movie carried on as
/// if the file were saved, and the player learned otherwise only on the next
/// visit. The page now hears about every attempt through a
/// `dirplayer:filePersist` event on `window`, whose `detail` is
/// `{ name, ok, size }`, plus `error` (the exception's name, or
/// "noStorage") and `bytes` (a Uint8Array copy of the file) when `ok` is
/// false, so a host can warn and offer the file as a download. A save is
/// several writes (createFile, each write, closeFile), so a failing save sends
/// several events. Returns whether the file was stored.
fn persist_file(file_name: &str, data: &[u8]) -> bool {
    use base64::Engine;
    let result: Result<(), String> =
        match web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            Some(storage) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                storage
                    .set_item(&persist_storage_key(file_name), &encoded)
                    .map_err(|e| {
                        js_sys::Reflect::get(&e, &"name".into())
                            .ok()
                            .and_then(|n| n.as_string())
                            .unwrap_or_else(|| "Error".to_string())
                    })
            }
            None => Err("noStorage".to_string()),
        };
    if result.is_ok() {
        // The file dialogs list files newest first, by the names the movie gave.
        record_file_meta(file_name);
    } else {
        log::warn!("FileIO: could not store {} ({:?})", file_name, result);
    }
    dispatch_persist_event(file_name, data, &result);
    result.is_ok()
}

fn dispatch_persist_event(file_name: &str, data: &[u8], result: &Result<(), String>) {
    let Some(window) = web_sys::window() else { return };
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &"name".into(), &base_name(file_name).into());
    let _ = js_sys::Reflect::set(&detail, &"ok".into(), &result.is_ok().into());
    let _ = js_sys::Reflect::set(&detail, &"size".into(), &(data.len() as f64).into());
    if let Err(error) = result {
        let _ = js_sys::Reflect::set(&detail, &"error".into(), &error.as_str().into());
        let bytes = js_sys::Uint8Array::from(data);
        let _ = js_sys::Reflect::set(&detail, &"bytes".into(), &bytes);
    }
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("dirplayer:filePersist", &init) {
        let _ = window.dispatch_event(&event);
    }
}

fn load_persisted_file(file_name: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten())?;
    let encoded = storage.get_item(&persist_storage_key(file_name)).ok()??;
    base64::engine::general_purpose::STANDARD.decode(encoded).ok()
}

fn remove_persisted_file(file_name: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.remove_item(&persist_storage_key(file_name));
        let _ = storage.remove_item(&format!("{}{}", META_PREFIX, base_name(file_name).to_lowercase()));
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
    /// The name the last displaySave answered with, until the movie closes
    /// a file by that name (or one with an extension added to it).
    pub pending_save: Option<String>,
}

impl FileIoXtraManager {
    pub fn new() -> Self {
        FileIoXtraManager {
            instances: FxHashMap::default(),
            instance_counter: 0,
            virtual_fs: FxHashMap::default(),
            pending_save: None,
        }
    }

    fn instance_mask(instance_id: u32) -> String {
        let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_ref().unwrap() };
        manager
            .instances
            .get(&instance_id)
            .map(|i| i.filter_mask.clone())
            .unwrap_or_default()
    }

    /// Whether a closed file is the one a displaySave answer named. Movies
    /// commonly tidy the answer up before using it, most often by adding
    /// the extension, so the stem is compared too.
    fn is_pending_save(&self, file_name: &str) -> bool {
        let Some(pending) = self.pending_save.as_deref() else { return false };
        let closed = base_name(file_name).to_lowercase();
        let pending = base_name(pending).to_lowercase();
        let stem = |n: &str| n.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| n.to_string());
        closed == pending || stem(&closed) == pending || stem(&closed) == stem(&pending)
    }

    pub fn create_instance(&mut self, _args: &Vec<DatumRef>) -> u32 {
        self.instance_counter += 1;
        self.instances
            .insert(self.instance_counter, FileIoXtraInstance::new());
        self.instance_counter
    }

    /// The dialogs wait for the player, so their command form
    /// (`displayOpen(fileObj)`) is awaited too. openFile keeps its sync
    /// command form, which covers the same sources.
    pub fn command_must_await(name: &str) -> bool {
        matches!(name.to_lowercase().as_str(), "displayopen" | "displaysave")
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
                // Args are (title, defaultName): the xtra instance itself is
                // the receiver, not an argument, same convention as openFile,
                // whose file name is args[0].
                let (title, suggested) = reserve_player_mut(|player| {
                    let title = args
                        .get(0)
                        .and_then(|a| player.get_datum(a).string_value().ok())
                        .unwrap_or_default();
                    let suggested = args
                        .get(1)
                        .and_then(|a| player.get_datum(a).string_value().ok())
                        .unwrap_or_default();
                    (title, suggested)
                });
                let mask = Self::instance_mask(instance_id);
                let answer = dialog::show_save_dialog(&title, &suggested, &mask).await;
                let chosen = match answer {
                    dialog::DialogAnswer::File { name, .. } => name,
                    dialog::DialogAnswer::Cancel => String::new(),
                };
                debug!("FileIO.displaySave(): chosen {:?}", chosen);
                // Once the movie has written and closed this file, the page
                // gets its bytes (closeFile below), to offer a copy.
                let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
                manager.pending_save = if chosen.is_empty() { None } else { Some(chosen.clone()) };
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(chosen))))
            }
            "displayopen" => {
                let mask = Self::instance_mask(instance_id);
                let files = list_vfs_files(&dialog::mask_patterns(&mask));
                let answer = dialog::show_open_dialog(&mask, &files).await;
                let chosen = match answer {
                    dialog::DialogAnswer::File { name, bytes } => {
                        if let Some(bytes) = bytes {
                            // A file from the player's computer: it becomes
                            // part of the virtual filesystem under its own
                            // name, which is what openFile will be asked for.
                            debug!("FileIO.displayOpen(): brought in {} ({} bytes)", name, bytes.len());
                            persist_file(&name, &bytes);
                            let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_mut().unwrap() };
                            manager.virtual_fs.insert(name.clone(), bytes);
                        }
                        name
                    }
                    dialog::DialogAnswer::Cancel => String::new(),
                };
                debug!("FileIO.displayOpen(): chosen {:?}", chosen);
                // EMPTY is what a cancelled dialog returns.
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(chosen))))
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
                    instance.is_open = false;
                    let file_name = instance.file_name.clone();
                    if manager.is_pending_save(&file_name) {
                        manager.pending_save = None;
                        let data = &manager.instances[&instance_id].data;
                        dialog::notify_save_written(base_name(&file_name), data);
                    }
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
                    let encoded = encode_file_text(&text);
                    let bytes = encoded.as_slice();
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
                    let byte = encode_file_text(&ch)[0];
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

            // -- Dialogs, when they cannot be awaited --
            // Both call forms normally reach the async handlers above, which
            // put up the page's dialog. What is left here answers without
            // asking: the name the movie suggests, and the newest file that
            // passes the filter mask.
            "displaysave" => {
                let suggested = args
                    .get(1)
                    .and_then(|a| reserve_player_mut(|player| player.get_datum(a).string_value().ok()))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "untitled".to_string());
                debug!("FileIO.displaySave(sync): using suggested name {}", suggested);
                reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(suggested))))
            }
            "displayopen" => {
                let mask = Self::instance_mask(instance_id);
                let newest = list_vfs_files(&dialog::mask_patterns(&mask))
                    .into_iter()
                    .next()
                    .map(|f| f.name)
                    .unwrap_or_default();
                debug!("FileIO.displayOpen(sync): newest file = {:?}", newest);
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

#[cfg(test)]
mod tests {
    use super::encode_file_text;

    #[test]
    fn file_text_is_written_in_the_windows_code_page() {
        assert_eq!(
            encode_file_text("S\u{f8}ren \u{e6}\u{e5}"),
            vec![b'S', 0xF8, b'r', b'e', b'n', b' ', 0xE6, 0xE5]
        );
        assert_eq!(encode_file_text("plain"), b"plain".to_vec());
        // Not representable in CP1252: kept as UTF-8, nothing lost.
        assert_eq!(encode_file_text("\u{3b1}"), "\u{3b1}".as_bytes().to_vec());
    }
}
