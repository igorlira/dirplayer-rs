use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_spectral_wizard.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// Park the cursor without pressing. `TestHarness::click` sets `mouse_loc` and
/// dispatches MouseDown in the same instant, so a movie that derives its
/// rollover state from `the mouseLoc` on a PREVIOUS frame never sees the cursor
/// arrive and treats the button as inactive. Spectral Wizard's menu does exactly
/// that — a bare click on NEW GAME left it sitting on the title screen.
fn hover(x: i32, y: i32) {
    vm_rust::player::reserve_player_mut(|player| {
        player.mouse_loc = (x, y);
    });
}

// Spectral Wizard — coverage for multi-line PFR text with `fixedLineSpace = 0`
// (Paige AUTO leading).
//
// The loading plaque and the intro speech bubbles are text members whose box is
// sized from the auto line height. `pfr_auto_line_height` returns the strike's
// INK EXTENT (cap top -> descender bottom), but the renderer draws each glyph at
// its raw atlas-cell row, so a line's ink occupies `[line_top + cap_top,
// line_top + desc_bottom]`. The box therefore has to reserve `cap_top` rows on
// top of `n * extent`, or the LAST line is clipped by exactly the cell's top
// padding — which for these display faces is most of the glyph.
browser_e2e_test!(test_misc_spectral_wizard_text, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    let snapshots = SnapshotContext::new("misc", "spectral_wizard");

    // The loading plaque ("LOADING / \"IT BEGINS\"") is a two-line auto-leading
    // member and shows the clip on its own, before any interaction.
    player.step_frames(900).await;
    probe(format!("frame after boot = {}", player.current_frame()));
    let _ = snapshots.verify("01_loading", player.snapshot_stage());

    // NEW GAME, the way a player does it: settle the cursor on the item for a
    // few frames so the menu's rollover picks it up, THEN press. Going through
    // the real menu (rather than calling `g.m.startGame()`) is what plays the
    // intro cutscene, and the cutscene's speech bubbles are the members under
    // investigation.
    hover(312, 279);
    player.step_frames(10).await;
    player.click(312, 279).await;
    player.step_frames(10).await;
    probe(format!("after NEW GAME click, mode = {:?}",
        player.eval_datum("string(g._mode)").await));

    // The synthetic click still leaves the menu in `_mode = 2`, so invoke what
    // NEW GAME invokes. `doMenuAction(1)` sets `gotoIntro = 1` and walks the
    // player into the scene — the INTRO cutscene, whose speech bubbles
    // (`parent_talkBox.putTalkBoxWithText`) bake a text member's `.image` and
    // copyPixels it onto the stage. `startGame()` skips all of that, which is
    // why the earlier run went straight to the world map.
    let acted = player.eval_datum("g.m.doMenuAction(1)").await;
    probe(format!("doMenuAction(1) -> {:?}", acted));

    // Run on into the intro cutscene, where the speech bubbles bake their
    // lines through the same path. Snapshot in CHUNKS: this movie is heavy
    // enough that a single long step can eat the harness's 900 s budget, and
    // a killed run still leaves every snapshot written before the stall.
    // The cutscene's beats are paced by `g.doGameTime()`, i.e. WALL-CLOCK
    // milliseconds — stepped frames outrun it, so the later beats (the nephew
    // arriving, the "GREETINGS DEAR NEPHEW" / "HELLO UNCLE" speech bubbles)
    // need a long run to arrive. Snapshot across the whole thing.
    let mut saw_bubble = false;
    for name in [
        "02_intro_a", "03_intro_b", "04_intro_c", "05_intro_d",
        "06_intro_e", "07_intro_f", "08_intro_g", "09_intro_h",
    ] {
        player.step_frames(250).await;
        if player.eval_datum("string(g.oPratBubbla.Active)").await
            .map(|d| format!("{:?}", d).contains('1')).unwrap_or(false)
        {
            saw_bubble = true;
        }
        probe(format!("{} at frame {}", name, player.current_frame()));
        let _ = snapshots.verify(name, player.snapshot_stage());
    }
    // The bubbles ARE the regression target — a run that never raised one would
    // snapshot an empty forest and pass while the defect sat untested.
    assert!(saw_bubble, "the intro cutscene never raised a speech bubble");

    // PLAY on the world map enters the level, via the dark
    // "LOADING / \"It Begins...\"" plaque — the other auto-leading member.
    player.click(600, 33).await;
    for name in ["10_level_a", "11_level_b", "12_level_c"] {
        player.step_frames(150).await;
        probe(format!("{} at frame {}", name, player.current_frame()));
        let _ = snapshots.verify(name, player.snapshot_stage());
    }

    Ok(())
});
