use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_spectral_wizard-scaled.toml");

fn hover(x: i32, y: i32) {
    vm_rust::player::reserve_player_mut(|player| {
        player.mouse_loc = (x, y);
    });
}

/// (stage image size, movie size).
fn stage_image_vs_movie() -> ((u16, u16), (u16, u16)) {
    vm_rust::player::reserve_player_ref(|p| {
        let img = p.stage_image
            .and_then(|r| p.bitmap_manager.get_bitmap(r))
            .map(|b| (b.width, b.height))
            .unwrap_or((0, 0));
        (img, (p.movie.rect.width() as u16, p.movie.rect.height() as u16))
    })
}

// Spectral Wizard with the stage SCALED (`swStretchStyle = meet`) — the layout
// fullscreen uses.
//
// This movie is an "imaging Lingo" title: its speech bubbles are baked into
// `(the stage).image` with copyPixels rather than drawn as sprites. That image
// is addressed in MOVIE coordinates, but it was being created at the CANVAS
// size — 1920x1080 for a 640x480 movie — so every draw landed in the top-left
// quarter and the compositor, which scales by (canvas / image), had nothing left
// to scale: the bubble rendered 1:1 in the corner while the scene around it grew.
//
// The bitmap is also PERSISTENT and was never resized, so coming back out of
// fullscreen left a 1920x1080 image against a 640x480 canvas and the overlay
// shrank to a third — the "it stays small" half of the report.
browser_e2e_test!(test_misc_spectral_wizard_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new("misc", "spectral_wizard_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    // 640x480 into 1920x1080 -> `meet` picks 1080/480 = 2.25 and letterboxes.
    vm_rust::set_stage_size(1920, 1080);

    player.step_frames(900).await;

    hover(312, 279);
    player.step_frames(10).await;
    player.click(312, 279).await;
    player.step_frames(10).await;
    let _ = player.eval_datum("g.m.doMenuAction(1)").await;

    for name in ["01_intro_a", "02_intro_b", "03_intro_c", "04_intro_d"] {
        player.step_frames(250).await;
        // The stage image must stay at the movie's own size no matter how the
        // stage is scaled — that is the whole bug in one assertion. The
        // snapshots then show it composited in the right place.
        let (img, movie) = stage_image_vs_movie();
        assert_eq!(img, movie,
            "{}: (the stage).image is {:?} but the movie is {:?} — scripts address \
             it in MOVIE coordinates, so a canvas-sized image puts every draw in \
             the corner", name, img, movie);
        let _ = snapshots.verify(name, player.snapshot_stage());
    }

    // ...and it must still match after the stage shrinks back, since the bitmap
    // is persistent. (No snapshot here: resizing mid-run leaves the harness's
    // canvas out of step with the VM, so the image would be misleading. The VM
    // state is the thing under test.)
    vm_rust::set_stage_size(640, 480);
    player.step_frames(60).await;

    // EVERY canvas on the page — the renderer keeps a preview canvas too, so
    // reading "the" canvas can easily read the wrong one.
    {
        let listing = js_sys::eval("(function(){var c=document.getElementsByTagName('canvas')[0];var g=c.getContext('webgl2');var vp=g?Array.from(g.getParameter(g.VIEWPORT)):'n/a';var fb=g?(g.getParameter(g.FRAMEBUFFER_BINDING)?'fbo':'default'):'n/a';return 'buffer='+c.width+'x'+c.height+' client='+c.clientWidth+'x'+c.clientHeight+' viewport='+vp+' bound='+fb;})()")
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| "<eval failed>".to_string());
        let vm = vm_rust::player::reserve_player_ref(|p| {
            let l = vm_rust::player::stage::stage_layout(p);
            format!("vm canvas={}x{} draw={:?}", l.canvas_width, l.canvas_height, l.draw_rect)
        });
        // ...and what is actually IN the stage image: its non-background extent.
        // If the scene occupies only a corner of a correctly-sized bitmap, the
        // problem is upstream of the composite.
        let ink = vm_rust::player::reserve_player_ref(|p| {
            let Some(r) = p.stage_image else { return "<no stage image>".to_string() };
            let Some(b) = p.bitmap_manager.get_bitmap(r) else { return "<missing>".to_string() };
            let (w, h) = (b.width as usize, b.height as usize);
            let bpp = (b.bit_depth / 8) as usize;
            if bpp == 0 || b.data.len() < w * h * bpp { return "<bad data>".to_string(); }
            let first = &b.data[0..bpp];
            let (mut x0, mut y0, mut x1, mut y1) = (w as i32, h as i32, -1i32, -1i32);
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) * bpp;
                    if &b.data[i..i + bpp] != first {
                        if (x as i32) < x0 { x0 = x as i32; }
                        if (y as i32) < y0 { y0 = y as i32; }
                        if (x as i32) > x1 { x1 = x as i32; }
                        if (y as i32) > y1 { y1 = y as i32; }
                    }
                }
            }
            format!("stage_image {}x{} depth={} ink=({},{})-({},{}) {}x{}",
                w, h, b.bit_depth, x0, y0, x1, y1,
                (x1 - x0 + 1).max(0), (y1 - y0 + 1).max(0))
        });
        web_sys::console::log_1(&format!("[PROBE] after shrink: {} || {} || {}", listing, vm, ink).into());
    }

    let (img, movie) = stage_image_vs_movie();
    assert_eq!(img, movie,
        "after the stage shrank back, (the stage).image is still {:?} against a \
         {:?} movie — the persistent bitmap was never resized", img, movie);
    // This movie is an imaging-Lingo engine — nearly everything you see IS
    // the stage image — so a stale size does not merely misplace an overlay,
    // it shrinks the whole game. This snapshot IS the "stays small when you
    // leave fullscreen" report; do not delete it again.
    let _ = snapshots.verify("05_back_to_windowed", player.snapshot_stage());

    Ok(())
});
