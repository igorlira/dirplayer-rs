use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness, datum};

const CONFIG: &str = include_str!("../configs/cc_cokestudios-scaled.toml");

/// The movie-space rect of a sprite by member name, and its scaled render rect.
fn rects(name: &str) -> Option<((i32, i32, i32, i32), (i32, i32, i32, i32))> {
    vm_rust::player::reserve_player_ref(|p| {
        for channel in &p.movie.score.channels {
            let s = &channel.sprite;
            let Some(mr) = s.member.as_ref() else { continue };
            let Some(m) = p.movie.cast_manager.find_member_by_ref(mr) else { continue };
            if !m.name.eq_ignore_ascii_case(name) {
                continue;
            }
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

/// Coke Studios with the stage SCALED (`swStretchStyle = meet`) — the layout
/// fullscreen uses, where sprite rects, font sizes and line spacing are all
/// multiplied by the stage scale before the text is rasterised.
///
/// The navigator's tab labels are the case that matters here: PFR Verdana 11 in
/// a 15px atlas cell with `fixedLineSpace = 14`, inside a 13px field box. The
/// line-box leading is derived from `fixedLineSpace - char_height`, and both
/// terms are scaled — so if either were left in movie units the labels would
/// drift down and lose their bottom rows again, only at this stage size.
browser_e2e_test!(test_cokestudios_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    cfg.apply_flash_config();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "cokestudios_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // Blow the stage up the way going fullscreen on a 1080p display does.
    // 1920/760 = 2.526, 1080/520 = 2.077 -> `meet` picks 2.077 and letterboxes.
    vm_rust::set_stage_size(1920, 1080);

    player
        .step_until(datum("ilk(oRoom)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(180.0)
        .await
        .map_err(|e| {
            format!(
                "{e}\nNever entered the lobby. This test needs the backend reachable, \
                 at the addresses the COKESTUDIOS_* variables in .env point to \
                 (see cc_cokestudios.toml and .env.example)."
            )
        })?;
    player
        .step_until(datum("_movie.frame").equals(StaticDatum::Int(63)))
        .timeout(120.0)
        .await?;
    player.step_frames(150).await;

    // A sprite's MOVIE-space rect must not move just because the stage is
    // scaled — `cokestudios`'s `lobby` snapshot is the unscaled reference for
    // the same field.
    let (tab_movie, tab_render) =
        rects("navi_publicRooms").expect("the navigator's Public View tab is on stage");
    assert_eq!(
        tab_movie,
        (440, 73, 521, 86),
        "navi_publicRooms' movie-space rect changed under a scaled stage (render {:?})",
        tab_render
    );
    // ...and the render rect is that rect scaled by 2.077, offset by the
    // horizontal letterbox ((1920 - 760*2.077) / 2 = 171).
    assert_eq!(
        tab_render,
        (1085, 152, 1253, 179),
        "navi_publicRooms' render rect is not the movie rect scaled by 2.077"
    );

    snapshots.verify_with_ratio("lobby", player.snapshot_stage(), 0.25)?;

    Ok(())
});
