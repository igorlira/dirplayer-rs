use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/hotwheels_figure8.toml");

browser_e2e_test!(test_hotwheels_figure8_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "figure8");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_frames(2).await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    // wait until the first car gets some distance
    player.step_frames(5).await;

    // click car 1
    player.click_sprite(sprite().member("car1")).await?;

    // wait until the two cars got more distance
    player.step_frames(30).await;

    // click car 2
    player.click_sprite(sprite().member("car2")).await?;

    // wait until the cars did got some more distance
    player.step_frames(50).await;

    snapshots.verify("in_match", player.snapshot_stage())?;

    Ok(())
});
