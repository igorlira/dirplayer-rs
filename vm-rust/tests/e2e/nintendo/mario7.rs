use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_mario7.toml");

browser_e2e_test!(test_nintendo_mario7_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "mario7");
    snapshots.max_diff_ratio = 0.05;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("init_game", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(2))).timeout(10.0).await?;

    snapshots.verify("start_game", player.snapshot_stage())?;

    player.click_sprite(sprite().member("Opening text")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(17))).timeout(10.0).await?;

    snapshots.verify("in_match", player.snapshot_stage())?;

    Ok(())
});
