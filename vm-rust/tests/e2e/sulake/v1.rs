use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/sulake_habbo_v1.toml");

browser_e2e_test!(test_habbo_v1_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "habbo_v1");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(sprite().member("loginname").visible(1.0)).timeout(200.0).await?;
    snapshots.verify("init", player.snapshot_stage())?;

    Ok(())
});
