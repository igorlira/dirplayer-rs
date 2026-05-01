use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_roboriders_onyx.toml");

browser_e2e_test!(test_roboriders_onyx_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "roboriders_onyx");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("game_start", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(10))).timeout(15.0).await?;

    player.step_frames(5).await;

    snapshots.verify("menu", player.snapshot_stage())?;

    player.click_sprite(sprite().member("swa_score")).await?; // SCORING TIPS

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(140))).timeout(15.0).await?;

    snapshots.verify("instructions_01", player.snapshot_stage())?;

    player.click_sprite(sprite().member("F1")).await?; // NEXT

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(141))).timeout(15.0).await?;

    snapshots.verify("instructions_02", player.snapshot_stage())?;

    player.click_sprite(sprite().member("F1")).await?; // NEXT

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(142))).timeout(15.0).await?;

    snapshots.verify("instructions_03", player.snapshot_stage())?;

    player.click_sprite(sprite().number(67)).await?; // BACK TO GAME

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(10))).timeout(15.0).await?;

    player.step_frames(5).await;

    player.click_sprite(sprite().number(73)).await?; // START LEVEL 1

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(20))).timeout(15.0).await?;

    player.step_until(sprite().member("rider onyx main").visible(1.0)).await?;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
