//! MoveCursor Xtra v1.6.3 — warps the mouse pointer to a screen position.
//!
//! Message table, as reported by `put xtra("movecursor")`:
//!
//! ```text
//! xtra MoveCursor -- version 1.6.3
//! + register object me, string SerialNumber
//! * move_cursor integer X, integer Y
//! ```
//!
//! `+` marks a CLASS method (`xtra("movecursor").register(...)`), `*` a global
//! handler (`move_cursor(x, y)`). Neither needs an instance, so this is a
//! static-only Xtra.
//!
//! A browser cannot move the OS pointer, but it doesn't need to: what the
//! calling movie actually depends on is that `_mouse.mouseLoc` reads back the
//! warped position. Miniclip's Rifleman uses the classic mouselook recentre in
//! `MovieScript 16 - Riflemen Camera`:
//!
//! ```lingo
//! pMousePrevPos = _mouse.mouseLoc
//! move_cursor(pMouseCenterPos[1], pMouseCenterPos[2])
//! pMouseCurrPos = _mouse.mouseLoc
//! lMouseDx = float(pMouseCurrPos.locH - pMousePrevPos.locH)
//! ```
//!
//! It reads `mouseLoc` back AFTER the warp and subtracts the pre-warp reading,
//! so the delta comes out as `centre - previous` — the NEGATIVE of the movement
//! since the last recentre. That sign convention is the movie's, not ours; see
//! [[fps-pointerlock-mouselook]]. Updating `player.mouse_loc` is therefore both
//! necessary and sufficient — without it the two reads are identical, every
//! delta is zero, and the first-person camera never turns.
//!
//! Behavior sourced from `MoveCursor.x32_export.json` (IDA). The native handler
//! maps the point through the movie's coordinate services, then `ClientToScreen`
//! + `SetCursorPos`; it returns an HRESULT to the Xtra host, so Lingo sees VOID.
//!
//! ## baMoveCursor
//!
//! `baMoveCursor.x32` is a SEPARATE, single-purpose Xtra (despite the Buddy API
//! "ba" prefix it is not part of BuddyAPI) exposing one global:
//!
//! ```text
//! * baMoveCursor integer X, integer Y
//! ```
//!
//! It does exactly what `move_cursor` does, so it is served from here rather
//! than from a module of its own. It has to be listed in `the xtraList` under
//! its own name as well: PHOSPHOR's `C_Input.new` only ever calls
//! `InitBaMoveCursor`, which gates `pbMouseLook` on
//! `FindXtra("baMoveCursor") <> 0` — a scan of `the xtraList` for an entry
//! whose `name` STARTS with "baMoveCursor". With no such entry `pbMouseLook`
//! stayed 0, so `CaptureMouse` exited at its first line: the cursor was never
//! hidden, the recentre never ran, `wants_pointer_lock` never went up, and
//! Lost Maps started with a camera you could not aim.

use crate::player::{reserve_player_mut, reserve_player_ref, DatumRef, ScriptError};

pub struct MoveCursorXtra;

impl MoveCursorXtra {
    pub fn has_handler(name: &str) -> bool {
        matches_ci(name, "register")
            || matches_ci(name, "move_cursor")
            // Checked BEFORE BudApiXtra in the manager's dispatch chain, which
            // claims every "ba"-prefixed name — so this arm has to exist here
            // or the warp silently becomes a BudAPI no-op.
            || matches_ci(name, "baMoveCursor")
    }

    pub fn call_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        if matches_ci(name, "register") {
            // Returns 1 when registered, 0 when the serial is rejected -- never
            // VOID. Callers test it: PHOSPHOR's C_Input does
            // `if RegisterMoveCursor(me) = 1 then pEXactive = 1`, and gates all
            // of mouselook (and the fire key binding) on that flag.
            // Serials are not validated here, so registration always succeeds.
            return Ok(reserve_player_mut(|player| {
                player.alloc_datum(crate::director::lingo::datum::Datum::Int(1))
            }));
        }
        if matches_ci(name, "move_cursor") || matches_ci(name, "baMoveCursor") {
            return move_cursor(args);
        }
        Err(ScriptError::new(format!("MoveCursor: no handler {}", name)))
    }
}

fn matches_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn move_cursor(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    let (x, y) = reserve_player_ref(|player| {
        let x = args
            .get(0)
            .ok_or_else(|| ScriptError::new("move_cursor requires an X argument".to_string()))?;
        let y = args
            .get(1)
            .ok_or_else(|| ScriptError::new("move_cursor requires a Y argument".to_string()))?;
        Ok((
            player.get_datum(x).int_value()?,
            player.get_datum(y).int_value()?,
        ))
    })?;

    // `warp_mouse_loc`, NOT a bare `mouse_loc` write: the shared path also
    // raises `wants_pointer_lock` when the cursor is hidden over live 3D, which
    // is what actually makes mouselook work in a browser. Writing the field
    // directly moved the reported position but left the real pointer free, so
    // Rifleman booted into its 3D scene with a camera you couldn't aim.
    reserve_player_mut(|player| {
        player.warp_mouse_loc(x as i32, y as i32);
    });
    Ok(DatumRef::Void)
}
