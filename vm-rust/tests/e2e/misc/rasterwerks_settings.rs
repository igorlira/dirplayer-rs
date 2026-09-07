use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_rasterwerks_alpha4.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Rasterwerks "PHOSPHOR" alpha 4 — the SETTINGS menu (F_Settings, frames
// SET1..SET5). `rasterwerks_alpha4.rs` covers the boot and the game; this walks
// the menu the game reaches from F_Start's SETTINGS button.
//
// Three engine defects it pins:
//
//  * The score stores a sprite's width/height SIGNED, and Director writes -4
//    as the width of a checkBox whose label is empty (the member's initialRect
//    right edge is -4). Read unsigned that became 65532, and the indicator's
//    opaque label area ran clear across the stage — Mute, Positional Effects
//    and Spectator each drew as a black bar over the panel art.
//
//  * `delete <chunkExpr>` reached as a CALL rather than as the DeleteChunk
//    opcode. `DropDown #SETITEMS` trims surplus rows with
//    `delete(sprite(...).member.line[n])`, so without it every dropdown kept
//    the previous menu's leftover lines.
//
//  * ...and that trim runs against a TEXT member, whose text the chunk-delete
//    read path could not reach — it unwrapped `as_field()` and panicked.
//
// Plus getRendererServices(): `renderer` is a SYMBOL (the page showed
// "#openGL"), depthBufferDepth was missing ("-bit"), and getHardwareInfo()
// left the whole driver block blank.
browser_e2e_test!(test_misc_rasterwerks_settings, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    let snapshots = SnapshotContext::new("misc", "rasterwerks_settings");

    macro_rules! ask {
        ($($t:tt)*) => {{
            let e = format!($($t)*);
            match player.eval_datum(&format!("string({})", e)).await {
                Ok(vm_rust::director::static_datum::StaticDatum::String(s)) => s,
                other => format!("<{:?}>", other),
            }
        }};
    }
    macro_rules! click_sprite {
        ($n:expr) => {{
            let l = ask!("sprite({}).loc", $n);
            let p: Vec<i32> = l.trim_start_matches("point(").trim_end_matches(')')
                .split(',').filter_map(|c| c.trim().split('.').next().unwrap_or("").parse().ok()).collect();
            if p.len() == 2 { player.click(p[0], p[1]).await; }
            l
        }};
    }

    let mut at_menu = false;
    for _ in 0..45 {
        player.step_frames(50).await;
        if ask!("the frameLabel") == "Start" { at_menu = true; break; }
    }
    assert!(at_menu, "boot never reached the Start menu (label {})", ask!("the frameLabel"));

    let mut in_settings = false;
    for _ in 0..20 {
        click_sprite!(26);                      // F_Start's SETTINGS hotspot
        player.step_frames(10).await;
        if ask!("the frameLabel").starts_with("SET") { in_settings = true; break; }
    }
    assert!(in_settings, "never reached the settings frames (label {})", ask!("the frameLabel"));
    player.step_frames(10).await;

    // Tabs are hotspot sprites 31..35 -> SwitchPage(rollover - 30).
    // F_Settings reads `the rollover` in exitFrame and acts on it in mouseUp,
    // so the first click after a page switch can land before the rollover has
    // been sampled. Click until the page actually changes.
    macro_rules! tab {
        ($page:expr) => {{
            for _ in 0..8 {
                if ask!("gFS.pTabPage") == $page.to_string() { break; }
                click_sprite!(30 + $page);
                player.step_frames(10).await;
            }
            probe(format!("tab {} -> label={} pTabPage={}", $page,
                ask!("the frameLabel"), ask!("gFS.pTabPage")));
            assert_eq!(ask!("gFS.pTabPage"), $page.to_string(),
                "clicking tab {} did not switch the page", $page);
        }};
    }

    for page in 1..=5 {
        tab!(page);
        let _ = snapshots.verify(&format!("0{}_set{}", page, page), player.snapshot_stage());
    }

    // The check boxes must be an indicator, not a bar across the panel.
    tab!(3);
    for s in [63, 67] {
        let rect = ask!("sprite({}).rect", s);
        probe(format!("SET3 checkbox sprite({}) rect={} width={} height={}",
            s, rect, ask!("sprite({}).width", s), ask!("sprite({}).height", s)));
        let w: i32 = ask!("sprite({}).width", s).parse().unwrap_or(-1);
        assert!(w > 0 && w <= 32,
            "checkbox sprite {} is {}px wide — it should be an indicator, not a bar ({})",
            s, w, rect);
    }

    // An open dropdown. `DropDown #SETITEMS` trims surplus rows with
    // `delete(... .member.line[n])`, so a 3-entry list must not show 10.
    tab!(2);
    probe(format!("Renderer dropdown at {}", click_sprite!(56)));
    player.step_frames(6).await;
    probe(format!("pState={} items={} target={} lines={:?}",
        ask!("gFS.pState"), ask!("gFS.plDropDownItems"),
        ask!("gFS.pDropDownTargetSprite"), ask!("sprite(131).member.text")));
    assert_eq!(ask!("gFS.pState"), "DropDown", "the Renderer dropdown never opened");
    let _ = snapshots.verify("06_dropdown_renderer", player.snapshot_stage());

    // ...and one with more entries: Texture Quality (hotspot 76) has three.
    // Dismiss the open one first — while pState is #DropDown every mouseUp is
    // routed to the dropdown itself, so a second click just closes this one.
    player.click(700, 560).await;
    player.step_frames(6).await;
    let _ = click_sprite!(76);
    player.step_frames(6).await;
    probe(format!("texture-quality items={} lineCount={} text={:?}",
        ask!("gFS.plDropDownItems"), ask!("sprite(131).member.lineCount"),
        ask!("sprite(131).member.text")));
    assert_eq!(ask!("sprite(131).member.lineCount"), "3",
        "the dropdown list member still holds {} lines for a 3-entry menu — `delete <chunk>` did not trim it",
        ask!("sprite(131).member.lineCount"));
    // ...and those three lines must be CR-delimited. The movie writes each entry
    // with `sprite(x).member.line[I] = ...`, and re-joining with CRLF put a BLANK
    // line between every entry — `lineCount` still said 3 (it folds CRLF), but the
    // renderer treats each CR and each LF as its own break, so the menu drew at
    // 40px a row and overflowed the 20px-a-row panel the movie sized for it.
    let raw = ask!("sprite(131).member.text");
    assert!(!raw.contains('\n'),
        "the dropdown list is CRLF-delimited ({:?}) — every entry gets a blank line after it",
        raw);
    let _ = snapshots.verify("07_dropdown_texture_quality", player.snapshot_stage());

    // Repeated tab switching must not strand the UI at the top-left corner:
    // SwitchPage rebuilds plSprUIPos from datSetPos<page> and prepareFrame
    // re-lays it out, so sprite 11 (the first tab) has to keep its shift.
    for p in [1, 3, 5, 2, 4, 1] {
        tab!(p);
        let l = ask!("sprite(11).loc");
        assert_ne!(l, "point(0, 0)",
            "after switching to tab {} the UI was never re-laid out (sprite 11 at {})", p, l);
    }
    let _ = snapshots.verify("08_after_reswitch", player.snapshot_stage());

    // ---- Settings -> Video -> Display Mode -> fullscreen ----
    //
    // The whole path runs through the Enhancer Xtra. C_Engine.SwitchToFullScreen
    // calls `cInput.pEX.set_resolution(w, h, depth)` and only commits
    // `pDisplayMode = #fullscreen` `if ret` — a VOID return both skips the state
    // change and pops an alert. And none of it is even reached unless
    // `pResolutionSwitchingEnabled`, which is set from `ilk(cInput.pEX) = #instance`.
    tab!(2);
    assert_eq!(ask!("cEngine.pResolutionSwitchingEnabled"), "1",
        "resolution switching is off — Display Mode cannot do anything");

    // Drive it the way the player does: open the Display Mode dropdown
    // (hotspot 66 over the value field at 65) and pick "fullscreen".
    let _ = click_sprite!(66);
    player.step_frames(6).await;
    assert_eq!(ask!("gFS.pState"), "DropDown", "the Display Mode dropdown never opened");
    probe(format!("display-mode items={}", ask!("gFS.plDropDownItems")));
    // Entry 2 is "fullscreen"; the rows sit 20px apart under the button.
    // [probe] Which row does sprite 133 actually cover, and what does the list
    // member look like? The row geometry is derived from the dropdown list TEXT
    // member, so a change in text line metrics can move the rows out from under
    // the sprite the test clicks.
    for n in [131, 132, 133, 134] {
        probe(format!(
            "row sprite {}: loc={} rect={} member={} height={} lineCount={} fls={} topSpacing={}",
            n,
            ask!("sprite({}).loc", n),
            ask!("sprite({}).rect", n),
            ask!("sprite({}).member.name", n),
            ask!("sprite({}).member.height", n),
            ask!("sprite({}).member.lineCount", n),
            ask!("sprite({}).member.fixedLineSpace", n),
            ask!("sprite({}).member.topSpacing", n),
        ));
    }
    probe(format!(
        "before pick: target={} items={} val={} charPosToLoc(1)={}",
        ask!("gFS.pDropDownTargetSprite"),
        ask!("gFS.plDropDownItems"),
        ask!("member(\"labelDisplayVal\").text"),
        ask!("sprite(131).member.charPosToLoc(1)"),
    ));
    let clicked_at = click_sprite!(133);
    player.step_frames(6).await;
    probe(format!(
        "after pick: clicked={} state={} val={} listText={}",
        clicked_at,
        ask!("gFS.pState"),
        ask!("member(\"labelDisplayVal\").text"),
        ask!("sprite(131).member.text"),
    ));
    assert_eq!(ask!("member(\"labelDisplayVal\").text"), "fullscreen",
        "picking the second dropdown row did not write it back to the value field");

    // OK (hotspot 7) applies the page and leaves the menu.
    let _ = click_sprite!(7);
    player.step_frames(30).await;
    assert_eq!(ask!("cEngine.pDisplayModeSettings"), "fullscreen",
        "ApplySettings did not record the fullscreen choice");

    // OK only RECORDS the choice and returns to wherever Settings was opened
    // from (`gGoFrame = gReturnFrame`, i.e. "Start" here). The switch itself is
    // F_Rez's job, and the only way into F_Rez is the START button —
    // `C_Engine.UpdateResolution` is what compares pDisplayMode against
    // pDisplayModeSettings and calls SwitchToFullScreen.
    let mut at_menu_again = false;
    for _ in 0..20 {
        if ask!("the frameLabel") == "Start" { at_menu_again = true; break; }
        player.step_frames(10).await;
    }
    assert!(at_menu_again, "OK did not return to the Start menu (label {})",
        ask!("the frameLabel"));
    // START -> "Rez". Clicked in a loop with the loc re-read each time: F_Start
    // lays its UI out from `prepareFrame`, so sprite 23 sits at the raw authored
    // point(280, 20) until that frame runs and only then moves to the centred
    // position. A single click taken too early lands where the button is not.
    for _ in 0..20 {
        if ask!("the frameLabel") != "Start" { break; }
        let _ = click_sprite!(23);
        player.step_frames(10).await;
    }

    // F_Rez runs the actual switch on the way into the game.
    let mut fullscreen = false;
    for _ in 0..40 {
        if ask!("cEngine.pDisplayMode") == "fullscreen" { fullscreen = true; break; }
        player.step_frames(20).await;
    }
    assert!(fullscreen,
        "C_Engine never switched to fullscreen — pDisplayMode is {} (set_resolution
         must answer TRUTHY, or SwitchToFullScreen alerts and gives up)",
        ask!("cEngine.pDisplayMode"));
    assert!(vm_rust::player::reserve_player_ref(|p| p.wants_fullscreen),
        "the movie switched to fullscreen but `wants_fullscreen` never rose, so the
         frontend would never call requestFullscreen()");
    probe("fullscreen engaged".to_string());

    Ok(())
});
