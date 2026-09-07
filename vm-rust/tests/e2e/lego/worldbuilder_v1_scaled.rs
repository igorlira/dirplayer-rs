use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_worldbuilder_v1-scaled.toml");

// WorldBuilder with the stage SCALED — see `junkbot_v1_scaled`.
browser_e2e_test!(test_worldbuilder_v1_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "worldbuilder_v1_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    player.step_frames(30).await;
    let _ = snapshots.verify("01_start", player.snapshot_stage());

    player.click_sprite(sprite().member("large_orange_button")).await?;
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(5))).timeout(15.0).await?;
    let _ = snapshots.verify("02_world_one", player.snapshot_stage());

    player.click_sprite(sprite().member("question_mark")).await?;
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(11))).timeout(15.0).await?;
    player.step_frames(50).await;
    let _ = snapshots.verify("03_in_game", player.snapshot_stage());

    Ok(())
});
