use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_summerresort.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// The map is a grid of 16x16 tile sprites in channels 10..79. Report each
/// live one as (channel, member, movie rect, DEVICE rect) so the scaled run
/// can be compared against this baseline.
pub fn tile_rects() -> Vec<(i16, String, (i32, i32, i32, i32), (i32, i32, i32, i32))> {
    vm_rust::player::reserve_player_ref(|p| {
        let mut out = Vec::new();
        for n in 10..=79i16 {
            let sprite = &p.movie.score.get_channel(n).sprite;
            if sprite.loc_v < 0 || !sprite.visible {
                continue;
            }
            let name = sprite
                .member
                .as_ref()
                .and_then(|r| p.movie.cast_manager.find_member_by_ref(r))
                .map(|m| m.name.clone())
                .unwrap_or_default();
            if name.is_empty() || name == "blank" {
                continue;
            }
            let m = vm_rust::player::score::get_concrete_sprite_rect(p, sprite);
            let d = vm_rust::player::score::get_concrete_sprite_render_rect(p, sprite);
            out.push((
                n,
                name,
                (m.left, m.top, m.right, m.bottom),
                (d.left, d.top, d.right, d.bottom),
            ));
        }
        out
    })
}

/// Enter the map. `prepareMovie` arms `tellStreamStatus`, and the game only
/// leaves the loader once `streamStatus` reports the movie fully downloaded —
/// which never happens for an already-resident movie — so drive the same
/// entry the instruction screen's `mouseUp` does.
pub async fn enter_game(player: &mut impl TestHarness) -> Result<(), String> {
    let _ = player.eval("showInstructions()").await;
    player.step_frames(20).await;
    probe(format!("after showInstructions: frame={}", player.current_frame()));

    let _ = player.eval("resetVars()").await;
    let _ = player.eval("useFinalData()").await;
    let _ = player.eval("changeGameStage(#MIDDLE)").await;
    let _ = player.eval("go(\"game\")").await;
    player.step_frames(40).await;
    probe(format!("after go(game): frame={}", player.current_frame()));

    if tile_rects().is_empty() {
        return Err("no tile sprites were built — `updateMap` never ran".to_string());
    }
    Ok(())
}

/// Report gaps/overlaps between tiles that ABUT in movie space.
///
/// `updateMap` lays the room out on an exact 16px grid, so two tiles whose
/// movie rects share an edge are neighbours with nothing between them. After
/// the stage layout maps them to device pixels they must still share an edge:
/// if each edge is rounded independently the shared boundary can land on two
/// different device columns, which opens a 1px seam of stage colour between
/// every pair of tiles — a whole grid of them across the map.
pub fn seams(
    tiles: &[(i16, String, (i32, i32, i32, i32), (i32, i32, i32, i32))],
) -> Vec<String> {
    let mut out = Vec::new();
    for a in tiles {
        for b in tiles {
            if a.0 >= b.0 {
                continue;
            }
            // Horizontal neighbours: a's right edge is b's left edge, and the
            // rows overlap.
            if a.2 .2 == b.2 .0 && a.2 .1 < b.2 .3 && b.2 .1 < a.2 .3 && a.3 .2 != b.3 .0 {
                out.push(format!(
                    "H seam ch{} '{}' right={} | ch{} '{}' left={} (movie x={}) -> device {} vs {}",
                    a.0, a.1, a.3 .2, b.0, b.1, b.3 .0, a.2 .2, a.3 .2, b.3 .0
                ));
            }
            // Vertical neighbours.
            if a.2 .3 == b.2 .1 && a.2 .0 < b.2 .2 && b.2 .0 < a.2 .2 && a.3 .3 != b.3 .1 {
                out.push(format!(
                    "V seam ch{} '{}' bottom={} | ch{} '{}' top={} (movie y={}) -> device {} vs {}",
                    a.0, a.1, a.3 .3, b.0, b.1, b.3 .1, a.2 .3, a.3 .3, b.3 .1
                ));
            }
        }
    }
    out
}

// Cartoon Network "Summer Resort" — the UNSCALED baseline for the fullscreen
// tile-alignment report (`summerresort_scaled.rs`). Everything the scaled run
// asserts has to hold here first, at scale 1 where there is no layout to get
// wrong.
browser_e2e_test!(test_misc_summerresort_map, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    let (mw, mh) = vm_rust::player::reserve_player_ref(|p| {
        (p.movie.rect.width(), p.movie.rect.height())
    });
    probe(format!("movie rect = {}x{}", mw, mh));

    enter_game(&mut player).await?;

    let tiles = tile_rects();
    probe(format!("{} tiles built", tiles.len()));
    for t in tiles.iter().take(12) {
        probe(format!("ch{} '{}' movie={:?} device={:?}", t.0, t.1, t.2, t.3));
    }

    let seams = seams(&tiles);
    for s in seams.iter().take(20) {
        probe(format!("UNSCALED {}", s));
    }
    assert!(
        seams.is_empty(),
        "{} tile seams at scale 1 — the baseline is already broken:\n{}",
        seams.len(),
        seams.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    );

    let snapshots = SnapshotContext::new("misc", "summerresort");
    let _ = snapshots.verify("01_map", player.snapshot_stage());

    Ok(())
});
