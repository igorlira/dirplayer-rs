use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_street_sesh2.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

fn num(answer: &str) -> f64 {
    answer
        .trim_start_matches("String(\"")
        .trim_end_matches("\")")
        .trim()
        .parse()
        .unwrap_or(f64::NAN)
}

// Street Sesh 2: Downhill (Zack Zeiler / etnies, Director 10.1, Shockwave 3D) —
// reported as not working. Like the first Street Sesh this is NOT a physics-Xtra
// game: no Havok cast member, no dynamiks.x32; the skater runs on the movie's
// own Lingo integrator in `_level.stepit`, raycasting the street with
// `modelsUnderRay`. Three defects, each of which killed the game outright:
//
// 1. `member.ambientColor` had neither a getter nor a setter. `_game.initGFX`
//    opens the world with `gWorld.ambientColor = rgb(128, 128, 128)`, so
//    `_game.new()` raised part-way through — `game` stayed VOID, the level and
//    the HUD were never built, and the movie sat on its setup frame forever.
//
// 2. The `node.worldPosition.<component> = v` lvalue chain was dropped whenever
//    the RIGHT-hand side read a property of its own. Director evaluates the RHS
//    BETWEEN the receiver read and the component write:
//        getprop my / getchainedprop worldPosition
//        getprop pStartPos / getobjprop x
//        setobjprop x
//    and `vector_prop_lvalue` was a single slot that `pStartPos.x` overwrote. So
//    `_level.new`'s closing `my.worldPosition.x = pStartPos.x` never put the
//    skater on the start line: he began ~67 units off-centre.
//
// 3. `model.rotate(position, axis, angle {, relativeTo})` — Director's third
//    `rotate` overload, "a rotation about an arbitrary axis passing through a
//    point in space" — was not implemented, and the pivot form fell through to
//    the Euler reader: both vectors answered 0 and the ANGLE was read as a
//    z-rotation. `checkG` aligns the skater to the road with
//        my.rotate(my.worldPosition, perpendicularTo(v1, tn), angleBetween(v1, tn), #world)
//    so instead of tilting onto the slope he was yawed about the WORLD ORIGIN a
//    few degrees per frame — he spiralled off the street sideways within about
//    two seconds and the camera never left its pre-roll pose.
browser_e2e_test!(test_misc_street_sesh2_skater_rides_downhill, |player| async move {
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

    let snapshots = SnapshotContext::new("misc", "street_sesh2");

    // The title screen holds on `hold`; the world and every `player_*` clip are
    // already parsed by then (`init_game` waits for state 4 on all seventeen).
    player.step_frames(300).await;
    probe(format!("title frame  = {}", player.current_frame()));
    probe(format!("world state  = {}", ask!("member(\"world\").state")));
    probe(format!("world models = {}", ask!("member(\"world\").model.count")));
    let _ = snapshots.verify("01_title", player.snapshot_stage());

    // CLICK TO PLAY! — the GoAway! behavior on the title button.
    player.click(280, 302).await;
    player.step_frames(20).await;

    let game = ask!("ilk(game)");
    probe(format!("game = {} frame = {}", game, player.current_frame()));
    assert_eq!(
        game, "Symbol(\"instance\")",
        "`_game.new()` never returned — `initGFX` raises unless the member's \
         `ambientColor` is settable"
    );

    // Defect 1, read back: `ambientColor` is the member's default ambient light
    // (Director 11.5 Scripting Dictionary), and `initGFX` sets it to mid-grey.
    let ambient = ask!("string(gWorld.ambientColor)");
    probe(format!("ambientColor = {}", ambient));
    assert!(
        ambient.contains("128"),
        "member.ambientColor did not read back what initGFX wrote, got {}", ambient
    );

    // Defect 2: the skater has to START on the centre line. `_level.new` clones
    // him off `player_mike`, hangs him and the board off `player_dummy` and then
    // recentres the dummy with `my.worldPosition.x = pStartPos.x`.
    let start_x = num(&ask!("string(game.level.pStartPos.x)"));
    let my_x = num(&ask!("string(game.level.my.worldPosition.x)"));
    probe(format!("spawn x = {} (pStartPos.x = {})", my_x, start_x));
    assert!(
        (my_x - start_x).abs() < 0.01,
        "the skater did not land on the start line: x = {}, pStartPos.x = {} — \
         `my.worldPosition.x = pStartPos.x` was dropped",
        my_x, start_x
    );

    // The pre-roll: `_level.stepit` does nothing at all until `pLaunch`, and only
    // the `clicktocontinue` behavior on the 3D sprite sets it.
    player.click(280, 240).await;
    player.step_frames(4).await;
    let launched = ask!("game.level.pLaunch");
    probe(format!("pLaunch = {}", launched));
    assert_eq!(launched, "Int(1)", "clicking the stage did not launch the run");

    // Roll down the hill. He starts locked (#pre) and the camera hands over to
    // #play once he passes the start line at pStartPos.y.
    let mut last_y = num(&ask!("string(game.level.my.worldPosition.y)"));
    for i in 0..8 {
        player.step_frames(30).await;
        let y = num(&ask!("string(game.level.my.worldPosition.y)"));
        probe(format!(
            "ride{} pos={} cam={} air={} crash={}",
            i,
            ask!("string(game.level.my.worldPosition)"),
            ask!("string(game.level.pCameraMode)"),
            ask!("game.level.pAirborne"),
            ask!("game.level.pCrash"),
        ));
        assert!(
            y < last_y,
            "the skater stopped going downhill at step {}: y {} -> {}",
            i, last_y, y
        );
        last_y = y;
        let _ = snapshots.verify(&format!("03_ride_{}", i), player.snapshot_stage());
    }

    // Defect 3, three ways. He must still be ON the street — the world-origin
    // yaw threw him hundreds of units sideways within a second or two...
    let x = num(&ask!("string(game.level.my.worldPosition.x)"));
    probe(format!("x after the run = {}", x));
    assert!(
        (x - start_x).abs() < 200.0,
        "the skater drifted off the street sideways (x = {}, started at {})",
        x, start_x
    );

    // ...he must be ON the deck, not sunk through it or launched off it...
    let z = num(&ask!("string(game.level.my.worldPosition.z)"));
    let floor = num(&ask!("string(game.level.pFloor)"));
    probe(format!("z = {} floor = {}", z, floor));
    assert!(
        (z - (floor + 5.0)).abs() < 20.0,
        "the skater is not riding the road surface: z = {}, pFloor = {}", z, floor
    );

    // ...and his up axis must have tilted onto the slope, which is the whole
    // point of the `rotate(position, axis, angle, #world)` call in `checkG`.
    probe(format!(
        "zAxis = {} normal = {}",
        ask!("string(game.level.my.transform.zAxis)"),
        ask!("string(game.level.pNormal)")
    ));
    let zy = num(&ask!("string(game.level.my.transform.zAxis.y)"));
    let ny = num(&ask!("string(game.level.pNormal.y)"));
    assert!(
        ny.abs() > 0.01 && (zy - ny).abs() < 0.05,
        "the skater never aligned to the road normal: zAxis.y = {}, normal.y = {}",
        zy, ny
    );

    // The chase camera has to have taken over and be following him down.
    let cam_mode = ask!("string(game.level.pCameraMode)");
    let cam_y = num(&ask!("string(gCam.worldPosition.y)"));
    probe(format!("cameraMode = {} camera y = {}", cam_mode, cam_y));
    assert_eq!(cam_mode, "String(\"play\")", "the camera never left the pre-roll pose");
    assert!(
        cam_y < -500.0,
        "the chase camera did not follow the skater downhill (y = {})", cam_y
    );

    Ok(())
});
