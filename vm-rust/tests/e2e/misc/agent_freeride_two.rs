use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, SnapshotOutput, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_agent_freeride2.toml");

fn probe(msg: String) {
    // `println!` from wasm goes nowhere; the page console is what E2E_CONSOLE
    // forwards to the terminal.
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Miniclip: Agent Free Ride 2 — Shockwave 3D + AGEIA PhysX (Dynamiks.x32), D11.5.
//
// Two things gate this movie:
//  1. Frame 1 hardcodes `gConfiguration = #release_web` and runs a domain check.
//     With no Miniclip shell there is no MiniclipGameManager, so `validDomainSBS()`
//     is the only way past it — hence the `_moviePath` in the config. Failing it
//     lands on `go(85)` = "Frame Invalid Domain", an infinite `go(the frame)`.
//  2. Everything from frame 2 on is driven by an embedded SWF calling back into
//     Lingo (flash_start_checker / flash_start_intro / flash_start), so the menu
//     has to be clicked — the same pattern as Agent Free Ride 1.
//
// Gameplay lives on frame 45. The level-start sequence runs on GAME time (a
// prerender hold plus a 2500 ms camera fade-in), so it takes a few hundred
// stepped frames before `GetGameStatus()` reaches #play.
browser_e2e_test!(test_misc_agent_freeride_two_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    // Settle on the off-game menu (frames 8..13).
    player.step_frames(80).await;
    probe(format!("menu frame={}", player.current_frame()));
    assert_ne!(player.current_frame(), 85,
        "domain check failed — stuck on Frame Invalid Domain");

    let snapshots = SnapshotContext::new("misc", "agent_freeride_two");
    let _ = snapshots.verify("01_offgame_menu", player.snapshot_stage());

    // PLAY on the title screen, then GO on the level briefing. Both buttons live
    // inside the full-stage SWF, so they're clicked by stage coordinate.
    //
    // HARNESS WORKAROUND (same spirit as rifleman's intro gate): the SWF runs on
    // Ruffle's wall clock, and under the movie-load warm-up it starts at BIND —
    // sprite begin, the Director-correct moment — instead of enjoying the old
    // ~3 s blocked-creation head start. A single blind click can therefore land
    // before the button's frame is armed. Wait for the SWF to park (stable
    // `_currentframe`), then click-and-retry until its playhead moves on.
    macro_rules! swf_frame {
        () => { ask!("string(sprite(2).getVariable(\"_currentframe\"))") };
    }
    let mut armed = swf_frame!();
    for _ in 0..40 {
        player.step_frames(5).await;
        let cur = swf_frame!();
        if cur == armed && !cur.contains("\"0\"") { break; }
        armed = cur;
    }
    for _ in 0..20 {
        player.click(325, 337).await;
        player.step_frames(20).await;
        if swf_frame!() != armed { break; }
    }
    let _ = snapshots.verify("02_after_play", player.snapshot_stage());

    let mut briefing = swf_frame!();
    for _ in 0..40 {
        player.step_frames(5).await;
        let cur = swf_frame!();
        if cur == briefing && !cur.contains("\"0\"") { break; }
        briefing = cur;
    }
    let menu_frame = player.current_frame();
    for _ in 0..20 {
        player.click(560, 415).await;
        player.step_frames(20).await;
        if player.current_frame() != menu_frame || swf_frame!() != briefing { break; }
    }

    // Streaming load: 14 preloads, 15+ loops until member(1).state = 4, then 45.
    for _ in 0..40 {
        if player.current_frame() == 45 {
            break;
        }
        player.step_frames(10).await;
    }
    assert_eq!(player.current_frame(), 45, "never reached the gameplay frame");

    player.step_frames(440).await;
    let _ = snapshots.verify("03_gameplay", player.snapshot_stage());

    Ok(())
});

// The HUD energy bar empties through its ALPHA channel alone — InGame.
// UpdateEnergyBar never touches the bar's RGB:
//
//   lAlpha = pEnergyBar.alpha.duplicate()        -- from image.extractAlpha()
//   lAlpha.fill(0, 16, 16, 16 + lValue, color(0))
//   lImage = pEnergyBar.color.duplicate()
//   lImage.useAlpha = 1
//   lImage.setAlpha(lAlpha)
//   pEnergyBar.texture.image = lImage            -- #fromImageObject texture, drawn as an overlay
//
// `extractAlpha()` used to return a 32-bit image while `setAlpha(alphaImage)`
// correctly requires 8-bit (Director 11.5 Scripting Dictionary, both entries),
// so setAlpha was a silent no-op and the bar sat at full no matter how much
// damage the player took. Damage itself was never the problem — the player
// could still die.
browser_e2e_test!(test_misc_agent_freeride_two_energy_bar, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    player.step_frames(80).await;
    // Same armed-frame wait + click-retry as the load test above — see the
    // HARNESS WORKAROUND comment there.
    macro_rules! swf_frame {
        () => { ask!("string(sprite(2).getVariable(\"_currentframe\"))") };
    }
    let mut armed = swf_frame!();
    for _ in 0..40 {
        player.step_frames(5).await;
        let cur = swf_frame!();
        if cur == armed && !cur.contains("\"0\"") { break; }
        armed = cur;
    }
    for _ in 0..20 {
        player.click(325, 337).await;
        player.step_frames(20).await;
        if swf_frame!() != armed { break; }
    }
    let mut briefing = swf_frame!();
    for _ in 0..40 {
        player.step_frames(5).await;
        let cur = swf_frame!();
        if cur == briefing && !cur.contains("\"0\"") { break; }
        briefing = cur;
    }
    let menu_frame = player.current_frame();
    for _ in 0..20 {
        player.click(560, 415).await;
        player.step_frames(20).await;
        if player.current_frame() != menu_frame || swf_frame!() != briefing { break; }
    }
    for _ in 0..40 {
        if player.current_frame() == 45 {
            break;
        }
        player.step_frames(10).await;
    }
    assert_eq!(player.current_frame(), 45, "never reached the gameplay frame");
    player.step_frames(440).await;
    assert_eq!(ask!("gGame.GetGameStatus()"), "Symbol(\"play\")",
        "level start never reached #play — OnHit early-returns on any other status");

    // extractAlpha must hand setAlpha something it will accept.
    assert_eq!(ask!("gGame.GetIngame().pEnergyBar.alpha.depth"), "Int(8)",
        "extractAlpha() must return an 8-bit grayscale image");
    assert_eq!(ask!("member(\"h_energybar\").image.duplicate().setAlpha(member(\"h_energybar\").image.extractAlpha())"), "Int(1)",
        "setAlpha(extractAlpha()) must succeed — the dictionary's own idiom");

    // Damage lands...
    let _ = player.eval("gGame.GetPlayerVehicle().OnHit(20.0, gGame.GetPlayerVehicle().getPosition(), vector(0.0, 0.0, 1.0))").await;
    assert_eq!(ask!("gGame.GetPlayerVehicle().GetEnergy()"), "Float(88.0)",
        "OnHit must drain energy by damage * pEnergyCoeff (0.6 for #player)");

    // ...and the bar on screen must actually follow it.
    fn bar_pixels(out: SnapshotOutput) -> Vec<u8> {
        match out.crop(0, 230, 60, 380) {
            SnapshotOutput::Rgba { data, .. } => data,
            SnapshotOutput::Base64Png(b64) => b64.into_bytes(),
        }
    }
    let _ = player.eval("gGame.GetIngame().UpdateEnergyBar(100)").await;
    player.step_frames(3).await;
    let full = bar_pixels(player.snapshot_stage());
    let _ = player.eval("gGame.GetIngame().UpdateEnergyBar(0)").await;
    player.step_frames(3).await;
    let empty = bar_pixels(player.snapshot_stage());
    assert_ne!(full, empty, "energy bar renders identically at 100 and 0 — HUD not updating");

    Ok(())
});
