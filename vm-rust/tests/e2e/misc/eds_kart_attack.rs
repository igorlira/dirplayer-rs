use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_eds_kart_attack.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Ed's Kart Attack — the Flash intro must actually PLAY.
//
// The intro is a loop=false Flash member on channel 1. "Wait for Flash"
// (BehaviorScript 66) loops its frame with `go(the frame)` and advances the
// INSTANT `sprite(1).playing = 0` — a hair-trigger: any window where the
// intro's Ruffle instance reads not-playing skips the intro outright.
//
// This test reproduces the DEV-APP sequence, which the normal harness flow
// does not: the dev app renders the stage while the movie is NOT yet playing
// (the Fetch & Load preview), and that pre-play render is what activates
// Flash sprites — the movie-load warm path (docs/flash-instance-warmup-
// handoff.md §5.4 race 3). The instance created there must (a) not run ahead
// of the Director playhead, and (b) read `playing = 1` once the movie starts,
// or the intro is skipped.
browser_e2e_test!(test_misc_eds_kart_attack_intro, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;

    // DEV-APP REPRO: let the page render the stage for a while BEFORE play —
    // is_playing is false here, so the renderer takes the pre-play (warm)
    // Flash dispatch path exactly like the dev app's load preview.
    for _ in 0..30 {
        player.step_frame().await;
    }

    player.init_movie().await; // = clicking PLAY

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    player.step_frames(5).await;
    let wait_frame = player.current_frame();
    probe(format!(
        "after play: frame={} sprite1.playing={} member.pausedAtStart={} member.loop={} swfFrame={}",
        wait_frame,
        ask!("string(sprite(1).playing)"),
        ask!("string(sprite(1).member.pausedAtStart)"),
        ask!("string(sprite(1).member.loop)"),
        ask!("string(sprite(1).getVariable(\"_currentframe\"))"),
    ));

    // The regression assert: within the first moments of playback the intro
    // must report playing = 1 at least once. When the pre-play instance is
    // mishandled it reads 0 on the very first exitFrame and "Wait for Flash"
    // jumps the movie past the intro immediately.
    let mut saw_playing = false;
    let mut frames_on_wait = 0u32;
    for i in 0..40 {
        let playing = ask!("string(sprite(1).playing)");
        if playing.contains("\"1\"") {
            saw_playing = true;
        }
        if player.current_frame() == wait_frame {
            frames_on_wait += 1;
        }
        if i < 6 || playing.contains("\"0\"") {
            probe(format!(
                "step {}: frame={} playing={} swfFrame={}",
                i, player.current_frame(), playing,
                ask!("string(sprite(1).getVariable(\"_currentframe\"))"),
            ));
        }
        if saw_playing && player.current_frame() != wait_frame {
            break; // intro played and finished
        }
        player.step_frames(10).await;
    }
    assert!(saw_playing,
        "sprite(1).playing never read 1 — the intro instance was not playing \
         when the movie started, so \"Wait for Flash\" skipped the intro");
    assert!(frames_on_wait >= 2,
        "the movie left the intro frame immediately (held it for {} probe \
         rounds) — the intro was skipped rather than played", frames_on_wait);

    // And the movie must not be STUCK either: the loop=false watcher parks the
    // SWF at its last frame, playing flips to 0, and the movie advances. The
    // intro runs on Ruffle's wall clock, so give it real time.
    let mut advanced = player.current_frame() != wait_frame;
    if !advanced {
        for _ in 0..200 {
            player.step_frames(10).await;
            if player.current_frame() != wait_frame {
                advanced = true;
                break;
            }
        }
    }
    probe(format!(
        "end: frame={} sprite1.playing={} swfFrame={}",
        player.current_frame(),
        ask!("string(sprite(1).playing)"),
        ask!("string(sprite(1).getVariable(\"_currentframe\"))"),
    ));
    assert!(advanced,
        "the movie never advanced past the intro frame {} — the loop=false \
         stop-at-end never fired", wait_frame);

    Ok(())
});
