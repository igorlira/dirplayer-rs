use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{sprite, datum, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/habbo_v1.toml");

browser_e2e_test!(test_habbo_v1_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "load");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(sprite().member("loginname").visible(1.0)).await?;
    snapshots.verify("init", player.snapshot_stage())?;

    Ok(())
});
