use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/worldbuilder_v1.toml");

browser_e2e_test!(test_worldbuilder1_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "load");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until_sprite_visible(30.0, "large_orange_button", 1.0).await?;
    snapshots.verify("init", player.snapshot_stage())?;

    player.click_member_prefix("large_orange_button").await?;
    player.step_until_sprite_visible(10.0, "landmass_2", 1.0).await?;
    snapshots.verify("world_one", player.snapshot_stage())?;

    player.click_member("question_mark").await?;
    player.step_until_sprite_visible(10.0, "resource.red.3", 1.0).await?;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
