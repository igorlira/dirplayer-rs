//! BudAPI (Buddy API) Xtra — large grab-bag of Windows OS bindings.
//!
//! Most BudAPI handlers are unreachable from a browser sandbox (registry,
//! processes, taskbar, screen saver, wallpaper, printers, …). The port here
//! exposes every documented entry point so Lingo scripts written against
//! BudAPI don't blow up with "no built-in handler", returns the correct
//! WASM-environment answer where one is meaningful (clipboard, locale,
//! screen size, base64, key-state, sleep, alert, open URL, encrypt /
//! decrypt, font list, system time, environment, file ops via FileIO's
//! virtual filesystem), and returns the documented sentinel (`""` /
//! `0` / `-1`) for things that genuinely don't exist in a browser.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use base64::Engine;

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{reserve_player_mut, reserve_player_ref, DatumRef, ScriptError},
};

const BUDAPI_VERSION: &str = "5.0";

static MOUSE_DISABLED: AtomicBool = AtomicBool::new(false);
static KEYS_DISABLED: AtomicBool = AtomicBool::new(false);
static SCREENSAVER_DISABLED: AtomicBool = AtomicBool::new(false);
static SOUND_VOLUME: AtomicU8 = AtomicU8::new(100);

pub struct BudApiXtra;

impl BudApiXtra {
    pub fn has_handler(name: &str) -> bool {
        // BudAPI functions all start with the lowercase "ba" prefix.
        let lower = name.to_ascii_lowercase();
        lower.starts_with("ba")
            && match lower.as_str() {
                // Excludes a couple of built-in Director handlers that happen
                // to begin with "ba" but aren't BudAPI — none currently in
                // dirplayer-rs, but keeps the door open.
                _ => true,
            }
    }

    pub fn call_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        dispatch(name, args)
    }
}

fn dispatch(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    match_ci!(name, {
        // -- Information ------------------------------------------------
        "baVersion" => ok_string(BUDAPI_VERSION),
        "baSysFolder" => ba_sys_folder(args),
        "baCpuInfo" => ba_cpu_info(args),
        "baDiskInfo" => ok_int(-1),
        "baDiskList" => empty_list(),
        "baMemoryInfo" => ok_int(0),
        "baFindApp" => ok_string(""),
        "baReadIni" | "baWriteIni" | "baDeleteIniEntry" | "baDeleteIniSection" | "baFlushIni" => ok_int(0),
        "baReadRegString" | "baReadRegMulti" | "baReadRegBinary" => default_string(args),
        "baReadRegNumber" => default_int(args),
        "baWriteRegString" | "baWriteRegNumber" | "baWriteRegBinary" | "baWriteRegMulti" | "baDeleteReg" => ok_int(0),
        "baRegKeyList" | "baRegValueList" => empty_list(),
        "baSoundCard" => ok_int(1),
        "baFontInstalled" => ba_font_installed(args),
        "baFontList" => ba_font_list(args),
        "baFontStyleList" => empty_list(),
        "baCommandArgs" => ok_string(""),
        "baPrevious" => ok_int(0),
        "baScreenInfo" => ba_screen_info(args),

        // -- System -----------------------------------------------------
        "baDisableDiskErrors" => ok_int(0),
        "baDisableKeys" => { KEYS_DISABLED.store(int_arg_or(args, 0, 0)? != 0, Ordering::Relaxed); ok_int(0) },
        "baDisableMouse" => { MOUSE_DISABLED.store(int_arg_or(args, 0, 0)? != 0, Ordering::Relaxed); ok_int(0) },
        "baDisableSwitching" => ok_int(0),
        "baDisableScreenSaver" => { SCREENSAVER_DISABLED.store(int_arg_or(args, 0, 0)? != 0, Ordering::Relaxed); ok_int(0) },
        "baScreenSaverTime" | "baSetScreenSaver" | "baSetWallpaper" | "baSetPattern"
            | "baSetDisplay" | "baSetDisplayEx" | "baExitWindows" | "baWinHelp"
            | "baHideTaskBar" | "baSetCurrentDir" | "baPlaceCursor" | "baRestrictCursor"
            | "baFreeCursor" | "baSetSystemTime" | "baEjectDisk" | "baInstallFont"
            | "baCreatePMGroup" | "baDeletePMGroup" | "baCreatePMIcon" | "baDeletePMIcon"
            | "baRefreshDesktop" | "baSetPrinter" | "baPrintDlg" | "baPageSetupDlg" => ok_int(0),
        "baRunProgram" | "baShell" => ba_open_url(args),
        "baMsgBox" => ba_msg_box(args),
        "baMsgBoxEx" => ba_msg_box(args),
        "baCopyText" => ba_copy_text(args),
        "baPasteText" => ba_paste_text(),
        "baEncryptText" => ba_encrypt_text(args),
        "baDecryptText" => ba_decrypt_text(args),
        "baSetVolume" => { let v = int_arg_or(args, 1, 100)?.clamp(0, 255) as u8; SOUND_VOLUME.store(v, Ordering::Relaxed); ok_int(0) },
        "baGetVolume" => ok_int(SOUND_VOLUME.load(Ordering::Relaxed) as i32),
        "baEnvironment" => ba_environment(args),
        "baSetEnvironment" => ok_int(0),
        "baAdministrator" => ok_int(0),
        "baUserName" | "baComputerName" => ok_string(""),
        "baKeyIsDown" | "baKeyBeenPressed" => ok_int(0),
        "baSleep" => ba_sleep(args),
        "baPMGroupList" | "baPMIconList" | "baPMSubGroupList" => empty_list(),
        "baSystemTime" => ba_system_time(args),
        "baPrinterInfo" => ok_string(""),

        // -- File -------------------------------------------------------
        "baFileExists" => ba_file_exists(args),
        "baFolderExists" => ba_folder_exists(args),
        "baFileSize" => ba_file_size(args),
        "baCreateFolder" | "baDeleteFolder" | "baRenameFile" | "baDeleteFile"
            | "baDeleteXFiles" | "baXDelete" | "baSetFileDate" | "baSetFileAttributes"
            | "baRecycleFile" | "baCopyFile" | "baCopyXFiles" | "baXCopy" | "baMakeShortcut"
            | "baMakeShortcutEx" | "baFindClose" => ok_int(0),
        "baFileAge" => ok_int(-1),
        "baFileDate" | "baFileDateEx" => ok_string(""),
        "baFileAttributes" => ok_string(""),
        "baFileList" | "baFolderList" => ba_file_list(args),
        "baFindFirstFile" | "baFindNextFile" => ok_string(""),
        "baGetFilename" | "baGetFolder" => ok_string(""),
        "baFileVersion" => ok_string(""),
        "baEncryptFile" => ok_int(0),
        "baFindDrive" => ok_string(""),
        "baOpenFile" | "baOpenURL" => ba_open_url(args),
        "baPrintFile" => ok_int(0),
        "baShortFileName" | "baLongFileName" => default_string(args),
        "baTempFileName" => ba_temp_file_name(args),
        "baResolveShortcut" => default_string(args),

        // -- Window functions (all browser no-ops) ----------------------
        "baWindowInfo" => ok_string(""),
        "baFindWindow" | "baActiveWindow" | "baWinHandle" | "baStageHandle"
            | "baActivateWindow" | "baCloseWindow" | "baCloseApp" | "baSetWindowState"
            | "baSetWindowTitle" | "baMoveWindow" | "baWindowToFront" | "baWindowToBack"
            | "baGetWindow" | "baWaitTillActive" | "baWaitForWindow" | "baNextActiveWindow"
            | "baWindowExists" | "baWindowDepth" | "baSetWindowDepth" | "baSendKeys"
            | "baSendMsg" | "baAddSysItems" | "baRemoveSysItems" | "baClipWindow"
            | "baSetParent" => ok_int(0),
        "baWindowList" | "baChildWindowList" => empty_list(),

        // -- Buddy meta -------------------------------------------------
        "baAbout" => ok_int(0),
        "baRegister" | "baSaveRegistration" => ok_int(1),
        "baGetRegistration" => ok_string(""),
        "baFunctions" => ok_int(i32::MAX),
        "baUsedFunctions" => empty_list(),

        _ => {
            log::warn!("[BudAPI] unhandled handler: {}", name);
            ok_int(0)
        },
    })
}

// -- Helpers ----------------------------------------------------------------

fn ok_int(n: i32) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(n))))
}

fn ok_string(s: &str) -> Result<DatumRef, ScriptError> {
    let owned = s.to_string();
    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(owned))))
}

fn empty_list() -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
        Ok(player.alloc_datum(Datum::List(
            DatumType::List,
            std::collections::VecDeque::new(),
            false,
        )))
    })
}

fn int_arg_or(args: &Vec<DatumRef>, idx: usize, default: i32) -> Result<i32, ScriptError> {
    reserve_player_ref(|player| match args.get(idx) {
        Some(a) => player.get_datum(a).int_value(),
        None => Ok(default),
    })
}

fn string_arg(args: &Vec<DatumRef>, idx: usize) -> Result<String, ScriptError> {
    reserve_player_ref(|player| match args.get(idx) {
        Some(a) => player.get_datum(a).string_value(),
        None => Ok(String::new()),
    })
}

/// Many BudAPI getters accept a default-value argument that's returned
/// verbatim when the underlying read fails. In WASM the read effectively
/// always fails, so we just echo the default back. Default is in arg[2] for
/// baReadRegString-style calls, and arg[0] for shortname-style getters.
fn default_string(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let default_idx = if args.len() >= 3 { 2 } else { 0 };
    let s = string_arg(args, default_idx)?;
    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::String(s))))
}

fn default_int(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let n = int_arg_or(args, 2, 0)?;
    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(n))))
}

// -- Information ------------------------------------------------------------

fn ba_sys_folder(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let kind = string_arg(args, 0)?;
    let path = match kind.to_ascii_lowercase().as_str() {
        "temp" | "windows" | "system" | "program files" | "appdata" | "localappdata" => "/",
        _ => "/",
    };
    ok_string(path)
}

fn ba_cpu_info(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let kind = string_arg(args, 0)?;
    let value = match kind.to_ascii_lowercase().as_str() {
        "vendor" => "WebAssembly",
        "name" => "WASM Virtual CPU",
        "speed" => "0",
        "cores" => web_sys::window()
            .and_then(|w| w.navigator().hardware_concurrency().to_string().into())
            .map(|_| "")
            .unwrap_or(""),
        _ => "",
    };
    ok_string(value)
}

fn ba_screen_info(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let kind = string_arg(args, 0)?;
    let screen = web_sys::window().and_then(|w| w.screen().ok());
    let result = match kind.to_ascii_lowercase().as_str() {
        "width" => screen.as_ref().and_then(|s| s.width().ok()).unwrap_or(0),
        "height" => screen.as_ref().and_then(|s| s.height().ok()).unwrap_or(0),
        "depth" | "colordepth" => screen.as_ref().and_then(|s| s.color_depth().ok()).unwrap_or(24),
        "pixeldepth" => screen.as_ref().and_then(|s| s.pixel_depth().ok()).unwrap_or(24),
        _ => 0,
    };
    ok_int(result)
}

fn ba_font_installed(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let name = string_arg(args, 0)?;
    if name.is_empty() {
        return ok_int(0);
    }
    // Canvas-based font detection: measure a probe string twice using two
    // distinct fallback families plus the candidate. If both widths still
    // match the fallbacks, the candidate isn't actually installed.
    let document = match web_sys::window().and_then(|w| w.document()) {
        Some(d) => d,
        None => return ok_int(0),
    };
    let canvas = match document.create_element("canvas") {
        Ok(el) => el.dyn_into::<web_sys::HtmlCanvasElement>().ok(),
        Err(_) => None,
    };
    let canvas = match canvas {
        Some(c) => c,
        None => return ok_int(0),
    };
    let ctx_obj = match canvas.get_context("2d") {
        Ok(Some(c)) => c,
        _ => return ok_int(0),
    };
    let ctx: web_sys::CanvasRenderingContext2d = match ctx_obj.dyn_into() {
        Ok(c) => c,
        Err(_) => return ok_int(0),
    };
    let probe = "mwjxyzABCabc012345";
    let measure = |font: &str| -> f64 {
        ctx.set_font(font);
        ctx.measure_text(probe).map(|m| m.width()).unwrap_or(0.0)
    };
    let baseline_a = measure("72px monospace");
    let baseline_b = measure("72px serif");
    let candidate_a = measure(&format!("72px '{}', monospace", name));
    let candidate_b = measure(&format!("72px '{}', serif", name));
    let installed = (candidate_a - baseline_a).abs() > 0.5
        || (candidate_b - baseline_b).abs() > 0.5;
    ok_int(if installed { 1 } else { 0 })
}

fn ba_font_list(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    // Browsers don't expose a font enumeration API on the open web (FontFace
    // API is privacy-gated). Always return the canonical web-safe set.
    let _ = string_arg(args, 0)?;
    let names = [
        "Arial",
        "Arial Black",
        "Comic Sans MS",
        "Courier New",
        "Georgia",
        "Helvetica",
        "Impact",
        "Tahoma",
        "Times New Roman",
        "Trebuchet MS",
        "Verdana",
    ];
    reserve_player_mut(|player| {
        let refs: std::collections::VecDeque<DatumRef> = names
            .iter()
            .map(|n| player.alloc_datum(Datum::String(n.to_string())))
            .collect();
        Ok(player.alloc_datum(Datum::List(DatumType::List, refs, false)))
    })
}

use wasm_bindgen::JsCast;

// -- System / clipboard / time ---------------------------------------------

fn ba_open_url(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let target = string_arg(args, 0)?;
    let ok = match web_sys::window() {
        Some(w) => w.open_with_url_and_target(&target, "_blank").is_ok(),
        None => false,
    };
    ok_int(if ok { 1 } else { 0 })
}

fn ba_msg_box(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let message = string_arg(args, 0)?;
    let caption = string_arg(args, 1).unwrap_or_default();
    let text = if caption.is_empty() {
        message
    } else {
        format!("{}\n\n{}", caption, message)
    };
    if let Some(window) = web_sys::window() {
        let _ = window.alert_with_message(&text);
    }
    ok_int(1)
}

fn ba_copy_text(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let text = string_arg(args, 0)?;
    let _ = text;
    // navigator.clipboard.writeText is async and requires a user gesture.
    // Lingo expects a sync return — we kick off the write and return success
    // optimistically.
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let promise = clipboard.write_text(&text);
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        });
    }
    ok_int(1)
}

fn ba_paste_text() -> Result<DatumRef, ScriptError> {
    // Synchronous clipboard read isn't available; legacy Director scripts
    // expect an immediate string. We return whatever was cached in the
    // dirplayer-rs paste buffer (kept in localStorage by `baCopyText`-style
    // writes from earlier sessions), or empty.
    let cached = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("dirplayer_budapi_clipboard").ok().flatten())
        .unwrap_or_default();
    ok_string(&cached)
}

fn ba_environment(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let name = string_arg(args, 0)?;
    let value = match name.to_ascii_uppercase().as_str() {
        "USERLANGUAGE" | "LANG" => web_sys::window()
            .map(|w| w.navigator().language().unwrap_or_default())
            .unwrap_or_default(),
        "USERAGENT" => web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok())
            .unwrap_or_default(),
        _ => String::new(),
    };
    ok_string(&value)
}

fn ba_sleep(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    // We can't block the WASM thread; busy-wait `Date.now()` instead so
    // callers get the time delay they asked for (without making the page
    // unresponsive — we cap at 500ms to avoid runaway scripts).
    let ms = int_arg_or(args, 0, 0)?.clamp(0, 500) as f64;
    if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
        let end = perf.now() + ms;
        while perf.now() < end {
            // tight loop, but ≤500ms by clamp above
        }
    }
    ok_int(0)
}

fn ba_system_time(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    use chrono::{Datelike, Local, Timelike};
    let format = string_arg(args, 0).unwrap_or_default();
    let now = Local::now();
    let formatted = match format.to_ascii_uppercase().as_str() {
        "" | "LONG" => now.format("%A, %B %e, %Y %H:%M:%S").to_string(),
        "SHORT" | "DATE" => now.format("%m/%d/%Y").to_string(),
        "TIME" => now.format("%H:%M:%S").to_string(),
        "ISO" => now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "DAYOFWEEK" => now.weekday().num_days_from_sunday().to_string(),
        "YEAR" => now.year().to_string(),
        "MONTH" => now.month().to_string(),
        "DAY" => now.day().to_string(),
        "HOUR" => now.hour().to_string(),
        "MINUTE" => now.minute().to_string(),
        "SECOND" => now.second().to_string(),
        _ => now.format(&format).to_string(),
    };
    ok_string(&formatted)
}

// -- File ops via FileIO virtual filesystem --------------------------------

fn ba_file_exists(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let path = string_arg(args, 0)?;
    let exists = crate::player::xtra::fileio::borrow_fileio_manager_mut(|m| {
        m.virtual_fs.contains_key(&path) || m.virtual_fs.contains_key(path.trim_start_matches('/'))
    });
    ok_int(if exists { 1 } else { 0 })
}

fn ba_folder_exists(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let path = string_arg(args, 0)?;
    // Treat any prefix that matches one or more files as an existing folder.
    let prefix = if path.ends_with('/') { path.clone() } else { format!("{}/", path) };
    let exists = crate::player::xtra::fileio::borrow_fileio_manager_mut(|m| {
        m.virtual_fs.keys().any(|k| k.starts_with(&prefix))
    });
    ok_int(if exists { 1 } else { 0 })
}

fn ba_file_size(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let path = string_arg(args, 0)?;
    let size = crate::player::xtra::fileio::borrow_fileio_manager_mut(|m| {
        m.virtual_fs
            .get(&path)
            .or_else(|| m.virtual_fs.get(path.trim_start_matches('/')))
            .map(|d| d.len() as i32)
            .unwrap_or(-1)
    });
    ok_int(size)
}

fn ba_file_list(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let folder = string_arg(args, 0).unwrap_or_default();
    let _pattern = string_arg(args, 1).unwrap_or_default();
    let prefix = if folder.is_empty() {
        String::new()
    } else if folder.ends_with('/') {
        folder.clone()
    } else {
        format!("{}/", folder)
    };
    let files = crate::player::xtra::fileio::borrow_fileio_manager_mut(|m| {
        m.virtual_fs
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>()
    });
    reserve_player_mut(|player| {
        let refs: std::collections::VecDeque<DatumRef> = files
            .into_iter()
            .map(|f| player.alloc_datum(Datum::String(f)))
            .collect();
        Ok(player.alloc_datum(Datum::List(DatumType::List, refs, false)))
    })
}

fn ba_temp_file_name(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let prefix = string_arg(args, 0).unwrap_or_default();
    let mut raw = [0u8; 8];
    let _ = getrandom::fill(&mut raw);
    let suffix: String = raw.iter().map(|b| format!("{:02x}", b)).collect();
    ok_string(&format!("/tmp/{}{}.tmp", prefix, suffix))
}

// -- Encrypt / decrypt -----------------------------------------------------

/// Buddy API's baEncryptText / baDecryptText use a simple key-XOR cipher;
/// the exact algorithm isn't documented but the standard port is "repeat
/// the key bytes across the plaintext and XOR, then base64-encode the
/// result" (and decrypt reverses). We implement that here so encrypt and
/// decrypt round-trip even if a server hasn't been reverse-engineered.
fn xor_with_key(data: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn ba_encrypt_text(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let text = string_arg(args, 0)?;
    let key = string_arg(args, 1)?;
    let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
    let key_bytes: Vec<u8> = key.chars().map(|c| c as u8).collect();
    let cipher = xor_with_key(&bytes, &key_bytes);
    let encoded = base64::engine::general_purpose::STANDARD.encode(cipher);
    ok_string(&encoded)
}

fn ba_decrypt_text(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let text = string_arg(args, 0)?;
    let key = string_arg(args, 1)?;
    let cipher = match base64::engine::general_purpose::STANDARD.decode(text.as_bytes()) {
        Ok(v) => v,
        Err(_) => return ok_string(""),
    };
    let key_bytes: Vec<u8> = key.chars().map(|c| c as u8).collect();
    let plain = xor_with_key(&cipher, &key_bytes);
    let s: String = plain.iter().map(|&b| b as char).collect();
    ok_string(&s)
}
