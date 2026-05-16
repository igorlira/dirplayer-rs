use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_blackstreet_pinball.toml");

browser_e2e_test!(test_blackstreet_pinball_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "blackstreet_pinball");
    snapshots.max_diff_ratio = 0.03;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("game_start", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(10))).timeout(15.0).await?;

    player.click_sprite(sprite().number(112)).await?;

    player.step_frames(2).await;

    player.click_sprite(sprite().number(112)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(5))).timeout(15.0).await?;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
