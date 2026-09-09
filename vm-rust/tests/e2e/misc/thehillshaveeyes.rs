use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_thehillshaveeyes.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// "The Hills Have Eyes — Run For The Hills" (Fox Searchlight movie promo,
// Director 10, Shockwave 3D) — reported as not working. NOT a physics-Xtra game:
// no Havok cast member, no dynamiks.x32. The mutants path around a Lingo
// `_Map` / `_PathFinder` pair, and the player has no gravity at all — his height
// comes from one downward `modelsUnderRay` per frame. It has no domain gate
// either: the only `the runMode` tests in the movie (`_game`, `_debugObj_set`)
// just switch debug output on under "author", so the browser default is what the
// game expects and no `_moviePath` is needed. It does use the Enhancer Xtra
// (`new(xtra("enhancer"), "AAENXP-...")` in init_game's `beginSprite`), which is
// therefore on the boot path. Four defects, the first of which sealed the movie
// off completely:
//
// 1. `collect_property_keyframes` dropped every keyframe whose value equalled the
//    property's DEFAULT, and rotation's default is 0 — which is exactly where a
//    spin-in animation comes to REST. All three menu cards fly in on a rotation
//    tween (180 -> 150 -> ... -> 30 -> 0 over five frames) and then sit upright
//    for the rest of the span, so each one froze at 30°. A 480x390 card left
//    tilted 30° about its centre sweeps a quad that reaches down over the row of
//    buttons beneath it, and with ink 0 (Copy) it painted them out: the HOW TO
//    PLAY card buried CONTINUE, so there was no way off the first menu screen and
//    the game could never be started. Fixed with `DEFAULT_IS_A_VALUE` on Rotation
//    and Skew in `score_keyframes.rs`.
//
// 2. A sprite span was always entered from the score cell of its START frame,
//    even when the playhead had jumped into the MIDDLE of that span. `_debug_skip`
//    runs `go("title")` from frame 12, landing on frame 70 — fifty frames into the
//    menu background's 20..260 span, whose opening cell is the first frame of a
//    fade-in with blend byte 255, i.e. fully transparent. The fade's blend
//    keyframes were long past, so nothing ever corrected it and `m_control_bg`
//    stayed invisible for the whole menu: the CONTROLS and GAME SIZE cards sat on
//    black instead of the rock photo. `score.rs::span_init_data` now prefers the
//    newest cell at or before the frame actually entered.
//
// 3. `<string>.symbol` — the dot form of `symbol(str)` (Using Director 11.5,
//    "Symbols") — raised "Invalid string built-in property symbol". Both
//    marker-picker behaviours build their range with
//    `marker_list[i] = _movie.markerlist[i].symbol` inside
//    `getPropertyDescriptionList`, so the GAME SIZE screen's buttons aborted while
//    being set up and the movie could not be started even after defect 1.
//
// 4. `#maxDistance` was applied as a cutoff on the HIT distance. Director 11.5,
//    `modelsUnderRay`: "If a model's BOUNDING SPHERE is within the maximum
//    distance specified, THAT MODEL IS INCLUDED. If the bounding sphere is in
//    range, then it may contain polygons in range and thus might be intersected."
//    It selects models; it never clips the intersection. `_controller_FPS`
//    seats the player with a single ray — `#maxDistance: 100` straight down onto
//    `L_C_floor`, then `worldPosition.z = hit.z + 160` — and the mine floor under
//    the spawn is ~530 units below. Director includes the model (the ray starts
//    deep inside its 3378-unit bounding sphere) and answers at 530; clamped to
//    100 the ray found nothing, and since the movie has no gravity the player
//    floated ~480 units above the mine, looking down through the tunnel roof, for
//    the entire game. Fixed in `raycast.rs` (bounding-sphere model cull, hit
//    distance unbounded) — which also retires the `maxDistance * |direction|`
//    scaling that had been reverse-engineered from SweeTarts under the old
//    reading.
//
// `transform.preMultiply` was missing as well (`_enemy.stepit` pins each mutant's
// collision proxies onto its skeleton with it); that one raised in `stepit`
// rather than at boot.
browser_e2e_test!(test_misc_thehillshaveeyes_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }
    // `ask!` formats with `{:?}`, so a Lingo string comes back quoted. Unwrap it.
    macro_rules! ask_str {
        ($expr:expr) => {
            ask!($expr).trim_start_matches("String(\"").trim_end_matches("\")").to_string()
        };
    }

    // The menu cards animate in, so a button is only clickable once its screen
    // has settled. Find the sprite carrying `name` and click the centre of its
    // rect; returns false if the button is not on stage.
    macro_rules! click_button {
        ($name:expr) => {{
            let mut hit = None;
            for n in 1..60 {
                // The rollover behaviour swaps the member to `<name>_over`
                // while the pointer sits on it, and consecutive cards put
                // their buttons in the same place, so match both forms.
                let nm = ask_str!(&format!("sprite({}).member.name", n));
                if nm == $name || nm == format!("{}_over", $name) {
                    let r = ask_str!(&format!("string(sprite({}).rect)", n));
                    hit = Some((n, r));
                    break;
                }
            }
            match hit {
                Some((n, r)) => {
                    // "rect(341, 430, 451, 470)" -> centre
                    let nums: Vec<i32> = r.trim_start_matches("rect(").trim_end_matches(')')
                        .split(',').filter_map(|s| s.trim().parse().ok()).collect();
                    if nums.len() == 4 {
                        let (x, y) = ((nums[0] + nums[2]) / 2, (nums[1] + nums[3]) / 2);
                        probe(format!("click {} on sprite {} at ({}, {})", $name, n, x, y));
                        player.click(x, y).await;
                        true
                    } else { false }
                }
                None => false,
            }
        }};
    }

    let snapshots = SnapshotContext::new("misc", "thehillshaveeyes");

    // ---- 1. Title -----------------------------------------------------------
    //
    // Frame 1 runs `_init_main` (gUtils / gSound / gGameSize), the intro plays,
    // and the movie holds on the "title" marker (frame 70) at frame 75.
    player.step_frames(120).await;
    if player.current_frame() != 75 {
        return Err(format!(
            "expected the title to hold on frame 75, got {} (marker {})",
            player.current_frame(), ask!("marker(0)")
        ).into());
    }
    let _ = snapshots.verify("01_title", player.snapshot_stage());

    // ---- 2. HOW TO PLAY -----------------------------------------------------
    //
    // The card spins in across frames 84-89 and must come to rest UPRIGHT. If
    // the closing rotation keyframe is lost the card stays tilted and covers
    // the CONTINUE button, which is what made the game unplayable.
    if !click_button!("m_btn_playgame") {
        return Err("the title screen has no m_btn_playgame sprite".into());
    }
    player.step_frames(40).await;
    let rot = ask_str!("string(sprite(9).rotation)");
    if !rot.starts_with("0.") {
        return Err(format!(
            "the HOW TO PLAY card never finished its spin-in: sprite(9).rotation = {} \
             (the tween's closing `rotation = 0` keyframe was dropped, so the card \
             stays tilted and buries the CONTINUE button)", rot
        ).into());
    }
    let _ = snapshots.verify("02_howtoplay", player.snapshot_stage());

    // ---- 3. CONTROLS --------------------------------------------------------
    //
    // Same spin-in, on the channel that holds the controls card.
    if !click_button!("m_btn_continue") {
        return Err("CONTINUE is not on stage on the HOW TO PLAY card — it is \
                    covered by the tilted card".into());
    }
    player.step_frames(60).await;
    probe(format!("controls: frame={} marker={}", player.current_frame(), ask!("marker(0)")));
    let _ = snapshots.verify("03_controls", player.snapshot_stage());

    // ---- 4. GAME SIZE -------------------------------------------------------
    if !click_button!("m_btn_continue") {
        return Err("CONTINUE is not on stage on the CONTROLS card".into());
    }
    player.step_frames(60).await;
    probe(format!("size: frame={} marker={}", player.current_frame(), ask!("marker(0)")));
    let _ = snapshots.verify("04_gamesize", player.snapshot_stage());

    // ---- 5. Load and play ---------------------------------------------------
    //
    // "LARGE" enters the game_play marker, where `init_game` clears the globals,
    // makes the Enhancer Xtra instance, and holds on its own frame until
    // `_world_manager.checkReady()` sees all 12 W3D casts at state 4. It then
    // clones the player / mutant / gun models and builds `shell`.
    if !click_button!("m_btn_large") {
        return Err("the GAME SIZE card has no m_btn_large sprite".into());
    }
    let mut ready = false;
    for i in 0..40 {
        player.step_frames(20).await;
        probe(format!(
            "load i={} frame={} marker={} shell={} game={}",
            i, player.current_frame(), ask!("marker(0)"),
            ask!("ilk(shell)"), ask!("ilk(game)")
        ));
        if ask_str!("ilk(shell)") != "Symbol(\"void\")" && ask_str!("ilk(shell)") != "void" {
            ready = true;
            break;
        }
    }
    if !ready {
        return Err(format!(
            "the game never built its `shell` — `_world_manager.checkReady()` is \
             still waiting on a W3D cast (world state {}, mutant state {}, player state {})",
            ask!("member(\"world\").state"),
            ask!("member(\"mutant\").state"),
            ask!("member(\"player\").state")
        ).into());
    }
    player.step_frames(60).await;
    probe(format!("in game: frame={} marker={}", player.current_frame(), ask!("marker(0)")));

    // The world is live: the level plus the cloned player, gun and mutants.
    let models = ask!("member(\"world\").model.count");
    if models == "Int(0)" || models.starts_with("<err") {
        return Err(format!("the 3D world has no models after setup ({})", models).into());
    }
    let _ = snapshots.verify("05_ingame", player.snapshot_stage());

    // Walking must actually move the camera. `_controller_FPS.stepit` polls
    // `keyPressed("w")`, so hold W down across a few frames and watch the
    // camera's world position change — a change-over-time check rather than a
    // pixel compare, which is the right assertion for the 3D path.
    let before = ask_str!("string(member(\"world\").camera(1).worldPosition)");
    player.key_down("w", 87).await;
    player.step_frames(30).await;
    player.key_up("w", 87).await;
    let after = ask_str!("string(member(\"world\").camera(1).worldPosition)");
    probe(format!("walk: {} -> {}", before, after));
    if before == after {
        return Err(format!(
            "holding W did not move the camera — `_controller_FPS.stepit` is not              driving the player (worldPosition stayed {})", before
        ).into());
    }
    let _ = snapshots.verify("06_walked", player.snapshot_stage());

    // The player must be SEATED on the mine floor, not floating over it.
    // `_controller_FPS.checkFloor` is the only thing that sets the player's Z —
    // the movie has no gravity — and it does so with
    //     modelsUnderRay(pos + (0,0,100), (0,0,-1),
    //         [..., #maxDistance: 100, #modelList: pFloorList])
    //     my.worldPosition.z = <hit>.z + 160
    // The floor under the spawn is ~530 units down, far beyond that 100, so this
    // only ever fires because `#maxDistance` selects MODELS by bounding sphere
    // and does NOT clip the hit (Director 11.5, `modelsUnderRay`). Read the floor
    // back the same way the movie does and check the offset it should be holding.
    let dummy_z: f64 = ask_str!("string(game.player.my.worldPosition.z)").parse().unwrap_or(f64::NAN);
    let floor = ask_str!(
        "string(member(\"world\").modelsUnderRay(game.player.my.worldPosition + vector(0,0,100),          vector(0,0,-1), [#maxNumberOfModels: 1, #levelOfDetail: #detailed,          #modelList: game.player.controller.pFloorList])[1][#isectPosition].z)");
    let floor_z: f64 = floor.parse().unwrap_or(f64::NAN);
    probe(format!("ground: player z={} floor z={}", dummy_z, floor_z));
    if !(dummy_z - (floor_z + 160.0)).abs().lt(&1.0) {
        return Err(format!(
            "the player is not standing on the mine floor: z={} but the floor below              is at {} (checkFloor seats the player at floor + 160). `#maxDistance` is              a bounding-sphere MODEL cull, not a cutoff on the hit distance — clipped              at 100 the floor ray finds nothing and the player floats.",
            dummy_z, floor_z
        ).into());
    }

    Ok(())
});
