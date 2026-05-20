//! SysMenu Xtra (v0.3, 2025 59de44955ebd) — manipulate Director's host-window
//! system menu bar.
//!
//! In a browser there is no host menu bar. Menu insert/remove/check/enable
//! operations are recorded against an in-memory model so that subsequent
//! `sysMenuRemoveItem` etc. can succeed, but nothing native is mounted. The
//! interactive handlers (`sysMenuMessageBox`, `sysMenuPrintMsg`) route to
//! `window.alert` and the developer console respectively.

use fxhash::FxHashMap;

use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, reserve_player_ref, DatumRef, ScriptError},
};

#[derive(Clone)]
struct MenuItem {
    name: String,
    id: i32,
    checked: bool,
    enabled: bool,
    sub_menu_pos: Option<i32>,
}

struct SysMenuState {
    items: Vec<MenuItem>,
    by_id: FxHashMap<i32, usize>,
    dark_mode: bool,
}

impl SysMenuState {
    fn new() -> Self {
        SysMenuState {
            items: Vec::new(),
            by_id: FxHashMap::default(),
            dark_mode: false,
        }
    }

    fn reindex(&mut self) {
        self.by_id.clear();
        for (i, item) in self.items.iter().enumerate() {
            if item.id != 0 {
                self.by_id.insert(item.id, i);
            }
        }
    }
}

static mut STATE: Option<SysMenuState> = None;

fn with_state_mut<R>(f: impl FnOnce(&mut SysMenuState) -> R) -> R {
    unsafe {
        if STATE.is_none() {
            STATE = Some(SysMenuState::new());
        }
        f(STATE.as_mut().unwrap())
    }
}

pub struct SysMenuXtra;

impl SysMenuXtra {
    pub fn has_handler(name: &str) -> bool {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "sysmenuinsertmenu"
                | "sysmenuinsertitem"
                | "sysmenuinsertseparator"
                | "sysmenucheckitem"
                | "sysmenuenableitem"
                | "sysmenuremoveitem"
                | "sysmenuusedarkmode"
                | "sysmenuprintmsg"
                | "sysmenumessagebox"
        )
    }

    pub fn call_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match_ci!(name, {
            "sysMenuInsertMenu" => insert_menu(args),
            "sysMenuInsertItem" => insert_item(args),
            "sysMenuInsertSeparator" => insert_separator(args),
            "sysMenuCheckItem" => check_item(args),
            "sysMenuEnableItem" => enable_item(args),
            "sysMenuRemoveItem" => remove_item(args),
            "sysMenuUseDarkMode" => use_dark_mode(args),
            "sysMenuPrintMsg" => print_msg(args),
            "sysMenuMessageBox" => message_box(args),
            _ => Err(ScriptError::new(format!("SysMenu: no handler {}", name))),
        })
    }
}

fn int_arg(args: &Vec<DatumRef>, idx: usize, name: &str) -> Result<i32, ScriptError> {
    let arg = args.get(idx).ok_or_else(|| {
        ScriptError::new(format!("{} requires argument at position {}", name, idx + 1))
    })?;
    reserve_player_ref(|player| player.get_datum(arg).int_value())
}

fn string_arg(args: &Vec<DatumRef>, idx: usize, name: &str) -> Result<String, ScriptError> {
    let arg = args.get(idx).ok_or_else(|| {
        ScriptError::new(format!("{} requires argument at position {}", name, idx + 1))
    })?;
    reserve_player_ref(|player| player.get_datum(arg).string_value())
}

fn optional_int(args: &Vec<DatumRef>, idx: usize) -> Option<i32> {
    args.get(idx)
        .and_then(|arg| reserve_player_ref(|player| player.get_datum(arg).int_value().ok()))
}

fn ok_int(n: i32) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Int(n))))
}

fn insert_menu(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let position = int_arg(args, 0, "sysMenuInsertMenu")?;
    let name = string_arg(args, 1, "sysMenuInsertMenu")?;
    let sub = optional_int(args, 2);
    with_state_mut(|state| {
        let item = MenuItem {
            name,
            id: 0,
            checked: false,
            enabled: true,
            sub_menu_pos: sub,
        };
        let idx = ((position - 1).max(0) as usize).min(state.items.len());
        state.items.insert(idx, item);
        state.reindex();
    });
    ok_int(1)
}

fn insert_item(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let position = int_arg(args, 0, "sysMenuInsertItem")?;
    let name = string_arg(args, 1, "sysMenuInsertItem")?;
    let id = int_arg(args, 2, "sysMenuInsertItem")?;
    let sub = optional_int(args, 3);
    with_state_mut(|state| {
        let item = MenuItem {
            name,
            id,
            checked: false,
            enabled: true,
            sub_menu_pos: sub,
        };
        let idx = ((position - 1).max(0) as usize).min(state.items.len());
        state.items.insert(idx, item);
        state.reindex();
    });
    ok_int(1)
}

fn insert_separator(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let position = int_arg(args, 0, "sysMenuInsertSeparator")?;
    let sub = optional_int(args, 1);
    with_state_mut(|state| {
        let item = MenuItem {
            name: "-".to_string(),
            id: 0,
            checked: false,
            enabled: true,
            sub_menu_pos: sub,
        };
        let idx = ((position - 1).max(0) as usize).min(state.items.len());
        state.items.insert(idx, item);
        state.reindex();
    });
    ok_int(1)
}

fn check_item(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let id = int_arg(args, 0, "sysMenuCheckItem")?;
    let checked = int_arg(args, 1, "sysMenuCheckItem")? != 0;
    let found = with_state_mut(|state| {
        if let Some(&idx) = state.by_id.get(&id) {
            state.items[idx].checked = checked;
            true
        } else {
            false
        }
    });
    ok_int(if found { 1 } else { 0 })
}

fn enable_item(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let id = int_arg(args, 0, "sysMenuEnableItem")?;
    let enabled = int_arg(args, 1, "sysMenuEnableItem")? != 0;
    let found = with_state_mut(|state| {
        if let Some(&idx) = state.by_id.get(&id) {
            state.items[idx].enabled = enabled;
            true
        } else {
            false
        }
    });
    ok_int(if found { 1 } else { 0 })
}

fn remove_item(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let id = int_arg(args, 0, "sysMenuRemoveItem")?;
    let removed = with_state_mut(|state| {
        if let Some(&idx) = state.by_id.get(&id) {
            state.items.remove(idx);
            state.reindex();
            true
        } else {
            false
        }
    });
    ok_int(if removed { 1 } else { 0 })
}

fn use_dark_mode(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let enable = optional_int(args, 0).unwrap_or(1) != 0;
    with_state_mut(|state| state.dark_mode = enable);
    ok_int(1)
}

fn print_msg(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let message = string_arg(args, 0, "sysMenuPrintMsg")?;
    let _ints: Vec<i32> = (1..args.len()).filter_map(|i| optional_int(args, i)).collect();
    web_sys::console::log_1(&format!("[SysMenu] {}", message).into());
    ok_int(1)
}

fn message_box(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let message = string_arg(args, 0, "sysMenuMessageBox")?;
    let caption = args
        .get(1)
        .map(|a| reserve_player_ref(|p| p.get_datum(a).string_value()))
        .transpose()?
        .unwrap_or_default();
    let _type = optional_int(args, 2).unwrap_or(0);
    let text = if caption.is_empty() {
        message
    } else {
        format!("{}\n\n{}", caption, message)
    };
    if let Some(window) = web_sys::window() {
        let _ = window.alert_with_message(&text);
    }
    // Windows MB_OK returns IDOK = 1.
    ok_int(1)
}
