use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness, datum, sprite};

const CONFIG: &str = include_str!("../configs/cc-furnifactory-scaled.toml");

/// The movie-space rect of a text sprite, and its scaled render rect.
fn rects(name: &str) -> Option<((i32, i32, i32, i32), (i32, i32, i32, i32))> {
    vm_rust::player::reserve_player_ref(|p| {
        for n in 1..=150i16 {
            let Some(s) = vm_rust::player::score::get_sprite_in_context(p, n) else { continue };
            let Some(mr) = s.member.as_ref() else { continue };
            let Some(m) = p.movie.cast_manager.find_member_by_ref(mr) else { continue };
            if m.name != name { continue; }
            let a = vm_rust::player::score::get_concrete_sprite_rect(p, s);
            let b = vm_rust::player::score::get_concrete_sprite_render_rect(p, s);
            return Some((
                (a.left, a.top, a.right, a.bottom),
                (b.left, b.top, b.right, b.bottom),
            ));
        }
        None
    })
}

// FurniFactory with the stage SCALED (`swStretchStyle = meet`) — the same layout
// fullscreen uses, where the canvas is sized to the container and sprite rects,
// font sizes and line spacing are all multiplied by the stage scale.
//
// Three defects it pins, all the same shape: a movie-space value being compared
// against, or mixed into, an already-scaled one.
//
//  * The text texture took its WIDTH from the scaled render rect but its HEIGHT
//    from the unscaled `text_member.height`, so the HUD boxes rasterised two
//    21px lines into a 24px-tall texture. The value line ("Braces", "0",
//    "10 of 10") fell outside it and vanished, leaving a box with only a label.
//
//  * `get_concrete_sprite_rect` measured MOVIE-space text against whatever atlas
//    the font cache last stored under the bare font name — which, on a scaled
//    stage, is the enlarged one. The alert panel measured 114px instead of 36
//    and its single line was pinned to the top of an over-tall box.
//
//  * The rotation/skew shader was handed the sprite's registration point in
//    movie coordinates while its vertices came from the scaled rect, so it
//    rotated scaled geometry about an unscaled pivot. The computer's clock
//    (rot 25, skew 337) landed in the middle of the factory floor.
browser_e2e_test!(test_furnifactory_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "furnifactory_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // Blow the stage up the way going fullscreen on a 1080p display does.
    // 1920/760 = 2.526, 1080/520 = 2.077 -> `meet` picks 2.077 and letterboxes.
    vm_rust::set_stage_size(1920, 1080);

    player
        .step_until(sprite().member("alertbox2_start-up").visible(1.0))
        .await?;
    player
        .click_sprite(sprite().member_prefix("alertbox2_start-up"))
        .await?;
    player
        .step_until(datum("ilk(oComputer)").equals(StaticDatum::Symbol("instance".into())))
        .await?;

    // The round-start alert panel, while it is still up.
    player.step_frames(30).await;

    // A sprite's MOVIE-space rect must not move just because the stage is
    // scaled. `furnifactory`'s `alert` snapshot is the unscaled reference for
    // this same moment; before the fix this measured 355x114 here and 355x36
    // there, purely because the font cache had an enlarged atlas under the
    // bare name.
    let (alert_movie, alert_render) = rects("alertbox_text")
        .expect("the alert panel's text sprite is on stage");
    assert_eq!(alert_movie, (203, 232, 558, 268),
        "alertbox_text's movie-space rect changed under a scaled stage (render {:?})",
        alert_render);

    let _ = snapshots.verify("00_alert", player.snapshot_stage());

    // ...and the game running, for the HUD boxes and the skewed clock.
    player
        .step_until(
            datum("not oComputer.oTimer.bPaused and oComputer.oTimer.iTime < 57")
                .equals(StaticDatum::Int(1)),
        )
        .await?;

    // A two-line HUD box: 98x24 authored, so 203x50 at 2.077. The texture is
    // built from this, and it has to be tall enough for BOTH lines.
    let (hud_movie, hud_render) = rects("displayTool_Target")
        .expect("the Tool Needed box is on stage");
    assert_eq!(hud_movie, (12, 482, 110, 506), "displayTool_Target moved in movie space");
    assert_eq!(hud_render, (196, 1001, 399, 1051),
        "displayTool_Target's render rect is not the movie rect scaled by 2.077");

    let _ = snapshots.verify("01_scaled", player.snapshot_stage());
    Ok(())
});
