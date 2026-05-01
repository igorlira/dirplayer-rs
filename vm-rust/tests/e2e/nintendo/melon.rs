use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_melon.toml");

browser_e2e_test!(test_nintendo_melon_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "melon");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("init_game", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(1))).timeout(10.0).await?;

    player.step_frames(1).await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    player.step_frames(15).await;

    player.click_sprite(sprite().member("play_button")).await?;

    player.step_frames(5).await;

    snapshots.verify("in_match", player.snapshot_stage())?;

    player.click_sprite(sprite().number(11)).await?;
    player.click_sprite(sprite().number(13)).await?;
    player.click_sprite(sprite().number(14)).await?;

    // wait until play again button shows up
    player.step_until(sprite().member("playAgain_button").visible(1.0)).timeout(45.0).await?;

    snapshots.verify("match_result", player.snapshot_stage())?;

    Ok(())
});
