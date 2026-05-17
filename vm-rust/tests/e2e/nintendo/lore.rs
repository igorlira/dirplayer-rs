use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_lore.toml");

browser_e2e_test!(test_nintendo_lore_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "lore");
    snapshots.pixel_tolerance = 10;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    // wait until showing game instructions
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(144))).timeout(30.0).await?;

    snapshots.verify_with_ratio("instructions", player.snapshot_stage(), 0.12)?;

    // click the play button to start the game
    player.click_sprite(sprite().number(14)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(676))).timeout(30.0).await?;

    player.click_sprite(sprite().number(16)).await?;

    snapshots.verify_with_ratio("question_01", player.snapshot_stage(), 0.05)?;

    player.click_sprite(sprite().number(14)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(503))).timeout(30.0).await?;

    player.step_until(datum("sprite(5).member.text").equals(StaticDatum::String("CORRECT".into()))).timeout(10.0).await?;

    snapshots.verify("question_01_result", player.snapshot_stage())?;

    Ok(())
});
