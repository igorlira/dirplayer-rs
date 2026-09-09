use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness, sprite};

const CONFIG: &str = include_str!("../configs/sulake_habbo_v26-scaled.toml");
const TEST_NAME: &str = "habbo_v26_scaled";

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Habbo v26 with the stage SCALED (`swStretchStyle = meet`) — the layout
// fullscreen uses. 720x540 into 1920x1080 gives 2.0x, letterboxed 240px each
// side.
//
// Habbo positions its own UI from `the stage.rect` / stageLeft..stageBottom —
// `Core Thread Class` centres the boot logo with
//     point((the stage).rect.width / 2, ((the stage).rect.height / 2) - h)
// and `Window Instance Class` centres every window the same way. Those must
// resolve in MOVIE coordinates; a container-sized stage rect throws all of them
// off toward the bottom-right.
browser_e2e_test!(test_habbo_v26_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();

    let snapshots = SnapshotContext::new(cfg.suite(), TEST_NAME);

    // Deliberately NOT `shared::assert_entry`: it loads the movie and snapshots
    // the boot itself, and with `swStretchStyle = meet` the canvas follows the
    // stage size — which is still the harness default until we set it — so those
    // shots would come out at the default size and mean nothing.
    let movie_path = player.asset_path(&cfg.movie.path);
    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);
    player.step_until(sprite().member("Logo").visible(1.0)).timeout(120.0).await?;

    probe(vm_rust::player::reserve_player_ref(|p| {
        let l = vm_rust::player::stage::stage_layout(p);
        let (sx, sy) = vm_rust::player::stage::stage_scale(p);
        format!("movie={}x{} canvas={}x{} stage_rect={:?} draw={:?} scale=({:.4},{:.4})",
            p.movie.rect.width(), p.movie.rect.height(),
            l.canvas_width, l.canvas_height, l.stage_rect, l.draw_rect, sx, sy)
    }));

    // The Lingo-facing stage must stay the movie's own rect.
    let stage_rect = match player.eval_datum("string((the stage).rect)").await {
        Ok(vm_rust::director::static_datum::StaticDatum::String(s)) => s,
        other => format!("<{:?}>", other),
    };
    assert_eq!(stage_rect, "rect(0, 0, 720, 540)",
        "(the stage).rect is {} on a scaled stage — Habbo centres every window \
         against it, so a container-sized rect puts them all off-screen", stage_rect);

    // The snapshot carries the other half: the movie must occupy exactly
    // `draw_rect` with clean letterbox bars either side. Habbo's hotel backdrop
    // is wider than its 720px stage, and nothing clips sprites to the movie
    // bounds — the canvas edge normally does that implicitly — so before the
    // fix the right-hand bar filled with scene while the left stayed black.
    let _ = snapshots.verify("01_entry_scaled", player.snapshot_stage());

    // Logged in, where the windows are: Habbo centres each one from
    // `the stage.rect`, so this is what the assertion above is protecting.
    player.step_until(sprite().member("entry_bar_ownhabbo_icon_image").visible(1.0)).timeout(150.0).await?;
    let _ = snapshots.verify("02_login_scaled", player.snapshot_stage());

    Ok(())
});
