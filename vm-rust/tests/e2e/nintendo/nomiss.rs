use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_nomiss.toml");

browser_e2e_test!(test_nintendo_nomiss_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "nomiss");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    // wait until showing game instructions
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(3))).timeout(20.0).await?;

    snapshots.verify("instructions", player.snapshot_stage())?;

    // click the play button to start the game
    player.click_sprite(sprite().member("play_button")).await?;

    // wait for first color to show up
    player.step_frames(5).await;

    snapshots.verify("in_match", player.snapshot_stage())?;

    player.step_frames(25).await;


    // # Round 1

    // click blue
    player.click_sprite(sprite().number(12)).await?;

    player.step_frames(75).await;


    // # Round 2

    // click blue
    player.click_sprite(sprite().number(12)).await?;

    // click green
    player.click_sprite(sprite().number(13)).await?;

    player.step_frames(100).await;


    // # Round 3

    // click blue
    player.click_sprite(sprite().number(12)).await?;

    // click green
    player.click_sprite(sprite().number(13)).await?;

    // click brown
    player.click_sprite(sprite().number(14)).await?;

    player.step_frames(125).await;


    // # Round 4

    // click blue
    player.click_sprite(sprite().number(12)).await?;

    // click green
    player.click_sprite(sprite().number(13)).await?;

    // click brown
    player.click_sprite(sprite().number(14)).await?;

    // click purple
    player.click_sprite(sprite().number(15)).await?;

    player.step_frames(150).await;


    // # Round 5

    // click blue
    player.click_sprite(sprite().number(12)).await?;

    // click green
    player.click_sprite(sprite().number(13)).await?;

    // click brown
    player.click_sprite(sprite().number(14)).await?;

    // click purple
    player.click_sprite(sprite().number(15)).await?;

    // click blue
    player.click_sprite(sprite().number(12)).await?;


    // # Match 1 finished

    player.step_until(sprite().member("playAgain_button").visible(1.0)).timeout(30.0).await?;

    snapshots.verify("match_result", player.snapshot_stage())?;

    Ok(())
});
