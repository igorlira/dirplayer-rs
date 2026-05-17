use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/sulake_mobiles_disco.toml");

browser_e2e_test!(test_mobilesdisco_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "mobiles_disco");
    snapshots.max_diff_ratio = 0.05;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(98))).timeout(15.0).await?;

    player.step_frames(1).await;

    snapshots.verify("init", player.snapshot_stage())?;

    player.click_sprite(sprite().member("info_btn")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(104))).timeout(15.0).await?;

    player.step_frames(1).await;

    snapshots.verify("credits", player.snapshot_stage())?;

    player.click_sprite(sprite().member("alkuun_btn")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(98))).timeout(15.0).await?;

    player.click_sprite(sprite().member("reg_btn")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(110))).timeout(15.0).await?;

    player.step_frames(1).await;

    snapshots.verify("registration", player.snapshot_stage())?;

    Ok(())
});
