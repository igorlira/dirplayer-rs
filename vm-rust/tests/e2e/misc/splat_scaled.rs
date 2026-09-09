use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_splat-scaled.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Splat with the stage SCALED (`swStretchStyle = meet`) — the layout fullscreen
// uses.
//
// Splat's lives bar and drink icons are painted into `(the stage).image` and
// composited by the renderer as a SUB-RECT quad covering rect(130, 430, 630,
// 470). Two things have to hold at once for that strip to land correctly when
// the stage is scaled:
//
//   * the stage image stays at the MOVIE's size (640x480) however big the
//     canvas is — scripts address it in movie coordinates;
//   * the overlay quad maps that sub-rect through the stage LAYOUT (letterbox
//     offset + scale), and its texture coordinates are a (x, y, WIDTH, HEIGHT)
//     window into the bitmap — passing the far edges instead crushed the whole
//     bottom of the stage into the strip's first rows.
//
// The unscaled test (`splat.rs`) covers the second point at scale 1, where the
// letterbox offset is zero and cannot expose an offset error. This one runs at
// 1920x1080, where `meet` picks 1080/480 = 2.25 and centres a 1440-wide image
// with 240px bars, so an overlay that ignored either the offset or the scale
// lands somewhere provably wrong.
browser_e2e_test!(test_misc_splat_scaled_hud, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new("misc", "splat_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    // The title screen runs a Flash intro paced by Ruffle's own wall clock, so
    // budget generously rather than by frame count. Same reasoning as the
    // unscaled test.
    for _ in 0..40 {
        player.step_frames(25).await;
    }
    probe(format!("title settled: frame={}", player.current_frame()));

    // Click "CLICK HERE TO PLAY". `titleJump.mouseDown` gates on
    // `sprite(3).mouseOverButton = 1`, so park the cursor first — and aim at
    // the button line, not the banner's empty centre. Coordinates are MOVIE
    // coordinates; the harness maps them through the stage layout.
    for (x, y) in [(180, 76), (180, 55), (120, 76)] {
        vm_rust::player::reserve_player_mut(move |p| { p.mouse_loc = (x, y); });
        player.step_frames(2).await;
        player.click(x, y).await;
        player.step_frames(4).await;
        if player.current_frame() >= 6 { break; }
    }
    if player.current_frame() < 6 {
        probe(format!("click did not advance (frame={}), falling back to go(8)",
            player.current_frame()));
        let _ = ask!("sprite(5).visible = 0");
        let _ = ask!("go(8)");
    }
    player.step_frames(150).await;

    // Wait for the level to actually build rather than guessing a budget.
    let mut saw_pips = false;
    for _ in 0..80 {
        player.step_frames(10).await;
        if ask!("string(voidP(member(\"scene\").model(\"pips\")))").contains('0') {
            saw_pips = true;
            break;
        }
    }
    assert!(saw_pips, "the `pips` model was never built — the level never loaded");
    player.step_frames(60).await;

    // The stage image must stay at the MOVIE's size no matter how big the
    // canvas is.
    let (img, movie) = vm_rust::player::reserve_player_ref(|p| {
        let img = p.stage_image
            .and_then(|r| p.bitmap_manager.get_bitmap(r))
            .map(|b| (b.width, b.height))
            .unwrap_or((0, 0));
        (img, (p.movie.rect.width() as u16, p.movie.rect.height() as u16))
    });
    probe(format!("stage image {:?} vs movie {:?}", img, movie));
    assert_eq!(img, movie,
        "(the stage).image is {:?} but the movie is {:?} — scripts address it in \
         MOVIE coordinates, so a canvas-sized image puts every draw in the corner",
        img, movie);

    let _ = snapshots.verify("01_hud", player.snapshot_stage());

    // Paint a marker the 3D scene can never produce into the lives bar, through
    // the same imaging-Lingo path the game uses, and check WHERE the compositor
    // put it. Its expected canvas position comes from the stage layout, so the
    // assertion tests the mapping rather than restating a magic number.
    let _ = ask!("(the stage).image.fill(rect(200, 440, 260, 465), rgb(255, 0, 255))");
    player.step_frames(1).await;

    let (ox, oy, sx, sy) = vm_rust::player::reserve_player_ref(|p| {
        let l = vm_rust::player::stage::stage_layout(p);
        let dw = (l.draw_rect[2] - l.draw_rect[0]) as f64;
        let dh = (l.draw_rect[3] - l.draw_rect[1]) as f64;
        (l.draw_rect[0] as f64, l.draw_rect[1] as f64,
         dw / movie.0 as f64, dh / movie.1 as f64)
    });
    probe(format!("layout offset=({},{}) scale=({},{})", ox, oy, sx, sy));

    let composited = player.snapshot_stage();
    for (mx, my) in [(230.0, 442.0), (230.0, 450.0), (230.0, 462.0)] {
        let cx = (ox + mx * sx) as u32;
        let cy = (oy + my * sy) as u32;
        let px = composited.pixel(cx, cy)
            .unwrap_or_else(|| panic!("stage snapshot has no pixel at ({}, {})", cx, cy));
        probe(format!("marker movie({},{}) -> canvas({},{}) = {:?}", mx, my, cx, cy, px));
        assert!(px.0 > 200 && px.1 < 80 && px.2 > 200,
            "the scaled stage-image overlay is misplaced: movie ({}, {}) maps to \
             canvas ({}, {}) which reads {:?}, but `(the stage).image` holds \
             magenta there. The overlay must go through the stage LAYOUT \
             (letterbox offset + scale) and use (x, y, WIDTH, HEIGHT) texture \
             coordinates", mx, my, cx, cy, px);
    }

    Ok(())
});
