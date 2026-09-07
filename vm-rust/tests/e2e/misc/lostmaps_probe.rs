use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::TestHarness;

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// Throwaway: does the .dcr extracted from the Phosphor map-tester projector
/// parse and load as a Director movie?
browser_e2e_test!(test_lostmaps_extract_probe, |player| async move {
    let movie_path = player.asset_path("dcr_misc/lost_maps/phosphor_beta2_032b_map_tester.dcr");
    player.load_movie(&movie_path).await;
    player.init_movie().await;
    probe(format!("[EXTRACT] loaded. stage rect={:?} casts={} frame={}",
        vm_rust::player::reserve_player_ref(|p| (p.movie.rect.width(), p.movie.rect.height())),
        vm_rust::player::reserve_player_ref(|p| p.movie.cast_manager.casts.len()),
        vm_rust::player::reserve_player_ref(|p| p.movie.current_frame)));
    let names = vm_rust::player::reserve_player_ref(|p| {
        p.movie.cast_manager.casts.iter()
            .map(|c| format!("{}({} members)", c.name, c.members.len()))
            .collect::<Vec<_>>().join(", ")
    });
    probe(format!("[EXTRACT] casts: {}", names));
    player.step_frames(30).await;
    probe(format!("[EXTRACT] after 30 frames, frame={}",
        vm_rust::player::reserve_player_ref(|p| p.movie.current_frame)));
    Ok(())
});

/// Mouselook must be AVAILABLE, which for this build means one thing:
/// `C_Input.new` only ever calls `InitBaMoveCursor`, which sets `pbMouseLook`
/// from `FindXtra("baMoveCursor") <> 0` — a scan of `the xtraList` for an entry
/// whose `name` starts with "baMoveCursor" (`baMoveCursor.x32`, its own Xtra,
/// listed in the .dir's Xtra table). With no such entry `pbMouseLook` stayed 0,
/// `CaptureMouse` exited at its first line, and clicking START gave a camera
/// you could not aim — the Start menu even prints "ERROR: unable to initialize
/// baMoveCursor Xtra - mouselook control disabled" in that state.
///
/// Then the actual capture: `CaptureMouse(#LOWERCENTER, ...)` hides the sprite
/// cursor (200) and warps through `baMoveCursor`, and it is that warp — routed
/// to `warp_mouse_loc` — that has to raise `wants_pointer_lock`, the flag the
/// frontend turns into `canvas.requestPointerLock()`.
browser_e2e_test!(test_lostmaps_mouselook_available, |player| async move {
    let movie_path = player.asset_path("dcr_misc/lost_maps/phosphor_beta2_032b_map_tester.dcr");
    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // `the xtraList` has to advertise baMoveCursor under its own name.
    let found = player.eval_datum("FindXtra(\"baMoveCursor\")").await
        .map_err(|e| format!("FindXtra failed: {}", e.message))?;
    probe(format!("FindXtra(baMoveCursor) = {:?}", found));
    if found.as_integer().unwrap_or(0) == 0 {
        let list = player.eval_datum("the xtraList").await
            .map(|d| format!("{:?}", d)).unwrap_or_default();
        return Err(format!(
            "FindXtra(\"baMoveCursor\") = 0 — mouselook stays disabled. xtraList: {}",
            list
        ));
    }

    // The warp itself: it is what raises the pointer-lock intent, and it must
    // move `the mouseH`/`mouseV` so ReadMouse's per-frame delta is meaningful.
    player.eval("cursor 200").await.ok();
    vm_rust::player::reserve_player_mut(|p| {
        // `warp_mouse_loc` only asks for the lock over live 3D, which is the
        // state the Start button leads to. Assert the WARP here, and the
        // gate separately below, without booting the whole map.
        p.mouse_loc = (0, 0);
    });
    player.eval("baMoveCursor(320, 240)").await
        .map_err(|e| format!("baMoveCursor call failed: {}", e.message))?;
    let loc = vm_rust::player::reserve_player_ref(|p| p.mouse_loc);
    if loc != (320, 240) {
        return Err(format!("baMoveCursor(320,240) left mouse_loc at {:?}", loc));
    }

    // cInput's own flag — the one every CaptureMouse call is gated on.
    // Built directly rather than by playing to the Start frame: only the .dcr
    // ships as a test asset, so the linked external casts (act_core.cct,
    // map_test.cct, snd_core.cct ...) 404 and the movie never leaves frame 1.
    // `C_Input.new` is self-contained — it only reads `the xtraList`.
    player.eval("cInput = script(\"C_Input\").new()").await
        .map_err(|e| format!("C_Input.new failed: {}", e.message))?;
    let mouselook = player.eval_datum("cInput.pbMouseLook").await
        .map(|d| d.as_integer().unwrap_or(0)).unwrap_or(0);
    let look_type = player.eval_datum("string(cInput.pMouseLookType)").await
        .map(|d| d.as_string().unwrap_or_default()).unwrap_or_default();
    probe(format!("cInput.pbMouseLook={} pMouseLookType={}", mouselook, look_type));
    if mouselook != 1 {
        return Err(format!(
            "cInput.pbMouseLook = {} (expected 1) — CaptureMouse exits immediately              and the camera cannot be aimed",
            mouselook
        ));
    }
    if look_type != "BAMC" {
        return Err(format!("cInput.pMouseLookType = {:?}, expected BAMC", look_type));
    }
    Ok(())
});
