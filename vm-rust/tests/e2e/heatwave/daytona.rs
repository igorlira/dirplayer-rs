use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/heatwave_daytona.toml");

// Heatwave Racing (hottamales.com arcade) — a Shockwave 3D stock-car race.
// Captures the track as it loads and once the car is rolling, so the W3D
// model/texture work has a reference that isn't just the menu.
browser_e2e_test!(test_heatwave_daytona_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "daytona");
    snapshots.max_diff_ratio = 0.03;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(7))).timeout(30.0).await?;

    snapshots.verify("loaded", player.snapshot_stage())?;

    player.click_sprite(sprite().number(31)).await?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(75))).timeout(45.0).await?;

    player.step_frames(20).await;

    snapshots.verify("menu", player.snapshot_stage())?;

    // The menu is a carousel: a click on an off-centre entry only scrolls it
    // there (the scroll behaviour calls stopEvent() so the jump-to-marker
    // behaviour on the same sprite is suppressed). "demo button" sits one step
    // above centre, so it takes two clicks — scroll, then activate.
    player.click_sprite(sprite().number(18)).await?; // demo button — scroll onto it
    player.step_frames(20).await;
    player.click_sprite(sprite().number(18)).await?; // demo button — activate

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(100))).timeout(45.0).await?;

    snapshots.verify("car", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(105))).timeout(120.0).await?;

    snapshots.verify("racing-01", player.snapshot_stage())?;

    player.step_frames(200).await;

    snapshots.verify("racing-02", player.snapshot_stage())?;

    Ok(())
});
