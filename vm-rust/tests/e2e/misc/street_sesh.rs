use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_street_sesh.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Street Sesh (Zack Zeiler / etnies, Director 10, Shockwave 3D) — reported as
// not working. NOT a physics-Xtra game: the skater runs on the movie's own
// Lingo integrator, so the defects were all in the plain 3D + scripting surface.
//
// 1. `the soundLevel` had no getter, only a silent setter. The title screen's
//    "sound toggle" behavior reads `_sound.soundLevel` in its `beginSprite`, so
//    the movie died on its FIRST frame with "Cannot get movie prop soundLevel".
//
// 2. `perpendicularTo(v1, v2)` was missing. `_PhysO_groundcollision` aligns the
//    skater to the ground normal with
//      my.rotate(my.worldPosition, perpendicularTo(v1, tn), angleBetween(v1, tn), #world)
//    and Director accepts the global (verb) form of every 3D vector command, not
//    just `v1.perpendicularTo(v2)`.
//
// 3. `my.worldPosition.z = pFloor + 5` was a no-op, and THAT is what made the
//    game "not work". Director compiles it to `getchainedprop worldPosition`
//    followed by `setobjprop z` — an lvalue chain that writes through to the
//    node — while dirplayer's `worldPosition` getter hands back a fresh vector
//    (it is derived: it walks the parent chain), so the component write landed
//    on a temporary. `checkGroundCollision` found the floor and clamped to it
//    every single frame with the clamp thrown away, so the skater sank through
//    the world at terminal velocity and the camera followed it under the street
//    — a featureless dark grey stage.
//
// 4. The skater drew buried to the waist. A cloned model carries the biped COM
//    fold its source node held, but is deliberately absent from
//    `model_root_com`, so the renderer fell through to its idle tier and
//    stripped a DIFFERENT matrix than the one folded: the fold (taken from
//    "player_mike", a member holding the rig and no clips, so the REST root) is
//    (0, 4.42, -0.87), while `cpy_player2_idle` — cloned in afterwards, and
//    therefore invisible to the parser — has its frame-0 root at the pelvis,
//    (17.05, 22.48, 105.72). The renderer now strips the r0 the hop actually
//    carried, but only while the node still HOLDS it: AreaZero's
//    `setup_Elite` replaces its cloned rig's transform outright, and stripping
//    a destroyed fold threw that game's first-person weapon out of frame.
//
// 5. Nothing animated. `removeLast()` cleared `playing`, but Director scopes
//    that flag to the playback ENGINE (only `pause()` stops it); the movie
//    calls `play()` once at setup and then drives everything through
//    `qAnim` = flush-with-removeLast + queue. So the first flush stopped the
//    player for good, and Mike slid around — and crashed — in his bind pose.
//
browser_e2e_test!(test_misc_street_sesh_skater_rides_and_crashes, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d))
                .unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    let snapshots = SnapshotContext::new("misc", "street_sesh");

    // Boot: `init_game` waits for every `player_*` W3D member plus "world" to
    // reach state 4, clones the skater and board out of `player_mike`, then
    // parks on the title screen's hold frame.
    player.step_frames(300).await;
    probe(format!("title frame  = {}", player.current_frame()));
    probe(format!("world state  = {}", ask!("member(\"world\").state")));
    probe(format!("world models = {}", ask!("member(\"world\").model.count")));
    let _ = snapshots.verify("01_title", player.snapshot_stage());

    // `the soundLevel` has to read back, or the sound-toggle behavior on this
    // very frame kills the movie before it ever draws.
    let sound_level = ask!("_sound.soundLevel");
    assert_eq!(sound_level, "Int(7)", "the soundLevel default should be 7, got {}", sound_level);

    // PLAY — the GoAway! behavior on rect(429, 371, 536, 417).
    player.click(482, 394).await;
    player.step_frames(60).await;
    probe(format!("game frame   = {} game={}", player.current_frame(), ask!("ilk(game)")));

    let gc = "game.player.pPhysicsObject.pTerrainManagerObject";
    let start_z = ask!("string(game.player.pStartPos.z)");
    probe(format!("spawn z      = {}", start_z));

    // Let the skater drop the ~45 units from its authored spawn onto the plaza
    // and settle.
    player.step_frames(60).await;

    let z = ask!("string(game.player.my.worldPosition.z)");
    let floor = ask!(&format!("string({}.pFloor)", gc));
    let airborne = ask!(&format!("{}.pAirborne", gc));
    probe(format!("settled: z={} floor={} airborne={}", z, floor, airborne));
    let _ = snapshots.verify("02_standing", player.snapshot_stage());

    // Skate: hold W. The FSM's skate state accelerates along the board's
    // heading, so the position has to CHANGE and the skater has to stay on the
    // deck while it does.
    let before = ask!("string(game.player.my.worldPosition)");
    player.key_down("w", 87).await;
    for i in 0..6 {
        player.step_frames(20).await;
        probe(format!(
            "skate {}: pos={} air={} state={}",
            i,
            ask!("string(game.player.my.worldPosition)"),
            ask!(&format!("{}.pAirborne", gc)),
            ask!("string(game.player.FSM.FSM.pCurrentStateObject)"),
        ));
    }
    player.key_up("w", 87).await;
    let after = ask!("string(game.player.my.worldPosition)");
    probe(format!("moved {} -> {}", before, after));
    let _ = snapshots.verify("03_skating", player.snapshot_stage());

    assert_ne!(
        before, after,
        "holding W did not move the skater — the skate state never accelerated"
    );

    // Still on the ground after skating: the whole point of the ground clamp is
    // that it holds every frame, not just the first.
    let z_after = ask!("string(game.player.my.worldPosition.z)");
    let z_after_val: f64 = z_after.trim_start_matches("String(\"").trim_end_matches("\")").parse()
        .unwrap_or(f64::NAN);
    probe(format!("z after skating = {}", z_after));
    assert!(
        z_after_val > -50.0,
        "the skater fell through the plaza while skating (z = {})", z_after
    );

    // Crash coverage: he ran into the planter, so `_STATE_crash` should own the
    // FSM and the bonesPlayer should be playing a crash clip, not idle.
    probe(format!("crashCount  = {}", ask!("game.pCrashCount")));
    let crash_state = ask!("string(game.player.FSM.FSM.pCurrentStateObject)");
    probe(format!("crash state = {}", crash_state));
    assert!(
        crash_state.contains("_STATE_crash"),
        "skating into the planter should have handed the FSM to _STATE_crash,          state is {}",
        crash_state
    );
    probe(format!("finalCrash  = {}", ask!("game.player.FSM.pState_crash.pFinalCrashFlag")));
    probe(format!("crashTimer  = {}", ask!("game.player.FSM.pState_crash.pCrashTimer")));
    probe(format!("playList    = {}", ask!("string(gWorld.model(\"player2\").bonesPlayer.playList)")));
    probe(format!("currentTime = {}", ask!("gWorld.model(\"player2\").bonesPlayer.currentTime")));
    let playing = ask!("gWorld.model(\"player2\").bonesPlayer.playing");
    probe(format!("playing     = {}", playing));
    let mut times: Vec<String> = Vec::new();
    for i in 0..4 {
        player.step_frames(8).await;
        let t = ask!("string(gWorld.model(\"player2\").bonesPlayer.currentTime)");
        probe(format!(
            "crash {}: t={} playing={} playList={}",
            i, t,
            ask!("gWorld.model(\"player2\").bonesPlayer.playing"),
            ask!("string(gWorld.model(\"player2\").bonesPlayer.playList)"),
        ));
        times.push(t);
        let _ = snapshots.verify(&format!("04_crash_{}", i), player.snapshot_stage());
    }

    // The crash clip has to be RUNNING: `playing` is the engine flag, and the
    // clock has to advance. Both were dead while `removeLast()` paused the
    // player, which is why Mike crashed standing up.
    assert_eq!(
        playing, "Int(1)",
        "the bonesPlayer engine is stopped after the crash — `play()` is only          called once, at setup, so nothing can restart it"
    );
    assert!(
        times.first() != times.last(),
        "the crash animation clock never advanced (currentTime stuck at {:?})",
        times.first()
    );

    Ok(())
});
