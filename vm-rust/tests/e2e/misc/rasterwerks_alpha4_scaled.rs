use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_rasterwerks_alpha4-scaled.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Rasterwerks PHOSPHOR alpha 4 with the stage SCALED (`swStretchStyle = meet`).
//
// This movie's whole HUD is 3D CAMERA OVERLAYS — `C_OverlaySys` does
// `pCam.addOverlay(tex, point(x, y), 0)` and drives `overlay[n].loc/regPoint/
// scale` per frame. Overlays are positioned in movie pixels but drawn into the
// sprite's enlarged render rect, the same as camera backdrops, so this is the
// coverage for the overlay half of that fix (estate only exercised backdrops).
browser_e2e_test!(test_misc_rasterwerks_alpha4_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new("misc", "rasterwerks_alpha4_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    macro_rules! ask {
        ($($t:tt)*) => {{
            let e = format!($($t)*);
            match player.eval_datum(&format!("string({})", e)).await {
                Ok(vm_rust::director::static_datum::StaticDatum::String(s)) => s,
                other => format!("<{:?}>", other),
            }
        }};
    }

    let mut at_menu = false;
    for _ in 0..45 {
        player.step_frames(50).await;
        if ask!("the frameLabel") == "Start" { at_menu = true; break; }
    }
    assert!(at_menu, "boot never reached the Start menu (label {})", ask!("the frameLabel"));
    let _ = snapshots.verify("01_menu", player.snapshot_stage());

    // START -> the game, where the HUD overlays live. Re-read the button's loc
    // each attempt: F_Start lays its UI out from `prepareFrame`, so a loc read
    // before that frame runs points where the button is not.
    let mut started = false;
    for _ in 0..40 {
        let loc = ask!("sprite(23).loc");
        let xy: Vec<i32> = loc.trim_start_matches("point(").trim_end_matches(')')
            .split(',').filter_map(|p| p.trim().split('.').next().unwrap_or("").parse().ok()).collect();
        if xy.len() == 2 { player.click(xy[0], xy[1]).await; }
        player.step_frames(60).await;
        if !ask!("cPlayer.pGameState").is_empty() { started = true; break; }
    }
    assert!(started, "never left the Start menu — cPlayer.pGameState is still VOID");

    let mut live = false;
    for _ in 0..60 {
        if ask!("cPlayer.pGameState") == "LIVE" { live = true; break; }
        player.step_frames(30).await;
    }
    assert!(live, "never reached gameplay — pGameState is {}", ask!("cPlayer.pGameState"));

    player.step_frames(120).await;
    probe(vm_rust::player::reserve_player_ref(|p| {
        use vm_rust::player::cast_member::CastMemberType;
        let l = vm_rust::player::stage::stage_layout(p);
        let (sx, _) = vm_rust::player::stage::stage_scale(p);
        let mut n_ovl = 0usize;
        for cast in p.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if let CastMemberType::Shockwave3d(w3d) = &m.member_type {
                    n_ovl += w3d.runtime_state.camera_overlays.values().map(|v| v.len()).sum::<usize>();
                }
            }
        }
        format!("movie={}x{} canvas={}x{} draw={:?} scale={:.4} camera_overlays={}",
            p.movie.rect.width(), p.movie.rect.height(),
            l.canvas_width, l.canvas_height, l.draw_rect, sx, n_ovl)
    }));

    let _ = snapshots.verify("02_hud", player.snapshot_stage());
    Ok(())
});
