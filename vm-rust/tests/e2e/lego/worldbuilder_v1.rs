use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_worldbuilder_v1.toml");

browser_e2e_test!(test_01_worldbuilder1_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "worldbuilder_v1");
    snapshots.max_diff_ratio = 0.005;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("init", player.snapshot_stage())?;

    player.step_frames(30).await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    player.click_sprite(sprite().member("large_orange_button")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(5))).timeout(15.0).await?;

    snapshots.verify("world_one", player.snapshot_stage())?;

    player.click_sprite(sprite().member("question_mark")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(11))).timeout(15.0).await?;

    player.step_frames(50).await;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
