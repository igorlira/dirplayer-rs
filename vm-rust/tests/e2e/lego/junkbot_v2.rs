use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_junkbot_v2.toml");

browser_e2e_test!(test_junkbot_v2_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "junkbot_v2");
    snapshots.max_diff_ratio = 0.02;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(10))).timeout(20.0).await?;

    player.step_until(sprite().member("skip_intro").visible(1.0)).await?;

    snapshots.verify_with_ratio("game_start", player.snapshot_stage(), 0.8)?;

    player.click_sprite(sprite().member("skip_intro")).await?; // SKIP INTRO

    player.step_until(sprite().number(17).visible(1.0)).await?; // CREDITS BTN

    player.step_frames(425).await;

    snapshots.verify("menu", player.snapshot_stage())?;


    player.click_sprite(sprite().number(17)).await?; // CREDITS

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(33))).timeout(15.0).await?;

    player.step_frames(10).await;

    snapshots.verify("credits", player.snapshot_stage())?;

    
    player.click_sprite(sprite().number(28)).await?; // HALL OF FAME

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(24))).timeout(15.0).await?;

    snapshots.verify("hall_of_fame", player.snapshot_stage())?;


    player.click_sprite(sprite().number(29)).await?; // HELP2

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(27))).timeout(15.0).await?;

    snapshots.verify("help_page_01", player.snapshot_stage())?;


    player.click_sprite(sprite().number(14)).await?; // halloffame_next_button

    player.step_until(sprite().member("haz_slickJump_dormant_1").visible(1.0)).await?;
    
    snapshots.verify("help_page_02", player.snapshot_stage())?;



    player.click_sprite(sprite().number(16)).await?; // halloffame_next_button

    player.step_until(sprite().member("haz_slickPipe_wet_7").visible(1.0)).await?;

    snapshots.verify("help_page_03", player.snapshot_stage())?;




    player.click_sprite(sprite().number(15)).await?; // halloffame_ok_button

    player.step_until(sprite().member("opening_memo").visible(1.0)).await?;

    snapshots.verify("welcome", player.snapshot_stage())?;

    
    player.click_sprite(sprite().number(74)).await?; // halloffame_ok_button

    player.step_frames(10).await;

    snapshots.verify("level_overview", player.snapshot_stage())?;



    player.click_sprite_at(sprite().number(4), 187, 20).await?; // DESCENT

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(16))).timeout(15.0).await?;

    player.step_frames(300).await; // Wait for intro to settle

    snapshots.verify_with_ratio("in_game", player.snapshot_stage(), 0.05)?;

    Ok(())
});
