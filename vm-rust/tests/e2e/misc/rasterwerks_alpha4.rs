use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_rasterwerks_alpha4.toml");

fn probe(msg: String) {
    // `println!` from wasm goes nowhere; the page console is what E2E_CONSOLE
    // forwards to the terminal.
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Rasterwerks "PHOSPHOR" alpha 4 — the self-contained predecessor of the beta2
// build covered by `rasterwerks.rs`. Everything it needs is in the one .dcr:
// `ReadProfileString` parses cast TEXT members rather than .ini files, and the
// map / sounds / actors live in internal casts.
//
// Boot chain:
//   F_PreLoader -> F_Loader -> F_SW3DLoader -> F_HWCheck -> F_Init -> "Start"
// F_Init runs a 23-step #Init pass building every C_* singleton, then a #load
// pass that loads the map, the scene graph, the nav net, the weapons and the
// bots. Clicking "Start" goes to "Rez" (resolution switch) and then "Main".
//
// Two engine defects kept this movie from ever reaching its menu, both fixed
// alongside this test:
//   * F_Loader polls `getStreamStatus(pURL)` from exitFrame on EVERY run mode
//     but only seeds pURL under "Plugin", so outside the plugin it polls with
//     VOID — which raised instead of answering a status.
//   * C_Object3D names members with chunk expressions (`member(a1).useAlpha`,
//     `pTexBaseName = a1.char[1..length(a1) - 2]`). A StringChunk fell through
//     `member()`'s numeric catch-all, where `int_value()` answers 0 for a
//     non-numeric string, so every such lookup silently became the invalid
//     (-1, -1) ref and the #load pass died on `useAlpha`.
browser_e2e_test!(test_misc_rasterwerks_alpha4_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // `string(...)` because StaticDatum has no vector variant — every 3D value
    // this movie holds would otherwise come back Void.
    macro_rules! ask {
        ($($t:tt)*) => {{
            let e = format!($($t)*);
            match player.eval_datum(&format!("string({})", e)).await {
                Ok(vm_rust::director::static_datum::StaticDatum::String(s)) => s,
                other => format!("<{:?}>", other),
            }
        }};
    }

    // Boot. F_Init advances one #load step per frame, so this is a long walk;
    // report where it got to so a stall shows up in the console rather than
    // only as a timeout.
    let mut at_menu = false;
    for i in 0..45 {
        player.step_frames(50).await;
        if ask!("the frameLabel") == "Start" {
            probe(format!("reached the Start menu after {} steps", i));
            at_menu = true;
            break;
        }
    }
    assert!(at_menu, "boot never reached the Start menu (frame {} label {}, gInitMode {})",
        ask!("the frame"), ask!("the frameLabel"), ask!("gInitMode"));

    // Every engine singleton F_Init's #Init pass builds must exist — a step
    // that raised would leave its global VOID and the ones after it unbuilt.
    for g in ["cEngine", "cMap", "cWorld", "cNavNet", "cCamera", "cOverlay",
              "cWeapon", "cMissileSys", "cPlayer", "cItemSys", "cBotSys"] {
        assert!(ask!("{}", g).starts_with("<offspring"),
            "F_Init never built {} — it is {}", g, ask!("{}", g));
    }

    // ...and the #load pass actually built a world, not just the objects that
    // manage one.
    let bots: i32 = ask!("clBot.count").parse().unwrap_or(0);
    assert!(bots > 0, "no bots were built");
    let nodes: i32 = ask!("cNavNet.plNavNode.count").parse().unwrap_or(0);
    assert!(nodes > 0, "the nav net is empty — the map's nav nodes did not load");
    probe(format!("{} bots, {} nav nodes, map {}", bots, nodes, ask!("gLevelDefName")));

    // "Start" is F_Start's button 1 — hotspot sprite 23, laid out at (280, 20)
    // plus the menu's centring shift for the viewport. Read the sprite's own
    // loc rather than hard-coding it, since the shift depends on stage size.
    //
    // Click until the game answers instead of at a fixed frame count: how far
    // the boot has got after N frames depends on how fast frames are stepping,
    // and that differs between this test alone and a full suite run.
    // cPlayer.pGameState stays VOID until C_Game.StartGame spawns the player.
    //
    // Re-read the loc EVERY attempt rather than capturing it once up front.
    // F_Start lays its UI out from `prepareFrame`, not `beginSprite`:
    //
    //     if pDoOnce then
    //       repeat with I = 1 to plSprUIPos.count
    //         sprite(plSprUIPos[I][1]).loc = plSprUIPos[I][2] + pViewportShift
    //
    // so until that frame runs, sprite 23 still sits at the raw authored
    // point(280, 20) instead of point(480, 170) (the 800x600 centring shift).
    // A single read taken before it lands pins the click to a spot the button
    // is never at, and all forty attempts miss — which is exactly what a
    // slower-than-usual boot produced in a full-suite run, where it looked
    // like the movie had hung on the menu.
    let mut started = false;
    let mut last_loc = String::new();
    for attempt in 0..40 {
        let loc = ask!("sprite(23).loc");
        let xy: Vec<i32> = loc.trim_start_matches("point(").trim_end_matches(')')
            .split(',').filter_map(|p| p.trim().split('.').next().unwrap_or("").parse().ok()).collect();
        if xy.len() == 2 {
            if loc != last_loc {
                probe(format!("clicking Start at {} (attempt {})", loc, attempt));
                last_loc = loc;
            }
            player.click(xy[0], xy[1]).await;
        }
        player.step_frames(60).await;
        if !ask!("cPlayer.pGameState").is_empty() {
            probe(format!("game started on attempt {} (frame label {})",
                attempt, ask!("the frameLabel")));
            started = true;
            break;
        }
    }
    assert!(started, "never left the Start menu — cPlayer.pGameState is still VOID          (frame label {})", ask!("the frameLabel"));

    // C_Player spawns into #Spawn and cGame's spawn timer promotes it to #LIVE.
    let mut live = false;
    for _ in 0..60 {
        if ask!("cPlayer.pGameState") == "LIVE" { live = true; break; }
        player.step_frames(30).await;
    }
    assert!(live, "never reached gameplay — cPlayer.pGameState is {}", ask!("cPlayer.pGameState"));
    probe(format!("spawned at {} with {} health",
        ask!("cPlayer.pvPosition"), ask!("cPlayer.pHealth")));

    // ---- Mouselook / pointer lock ----
    //
    // C_Input picks a cursor-warping Xtra at startup: Enhancer first, else
    // MoveCursor. EVERY mouse path is gated on the flag that search sets --
    // `CaptureMouse` opens with `if not pEXactive then exit` -- and the failure
    // branch also REBINDS fire to the ENTER key. So one bad answer from the
    // Xtra costs both aiming and shooting, which is exactly what happened:
    // `xtra("MoveCursor").register(serial)` returned VOID, and C_Input tests
    //     if RegisterMoveCursor(me) = 1 then pEXactive = 1
    // The real Xtra writes an integer into the Lingo return slot (1 registered,
    // 0 rejected -- IDA `sub_10001911`).
    assert_eq!(ask!("xtra(\"MoveCursor\").register(\"AAMOVC-97445-75238-39113\")"), "1",
        "MoveCursor.register must answer the integer 1, not VOID — C_Input tests `= 1`");
    assert_eq!(ask!("cInput.pEXactive"), "1",
        "C_Input found no cursor-warping Xtra, so mouselook is disabled entirely");
    // Enhancer is preferred and MoveCursor is only the FALLBACK:
    //     if FindXtra(me, "Enhancer") <> 0 then InstantiateEnhancer(me) ...
    //     if pEXactive = 0 then  -- only now try MoveCursor
    // so with the Enhancer Xtra ported, `pUseMC` stays 0 and CaptureMouse warps
    // through `pEX.move_cursor(...)` instead of the bare global. Both land on
    // `warp_mouse_loc`, so mouselook behaves identically either way — the
    // assertions below still hold.
    assert_eq!(ask!("ilk(cInput.pEX)"), "instance",
        "C_Input never built an Enhancer instance — C_Engine gates ALL of
         resolution switching on `ilk(cInput.pEX) = #instance`");
    assert_eq!(ask!("cInput.pUseMC"), "0",
        "C_Input fell back to MoveCursor even though Enhancer is registered");

    // ...and that instance is what makes fullscreen reachable at all.
    assert_eq!(ask!("cEngine.pResolutionSwitchingEnabled"), "1",
        "resolution switching is disabled, so Settings -> Display Mode is inert");
    assert_eq!(ask!("cEngine.pDisplayStart.count"), "3",
        "get_current_display_mode must answer [width, height, depth] — C_Engine
         reads `pDisplayStart[3]` for the colour depth");

    // ...and the fire binding survived. 1004 is MOUSE1 in the movie's keycode
    // table; 36 is ENTER, the binding C_Input installs when it gives up on
    // mouselook. Asserting this is what catches "can't shoot" separately from
    // "can't aim" — they share a cause but not a symptom.
    assert_eq!(ask!("cInput.plKeyCode[cPlayer.KEY_FIRE]"), "1004",
        "fire was rebound off MOUSE1 — C_Input took its no-mouselook fallback");

    // CaptureMouse(#LOWERCENTER) runs from F_Main: it hides the cursor and
    // warps it, and `warp_mouse_loc` raises `wants_pointer_lock` when the cursor
    // is hidden over live 3D. That flag is what the frontend polls to call
    // `canvas.requestPointerLock()`, so it is the actual "can I aim?" bit.
    assert_eq!(ask!("cInput.pMouseLock"), "1", "CaptureMouse never engaged the mouse lock");
    let (locked, hidden, w3d) = vm_rust::player::reserve_player_ref(
        |p| (p.wants_pointer_lock, p.cursor_is_hidden, p.w3d_any_rendered));
    assert!(hidden, "the movie never hid the cursor — CaptureMouse exited early");
    assert!(w3d, "no 3D is being rendered, so mouselook would be refused");
    assert!(locked, "wants_pointer_lock never rose, so the frontend never requests          pointer lock and the camera cannot be aimed");

    // The spawn point must be a real nav node in the world, not the origin the
    // player datum is constructed with.
    let pos: Vec<f64> = ask!("cPlayer.pvPosition")
        .trim_start_matches("vector(").trim_end_matches(')')
        .split(',').filter_map(|p| p.trim().parse().ok()).collect();
    assert_eq!(pos.len(), 3, "cPlayer.pvPosition is not a vector");
    assert!(pos.iter().any(|c| c.abs() > 1.0),
        "the player spawned at the origin — C_NavNet never handed out a player start");

    // ---- Light coronas must not shine through the back of your head ----
    //
    // C_Overlay decides whether a light corona is on screen with nothing but the
    // documented out-of-view contract of worldSpaceToSpriteSpace:
    //     pCorona[COR_POS2D] = cCamera.pCam.worldSpaceToSpriteSpace(pCorona[COR_POS3D])
    //     if plCoronaSys[I][COR_POS2D] = VOID then pCorona[COR_VIEWVIS] = 0
    // There is no dot product and no frustum test anywhere else in the system, so
    // a point that projects to a POINT instead of VOID is a corona that stays lit.
    //
    // Behind the camera w_clip is negative, and dividing by it mirrors the point
    // back into frame — so the failure is not just "still visible" but "visible on
    // the wrong side". Probe straight along +z of the camera (its BACK; the view
    // looks down -z) and straight ahead as a control.
    // `voidP`, not the stringified value: `string(VOID)` is EMPTY, which is
    // indistinguishable from a failed eval.
    const BEHIND: &str = "cCamera.pCam.worldSpaceToSpriteSpace(cCamera.pCam.worldPosition          + (cCamera.pCam.getWorldTransform().zAxis * 500))";
    const AHEAD: &str = "cCamera.pCam.worldSpaceToSpriteSpace(cCamera.pCam.worldPosition          - (cCamera.pCam.getWorldTransform().zAxis * 500))";
    assert_eq!(ask!("voidP({})", BEHIND), "1",
        "a point 500 units BEHIND the camera projected to {} instead of VOID —          every light behind you keeps its corona drawn, mirrored into frame",
        ask!("{}", BEHIND));
    assert_eq!(ask!("voidP({})", AHEAD), "0",
        "a point 500 units in FRONT of the camera must still project to a point");
    assert!(ask!("{}", AHEAD).starts_with("point("),
        "the forward projection is not a point: {}", ask!("{}", AHEAD));
    probe(format!("worldSpaceToSpriteSpace: behind=VOID ahead={}", ask!("{}", AHEAD)));

    // ---- setPixel's integer form carries alpha on a 32-bit image ----
    //
    // C_ScoreBoard clears its 512x256 text layer to TRANSPARENT white before
    // blitting the frag table into it:
    //     pTextImg.setPixel(x, y, RGBtoInteger(rgb(255, 255, 255), a1))   -- a1 = 0
    // RGBtoInteger builds Director's signed 32-bit AARRGGBB (negative once
    // alpha >= 128). Dropping that alpha byte made the clear OPAQUE, so the
    // scoreboard drew as a solid white box over the panel art beneath it
    // instead of the translucent panel the HUD uses everywhere else.
    let _ = player.eval("gTestImg = image(4, 4, 32)").await;
    let _ = player.eval("gTestImg.useAlpha = 1").await;
    // alpha 0, white -> 0x00FFFFFF. Round-trips through getPixel(#integer).
    let _ = player.eval("gTestImg.setPixel(0, 0, RGBtoInteger(rgb(255, 255, 255), 0))").await;
    assert_eq!(ask!("gTestImg.getPixel(point(0, 0), #integer)"), "16777215",
        "setPixel dropped the alpha byte of a 32-bit integer — a transparent          clear writes opaque pixels");
    let _ = player.eval("gTestImg.setPixel(1, 1, RGBtoInteger(rgb(255, 255, 255), 255))").await;
    assert_eq!(ask!("gTestImg.getPixel(point(1, 1), #integer)"), "16777215",
        "the negative (alpha >= 128) integer form lost its colour bits");

    // The alpha itself is read through extractAlpha rather than
    // getPixel(#integer), which deliberately answers 24-bit RGB on a 32-bit
    // image so hit-test idioms comparing against white keep working.
    let _ = player.eval("gTestAlpha = gTestImg.extractAlpha()").await;
    assert_eq!(ask!("gTestAlpha.getPixel(point(0, 0), #integer)"), "0",
        "alpha 0 was written opaque — the transparent clear becomes a solid box");
    assert_eq!(ask!("gTestAlpha.getPixel(point(1, 1), #integer)"), "255",
        "alpha 255 did not survive the negative signed-integer form");

    // Scan the member's RENDERED image for the first/last row carrying ink, so
    // we can see where the baseline actually falls inside the authored box.
    // These are text members, so `.image` is rasterised on demand — evaluate it
    // and read the bitmap the DatumRef points at.
    for m in ["txtArialBold18_256", "txtArialBold24_256_CENTER", "txtArialBold30_256_CENTER"] {
        let _ = player.eval(&format!(
            "member(\"{}\").text = \"Playing Frags Ping gjpqy\"", m)).await;
        let img = player.eval(&format!("member(\"{}\").image", m)).await;
        let info = match img {
            Ok(dr) => vm_rust::player::reserve_player_ref(|p| {
                let bmp_ref = match p.get_datum(&dr) {
                    vm_rust::director::lingo::datum::Datum::BitmapRef(b) => *b,
                    other => return format!("<not a bitmap: {}>", other.type_str()),
                };
                let Some(bmp) = p.bitmap_manager.get_bitmap(bmp_ref) else {
                    return "<no bitmap>".to_string();
                };
                let (w, h) = (bmp.width as usize, bmp.height as usize);
                let bpp = bmp.bit_depth as usize / 8;
                let (mut first, mut last) = (-1i32, -1i32);
                for y in 0..h {
                    let mut ink = false;
                    for x in 0..w {
                        let i = (y * w + x) * bpp;
                        if bmp.data[i..i + bpp] != bmp.data[0..bpp] { ink = true; break; }
                    }
                    if ink {
                        if first < 0 { first = y as i32; }
                        last = y as i32;
                    }
                }
                format!("image {}x{} depth {} ink rows {}..{} (top gap {}, bottom gap {})",
                    w, h, bmp.bit_depth, first, last, first, h as i32 - 1 - last)
            }),
            Err(e) => format!("<eval failed: {:?}>", e),
        };
        probe(format!("{}: size={} fls={} {}",
            m, ask!("member(\"{}\").fontSize", m),
            ask!("member(\"{}\").fixedLineSpace", m), info));
    }

    probe(vm_rust::player::reserve_player_ref(|p| {
        let mut out = String::new();
        for (fref, f) in p.font_manager.fonts.iter() {

            let cap = p.bitmap_manager.get_bitmap(f.bitmap_ref)
                .map(|fb| vm_rust::player::font::pfr_strike_vertical_metrics(f, fb))
                .unwrap_or((None, None));
            out.push_str(&format!(
                "
  {:?} '{}' size={} cell={}x{} grid_cell={}x{} off=({},{}) widths={} capTop/descBot={:?}",
                fref, f.font_name, f.font_size, f.char_width, f.char_height,
                f.grid_cell_width, f.grid_cell_height, f.char_offset_x, f.char_offset_y,
                f.char_widths.is_some(), cap));
        }
        // Also: what font MEMBERS exist in the casts, and did a system font load?
        let mut members = String::new();
        for cast in p.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                let t = m.member_type.type_string();
                if t.eq_ignore_ascii_case("font") {
                    members.push_str(&format!("
  FONT MEMBER '{}'", m.name));
                }
            }
        }
        format!("fonts loaded: {} entries, system_font={}{}
font members:{}",
            p.font_manager.fonts.len(), p.font_manager.system_font.is_some(), out, members)
    }));

    // ---- ESC/pause menu: every button must have a hotspot ----
    //
    // C_HotSpot builds its rect table by COUNTING lines and then indexing them:
    //     pHsCnt = m.lineCount
    //     repeat with I = 1 to pHsCnt
    //       plHsRect[I] = value(l.item[2])
    // so `lineCount` and the `line[]` accessor have to agree. HSR.PausePanel is
    // CR-delimited, and `str::lines()` does not break on a bare  — it counted
    // 1 while line[2]/line[3] returned real rows, leaving the pause panel with a
    // single hotspot. RESUME (rollover 1) worked; SETTINGS (2) and QUIT (3) were
    // dead, since F_Main switches on exactly that rollover index.
    assert_eq!(ask!("member(\"HSR.PausePanel\").lineCount"), "3",
        "HSR.PausePanel is 3 CR-delimited lines but lineCount says {} — the pause          menu will only register its first button",
        ask!("member(\"HSR.PausePanel\").lineCount"));
    assert_eq!(ask!("cOverlay.pPausePanelHS.pHsCnt"), "3",
        "the pause panel built {} hotspots, so the lower buttons cannot be clicked",
        ask!("cOverlay.pPausePanelHS.pHsCnt"));
    assert_eq!(ask!("cOverlay.pPausePanelHS.plHsRect"),
        "[rect(0, 36, 128, 68), rect(0, 72, 128, 104), rect(0, 108, 128, 140)]",
        "the pause hotspot rects are not the three authored rows");

    // ...and each rect must actually answer a hit, at the panel's real offset —
    // F_Main reads `cOverlay.Step_Pause()` and switches on the rollover index.
    for (btn, cy) in [(1, 36 + 16), (2, 72 + 16), (3, 108 + 16)] {
        let hit = ask!("cOverlay.pPausePanelHS.Step(point(336 + 64, 212 + {}))", cy);
        assert_eq!(hit, btn.to_string(),
            "pause button {} does not hit-test (got rollover {})", btn, hit);
    }

    // Gameplay runs: the frame loop must keep stepping the world rather than
    // parking. C_Player.Step integrates against the ground every frame, so the
    // eye position tracking the body is evidence the whole per-frame chain ran.
    let health_before = ask!("cPlayer.pHealth");
    player.step_frames(120).await;
    assert_eq!(ask!("the frameLabel"), "Main", "the game left the Main frame");
    assert_eq!(ask!("cPlayer.pGameState"), "LIVE",
        "the player did not survive 120 frames of standing still (health {} -> {})",
        health_before, ask!("cPlayer.pHealth"));

    Ok(())
});
