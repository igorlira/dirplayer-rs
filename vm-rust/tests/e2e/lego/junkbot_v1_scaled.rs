use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/lego_junkbot_v1-scaled.toml");

// Junkbot with the stage SCALED (`swStretchStyle = meet`) — the layout
// fullscreen uses. Mirrors every screen `junkbot_v1` visits, so each one can be
// compared against its unscaled counterpart rather than spot-checking a couple.
//
// The screen this exists for is the LEVEL OVERVIEW: its level.num / level.name
// columns take their line stride from the member's XMED par_runs
// (`per_line_spacings`), which the renderer was using RAW. At 3x the glyphs grew
// but the rows did not, so the list closed up on itself. The same table drives
// WorldBuilder's "MISSION 1 / TUTORIAL" label, which printed both lines on top
// of each other.
browser_e2e_test!(test_junkbot_v1_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "junkbot_v1_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(7))).timeout(20.0).await?;
    player.step_until(sprite().member("skip_intro").visible(1.0)).await?;
    let _ = snapshots.verify("01_game_start", player.snapshot_stage());

    player.click_sprite(sprite().member("skip_intro")).await?;
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(7))).timeout(15.0).await?;
    player.step_until(sprite().number(17).visible(1.0)).await?;
    player.step_frames(600).await;
    let _ = snapshots.verify("02_menu", player.snapshot_stage());

    player.mouse_move(354, 172).await;
    player.step_frames(10).await;
    player.click_sprite(sprite().number(17)).await?; // CREDITS
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(29))).timeout(15.0).await?;
    player.step_frames(10).await;
    let _ = snapshots.verify("03_credits", player.snapshot_stage());

    player.mouse_move(556, 331).await;
    player.step_frames(10).await;
    player.click_sprite(sprite().number(28)).await?; // HALL OF FAME
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(21))).timeout(15.0).await?;
    let _ = snapshots.verify("04_hall_of_fame", player.snapshot_stage());

    player.mouse_move(558, 361).await;
    player.step_frames(10).await;
    player.click_sprite(sprite().number(29)).await?; // HELP
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(24))).timeout(15.0).await?;
    let _ = snapshots.verify("05_help_page_01", player.snapshot_stage());

    player.mouse_move(558, 361).await;
    player.step_frames(10).await;
    player.click_sprite(sprite().number(14)).await?;
    player.step_until(sprite().member("haz_slickJump_dormant_1").visible(1.0)).await?;
    let _ = snapshots.verify("06_help_page_02", player.snapshot_stage());

    player.mouse_move(387, 361).await;
    player.step_frames(10).await;
    player.click_sprite(sprite().number(15)).await?;
    player.step_until(sprite().member("opening_memo").visible(1.0)).await?;
    let _ = snapshots.verify("07_welcome", player.snapshot_stage());

    player.mouse_move(231, 347).await;
    player.step_frames(10).await;
    player.click_sprite(sprite().number(74)).await?;
    player.step_until(sprite().member("building_icon_1").visible(1.0)).await?;
    player.step_frames(50).await;
    let _ = snapshots.verify("08_level_overview", player.snapshot_stage());

    player.mouse_move(49, 98).await;
    player.step_frames(10).await;
    player.click_sprite(sprite().number(40)).await?; // NEW EMPLOYEE TRAINING
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(14))).timeout(15.0).await?;
    player.step_frames(75).await;
    let _ = snapshots.verify("09_in_game", player.snapshot_stage());

    Ok(())
});
