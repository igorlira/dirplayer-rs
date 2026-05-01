use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/hd_properties.toml");

browser_e2e_test!(test_havok_demo_properties_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "properties");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(5))).timeout(10.0).await?;

    snapshots.verify("start_game", player.snapshot_stage())?;


    player.click_sprite(sprite().member("Restitution")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(30))).timeout(10.0).await?;

    player.step_frames(250).await;

    snapshots.verify("restitution", player.snapshot_stage())?;


    player.click_sprite(sprite().member("Friction")).await?;
    
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(55))).timeout(10.0).await?;

    player.step_frames(225).await;

    snapshots.verify("friction", player.snapshot_stage())?;

    Ok(())
});
