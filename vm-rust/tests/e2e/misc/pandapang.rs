use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_pandapang.toml");

browser_e2e_test!(test_03_pandapang_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "pandapang");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // wait for loading screen to appear
    player.step_frames(2).await;
    // verify loading screenshot
    snapshots.verify("loading_state", player.snapshot_stage())?;
    // wait until back to main menu frame 20
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(20))).timeout(10.0).await?;
    // verify in game screenshot
    snapshots.verify("in_game", player.snapshot_stage())?;

    // click instructions
    player.click_sprite(sprite().member("instBtn")).await?;
    // wait until frame 175 is reached
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(175))).timeout(10.0).await?;
    // verify instructions screenshot
    snapshots.verify("instructions", player.snapshot_stage())?;
    // navigate back to main menu
    player.click_sprite(sprite().member("backBtn")).await?;
    // wait until back to main menu frame 20
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(20))).timeout(10.0).await?;

    // click on items
    player.click_sprite(sprite().member("itemBtn")).await?;
    // wait until frame 215 is reached
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(215))).timeout(10.0).await?;
    // verify items screenshot
    snapshots.verify("items", player.snapshot_stage())?;
    // navigate back to main menu
    player.click_sprite(sprite().member("backBtn")).await?;
    // wait until back to main menu frame 20
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(20))).timeout(10.0).await?;

    // click on start game
    player.click_sprite(sprite().member("startBtn")).await?;
    // wait until frame 50 is reached
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(50))).timeout(10.0).await?;
    // wait until the invader is touching the ground
    player.step_until(sprite().member("b06bound").visible(1.0)).timeout(30.0).await?;
    // verify in match screenshot
    snapshots.verify("in_match", player.snapshot_stage())?;

    Ok(())
});
