use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/intel_do_not_push.toml");

browser_e2e_test!(test_intel_do_not_push_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "do_not_push");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    Ok(())
});
