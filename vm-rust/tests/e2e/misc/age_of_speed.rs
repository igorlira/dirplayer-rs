use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_age_of_speed.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Miniclip: Age of Speed — Shockwave 3D + Havok, D11.5. Shares the Miniclip 3D
// engine (`MovieScript 36 - Functions.ls`) with Agent Free Ride.
//
// The HUD numbers (time / lap / KM/H / score) are not sprites: each is text
// rasterised into a 3D overlay TEXTURE by `utiCreateTextureFromText`, which
// composes the text member's `.image` onto an `image(w,h,32)` canvas
// pre-filled with `setAlpha(1)`. They render as near-invisible ghosts, so this
// test exists to walk the movie far enough into gameplay for those textures to
// be built and refreshed. See docs/age-of-speed-hud-alpha.md.
browser_e2e_test!(test_misc_age_of_speed_hud, |player| async move {
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

    player.step_frames(60).await;
    probe(format!("boot frame={}", player.current_frame()));

    let snapshots = SnapshotContext::new("misc", "age_of_speed");
    let _ = snapshots.verify("01_boot", player.snapshot_stage());

    // Drive the menu by RESULT, not by a fixed frame count. The movie holds on
    // its Miniclip intro frame until the SWF reports in, and under the Flash
    // warm-up that hold is ~3 s — every instance costs the full 3000 ms AS-init
    // cap in the harness (docs/flash-instance-warmup-handoff.md §6). A fixed
    // `step_frames(60)` fired all three clicks into the intro, the menu never
    // took them, and the race never started: `gPlayerVehicle` stayed VOID and the
    // hover assertion below read an empty `pIsHoveringList`.
    //
    // The three clicks are the menu path (mode -> level -> start). Re-running the
    // sequence is harmless while the menu is still up, and the loop stops the
    // moment the game answers with a vehicle.
    let mut started = false;
    for attempt in 0..25 {
        player.click(90, 343).await;
        player.step_frames(20).await;
        player.click(201, 279).await;
        player.step_frames(20).await;
        player.click(438, 359).await;
        player.step_frames(60).await;
        // NB: this test's `ask!` formats with `{:?}`, so an empty answer comes
        // back as the TEXT `String("")` — never as an empty Rust string. Key off
        // the value the rest of the test depends on instead: the Havok world
        // having bodies in it.
        let bodies = ask!("string(gHavok.rigidBody.count)");
        if bodies.contains('"') && !bodies.contains("\"0\"") && !bodies.contains("<err") {
            probe(format!("menu taken on attempt {} (frame {}, bodies {})",
                attempt, player.current_frame(), bodies));
            started = true;
            break;
        }
    }
    assert!(started,
        "the menu never started a race — gPlayerVehicle is still VOID at frame {}.          The movie holds on its intro frame until the Flash SWF reports in.",
        player.current_frame());

    // Walk into the race. The level streams a W3D track before gameplay starts.
    player.step_frames(400).await;
    probe(format!("loaded frame={}", player.current_frame()));
    player.step_frames(200).await;

    // The teleport handles: gHavok + gVehiclePrefix are real Lingo globals, so
    // the chassis rigid body is reachable without chasing member(179+gLevelID).
    probe(format!(
        "prefix={} bodies={} pos={} rot={}",
        ask!("string(gVehiclePrefix)"),
        ask!("string(gHavok.rigidBody.count)"),
        ask!("string(gHavok.rigidBody(gVehiclePrefix & \"_chassis\").position)"),
        ask!("string(gHavok.rigidBody(gVehiclePrefix & \"_chassis\").rotation)"),
    ));

    // What are the HUD text members, and what does their .image actually carry?
    for name in ["TxtTime", "TxtLap", "TxtSpeed", "TxtScore", "TxtRecord"] {
        probe(format!(
            "{}: type={} color={} image.useAlpha={} depth={} rect={}",
            name,
            ask!(&format!("string(member(\"{}\").type)", name)),
            ask!(&format!("string(member(\"{}\").color)", name)),
            ask!(&format!("string(member(\"{}\").image.useAlpha)", name)),
            ask!(&format!("string(member(\"{}\").image.depth)", name)),
            ask!(&format!("string(member(\"{}\").image.rect)", name)),
        ));
    }

    let _ = snapshots.verify("02_gameplay", player.snapshot_stage());

    // ---- Teleport to the top of the loop, then drive down it ----------------
    //
    // Reported by hand from the Message Inspector. The chained form
    // `gHavok.rigidBody(...).position = v` parses but silently writes nothing,
    // so the body has to be bound to a variable first.
    //
    // The car is a HOVER vehicle (`InitHover` sets friction 0 on every body), so
    // the velocities are zeroed on arrival or it slides off before it settles.
    // Dropped at the loop, the car does NOT drive: with the throttle held it
    // creeps at v_y ~3.7 for a full 3.3s while vz thrashes +-50 and z jitters
    // around 10060 — it is stuck in the road surface, getting no drive at all.
    // (Around sample 10 the game's own recovery rescues it, so only the early
    // samples measure the defect.) The chained form
    // `gHavok.rigidBody(...).position = v` parses but writes nothing, so the
    // body has to be bound to a variable first. The car is a HOVER vehicle
    // (`InitHover` sets friction 0 on every body), hence zeroing the velocities.
    player.eval("gRB = gHavok.rigidBody(gVehiclePrefix & \"_chassis\")")
        .await.map_err(|e| e.message)?;
    probe(format!("race start at {}", ask!("string(gRB.position)")));

    // Does the car actually SETTLE onto the road at the start of the race?
    // `applyGravityForce` (registerStepCallback) is the only thing pulling it
    // down, so a chassis that stays high with no wheels hovering means the step
    // callback isn't reaching the body. This is the case the rest of the test
    // misses: it teleports to the loop, where the car is already supported, so
    // a car left floating at the start still passes everything below.
    for i in 0..6 {
        probe(format!(
            "start settle {}: z={} hover={} kmh={}",
            i,
            ask!("string(gRB.position.z)"),
            ask!("string(gPlayerVehicle.pIsHoveringList)"),
            ask!("string(gPlayerVehicle.pSpeedKmh)"),
        ));
        player.step_frames(20).await;
    }

    // The car must be SUPPORTED at the start line. `pIsHoveringList` is one flag
    // per wheel, set by `updateHover`'s raycast; all four means the chassis is
    // riding the road. This is the only assertion that catches "cars hang in the
    // air at the start" — the teleport section below drops the car onto the loop
    // where it is supported regardless, so a floating start passes everything
    // else in this test. Measured healthy: z settles into a 62-65 band with
    // hover [1,1,1,1] and the car stationary.
    let hover = ask!("string(gPlayerVehicle.pIsHoveringList)");
    assert!(hover.matches('1').count() >= 4,
        "car is not resting on the road at the start line: pIsHoveringList = {}          (gravity from the applyGravityForce step callback is not reaching the chassis)",
        hover);

    player.key_down("ArrowUp", 38).await;
    // `updateHover` ray-casts per wheel and sets pIsHoveringList; pPowerCoeff is
    // the SUM of those four flags, so it reports how much of the car is actually
    // supported. gPlayerVehicle is a global, so both are readable. Speed is the
    // headline: the reported bug is that the car sinks and SLOWS coming down the
    // loop, so a speed collapse paired with a z descent is the signature.
    // Get the race actually RUNNING before teleporting. Dropping the car in
    // during the pre-race hold makes it creep at ~3.6 with no drive and then the
    // game's recovery snaps it away — that measures the countdown, not the loop.
    for _ in 0..25 {
        player.step_frames(10).await;
    }
    probe(format!(
        "racing at {} kmh={}",
        ask!("string(gRB.position)"), ask!("string(gPlayerVehicle.pSpeedKmh)")
    ));
    probe(format!(
        "chassisMass={} rbMass={} friction={}",
        // NOT rb.mass: `InitVehicle` overwrites pChassisMass from kSetupData, and
        // the hover impulse is scaled by THIS, not by the body's real mass.
        ask!("string(gPlayerVehicle.pChassisMass)"),
        ask!("string(gRB.mass)"),
        // InitHover zeroes friction on every vehicle body; the static-contact
        // filter keys off that to decide whether an upward road contact is safe
        // to keep.
        ask!("string(gRB.friction)"),
    ));
    probe(format!(
        "wheels={} chassisOfs={} worldUp={}",
        ask!("string(gPlayerVehicle.pWheelPointList)"),
        ask!("string(gPlayerVehicle.pChassisMDL.transform.position)"),
        ask!("string(gPlayerVehicle.pWorldUp)"),
    ));
    // `console.read()` DRAINS the buffer, so this discards the pre-race settle
    // and leaves the final read holding the loop run only. Without it the dump
    // is swamped by the spawn drop (which is all the old 12-line head showed).
    let _ = vm_rust::player::reserve_player_ref(|p| p.console.read());

    // Now drop it into the loop, carrying speed so it can actually run round.
    const LOOP: &str = "vector(385.6400, 2413.6635, 10062.4267)";
    const LOOP_FACING: &str = "[vector(0.0000, -0.0012, -1.0000), 179.6216]";
    player.eval(&format!("gRB.position = {}", LOOP)).await.map_err(|e| e.message)?;
    player.eval(&format!("gRB.rotation = {}", LOOP_FACING)).await.map_err(|e| e.message)?;
    player.eval("gRB.linearVelocity = vector(0, -6000, 0)").await.map_err(|e| e.message)?;
    player.eval("gRB.angularVelocity = vector(0, 0, 0)").await.map_err(|e| e.message)?;

    // NB: scaling the hover impulse uniformly does NOT fix the loop. Measured
    // 200% -> rides at dist 67-72, 50% -> rides at dist 8-9 (visibly sunk on
    // FLAT road too), and both still over-penetrate at the loop and launch,
    // with speed collapsing the same way. The failure is not overall spring gain.
    // EXPERIMENT: pin gTimeStep to Director's measured median (0.013) instead of
    // our 0.022-0.050. `updateHover` reads gTimeStep BEFORE exitFrame reassigns
    // it, so writing it between frames is what the hover actually sees. Linearised
    // stiffness gives w ~ 99 rad/s and a once-per-frame impulse is stable only
    // while w*dt < 2 (dt < ~0.020) — Director sits inside that band, we don't.
    // Set to 0.0 to disable and use the movie's own value.
    // RESULT: pinning it INVALIDATES the test — it shrinks the hover impulse
    // while the frame interval is unchanged, i.e. it just weakens the spring.
    // Measured at 0.013 the car rode at dist ~8.2 on FLAT road. Leave at 0.0.
    const PIN_GTS: f64 = 0.0;

    for i in 0..40 {
        for _ in 0..10 {
            if PIN_GTS > 0.0 {
                let _ = player.eval(&format!("gTimeStep = {}", PIN_GTS)).await;
            }
            player.step_frames(1).await;
        }
        // pModelList is what the culling manager hands the hover raycast as
        // #modelList. If it empties out over the loop, the rays have nothing to
        // hit and the car drops through geometry that is physically right there.
        probe(format!(
            "{:03} pos={} v={} hover={} kmh={} models={} dists={}", i,
            ask!("string(gRB.position)"),
            ask!("string(gRB.linearVelocity)"),
            ask!("string(gPlayerVehicle.pIsHoveringList)"),
            ask!("string(gPlayerVehicle.pSpeedKmh)"),
            ask!("string(gPlayerVehicle.pModelList.count)"),
            ask!("string(gPlayerVehicle.pHoverDistList)"),
        ));
        // What holds the car to a loop is the movie's ROTATED gravity:
        // `pGravity = pShadow.GetGroundNormal() * -5500` (Shadow Controller ray).
        // If that ray misses, UpdateShadow resets it to (0,0,-5500) and the car
        // flies off the loop instead of sticking to it.
        // Which axle buries is set by how hard the body PITCHES. Drive force goes
        // through `applyImpulse` (no point => no torque), so the rotation comes
        // from the hover impulses' levers plus `FlushAngularImpulse`. Watch the
        // body's angular velocity: an over-strong angular response is what would
        // split the axles by 23 units where Director splits by 3.
        probe(format!(
            "    grav={} angvel={} jump={}",
            ask!("string(gPlayerVehicle.pGravity)"),
            ask!("string(gPlayerVehicle.pCurrAngVel)"),
            ask!("string(gPlayerVehicle.pJumpHeight)"),
        ));
        if i == 0 || i == 14 {
            // The hover impulse is scaled by gTimeStep (`diff / 1000.0`, set by
            // the Physics Controller). Too small a delta scales every lift force
            // down with it.
            probe(format!(
                "    tuning gTimeStep={} strength={} damping={} hoverDist={}",
                ask!("string(gTimeStep)"),
                ask!("string(gPlayerVehicle.pStrength)"),
                ask!("string(gPlayerVehicle.pDamping)"),
                ask!("string(gPlayerVehicle.pHoverDist)"),
            ));
            // Age of Speed disables engine gravity and applies its own via
            // `applyForce` from a step callback, so gravity goes through
            // step_native's game-force divider — which is calibrated as N² for
            // SuperSonic's N=7 substeps. If subSteps differs here, that
            // calibration is wrong for this movie and skews hover vs gravity.
            // The spring is `power(lDist, 1.2)` and the damper `power(v, 1.15)`.
            // If power() is wrong the whole hover law is wrong.
            probe(format!(
                "    power(13.03,1.2)={} power(2,0.5)={} power(10,2)={} power(4,1.15)={}",
                ask!("string(power(13.03, 1.2))"),
                ask!("string(power(2, 0.5))"),
                ask!("string(power(10, 2))"),
                ask!("string(power(4, 1.15))"),
            ));
            probe(format!(
                "    subSteps={} gravity={} chassisMass={}",
                ask!("string(gHavok.subSteps)"),
                ask!("string(gPlayerVehicle.pGravity)"),
                ask!("string(gRB.mass)"),
            ));
        }
    }
    player.key_up("ArrowUp", 38).await;

    // The debug movie (`age_of_speed_dbg.dcr`) carries `put` probes emitting the
    // same HOV/COL/BOOST lines as the Director capture in
    // `Age_of_speed_looping.txt`. Lingo `put` goes to the player's console buffer,
    // not the page console, so drain it here for a field-for-field diff.
    let dumped = vm_rust::player::reserve_player_ref(|player| player.console.read());
    probe(format!("DBG console lines={}", dumped.lines().count()));
    let loop_lines: Vec<&str> = dumped.lines().collect();

    // Director's capture has 1681 `car3_chassis` contacts, ALL against the loop
    // segments l_t21..l_t25 — its collision hull scrapes the loop the whole way
    // round at jump 28-43, which is extra support the hover spring never
    // provides. Histogram which segments we actually contact, so "we get zero
    // loop contacts" is a count rather than an impression.
    let mut seg_counts: Vec<(String, usize)> = Vec::new();
    for line in loop_lines.iter().filter(|l| l.contains("COL")) {
        let seg = line.split_whitespace().nth(3).unwrap_or("?").to_string();
        match seg_counts.iter_mut().find(|(s, _)| *s == seg) {
            Some((_, n)) => *n += 1,
            None => seg_counts.push((seg, 1)),
        }
    }
    probe(format!("DBG COL segments {:?}", seg_counts));
    // Field 6 of a COL line is pJumpHeight. Director's chassis first touches at
    // jump 43.5 and never gets below 27.8, i.e. the hull catches the car exactly
    // where the hover spring goes over its peak (max lift is at dist 35, and it
    // FALLS to zero as dist -> 0). Where OUR contacts sit on that scale says
    // whether the collision shape reaches as deep as Director's.
    let mut jumps: Vec<f64> = loop_lines.iter().filter(|l| l.contains("COL"))
        .filter_map(|l| l.split_whitespace().nth(8).and_then(|s| s.parse().ok()))
        .collect();
    jumps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    probe(format!(
        "DBG COL jump min={:?} p50={:?} max={:?} n={}",
        jumps.first(), jumps.get(jumps.len() / 2), jumps.last(), jumps.len()
    ));

    // Hover trace for the loop. The 10-frame harness sampling aliases the entry
    // transient, which is where the four wheels split apart, so sample the
    // movie's own per-frame HOV probes instead.
    for (i, line) in loop_lines.iter().filter(|l| l.contains("HOV")).enumerate() {
        if i % 4 == 0 { probe(format!("DBG {}", line)); }
    }

    let _ = snapshots.verify("04_loop_descent", player.snapshot_stage());

    // Keep stepping so the per-frame HUD refresh path (`texture.image = lImg`)
    // runs, not just the first-time `newTexture` path.
    player.step_frames(120).await;
    probe(format!("hud frame={}", player.current_frame()));

    let _ = snapshots.verify("03_hud", player.snapshot_stage());

    Ok(())
});
