use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/intel_mrm_lod.toml");

browser_e2e_test!(test_intel_mrm_lod_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "mrm_lod");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(3))).timeout(10.0).await?;

    snapshots.verify("start_game", player.snapshot_stage())?;

    Ok(())
});
