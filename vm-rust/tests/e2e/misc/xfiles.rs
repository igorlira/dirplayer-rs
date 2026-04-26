use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_xfiles.toml");

browser_e2e_test!(test_04_xfiles_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "xfiles");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    snapshots.verify("start_game", player.snapshot_stage())?;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(20))).timeout(20.0).await?;

    snapshots.verify("menu", player.snapshot_stage())?;

    player.step_frames(75).await;

    player.click_sprite(sprite().member("scully")).await?;

    player.step_frames(15).await;

    player.click_sprite(sprite().member("s1n")).await?;

    player.step_frames(15).await;

    snapshots.verify("forest_mites", player.snapshot_stage())?;

    player.click_sprite(sprite().member("exit")).await?;

    player.step_frames(50).await;


    player.click_sprite(sprite().member("mulder")).await?;

    player.step_frames(15).await;

    player.click_sprite(sprite().member("s2n")).await?;

    player.step_frames(15).await;

    snapshots.verify("flunkeman", player.snapshot_stage())?;

    player.click_sprite(sprite().member("exit")).await?;

    player.step_frames(50).await;


    player.click_sprite(sprite().member("scully")).await?;

    player.step_frames(15).await;

    player.click_sprite(sprite().member("s3n")).await?;

    player.step_frames(15).await;

    snapshots.verify("lord_kinbote", player.snapshot_stage())?;

    player.click_sprite(sprite().member("exit")).await?;

    player.step_frames(50).await;


    player.click_sprite(sprite().member("mulder")).await?;

    player.step_frames(15).await;

    player.click_sprite(sprite().member("s4n")).await?;

    player.step_frames(15).await;

    snapshots.verify("little_grays", player.snapshot_stage())?;

    Ok(())
});
