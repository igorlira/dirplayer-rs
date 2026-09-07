use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_mario7-scaled.toml");

/// (movie-space rect, scaled render rect) of the first sprite showing `name`.
fn rects(name: &str) -> Option<((i32, i32, i32, i32), (i32, i32, i32, i32))> {
    vm_rust::player::reserve_player_ref(|p| {
        for n in 1..=80i16 {
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

// Mario Net Quest with the stage SCALED (`swStretchStyle = meet`) — the layout
// fullscreen uses. 512x342 into 1920x1080 gives a 3.158x scale, letterboxed.
//
// Two defects it pins, both movie-space values reaching code that had already
// switched to render space:
//
//  * Film loops REBUILD `sprite_rect` from the sprite's registration point and
//    the loop's authored size when the score dimensions disagree with the
//    member — both movie units, which overwrote the scaled rect. Every torch,
//    the fuse and Mario himself drew at 1:1 in a 3.16x room.
//
//  * The Field wrap width took `min(field_member.width, sprite_render_width)`
//    with the first still in movie units, so it always won on a scaled stage:
//    the help panel rasterised its text at `font_size * 3.16` but folded it at
//    the unscaled 348px, wrapping after two or three words per line.
//
// Unlike the FurniFactory pair this movie has NO embedded PFR font — the field
// resolves to a system font and renders through the Canvas2D path, so this also
// covers native text under scale.
browser_e2e_test!(test_nintendo_mario7_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "mario7_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(2))).timeout(10.0).await?;
    player.step_frames(10).await;

    // `the stage.rect` must stay the MOVIE's rect. Lingo works in movie
    // coordinates throughout, and movies centre their own UI against it —
    // Habbo v7 positions every window with
    //     tX = ((the stageRight - the stageLeft) / 2) - (pwidth / 2)
    // so a container-sized stage rect throws each one off-screen.
    let stage_rect = match player.eval_datum("string((the stage).rect)").await {
        Ok(vm_rust::director::static_datum::StaticDatum::String(s)) => s,
        other => format!("<{:?}>", other),
    };
    assert_eq!(stage_rect, "rect(0, 0, 512, 342)",
        "(the stage).rect reports {} on a scaled stage — it must stay the movie's          own rect, with the scaling living in drawRect", stage_rect);

    // Movie-space rects must not move just because the stage is scaled, and the
    // render rect must be exactly that rect scaled by 3.158 and offset by the
    // letterbox (draw_rect left = 151.58).
    let (text_movie, text_render) = rects("Opening text")
        .expect("the help panel field is on stage");
    assert_eq!(text_movie, (82, 150, 430, 326), "Opening text moved in movie space");
    assert_eq!(text_render, (411, 474, 1509, 1029),
        "Opening text's render rect is not its movie rect scaled and letterboxed");

    let (torch_movie, torch_render) = rects("Torch Loop")
        .expect("a torch film loop is on stage");
    assert_eq!(torch_movie, (51, 70, 73, 137), "Torch Loop moved in movie space");
    assert_eq!(torch_render, (313, 221, 382, 433),
        "Torch Loop's render rect is not its movie rect scaled and letterboxed");

    // The snapshot is what actually catches the two bugs above: both left the
    // rects correct and only the DRAWN content wrong.
    let _ = snapshots.verify("01_opening", player.snapshot_stage());
    Ok(())
});
