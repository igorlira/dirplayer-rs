use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_junkbot_v2-scaled.toml");

// Junkbot Undercover with the stage SCALED — mirrors every screen `junkbot_v2`
// visits. See `junkbot_v1_scaled` for what the level overview is testing.
browser_e2e_test!(test_junkbot_v2_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "junkbot_v2_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(10))).timeout(20.0).await?;
    player.step_until(sprite().member("skip_intro").visible(1.0)).await?;
    let _ = snapshots.verify("01_game_start", player.snapshot_stage());

    player.click_sprite(sprite().member("skip_intro")).await?;
    player.step_until(sprite().number(17).visible(1.0)).await?;
    player.step_frames(425).await;
    let _ = snapshots.verify("02_menu", player.snapshot_stage());

    player.click_sprite(sprite().number(17)).await?; // CREDITS
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(33))).timeout(15.0).await?;
    player.step_frames(10).await;
    let _ = snapshots.verify("03_credits", player.snapshot_stage());

    player.click_sprite(sprite().number(28)).await?; // HALL OF FAME
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(24))).timeout(15.0).await?;
    let _ = snapshots.verify("04_hall_of_fame", player.snapshot_stage());

    player.click_sprite(sprite().number(29)).await?; // HELP
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(27))).timeout(15.0).await?;
    let _ = snapshots.verify("05_help_page_01", player.snapshot_stage());

    player.click_sprite(sprite().number(14)).await?;
    player.step_until(sprite().member("haz_slickJump_dormant_1").visible(1.0)).await?;
    let _ = snapshots.verify("06_help_page_02", player.snapshot_stage());

    player.click_sprite(sprite().number(16)).await?;
    player.step_until(sprite().member("haz_slickPipe_wet_7").visible(1.0)).await?;
    let _ = snapshots.verify("07_help_page_03", player.snapshot_stage());

    player.click_sprite(sprite().number(15)).await?;
    player.step_until(sprite().member("opening_memo").visible(1.0)).await?;
    let _ = snapshots.verify("08_welcome", player.snapshot_stage());

    player.click_sprite(sprite().number(74)).await?;
    player.step_frames(50).await;
    let _ = snapshots.verify("09_level_overview", player.snapshot_stage());

    player.click_sprite_at(sprite().number(4), 187, 20).await?; // DESCENT
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(16))).timeout(15.0).await?;
    player.step_frames(100).await;
    let _ = snapshots.verify("10_in_game", player.snapshot_stage());

    Ok(())
});
