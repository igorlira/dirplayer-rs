use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_sewerrun.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Miniclip "Sewer Run" (D10 — Shockwave 3D + Havok, driven by a Flash UI).
//
// Everything the player touches lives in ONE SWF in channel 1
// (`interface_miniclip.swf`): the preloader, the title screen, the main menu,
// the character chooser and the in-game HUD. It talks to Director by calling
// `getURL("event:<handler>")`, and Director talks back through the Flash
// variable DOT SYNTAX — `sprite(1).load_percent = 100`,
// `course = integer(sprite(1).track)` (Using Director 11.5, "Using Lingo or
// JavaScript syntax with Flash variables"). Channel 2 is the Shockwave 3D
// sprite, which also carries the Havok Physics behavior.
//
// The movie was reported as not working at all, and three defects stacked up
// along that one bridge:
//
//  1. The dot syntax was not wired to the SWF at all. Unknown sprite props fell
//     through to the sprite's BEHAVIOURS, so the loader's
//     `sprite(1).load_percent = 100` invented a Director-side property the SWF
//     could never see and the game sat forever on "The game is loading (0%)".
//     Fixed in `score.rs::sprite_get_prop` / `sprite_set_prop`.
//  2. `coerceFlashValue` only stringified booleans, so every NUMERIC Flash
//     variable arrived as a JS number, failed `as_string()` and read VOID.
//     The whole menu selection (`track` / `challenge` / `enviro` / `kudos`) is
//     numeric, so `startGame` built gGame with a VOID `boarderTotal` and
//     `setupCharacter`'s `repeat with i = 1 to gGame.boarderTotal` created no
//     boarders — `setupCamera` then died on `pBoarder[1]`.
//  3. MenuBehaviour's `setupCharacter` multiplies a MODEL by 2
//     (`#ftWheels: (board * 2) - 1` — an author slip that `changeWheels(1)`
//     overwrites two lines later). Director coerces the object to 0; raising
//     aborted `beginSprite` and left the chooser empty. See
//     `datum_operations.rs::object_as_arithmetic_operand`.
browser_e2e_test!(test_misc_sewerrun_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    cfg.apply_flash_config();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }
    // `ask!` formats with `{:?}`, so a Lingo string comes back quoted —
    // `String("100")`. Unwrap it so tests can compare against plain text.
    macro_rules! ask_str {
        ($expr:expr) => {
            ask!($expr).trim_start_matches("String(\"").trim_end_matches("\")").to_string()
        };
    }

    let snapshots = SnapshotContext::new("misc", "sewerrun");

    // ---- 1. Loader ---------------------------------------------------------
    //
    // Frame 1 loops on `Loader Loop`, which polls getStreamStatus() and pushes
    // the percentage into the SWF's `load_percent`. The SWF's own preloader
    // only hands over to the title screen once it reads 100 there, so this
    // round trip through the Flash variable IS the boot gate.
    player.step_frames(30).await;
    if player.current_frame() != 1 {
        return Err(format!("expected the loader loop on frame 1, got {}", player.current_frame()).into());
    }
    let _ = snapshots.verify("01_loading", player.snapshot_stage());

    let mut load_reported = false;
    for _ in 0..20 {
        player.step_frames(30).await;
        if ask_str!("string(sprite(1).load_percent)") == "100" {
            load_reported = true;
            break;
        }
    }
    if !load_reported {
        return Err(format!(
            "the loader never got 100 back out of the SWF — `sprite(1).load_percent = 100` \
             is not reaching the Flash variable (read back {})",
            ask!("string(sprite(1).load_percent)")
        ).into());
    }
    // `load_percent` reaching 100 only means the SWF has been TOLD the stream is
    // in; its own preloader still has to play out to the title screen. Wait for
    // the PLAY button to actually be under the pointer rather than guessing a
    // frame count — `mouseOverButton` is `hitTest(mouseLoc) = #button`, so it is
    // TRUE exactly once the title screen's button exists there.
    // The SWF then plays its Miniclip splash (playhead 1 -> 3) and the title
    // transition (3 -> 80) before it stops on the title screen with the PLAY
    // button. Watch its PLAYHEAD rather than guessing a frame budget: once it
    // has run past the splash and then held still for two samples, the title is
    // up. `mouseOverButton` is no use here — the splash is itself a button, so
    // it reads TRUE the whole way through.
    let mut last = -1i32;
    let mut still = 0;
    let mut title_up = false;
    for _ in 0..60 {
        player.step_frames(20).await;
        let f: i32 = ask_str!("string(sprite(1).currentFrame)").parse().unwrap_or(-1);
        if f > 3 && f == last { still += 1; } else { still = 0; }
        last = f;
        if still >= 2 { title_up = true; break; }
    }
    if !title_up {
        return Err(format!(
            "the SWF preloader never settled on the title screen (playhead {})", last
        ).into());
    }
    probe(format!("title up: sp1frame={}", last));
    let _ = snapshots.verify("02_title", player.snapshot_stage());

    // ---- 2. Title -> main menu --------------------------------------------
    //
    // PLAY on the title screen makes the SWF fire `event:gotoMenu`, whose movie
    // script handler does `_movie.go("menu")` — frame 5, where MenuBehaviour
    // builds the 3D character chooser out of five W3D casts.
    let mut in_menu = false;
    for _ in 0..12 {
        player.click(512, 590).await;
        player.step_frames(30).await;
        if player.current_frame() >= 5 {
            in_menu = true;
            break;
        }
    }
    if !in_menu {
        return Err(format!(
            "PLAY on the title screen never reached the \"menu\" marker — the SWF's \
             `getURL(\"event:gotoMenu\")` is not landing (still on frame {})",
            player.current_frame()
        ).into());
    }
    player.step_frames(120).await;
    probe(format!("menu frame={} state={} w3d models={}",
        player.current_frame(), ask!("string(sprite(1).state)"),
        ask!("string(sprite(2).member.model.count)")));

    // MenuBehaviour::setupCharacter clones a boarder, a board and four wheel
    // pairs into the chooser world, then adds a backdrop and two overlays. If
    // `board * 2` raises, beginSprite aborts before any of it exists.
    let models: i32 = ask_str!("string(sprite(2).member.model.count)").parse().unwrap_or(0);
    if models < 10 {
        return Err(format!(
            "the character chooser built only {} models — MenuBehaviour's beginSprite \
             did not run to completion", models
        ).into());
    }
    if ask_str!("string(sprite(1).state)") != "menu" {
        return Err(format!(
            "the SWF is not on its menu state (got {})", ask!("string(sprite(1).state)")
        ).into());
    }
    let _ = snapshots.verify("03_menu", player.snapshot_stage());

    // The menu keeps every selection in a NUMERIC root variable. These are what
    // `startGame` reads, so a VOID here is the difference between a race and an
    // empty world.
    for v in ["track", "challenge", "enviro"] {
        let got = ask_str!(&format!("string(sprite(1).{})", v));
        if got.parse::<i32>().is_err() {
            return Err(format!(
                "the numeric Flash variable `{}` read back as {}, not a number — \
                 getVariable is dropping non-string ActionScript values", v, got
            ).into());
        }
    }

    // ---- 3. Main menu -> race ---------------------------------------------
    //
    // PLAY here fires `event:startGame`, which builds gGame from those
    // variables and does `_movie.go("game")` — frame 10, GameBehaviour +
    // the Havok Physics behavior.
    let mut in_game = false;
    for _ in 0..10 {
        player.click(757, 590).await;
        player.step_frames(40).await;
        if player.current_frame() >= 10 {
            in_game = true;
            break;
        }
    }
    if !in_game {
        return Err(format!(
            "PLAY on the main menu never started a race — still on frame {}",
            player.current_frame()
        ).into());
    }
    probe(format!("game frame={} gGame={}", player.current_frame(), ask!("string(gGame)")));

    // gGame is assembled entirely out of Flash variables; a VOID boarderTotal
    // is what left `setupCharacter` building no boarders at all.
    let boarders: i32 = ask_str!("string(gGame.boarderTotal)").parse().unwrap_or(0);
    if boarders < 1 {
        return Err(format!(
            "gGame.boarderTotal is {} — startGame read the menu's numeric Flash \
             variables as VOID, so the race has no boarders",
            ask!("string(gGame.boarderTotal)")
        ).into());
    }

    // Havok: the behavior's `Initialize` runs from beginSprite, and every
    // boarder gets a rigid body.
    let bodies: i32 = ask_str!("string(gHavok.rigidBody.count)").parse().unwrap_or(0);
    if bodies < boarders {
        return Err(format!(
            "the Havok world holds {} rigid bodies for {} boarders — physics init \
             did not run for the whole field", bodies, boarders
        ).into());
    }
    probe(format!("physics boarders={} bodies={}", boarders, bodies));

    // ---- 4. The intro fade and the race -----------------------------------
    //
    // `intro` holds 2 s, releases the boarders one at a time on an impulse, then
    // hands control over and switches gGame.mode to #race. The white overlay
    // (overlay[4], blend 100) fades out across the same stretch, so this is also
    // what takes the stage from a white wash to the sewer.
    // pBoarder is a BEHAVIOUR property of GameBehaviour, so it is only reachable
    // through the sprite it is attached to — `eval` cannot see it as a global.
    let start_pos = ask!("string(sprite(2).pBoarder[1].player.worldPosition)");
    let mut racing = false;
    for i in 0..12 {
        player.step_frames(60).await;
        let mode = ask_str!("string(gGame.mode)");
        probe(format!("race{} frame={} mode={} pos={} white={}",
            i, player.current_frame(), mode,
            ask!("string(sprite(2).pBoarder[1].player.worldPosition)"),
            ask!("string(sprite(2).member.camera[1].overlay[4].blend)")));
        if mode == "race" {
            racing = true;
            break;
        }
    }
    if !racing {
        return Err(format!(
            "gGame.mode never reached #race — the intro sequence stalled (mode {})",
            ask!("string(gGame.mode)")
        ).into());
    }

    // Under Havok the boarder is dropped in and pushed down the sewer, so the
    // player's world position must have MOVED by the time the race starts.
    let now_pos = ask!("string(sprite(2).pBoarder[1].player.worldPosition)");
    if now_pos == start_pos {
        return Err(format!(
            "the boarder never moved during the intro ({} throughout) — the Havok \
             step is not driving the rigid bodies", start_pos
        ).into());
    }

    player.step_frames(120).await;
    let _ = snapshots.verify("04_race", player.snapshot_stage());

    // The boarder must RIDE the body, not float above it: `setBoarder` drives
    // `playerNull` (and, pinned to it, the rider) off the rigid body every frame,
    // so the two have to stay within a board's length of each other vertically.
    //
    // They did not. `checkPhoto` / `checkWeapon` take a COPY of the node's
    // position and raise it to chest height —
    //
    //     pos = pBoarder[b].player.worldPosition
    //     pos.y = pos.y + 375
    //
    // — and the register IR's `SetLocal` did not end the pending
    // `node.<vectorProp>.<component> =` lvalue chain the way the interpreter's
    // does, so that `+375` was written back onto the NODE, once per pickup per
    // frame. The node climbed until it settled ~68 750 units above the body and
    // the stage showed a riderless board in the sky.
    let body_y: f64 = ask_str!("string(sprite(2).pBoarder[1].rb.position.y)").parse().unwrap_or(0.0);
    let node_y: f64 = ask_str!("string(sprite(2).pBoarder[1].player.worldPosition.y)").parse().unwrap_or(0.0);
    probe(format!("ride: body_y={} node_y={}", body_y, node_y));
    if (node_y - body_y).abs() > 1000.0 {
        return Err(format!(
            "the boarder's playerNull node is {:.0} units off its rigid body              (body y={:.1}, node y={:.1}) — the node the rider is pinned to is not              following the physics",
            node_y - body_y, body_y, node_y
        ).into());
    }

    // NOTE: the sibling `transform_sub_refs` write-back (behind
    // `p = model.transform.position` … `p.z = …`) has the same
    // must-end-at-a-variable-store rule and is fixed alongside this, but is NOT
    // covered here — Sewer Run never uses that idiom, and the harness's `eval`
    // is a standalone expression evaluator, not the bytecode VM, so it cannot
    // exercise a `SetLocal`. Covering it needs an authored test movie.
    // See docs/sewerrun-player-node-drift.md.

    Ok(())
});
