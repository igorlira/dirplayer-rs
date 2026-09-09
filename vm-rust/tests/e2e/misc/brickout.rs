use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{sprite, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_brickout.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// Read a Lingo global as a formatted string — `eval` cannot reach bare globals.
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

/// Bricks live in channels 8..19 (`vFirstSprite` = 8, `vMaxSprite` = 12) and are
/// hidden one at a time as the ball hits them.
fn visible_bricks() -> usize {
    vm_rust::player::reserve_player_ref(|p| {
        (8..=19i16).filter(|n| p.movie.score.get_channel(*n).sprite.visible).count()
    })
}

// BrickOut (D5, French). Title screen on the "Intro" marker (frame 1), gameplay
// on "Jeux" (frame 7).
//
// The whole game hangs off one line in the movie script's `startGame`:
//
//     puppetTransition(32, 1, 5)
//     puppetSprite(vLogoSprite, 0)
//     go("Jeux")
//
// `puppetTransition` REGISTERS an effect to play on the next frame change; it
// must not freeze the playhead before that change happens. When it started the
// transition hold immediately, `advance_frame` took its "a transition is
// animating, hold the playhead" early return and dropped the `go` outright —
// every global was initialised (vGameStarted/vLevel/vBallUsed all set) but the
// playhead stayed on frame 1's logo loop, so the game-loop frame script
// (ScoreScript 3) never ran and nothing ever moved.
browser_e2e_test!(test_misc_brickout_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_frames(30).await;
    if player.current_frame() != 1 {
        return Err(format!("expected the title loop on frame 1, got {}", player.current_frame()).into());
    }

    // Sprites 4/5/6 on the title screen carry ScoreScript 4 -> `on mouseUp startGame()`.
    player.click_sprite(sprite().number(4)).await?;
    player.step_frames(10).await;

    probe(format!("after start: frame={} vGameStarted={} vLevel={} vBallUsed={}",
        player.current_frame(), global("vGameStarted"), global("vLevel"), global("vBallUsed")));

    if global("vGameStarted") != "1" {
        return Err("clicking the title screen never ran startGame()".to_string().into());
    }
    if player.current_frame() != 7 {
        return Err(format!(
            "startGame's `go(\"Jeux\")` did not move the playhead — still on frame {}. \
             The preceding puppetTransition must not hold the playhead before the \
             frame change it is meant to accompany.",
            player.current_frame()
        ).into());
    }

    let bricks_at_start = visible_bricks();
    if bricks_at_start == 0 {
        return Err("newLevel built no bricks".to_string().into());
    }

    // The ball is stuck to the paddle until a mouseUp fires `newBall`
    // (`when mouseUp then newBall`). The paddle itself tracks the mouse via
    // `movePaddle` -> constrainH, which only runs from the game frame script.
    player.mouse_move(300, 400).await;
    player.step_frames(4).await;

    let ball_at_launch = sprite_loc(4);
    let mut moved = false;
    let mut cleared = false;
    let mut launched = false;
    for i in 0..16 {
        // Serve (again, after a life is lost) the way a player does.
        if global("vBallLoosed") == "1" {
            player.click(300, 400).await;
            player.step_frames(2).await;
        }
        player.step_frames(15).await;
        // `newBall` clears vBallLoosed; it lands a tick or two after the click
        // (and the puppetTransition hold swallows the first ~2s of ticks), so
        // watch for it across the loop rather than right after the press.
        if global("vBallLoosed") == "0" {
            launched = true;
        }
        if sprite_loc(4) != ball_at_launch {
            moved = true;
        }
        if visible_bricks() < bricks_at_start {
            cleared = true;
        }
        probe(format!("play{} frame={} ball={:?} bricks={} vBallLoosed={} vBallUsed={}",
            i, player.current_frame(), sprite_loc(4), visible_bricks(),
            global("vBallLoosed"), global("vBallUsed")));
        if moved && cleared {
            break;
        }
    }

    if !launched {
        return Err(format!(
            "the ball was never launched — `when mouseUp then newBall` did not fire              (vBallLoosed={})", global("vBallLoosed")
        ).into());
    }
    if !moved {
        return Err("the ball never moved — the gameplay frame script (ScoreScript 3) is not running".to_string().into());
    }
    if !cleared {
        return Err(format!(
            "no brick was ever destroyed — testBrick's `inside(point, the rect of sprite)`              collision is not connecting (bricks still visible: {})", visible_bricks()
        ).into());
    }

    Ok(())
});
