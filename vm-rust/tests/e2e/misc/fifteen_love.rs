use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_15_love.toml");

browser_e2e_test!(test_fifteen_love_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "15_love");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(35))).timeout(120.0).await?;

    snapshots.verify("game_loading", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(43))).timeout(120.0).await?;

    snapshots.verify("game_start", player.snapshot_stage())?;


    player.click_sprite(sprite().member("start_a")).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(60))).timeout(60.0).await?;

    snapshots.verify("game_control", player.snapshot_stage())?;


    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(99))).timeout(60.0).await?;

    snapshots.verify("choose_player", player.snapshot_stage())?;

    player.step_frames(75).await;

    player.click_sprite(sprite().number(75)).await?; // chooseplayer: MONIQ

    player.step_frames(75).await;

    snapshots.verify("choose_player_selected", player.snapshot_stage())?;

    player.click_sprite(sprite().number(80)).await?; // play off | play on


    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(105))).timeout(60.0).await?;

    snapshots.verify("game_round", player.snapshot_stage())?;


    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(106))).timeout(60.0).await?;

    snapshots.verify("good_luck", player.snapshot_stage())?;


    player.step_frames(375).await;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
