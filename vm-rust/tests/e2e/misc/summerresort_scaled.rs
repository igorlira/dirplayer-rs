use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

use super::summerresort::{enter_game, seams, tile_rects};

const CONFIG: &str = include_str!("../configs/misc_summerresort-scaled.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Cartoon Network "Summer Resort" with the stage SCALED — the layout
// fullscreen uses. Reported defect: "in fullscreen the tiles do not match like
// normal view".
//
// The room is not a background bitmap. `updateMap` (MovieScript "#main")
// rebuilds channels 10..79 per room from `map.data`, and each entry is a
// filled #rect SHAPE member whose pattern is a VWTL custom tile — one shape
// per contiguous region of the same tile, sized to the region:
//
//     sprite.loc    = point(col * 16 + WSHIFT, row * 16 + HSHIFT)
//     sprite.width  = <region width>       sprite.ink = 36
//
// So the picture is a mosaic of adjacent tiled shapes, and it only reads as
// one room while every shape starts its tile pattern on the SAME grid. The
// renderer rasterizes each shape into a texture at the sprite's MOVIE size and
// phases the tile by the sprite's origin; passing the RENDER rect's origin
// instead agrees with that grid at scale 1 and nowhere else, because on a
// scaled stage it is `drawRect.left + left * scale`. Every region then began
// its pattern somewhere of its own choosing: pavement mortar lines stepped
// between columns, flower beds restarted mid-flower, and the map came apart —
// while every sprite rect stayed exactly where it belonged, which is why the
// geometry checks below all pass either way.
//
// The real assertion is therefore the direct one: render the SAME room at
// scale 1 and scaled, and require the scaled frame to be a magnification of
// the unscaled one. Nothing in the room moves on its own (`x.map`'s exitFrame
// returns unless the player is walking), so the two frames are comparable
// pixel for pixel.
browser_e2e_test!(test_misc_summerresort_scaled_map, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    let (mw, mh) = vm_rust::player::reserve_player_ref(|p| {
        (p.movie.rect.width() as i32, p.movie.rect.height() as i32)
    });
    enter_game(&mut player).await?;

    // Baseline: the same room at scale 1. The canvas has to be sized to the
    // movie first — this config asks for `swStretchStyle = meet`, so the movie
    // is laid out to whatever the canvas is, and the harness starts small.
    vm_rust::set_stage_size(mw as u32, mh as u32);
    player.step_frames(10).await;
    let unscaled = player.snapshot_stage().to_rgba();
    let base_scale = vm_rust::player::reserve_player_ref(|p| {
        vm_rust::player::stage::stage_scale(p)
    });
    probe(format!("baseline scale = {:?}", base_scale));
    assert!(
        (base_scale.0 - 1.0).abs() < 1e-6 && (base_scale.1 - 1.0).abs() < 1e-6,
        "the baseline pass is not at scale 1 ({:?}) — it is not a baseline",
        base_scale
    );

    vm_rust::set_stage_size(1920, 1080);
    player.step_frames(10).await;

    // Read the layout AFTER stepping: `set_stage_size` only reaches the canvas
    // on the next frame, so a read straight after it still describes the
    // pre-resize canvas.
    let (ox, oy, sx, sy) = vm_rust::player::reserve_player_ref(|p| {
        let l = vm_rust::player::stage::stage_layout(p);
        let (sx, sy) = vm_rust::player::stage::stage_scale(p);
        (l.draw_rect[0], l.draw_rect[1], sx, sy)
    });
    probe(format!(
        "movie={}x{} layout offset=({}, {}) scale=({}, {})",
        mw, mh, ox, oy, sx, sy
    ));
    assert!(sx > 1.5 && sy > 1.5, "the stage never scaled — scale is {}x{}", sx, sy);

    let tiles = tile_rects();
    probe(format!("{} tiles built", tiles.len()));
    for t in tiles.iter().take(6) {
        probe(format!("ch{} '{}' movie={:?} device={:?}", t.0, t.1, t.2, t.3));
    }

    // 1. Every tile lands at `drawRect origin + movie * scale`.
    let mut misplaced = Vec::new();
    for t in &tiles {
        let want = (
            (ox + t.2 .0 as f64 * sx).round() as i32,
            (oy + t.2 .1 as f64 * sy).round() as i32,
            (ox + t.2 .2 as f64 * sx).round() as i32,
            (oy + t.2 .3 as f64 * sy).round() as i32,
        );
        if want != t.3 {
            misplaced.push(format!(
                "ch{} '{}' movie={:?} device={:?} but the layout says {:?}",
                t.0, t.1, t.2, t.3, want
            ));
        }
    }

    // 2. Tiles that abut in movie space still abut in device space.
    let seams = seams(&tiles);

    // 3. The scaled frame is a magnification of the unscaled one. Sample the
    //    centre of each movie pixel's device footprint, which is exact for a
    //    point-sampled magnification and does not care what the scale is.
    let scaled_snapshot = player.snapshot_stage();
    let scaled = scaled_snapshot.to_rgba();
    let mut sampled = 0usize;
    let mut wrong = 0usize;
    let mut first = Vec::new();
    for my in 0..mh {
        for mx in 0..mw {
            let want = match unscaled.pixel(mx as u32, my as u32) {
                Some(p) => p,
                None => continue,
            };
            let cx = (ox + mx as f64 * sx + sx / 2.0) as u32;
            let cy = (oy + my as f64 * sy + sy / 2.0) as u32;
            let got = match scaled.pixel(cx, cy) {
                Some(p) => p,
                None => continue,
            };
            sampled += 1;
            let d = (want.0 as i32 - got.0 as i32).abs()
                .max((want.1 as i32 - got.1 as i32).abs())
                .max((want.2 as i32 - got.2 as i32).abs());
            if d > 8 {
                wrong += 1;
                if first.len() < 10 {
                    first.push(format!(
                        "movie({}, {}) -> canvas({}, {}): {:?} scaled vs {:?} unscaled",
                        mx, my, cx, cy, got, want
                    ));
                }
            }
        }
    }
    let ratio = wrong as f64 / sampled.max(1) as f64;
    probe(format!("{}/{} sampled pixels differ ({:.2}%)", wrong, sampled, ratio * 100.0));

    let snapshots = SnapshotContext::new("misc", "summerresort_scaled");
    let _ = snapshots.verify("01_map", scaled_snapshot);

    assert!(
        misplaced.is_empty(),
        "{} of {} tiles are not at `drawRect origin + movie * scale`:\n{}",
        misplaced.len(), tiles.len(),
        misplaced.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    );
    assert!(
        seams.is_empty(),
        "{} tile seams on the SCALED stage — neighbouring tiles that share an \
         edge in movie space no longer share one on screen:\n{}",
        seams.len(),
        seams.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    );
    // 1% absorbs the handful of pixels where a movie pixel's device footprint
    // straddles an edge; the defect this guards put a quarter of the frame
    // (and half of every tiled region) out.
    assert!(
        ratio < 0.01,
        "{:.1}% of the room renders differently once the stage is scaled ({} of \
         {} sampled pixels). The scaled frame must be a magnification of the \
         unscaled one — a tiled shape has to phase its pattern by the sprite's \
         MOVIE origin, because its texture is rasterized at the sprite's movie \
         size. Examples:\n{}",
        ratio * 100.0, wrong, sampled, first.join("\n")
    );

    Ok(())
});
