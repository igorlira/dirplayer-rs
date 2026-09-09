use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_rasterwerks.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// PROBE: PHOSPHOR beta2's settings menu (six tabs). Its F_Settings does a great
// deal of work in `beginSprite` -- C_NetText, cActorSys.Settings, C_GlobalHostList,
// cNetLobby.StartSession -- all BEFORE `SwitchPage(me, 1)`, which is what builds
// `plSprUIPos` and what `prepareFrame` then lays the UI out from. A raise
// anywhere in that prologue leaves every UI sprite at its authored score
// position instead of `plSprUIPos + pViewportShift`.
browser_e2e_test!(test_misc_rasterwerks_settings_beta, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    let snapshots = SnapshotContext::new("misc", "rasterwerks_settings_beta");

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

    // F_Start's SETTINGS hotspot is sprite 26.
    let mut in_settings = false;
    for _ in 0..20 {
        click_sprite!(26);
        player.step_frames(10).await;
        if ask!("the frameLabel").starts_with("SET") { in_settings = true; break; }
    }
    assert!(in_settings, "never reached the settings frames (label {})", ask!("the frameLabel"));
    player.step_frames(15).await;

    probe(format!("SET1: pTabPage={} pViewportShift={} plSprUIPos.count={} pDoOnce={}",
        ask!("gFS.pTabPage"), ask!("gFS.pViewportShift"),
        ask!("gFS.plSprUIPos.count"), ask!("gFS.pDoOnce")));
    for s in [1, 11, 12, 13, 14, 15, 16, 31, 32, 33, 34, 35, 36] {
        probe(format!("SET1 sprite({}) loc={} member={}", s,
            ask!("sprite({}).loc", s), ask!("sprite({}).member", s)));
    }
    let _ = snapshots.verify("01_set1", player.snapshot_stage());

    macro_rules! tab {
        ($page:expr) => {{
            for _ in 0..8 {
                if ask!("gFS.pTabPage") == $page.to_string() { break; }
                click_sprite!(26 + $page);
                player.step_frames(10).await;
            }
            probe(format!("tab {} -> label={} pTabPage={} plSprUIPos.count={} sprite(11).loc={}",
                $page, ask!("the frameLabel"), ask!("gFS.pTabPage"),
                ask!("gFS.plSprUIPos.count"), ask!("sprite(11).loc")));
        }};
    }

    // Tabs are hotspot sprites 27..32 -> SwitchPage(rollover - 26).
    for page in 2..=6 {
        tab!(page);
        assert_eq!(ask!("gFS.pTabPage"), page.to_string(),
            "clicking tab {} did not switch the page", page);
        let _ = snapshots.verify(&format!("0{}_set{}", page, page), player.snapshot_stage());
    }

    // ...and switching back and forth must keep re-laying the UI out.
    for p in [1, 4, 2, 6, 3, 1] {
        tab!(p);
        let l = ask!("sprite(11).loc");
        assert_ne!(l, "point(0, 0)",
            "after switching to tab {} the UI was never re-laid out (sprite 11 at {})", p, l);
    }
    let _ = snapshots.verify("07_after_reswitch", player.snapshot_stage());

    Ok(())
});
