use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, sprite, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/nintendo_mario7.toml");

/// Read a Lingo global as a formatted string (`eval` can't reach bare globals).
fn global(name: &str) -> String {
    use vm_rust::player::datum_formatting::format_concrete_datum;
    use vm_rust::player::reserve_player_ref;
    use vm_rust::player::symbols::symbol::Symbol;
    reserve_player_ref(|p| match p.globals.get(&Symbol::from_str(name)) {
        Some(r) => format_concrete_datum(&p.get_datum(r), p),
        None => "<unset>".to_string(),
    })
}

fn sprite_loc(n: i16) -> (i32, i32) {
    vm_rust::player::reserve_player_ref(|p| {
        let s = &p.movie.score.get_channel(n).sprite;
        (s.loc_h, s.loc_v)
    })
}

/// The playable region updateCursor tests before it lets the Mario hand follow
/// the mouse: a diamond around (cCenterH 257, cCenterV 176) out to cRight 482 /
/// cBottom 296. Outside it the movie shows the "out of bounds" cursor and
/// leaves the hand sprite parked, so a grab there can never connect.
fn in_bounds(h: i32, v: i32) -> bool {
    (v - 176).abs() <= (225 - (h - 257).abs()) * 120 / 225
}

/// Clicking a popped-up character must collect it.
///
/// The whole grab runs off the movie script's `on mouseDown` -> `grabForIt()`,
/// which reaches the sprites only if the mouseDown message propagates past the
/// background sprites. Those carry the title screen's "click to start" score
/// script on the title frames ONLY; leaving it attached for the rest of the
/// movie swallowed every mouseDown and nothing was ever collectable.
browser_e2e_test!(test_nintendo_mario7_grab, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(2))).timeout(10.0).await?;
    player.click_sprite(sprite().member("Opening text")).await?;

    // The gameplay loop (`on exitFrame animate()`) starts at frame 29.
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(30))).timeout(30.0).await?;

    // Boxes pop up at random; wait for a character (channels 15-19) to surface
    // somewhere the Mario hand is allowed to reach.
    let mut target = None;
    for _ in 0..600 {
        target = vm_rust::player::reserve_player_ref(|p| {
            (15..=19i16).find(|n| {
                let s = &p.movie.score.get_channel(*n).sprite;
                s.loc_h > -500 && in_bounds(s.loc_h as i32, s.loc_v as i32)
            })
        });
        if target.is_some() {
            break;
        }
        player.step_frame().await;
    }
    let target = target.ok_or("no character ever popped up inside the playable area")?;
    let (h, v) = sprite_loc(target);

    // The hand sprite only tracks the mouse from updateCursor, which runs on
    // exitFrame — give it frames to catch up before grabbing.
    player.mouse_move(h, v).await;
    for _ in 0..12 {
        player.step_frame().await;
    }
    if sprite_loc(46) != (h, v) {
        return Err(format!(
            "Mario hand did not follow the mouse to ({}, {}), it is at {:?}",
            h, v, sprite_loc(46)
        ).into());
    }

    player.mouse_down(h, v).await;

    if global("gDownBefore") != "1" {
        return Err(format!(
            "movie script `on mouseDown` never ran (gDownBefore={}) — the mouseDown \
             message did not reach the movie script",
            global("gDownBefore")
        ).into());
    }
    // grabForIt() zaps the character it hit: score moves off zero and the
    // character sprite is parked off-stage.
    if global("gScoreCounter") == "0" {
        return Err(format!(
            "grabbing character sprite {} at ({}, {}) did not score",
            target, h, v
        ).into());
    }
    if sprite_loc(target).0 > -500 {
        return Err(format!(
            "collected character sprite {} was not removed, still at {:?}",
            target, sprite_loc(target)
        ).into());
    }

    player.mouse_up(h, v).await;
    Ok(())
});
