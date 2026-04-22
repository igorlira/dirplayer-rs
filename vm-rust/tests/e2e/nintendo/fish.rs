use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_fish.toml");

browser_e2e_test!(test_nintendo_fish_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "fish");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(3))).timeout(10.0).await?;

    snapshots.verify("start_game", player.snapshot_stage())?;

    player.click_sprite(sprite().number(11)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(39))).timeout(10.0).await?;

    snapshots.verify("instructions", player.snapshot_stage())?;

    player.click_sprite(sprite().number(10)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(3))).timeout(10.0).await?;

    player.click_sprite(sprite().number(10)).await?;

    player.step_frames(1).await;

    snapshots.verify("in_match", player.snapshot_stage())?;

    Ok(())
});
