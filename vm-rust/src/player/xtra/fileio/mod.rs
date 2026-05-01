use fxhash::FxHashMap;
use log::{debug, warn};

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, ScriptError},
};

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
            // Always stop on newlines for readLine/readToken/readWord
            if delimiter.is_some() && (b == b'\r' || b == b'\n') {
                break;
            }
            end += 1;
        }
        let result = String::from_utf8_lossy(&self.data[start..end]).to_string();
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
                    let player = unsafe { crate::PLAYER_OPT.as_mut().unwrap() };
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
            "displayopen" | "displaysave" => {
                debug!(
                    "FileIO.{}(): file dialogs not yet supported in WASM, returning empty",
                    handler_name
                );
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(String::new())))
                })
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
            // openfile is handled by call_instance_async_handler
            "openfile" => {
                // Should not reach here - async handler takes priority
                Err(ScriptError::new("openFile should be handled by async handler".to_string()))
            }
            "createfile" => {
                let file_name = reserve_player_mut(|player| {
                    player.get_datum(&args[0]).string_value()
                })?;
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                instance.file_name = file_name;
                instance.data = Vec::new();
                instance.position = 0;
                instance.is_open = true;
                instance.last_error = 0;

                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            "closefile" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                if instance.is_open && !instance.file_name.is_empty() {
                    // Persist to virtual filesystem
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
                }
                Ok(DatumRef::Void)
            }

            // -- Read operations --
            "readfile" => {
                let instance = manager.instances.get_mut(&instance_id).unwrap();
                let result = if instance.is_open {
                    String::from_utf8_lossy(&instance.data[instance.position..]).to_string()
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
                let token = String::from_utf8_lossy(&instance.data[start..instance.position]).to_string();
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
            "displayopen" | "displaysave" => {
                debug!(
                    "FileIO.{}(): sync fallback, returning empty",
                    handler_name
                );
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(String::new())))
                })
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
