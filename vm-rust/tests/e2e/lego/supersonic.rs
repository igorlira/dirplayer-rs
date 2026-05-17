use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_supersonic.toml");

browser_e2e_test!(test_supersonic_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "supersonic");
    snapshots.max_diff_ratio = 0.05;
    snapshots.pixel_tolerance = 30;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(90))).timeout(15.0).await?;

    player.step_frames(100).await;

    snapshots.verify("game_start", player.snapshot_stage())?;

    player.click_sprite(sprite().number(61)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(80))).timeout(15.0).await?;

    player.step_frames(100).await;

    snapshots.verify("game_menu", player.snapshot_stage())?;

    player.click_sprite(sprite().number(63)).await?; // FREE DRIVING

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(95))).timeout(15.0).await?;

    player.step_frames(100).await;

    snapshots.verify("game_control", player.snapshot_stage())?;

    player.click_sprite(sprite().number(67)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(70))).timeout(15.0).await?;

    player.step_frames(100).await;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
