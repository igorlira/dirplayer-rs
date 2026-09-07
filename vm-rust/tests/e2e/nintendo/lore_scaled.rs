use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_lore-scaled.toml");

// Lore with the stage SCALED (`swStretchStyle = meet`) — the layout fullscreen
// uses.
//
// The quiz answers are Button members (radioButton) with text labels, which the
// renderer draws itself into an offscreen bitmap rather than going through the
// text path. Every piece of that chrome was a literal pixel count — a 10px check
// box, an 11px radio circle, a 3px gap, 1px frame lines — and the label used the
// member's unscaled font size, while the bitmap around them came from the scaled
// sprite rect. So the answers kept their authored size while the rows holding
// them tripled apart.
browser_e2e_test!(test_nintendo_lore_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "lore_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(144))).timeout(30.0).await?;
    player.click_sprite(sprite().number(14)).await?;
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(676))).timeout(30.0).await?;
    player.click_sprite(sprite().number(16)).await?;

    let _ = snapshots.verify("01_question", player.snapshot_stage());
    Ok(())
});
