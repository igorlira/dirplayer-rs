//! Enhancer Xtra v1.6.11 — display-mode switching, cursor warping and a grab
//! bag of OS services, from `Enhancer.x32` (Mark Andrade / director-xtras.com).
//!
//! Message table, verbatim from the shipped `Enhancer Interface.txt`:
//!
//! ```text
//! new object me, *                              -- serial number
//! test_resolution object me, *, *, *            -- is the resolution available
//! set_resolution object me, *, *, *             -- set the screen resolution
//! reset_resolution object me                    -- restore the original state
//! set_auto_switching object me, integer         -- toggle switching on app switch
//! get_display_modes object me                   -- list of available display modes
//! get_current_display_mode object me            -- the current display mode
//! center_stage object me                        -- centre the stage window
//! get_joysticks / joystick_get_info / joystick_get_axes / joystick_get_buttons
//! open_joystick_control_panel / get_cpu_speed / pop_menu / get_installed_fonts
//! enable_task_switching / disable_task_switching
//! hide_desktop object me, integer r, g, b       -- cover the desktop
//! show_desktop / show_taskbar / hide_taskbar
//! move_cursor object me, integer, integer       -- warp to a stage coordinate
//! is_front_window / show_titlebar / hide_titlebar
//! prepare_browser object me                     -- ready the browser for a switch
//! directX8_installed / set_file_attributes / get_file_attributes / set_file_date
//! version / get_error
//! write_key / get_key_value / delete_key        -- registry
//! alert object me, string, string, symbol, symbol, int
//! check_virtual_key object me, string
//! forget object me
//! ```
//!
//! Both Rasterwerks PHOSPHOR builds (alpha 4 and beta2 043) call exactly twelve
//! of these, and the twelve are identical between them:
//!
//!   center_stage  enable_task_switching  forget  get_cpu_speed
//!   get_current_display_mode  move_cursor  prepare_browser  reset_resolution
//!   set_resolution  show_control_strip  show_taskbar  test_resolution
//!
//! (`show_control_strip` is a Mac-only leftover that is not in the v1.6.11 table
//! at all; C_Engine only reaches it under `the platform = "Macintosh,PowerPC"`.)
//!
//! # Why this Xtra gates fullscreen entirely
//!
//! `C_Engine.InitResolutionSwitching` opens with
//!
//! ```lingo
//! if ilk(cInput.pEX) = #instance then
//!   pResolutionSwitchingEnabled = 1
//!   pDisplayStart = cInput.pEX.get_current_display_mode()
//!   pDisplayDepth = pDisplayStart[3]
//!   TestResolutions(me)
//! else
//!   pResolutionSwitchingEnabled = 0
//! end if
//! ```
//!
//! and every one of `UpdateResolution`, `SwitchToFullScreen` and
//! `SwitchToWindowed` opens with `if not pResolutionSwitchingEnabled then exit`.
//! Without an Enhancer instance `C_Input.InitEnhancer` falls through to the
//! MoveCursor Xtra — which restores mouselook, but leaves `pEX = 0`, so the
//! whole resolution path is dead and picking "fullscreen" in Settings does
//! nothing at all.
//!
//! # What "resolution" means in a browser
//!
//! The native Xtra calls `ChangeDisplaySettingsA` to retune the monitor and then
//! `SetWindowPos` to recentre the stage on it. A browser cannot retune the
//! monitor and does not need to: the movie asks for its OWN viewport size as the
//! display mode (`SwitchToFullScreen(me, pViewportWidth, pViewportHeight,
//! pDisplayDepth)`), so the stage never resizes — only the screen around it
//! does. The faithful browser equivalent is the Fullscreen API on the canvas,
//! with the player's existing scaling filling the screen. So `set_resolution`
//! raises `wants_fullscreen` and `reset_resolution` clears it; the frontend
//! polls that flag exactly as it polls `wants_pointer_lock`.
//!
//! # Return values
//!
//! The movie tests these, so the sentinels matter:
//!
//! * `set_resolution` / `reset_resolution` — TRUTHY on success. `SwitchToFullScreen`
//!   does `if ret then ... pDisplayMode = #fullscreen else alert("... failed")`,
//!   so a VOID return both skips the state change and pops an alert.
//! * `test_resolution` — truthy when the mode is available. (PHOSPHOR immediately
//!   overwrites the answer with a hardcoded 1 on the next line, but other callers
//!   would not.)
//! * `get_current_display_mode` — a LIST indexed `[3]` for the colour depth, i.e.
//!   `[width, height, depth]`.
//! * `get_cpu_speed` — an integer; C_SystemReport does `string(...)` on it and
//!   C_Command prints it with `& " mhz"`.

use crate::director::lingo::datum::{Datum, DatumType};
use crate::player::{reserve_player_mut, reserve_player_ref, DatumRef, ScriptError};
use std::collections::VecDeque;

pub struct EnhancerXtra;

/// The display modes the movie probes in `C_Engine.TestResolutions`. Reported as
/// available so the movie's resolution menu is not empty; the browser honours
/// the size by scaling the canvas rather than by retuning a monitor.
const DISPLAY_MODES: &[(i32, i32, i32)] = &[
    (640, 480, 16),
    (800, 600, 16),
    (1024, 768, 16),
    (640, 480, 32),
    (800, 600, 32),
    (1024, 768, 32),
];

impl EnhancerXtra {
    pub fn has_handler(name: &str) -> bool {
        HANDLERS.iter().any(|h| name.eq_ignore_ascii_case(h))
    }

    pub fn call_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        // ---- display mode ----
        if name.eq_ignore_ascii_case("get_current_display_mode") {
            let (w, h) = screen_size();
            return Ok(int_list(&[w, h, 32]));
        }
        if name.eq_ignore_ascii_case("get_display_modes") {
            return Ok(reserve_player_mut(|player| {
                let rows: Vec<DatumRef> = DISPLAY_MODES
                    .iter()
                    .map(|(w, h, d)| {
                        let items: Vec<DatumRef> = [*w, *h, *d]
                            .iter()
                            .map(|n| player.alloc_datum(Datum::Int(*n)))
                            .collect();
                        player.alloc_datum(Datum::List(
                            DatumType::List,
                            VecDeque::from(items),
                            false,
                        ))
                    })
                    .collect();
                player.alloc_datum(Datum::List(DatumType::List, VecDeque::from(rows), false))
            }));
        }
        if name.eq_ignore_ascii_case("test_resolution") {
            let (w, h, d) = three_ints(args)?;
            let (sw, sh) = screen_size();
            // Available when it is a mode the Xtra knows AND it fits on the
            // screen we actually have. A mode larger than the display would
            // need real downscaling, which the native Xtra could not do either.
            let known = DISPLAY_MODES
                .iter()
                .any(|(mw, mh, md)| *mw == w && *mh == h && *md == d);
            let fits = w <= sw && h <= sh;
            return Ok(int_datum(if known && fits { 1 } else { 0 }));
        }
        if name.eq_ignore_ascii_case("set_resolution") {
            // Two arities: (w, h, depth) from a projector, (modeID, depth) from
            // the plugin branch. Either way the stage keeps its own size, so
            // both mean the same thing here — go fullscreen.
            reserve_player_mut(|player| {
                player.wants_fullscreen = true;
            });
            return Ok(int_datum(1));
        }
        if name.eq_ignore_ascii_case("reset_resolution") {
            reserve_player_mut(|player| {
                player.wants_fullscreen = false;
            });
            return Ok(int_datum(1));
        }

        // ---- cursor ----
        if name.eq_ignore_ascii_case("move_cursor") {
            let (x, y) = two_ints(args)?;
            // `warp_mouse_loc`, not a bare `mouse_loc` write — the shared path
            // also raises `wants_pointer_lock` when the cursor is hidden over
            // live 3D, which is what makes mouselook work in a browser. Same
            // reasoning as the MoveCursor Xtra.
            reserve_player_mut(|player| {
                player.warp_mouse_loc(x, y);
            });
            return Ok(int_datum(1));
        }

        // ---- identity ----
        if name.eq_ignore_ascii_case("version") {
            return Ok(reserve_player_mut(|player| {
                player.alloc_datum(Datum::String("1.6.11.r41029".to_string()))
            }));
        }
        // 0 is "no error". C_Engine never reads it, but a caller that did would
        // otherwise read a failure out of VOID.
        if name.eq_ignore_ascii_case("get_error") {
            return Ok(int_datum(0));
        }
        if name.eq_ignore_ascii_case("get_cpu_speed") {
            // No browser API reports clock speed, and none of the callers do
            // anything with it but print it. Answer a plausible constant rather
            // than VOID so C_SystemReport's `string(...)` has something to show.
            return Ok(int_datum(2400));
        }

        // ---- window / desktop chrome: nothing a browser can or should do ----
        //
        // Reported as SUCCEEDED (1). These are all fire-and-forget in the
        // callers — `center_stage`, `show_taskbar` and `enable_task_switching`
        // have their results dropped — but answering 1 keeps any caller that
        // does test one on its success path.
        if NO_OP_OK.iter().any(|h| name.eq_ignore_ascii_case(h)) {
            return Ok(int_datum(1));
        }

        // ---- things we genuinely do not have ----
        if name.eq_ignore_ascii_case("directX8_installed") {
            // WebGL2, not DirectX.
            return Ok(int_datum(0));
        }
        if EMPTY_LIST.iter().any(|h| name.eq_ignore_ascii_case(h)) {
            return Ok(reserve_player_mut(|player| {
                player.alloc_datum(Datum::List(DatumType::List, VecDeque::new(), false))
            }));
        }
        // Registry, file attributes, popup menus, modal alerts and virtual-key
        // probes: no browser equivalent, and no PHOSPHOR caller. 0 is the
        // Xtra's own "did nothing" answer for the integer-returning ones.
        if ZERO.iter().any(|h| name.eq_ignore_ascii_case(h)) {
            return Ok(int_datum(0));
        }

        Err(ScriptError::new(format!("Enhancer: no handler {}", name)))
    }
}

/// Handlers that report success without doing anything, because the browser has
/// no desktop to hide, no taskbar to show and no titlebar to strip.
const NO_OP_OK: &[&str] = &[
    "center_stage",
    "enable_task_switching",
    "disable_task_switching",
    "set_auto_switching",
    "hide_desktop",
    "show_desktop",
    "show_taskbar",
    "hide_taskbar",
    "show_titlebar",
    "hide_titlebar",
    "prepare_browser",
    "is_front_window",
    "open_joystick_control_panel",
    "forget",
    // Mac-only, and absent from the v1.6.11 table — C_Engine.SwitchToWindowed
    // still names it under `the platform = "Macintosh,PowerPC"`.
    "show_control_strip",
    "hide_control_strip",
];

/// Handlers whose Director contract is "a list", answered empty.
const EMPTY_LIST: &[&str] = &[
    "get_joysticks",
    "joystick_get_info",
    "joystick_get_axes",
    "joystick_get_buttons",
    "get_installed_fonts",
];

/// Handlers answered with 0 — no browser equivalent and no PHOSPHOR caller.
const ZERO: &[&str] = &[
    "pop_menu",
    "set_file_attributes",
    "get_file_attributes",
    "set_file_date",
    "write_key",
    "get_key_value",
    "delete_key",
    "alert",
    "check_virtual_key",
];

const HANDLERS: &[&str] = &[
    "test_resolution",
    "set_resolution",
    "reset_resolution",
    "get_display_modes",
    "get_current_display_mode",
    "move_cursor",
    "version",
    "get_error",
    "get_cpu_speed",
    "directX8_installed",
    "center_stage",
    "enable_task_switching",
    "disable_task_switching",
    "set_auto_switching",
    "hide_desktop",
    "show_desktop",
    "show_taskbar",
    "hide_taskbar",
    "show_titlebar",
    "hide_titlebar",
    "prepare_browser",
    "is_front_window",
    "open_joystick_control_panel",
    "forget",
    "show_control_strip",
    "hide_control_strip",
    "get_joysticks",
    "joystick_get_info",
    "joystick_get_axes",
    "joystick_get_buttons",
    "get_installed_fonts",
    "pop_menu",
    "set_file_attributes",
    "get_file_attributes",
    "set_file_date",
    "write_key",
    "get_key_value",
    "delete_key",
    "alert",
    "check_virtual_key",
];

/// The screen the movie is being shown on. Falls back to a 1024x768 desktop when
/// there is no window (the native test harness), which keeps `test_resolution`
/// answering yes for every mode PHOSPHOR probes.
fn screen_size() -> (i32, i32) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(screen) = win.screen() {
                let w = screen.width().unwrap_or(1024);
                let h = screen.height().unwrap_or(768);
                if w > 0 && h > 0 {
                    return (w, h);
                }
            }
        }
    }
    (1024, 768)
}

fn int_datum(n: i32) -> DatumRef {
    reserve_player_mut(|player| player.alloc_datum(Datum::Int(n)))
}

fn int_list(values: &[i32]) -> DatumRef {
    reserve_player_mut(|player| {
        let items: Vec<DatumRef> = values
            .iter()
            .map(|n| player.alloc_datum(Datum::Int(*n)))
            .collect();
        player.alloc_datum(Datum::List(DatumType::List, VecDeque::from(items), false))
    })
}

fn two_ints(args: &Vec<DatumRef>) -> Result<(i32, i32), ScriptError> {
    reserve_player_ref(|player| {
        let a = args
            .get(0)
            .ok_or_else(|| ScriptError::new("Enhancer: missing argument 1".to_string()))?;
        let b = args
            .get(1)
            .ok_or_else(|| ScriptError::new("Enhancer: missing argument 2".to_string()))?;
        Ok((
            player.get_datum(a).int_value()?,
            player.get_datum(b).int_value()?,
        ))
    })
}

fn three_ints(args: &Vec<DatumRef>) -> Result<(i32, i32, i32), ScriptError> {
    reserve_player_ref(|player| {
        let mut out = [0i32; 3];
        for (i, slot) in out.iter_mut().enumerate() {
            let a = args.get(i).ok_or_else(|| {
                ScriptError::new(format!("Enhancer: missing argument {}", i + 1))
            })?;
            *slot = player.get_datum(a).int_value()?;
        }
        Ok((out[0], out[1], out[2]))
    })
}
