use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/hotwheels_rw_flag.toml");

browser_e2e_test!(test_hotwheels_rw_flag_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "rw_flag");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(sprite().member("left platform").visible(1.0)).await?;
    snapshots.verify("init", player.snapshot_stage())?;

    player.step_until(sprite().member_prefix("straightDown45").visible(1.0)).await?;
    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
