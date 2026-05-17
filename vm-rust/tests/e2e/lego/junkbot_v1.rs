use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_junkbot_v1.toml");

browser_e2e_test!(test_junkbot_v1_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "junkbot_v1");
    snapshots.max_diff_ratio = 0.01;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(7))).timeout(20.0).await?;

    player.step_until(sprite().member("skip_intro").visible(1.0)).await?;

    snapshots.verify("game_start", player.snapshot_stage())?;

    player.click_sprite(sprite().member("skip_intro")).await?; // SKIP INTRO

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(7))).timeout(15.0).await?;

    player.step_until(sprite().number(17).visible(1.0)).await?; // CREDITS BTN

    player.step_frames(600).await;

    snapshots.verify_with_ratio("menu", player.snapshot_stage(), 0.07)?;


    player.mouse_move(354, 172).await;

    player.step_frames(10).await;

    player.click_sprite(sprite().number(17)).await?; // CREDITS

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(29))).timeout(15.0).await?;

    player.step_frames(10).await;

    snapshots.verify("credits", player.snapshot_stage())?;

    
    player.mouse_move(556, 331).await;

    player.step_frames(10).await;

    player.click_sprite(sprite().number(28)).await?; // HALL OF FAME

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(21))).timeout(15.0).await?;

    snapshots.verify("hall_of_fame", player.snapshot_stage())?;


    player.mouse_move(558, 361).await;

    player.step_frames(10).await;

    player.click_sprite(sprite().number(29)).await?; // HELP

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(24))).timeout(15.0).await?;

    snapshots.verify("help_page_01", player.snapshot_stage())?;


    player.mouse_move(558, 361).await;

    player.step_frames(10).await;

    player.click_sprite(sprite().number(14)).await?; // halloffame_next_button

    player.step_until(sprite().member("haz_slickJump_dormant_1").visible(1.0)).await?;

    snapshots.verify("help_page_02", player.snapshot_stage())?;



    player.mouse_move(387, 361).await;

    player.step_frames(10).await;

    player.click_sprite(sprite().number(15)).await?; // halloffame_ok_button

    player.step_until(sprite().member("opening_memo").visible(1.0)).await?;

    snapshots.verify("welcome", player.snapshot_stage())?;

    
    player.mouse_move(231, 347).await;

    player.step_frames(10).await;

    player.click_sprite(sprite().number(74)).await?; // halloffame_ok_button


    player.step_until(sprite().member("building_icon_1").visible(1.0)).await?;

    player.step_frames(25).await;

    snapshots.verify("level_overview", player.snapshot_stage())?;

    player.mouse_move(49, 98).await;

    player.step_frames(10).await;

    player.click_sprite(sprite().number(40)).await?; // NEW EMPLOYEE TRAINING

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(14))).timeout(15.0).await?;

    player.step_frames(75).await;

    snapshots.verify("in_game", player.snapshot_stage())?;

    Ok(())
});
