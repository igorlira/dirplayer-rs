use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/sulake_lumisota.toml");

browser_e2e_test!(test_lumisota_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "lumisota");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(2))).timeout(15.0).await?;

    snapshots.verify("init", player.snapshot_stage())?;

    player.click_sprite(sprite().member("newchar")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(81))).timeout(15.0).await?;

    snapshots.verify("new_character", player.snapshot_stage())?;

    Ok(())
});
