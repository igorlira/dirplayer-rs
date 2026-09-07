use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_sewerrun2.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Kinelco / mohsye "Sewer Run 2: Nite Ops" (D10 — Shockwave 3D + Havok behind a
// Flash UI), as published on shockwave.com.
//
// Structurally the sequel to `sewerrun.rs`: ONE SWF in channel 1 is the
// preloader, the title screen, the menu and the in-game HUD; it talks to
// Director with `getURL("event:<handler>")` and Director reads it back through
// the Flash variable dot syntax. Channel 2 is the Shockwave 3D sprite, which on
// the "game" marker is re-pointed at one of the five Track0N casts
// (`sprite(2).memberNum = member(34 + gGame.course)`) and carries the
// "Havok Physics (No HKE)" behavior.
//
// The movie was reported as not working. It boots and races once the two domain
// gates are satisfied from the config, but the sewer had no working WALLS: the
// boarder rode the floor and then went straight through the tube and off the
// course.
//
// The cause was in `apply_surface_contacts`, the ONLY thing that touches a body
// in resting contact — `detect_all_collisions` skips those bodies outright. Two
// defects stacked there:
//
//  1. A resting contact is ONE plane, sampled at one point of one triangle, and
//     it was held until the body left the owning mesh's AABB. Sewer Run 2 gives
//     the WHOLE course a single fixed body, so its AABB is the entire level: the
//     boarder glued itself to the tangent plane of the first triangle it touched
//     and slid along it at 7000 units/s with no per-triangle collision ever
//     running again — straight out through the side of the tube and off the map.
//     The plane is now only trusted near where it was sampled.
//  2. Every HKE-authored scene is Z-UP (the Havok Xtra's default gravity is
//     `[0,0,-g]`), so the plane clamp adjusted the body along index 2, guarded by
//     `if n[2].abs() > 0.01`. Sewer Run 2 is a Shockwave 3D world in Director's
//     Y-UP space and says so — `hk.gravity = vector(0, -8000, 0)`. With a floor
//     normal of (0,1,0) that guard is false, so even a correctly-held resting
//     boarder was never actually clamped to the surface, and the "has it run off
//     the platform" test compared the boarder's HEIGHT against the mesh's
//     horizontal extent. Both now key off `scene_up_axis(gravity)`, which is
//     exactly +Z — and so exactly the old code — for every Z-up scene.
//
browser_e2e_test!(test_misc_sewerrun2_load, |player| async move {
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

    let snapshots = SnapshotContext::new("misc", "sewerrun2");

    // ---- 1. Loader ---------------------------------------------------------
    //
    // Frame 1 loops on `Loader Loop`, which polls getStreamStatus() and pushes
    // the percentage into the SWF's `load_percent`. The SWF's own preloader only
    // hands over to the title screen once it reads 100 there, so this round trip
    // through the Flash variable IS the boot gate.
    player.step_frames(30).await;
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
    // in; its own preloader still has to play out. Watch its PLAYHEAD rather than
    // guessing a frame budget: once it has run past the intro and then held still
    // for two samples, the menu is up.
    let mut last = -1i32;
    let mut still = 0;
    let mut menu_up = false;
    for _ in 0..60 {
        player.step_frames(20).await;
        let f: i32 = ask_str!("string(sprite(1).currentFrame)").parse().unwrap_or(-1);
        if f > 3 && f == last { still += 1; } else { still = 0; }
        last = f;
        if still >= 2 { menu_up = true; break; }
    }
    if !menu_up {
        return Err(format!(
            "the SWF preloader never settled (playhead {})", last
        ).into());
    }
    probe(format!("menu: sp1frame={} state={} movieframe={}",
        last, ask!("string(sprite(1).state)"), player.current_frame()));
    let _ = snapshots.verify("02_menu", player.snapshot_stage());

    if ask_str!("string(sprite(1).state)") != "menu" {
        return Err(format!(
            "the SWF is not on its menu state (got {}) — the two domain gates \
             (`checkLoc`'s moviePath whitelist and \"SW Security\"'s runMode/sw8 \
             token check) are both satisfied from the config, so this means the \
             boot sequence broke somewhere else",
            ask!("string(sprite(1).state)")
        ).into());
    }

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

    // ---- 2. Menu -> race ---------------------------------------------------
    //
    // PLAY fires `event:startGame`, which builds gGame out of those Flash
    // variables and does `_movie.go("game")`.
    let mut in_game = false;
    for _ in 0..12 {
        player.click(370, 420).await;
        player.step_frames(40).await;
        if ask_str!("string(sprite(1).state)") == "game" {
            in_game = true;
            break;
        }
    }
    if !in_game {
        return Err(format!(
            "PLAY on the menu never started a race — the SWF is still on state {} \
             (movie frame {})",
            ask!("string(sprite(1).state)"), player.current_frame()
        ).into());
    }

    // gGame is assembled entirely out of Flash variables; a VOID boarderTotal is
    // what would leave `setupCharacter` building no boarders at all.
    let boarders: i32 = ask_str!("string(gGame.boarderTotal)").parse().unwrap_or(0);
    if boarders < 1 {
        return Err(format!(
            "gGame.boarderTotal is {} — startGame read the menu's numeric Flash \
             variables as VOID, so the race has no boarders",
            ask!("string(gGame.boarderTotal)")
        ).into());
    }

    // setupCourse walks every model whose name starts with the course name /
    // "Kicker" / "Outle" / "Shield" and gives each a Havok FIXED rigid body — the
    // sewer's floor and walls. Those plus the boarders are the whole physics
    // world.
    let bodies: i32 = ask_str!("string(gHavok.rigidBody.count)").parse().unwrap_or(0);
    if bodies < boarders + 10 {
        return Err(format!(
            "the Havok world holds only {} rigid bodies for {} boarders — \
             setupCourse did not turn the sewer geometry into fixed bodies",
            bodies, boarders
        ).into());
    }
    player.step_frames(60).await;
    probe(format!("game: frame={} boarders={} bodies={}",
        player.current_frame(), boarders, bodies));
    let _ = snapshots.verify("03_game", player.snapshot_stage());

    // ---- 3. Intro -> race --------------------------------------------------
    //
    // `intro` holds, releases the boarders one at a time on an impulse, then
    // hands control over and switches gGame.mode to #race.
    let start_pos = ask!("string(sprite(2).pBoarder[1].proxy.worldPosition)");
    let mut racing = false;
    for _ in 0..14 {
        player.step_frames(60).await;
        if ask_str!("string(gGame.mode)") == "race" { racing = true; break; }
    }
    if !racing {
        return Err(format!(
            "gGame.mode never reached #race — the intro sequence stalled (mode {})",
            ask!("string(gGame.mode)")
        ).into());
    }
    if ask!("string(sprite(2).pBoarder[1].proxy.worldPosition)") == start_pos {
        return Err(format!(
            "the boarder never moved during the intro ({} throughout) — the Havok \
             step is not driving the rigid bodies", start_pos
        ).into());
    }
    player.step_frames(180).await;
    let _ = snapshots.verify("04_race", player.snapshot_stage());

    // ---- 4. The sewer has walls -------------------------------------------
    //
    // This is the regression the movie was reported for. Hold LEFT (GameBehaviour
    // polls `keyPressed(123)` — the MAC virtual keycode, so this also covers the
    // browser-keyCode -> Mac keycode map) to drive the boarder off the racing
    // line and into the side of the tube.
    //
    // The oracle is the movie's OWN surface classifier: `getInfo` casts a ray
    // straight down onto the course and names the shader it lands on —
    // #ontrack / #rim / #sides / #offtrack. With the walls collidable the boarder
    // climbs the rim and is pushed back down onto the track; with the up-axis bug
    // it went through the tube and stayed off the course.
    //
    // Under a continuously held LEFT the boarder does NOT settle on the wall: it
    // climbs the side, gets pushed back down onto the track, drifts out again,
    // and so on, with a period of a few samples. So the surfaces must be judged
    // as a SEQUENCE OF EVENTS, not by reading a fixed window and asking what the
    // samples in it happen to say — which phase of that oscillation a given
    // sample lands on varies with wall-clock timing (the movie steps on `the
    // milliSeconds`), and it made this test fail randomly in BOTH directions:
    // "never left the racing line" when the window caught it on-track
    // throughout, and "never came back down" when the window happened to end
    // while it was riding the side.
    // Start from the racing line, so an excursion is attributable to the
    // steering rather than to wherever the intro happened to leave the boarder.
    let mut on_line = false;
    for _ in 0..20 {
        if ask_str!("string(sprite(2).pBoarder[1].surface)") == "ontrack" {
            on_line = true;
            break;
        }
        player.step_frames(15).await;
    }
    if !on_line {
        return Err(format!(
            "the boarder never settled onto the racing line before the steering test              (surface {}) — it is not riding the course at all",
            ask!("string(sprite(2).pBoarder[1].surface)")
        ).into());
    }

    player.key_down("ArrowLeft", 37).await;
    player.step_frames(10).await;

    // The steering has to actually reach the movie, or nothing below proves
    // anything. Ask the movie directly instead of inferring it from where the
    // boarder ended up — that inference was the flaky half of the old check.
    let sees_key = ask_str!("string(keyPressed(123))");
    if sees_key != "1" {
        return Err(format!(
            "`keyPressed(123)` does not see the held arrow key (answered {}) — the \
             browser keyCode -> Mac virtual keycode map is not reaching the movie, \
             so this run cannot exercise the walls at all", sees_key
        ).into());
    }

    // Ride into the side of the tube, then off it again. BOTH phases are polled
    // for rather than assumed to fall inside a fixed window.
    let mut surfaces: Vec<String> = Vec::new();
    let mut pushed_off: Option<usize> = None;
    let mut recovered: Option<usize> = None;
    let mut height = f64::NAN;
    for i in 0..40 {
        player.step_frames(30).await;
        let s = ask_str!("string(sprite(2).pBoarder[1].surface)");
        height = ask_str!("string(sprite(2).pBoarder[1].height)").parse().unwrap_or(f64::NAN);
        probe(format!("wall{} surface={} pos={} height={:.1}", i, s,
            ask!("string(sprite(2).pBoarder[1].proxy.worldPosition)"), height));
        surfaces.push(s.clone());
        match pushed_off {
            None => {
                if s == "sides" || s == "rim" {
                    pushed_off = Some(i);
                }
            }
            Some(_) => {
                if s == "ontrack" {
                    recovered = Some(i);
                    break;
                }
            }
        }
    }
    player.key_up("ArrowLeft", 37).await;
    let _ = snapshots.verify("05_wall", player.snapshot_stage());

    let Some(pushed_off) = pushed_off else {
        return Err(format!(
            "holding LEFT never moved the boarder off the racing line in {} samples \
             (#ontrack throughout: {:?}) — the movie DOES see the key, so the steering \
             reaches GameBehaviour but never moves the rigid body off the centre line",
            surfaces.len(), surfaces
        ).into());
    };
    let Some(recovered) = recovered else {
        return Err(format!(
            "the boarder reached the side of the tube at sample {} and never came back \
             down onto the course in the {} samples after it (surfaces {:?}) — the \
             sewer's walls are not stopping it. The Havok fixed bodies built from the \
             track geometry are only consulted along the scene's UP axis, and this \
             movie is Y-up (`hk.gravity = vector(0,-8000,0)`); see `scene_up_axis` in \
             havok_physics.rs",
            pushed_off, surfaces.len() - pushed_off - 1, surfaces
        ).into());
    };

    // …and back over the course it must come down and RIDE it, rather than
    // having been flung out of the tube. `height` is the down-ray distance from
    // the boarder to the course, measured by the movie itself; it reads ~20 on
    // the racing line.
    //
    // Measure it only after the steering is released and the boarder has had
    // time to drop. The surface classifier flips to #ontrack the moment the
    // down-ray first lands on track shader, and that happens while the boarder
    // is still high up coming off the wall — at that instant `height` reads a
    // consistent ~1000, so testing it there failed a movie that was behaving
    // perfectly well.
    let mut settled = f64::NAN;
    let mut riding = false;
    for _ in 0..12 {
        player.step_frames(30).await;
        let s = ask_str!("string(sprite(2).pBoarder[1].surface)");
        settled = ask_str!("string(sprite(2).pBoarder[1].height)").parse().unwrap_or(f64::NAN);
        if s == "ontrack" && (-200.0..600.0).contains(&settled) {
            riding = true;
            break;
        }
    }
    if !riding {
        return Err(format!(
            "the boarder reached #ontrack again at sample {} but never came back down              onto the course once the steering was released — it ends {:.0} units from              the surface, so it is no longer riding the sewer (surfaces {:?})",
            recovered, settled, surfaces
        ).into());
    }
    height = settled;
    probe(format!(
        "wall: off the line at sample {}, back on the course at {} (surfaces {:?}), height {:.1}",
        pushed_off, recovered, surfaces, height
    ));

    Ok(())
});
