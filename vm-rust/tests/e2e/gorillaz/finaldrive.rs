use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/gorillaz_finaldrive.toml");

browser_e2e_test!(test_finaldrive_traction, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "finaldrive");
    snapshots.max_diff_ratio = 0.05;
    snapshots.pixel_tolerance = 30;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("game_start", player.snapshot_stage())?;

    // The movie opens on a menu; sprite 8 enters the 3D scene.
    player.step_frames(20).await;
    player.click_sprite(sprite().number(8)).await?;

    // Let the car spawn, fall and settle on its wheels.
    player.step_frames(220).await;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
