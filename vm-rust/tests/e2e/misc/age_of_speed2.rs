use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_age_of_speed2.toml");

const FORCE_DIRECTOR_INERTIA: bool = false;

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// The chassis rigid body's mass, inertia tensor and the member's worldScale,
/// straight out of the Havok state. Director does not expose the tensor to
/// Lingo, so the comparison oracle is derived from its `angularMomentum` /
/// `angularVelocity` pairs — see docs/age-of-speed2-corkscrew-reset.md.
fn chassis_inertia(name: &str) -> String {
    use vm_rust::player::cast_member::CastMemberType;
    vm_rust::player::reserve_player_ref(|player| {
        for member in player.movie.cast_manager.casts.iter().flat_map(|c| c.members.iter()) {
            let CastMemberType::HavokPhysics(h) = &member.1.member_type else { continue };
            for rb in h.state.rigid_bodies.iter() {
                if !rb.name.as_str().eq_ignore_ascii_case(name) { continue }
                let i = rb.inertia_tensor;
                return format!(
                    "mass={:.1} worldScale={:.6} halfExtents={:?} isBox={} convex={} driven={} com={:?} hull={} I=[{:.4e} {:.4e} {:.4e} / {:.4e} {:.4e} {:.4e} / {:.4e} {:.4e} {:.4e}]",
                    rb.mass, h.state.scale, rb.inertia_half_extents, rb.is_box, rb.is_convex,
                    rb.driven, rb.center_of_mass,
                    rb.collision_hull_local.len(),
                    i[0], i[1], i[2], i[3], i[4], i[5], i[6], i[7], i[8]);
            }
        }
        format!("{}: NOT FOUND", name)
    })
}

/// EXPERIMENT: overwrite the chassis inertia with Director's measured tensor.
///
/// Director does not expose the tensor, but `angularMomentum` / `angularVelocity`
/// do: over 1933 frames of a Director capture, whenever omega lies along one body
/// axis, L/omega is 259.9 (X, n=932), 279.2 (Y, n=44) and 489.8 (Z, n=6). Those
/// are Lingo-scale, i.e. display-unit inertia x worldScale^2. Ours comes out
/// (678, 422, 1051) — same Z half-extent when solved back to a box, but too big
/// in the horizontal axes. Setting the measured values here isolates "is the
/// inertia what is costing us the roll correction" from everything else.
fn force_director_inertia(name: &str) -> String {
    use vm_rust::player::cast_member::CastMemberType;
    vm_rust::player::reserve_player_mut(|player| {
        for member in player.movie.cast_manager.casts.iter_mut().flat_map(|c| c.members.iter_mut()) {
            let CastMemberType::HavokPhysics(h) = &mut member.1.member_type else { continue };
            let ws = h.state.scale;
            for rb in h.state.rigid_bodies.iter_mut() {
                if !rb.name.as_str().eq_ignore_ascii_case(name) { continue }
                let k = 1.0 / (ws * ws);
                let d = [259.9 * k, 279.2 * k, 489.8 * k];
                rb.inertia_tensor = [d[0],0.0,0.0, 0.0,d[1],0.0, 0.0,0.0,d[2]];
                rb.inverse_inertia_tensor =
                    [1.0/d[0],0.0,0.0, 0.0,1.0/d[1],0.0, 0.0,0.0,1.0/d[2]];
                if rb.mass > 0.0 {
                    let m = rb.mass;
                    rb.unit_inertia_tensor =
                        [d[0]/m,0.0,0.0, 0.0,d[1]/m,0.0, 0.0,0.0,d[2]/m];
                }
                return format!("forced I to Director's: [{:.4e} {:.4e} {:.4e}]", d[0], d[1], d[2]);
            }
        }
        format!("{}: NOT FOUND", name)
    })
}

// Miniclip "Age of Speed 2" — steering-response harness. The sequel splits the
// vehicle across `Vehicle Base` (turn impulse) and `Human Input Controller`
// (the pSteering integrator), where AoS1 had both in one behavior.
browser_e2e_test!(test_misc_age_of_speed2_steering, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($($t:tt)*) => {{
            let e = format!($($t)*);
            match player.eval_datum(&format!("string({})", e)).await {
                Ok(vm_rust::director::static_datum::StaticDatum::String(s)) => s,
                other => format!("<{:?}>", other),
            }
        }};
    }

    // Wait for the movie to BUILD gGame rather than stepping a fixed count.
    // Frame 2 is "Frame Checker" — `go(the frame)`, held until the intro SWF
    // calls `flash_start_checker` through getURL("lingo:") — and only once it
    // releases does "Frame Initialize" run `gGame = script("AgeOfSpeed2 Game").new()`.
    //
    // Under the Flash warm-up that hold is ~3 s: the harness's `_level0._totalframes`
    // AS-init probe never succeeds headless, so every instance costs the full
    // 3000 ms cap (docs/flash-instance-warmup-handoff.md §6). A fixed
    // `step_frames(60)` landed well inside that window, so `flash_start` below ran
    // against a VOID gGame — every `gGame.SetX()` in it quietly no-oped, and its
    // closing `go(13)` stranded the movie on frame 15's loader loop, which then
    // polls `gGame.IsLoaded()` on VOID forever. Wait for the object instead.
    let mut booted = false;
    for i in 0..40 {
        player.step_frames(20).await;
        if !ask!("gGame").is_empty() {
            probe(format!("gGame built at boot step {} (frame {})", i, player.current_frame()));
            booted = true;
            break;
        }
    }
    assert!(
        booted,
        "gGame was never built — the movie never left the Frame Checker hold (frame {});          the intro SWF's `flash_start_checker` callback is the gate",
        player.current_frame()
    );

    // The Flash shell normally calls `flash_start` through getURL("lingo:").
    // Call it directly: quickRace=1, score=0, audio=1, help=1, levelId=1,
    // playerId=1, bestTime=0. It ends with go(13) into the loader.
    let r = player.eval("flash_start(1, 0, 1, 1, 1, 1, 0)").await;
    probe(format!("flash_start -> {:?}", r.is_ok()));
    player.step_frames(300).await;
    probe(format!("after start frame={} exitCode={}", player.current_frame(), ask!("gGame.GetExitCode()")));
    player.step_frames(300).await;
    probe(format!("frame={} ingame={}", player.current_frame(), ask!("gGame.GetIngame()")));

    // Wait for the race to actually start — pPlayers stays empty through the
    // load, and the vehicle only exists once the countdown hands over control.
    let mut racing = false;
    for i in 0..60 {
        player.step_frames(30).await;
        let n = ask!("gGame.GetPlayers().count");
        let veh = ask!("gGame.GetPlayerVehicle()");
        if i % 6 == 0 {
            probe(format!("wait {}: frame={} players={} vehicle={} label={}",
                i, player.current_frame(), n, veh, ask!("the frameLabel")));
        }
        if !veh.is_empty() && veh != "0" {
            probe(format!("race up at wait {} frame={} players={}", i, player.current_frame(), n));
            racing = true;
            break;
        }
    }
    assert!(racing, "the race never started — gGame.GetPlayers() is {} at frame {}",
        ask!("gGame.GetPlayers().count"), player.current_frame());

    // The countdown gates input: `Gameplay.go` ends it by calling
    // StartInputControl -> SetEnableUserInput(1) and SetGameStatus(#play).
    // Let it run, and if it has not fired yet, do what it does.
    let mut playing = false;
    for _ in 0..40 {
        player.step_frames(15).await;
        if ask!("gGame.GetGameStatus()") == "play" { playing = true; break; }
    }
    probe(format!("countdown finished on its own: {} (status {})",
        playing, ask!("gGame.GetGameStatus()")));
    if !playing {
        let _ = player.eval("gGame.GetPlayerVehicle().GetInputController().SetEnableUserInput(1)").await;
        let _ = player.eval("gGame.SetGameStatus(#play)").await;
        player.step_frames(5).await;
        probe(format!("forced start, status now {}", ask!("gGame.GetGameStatus()")));
    }

    // Accelerate to a steady speed, then hold left and sample the same fields
    // the Director probe emits.
    player.key_down("ArrowUp", 38).await;
    for _ in 0..40 { player.step_frames(5).await; }
    probe(format!("speed before turn = {} kmh",
        ask!("gGame.GetPlayerVehicle().GetVehicle().pSpeedKmh")));
    player.key_down("ArrowLeft", 37).await;
    for i in 0..18 {
        player.step_frames(1).await;
        probe(format!("AOS2 f{} steer={} yaw={} slide={} grip={} pc={} hov={} kmh={}",
            i,
            ask!("gGame.GetPlayerVehicle().GetVehicle().pSteering"),
            ask!("gGame.GetPlayerVehicle().GetVehicle().pCurrAngVel.dot(gGame.GetPlayerVehicle().GetVehicle().pWorldUp)"),
            ask!("gGame.GetPlayerVehicle().GetVehicle().pSlideSpeed"),
            ask!("gGame.GetPlayerVehicle().GetVehicle().pGrip"),
            ask!("gGame.GetPlayerVehicle().GetVehicle().pPowerCoeff"),
            ask!("gGame.GetPlayerVehicle().GetVehicle().pIsHoveringList"),
            ask!("gGame.GetPlayerVehicle().GetVehicle().pSpeedKmh")));
    }
    // ---- ResetToTrack probe ----
    // The movie places the car by ray-casting DOWN from 8000 units above the
    // token position and taking the first non-"_cam" hit. If that ray returns
    // nothing the z is never corrected and the car is dropped wherever
    // TokenToWorld put it. The movie `put`s its own diagnostics, so read them.
    let before = ask!("gGame.GetPlayerVehicle().GetVehicle().getPosition()");
    let _ = vm_rust::player::reserve_player_ref(|pl| pl.console.read()); // drain
    let _ = player.eval("gGame.GetPlayerVehicle().GetVehicle().ResetToTrack()").await;
    player.step_frames(4).await;
    let log = vm_rust::player::reserve_player_ref(|pl| pl.console.read());
    for line in log.lines().filter(|l| l.contains("reset") || l.contains("ResetToTrack")) {
        probe(format!("RESET {}", line));
    }
    probe(format!("RESET pos before={} after={}", before,
        ask!("gGame.GetPlayerVehicle().GetVehicle().getPosition()")));
    // Does the reset ray return anything at all, with the movie's own args?
    let _ = player.eval("gRayPos = gGame.GetPlayerVehicle().GetVehicle().getPosition()").await;
    let _ = player.eval("gRayModels = gGame.GetCullingManager().GetCullerBlockModels(gRayPos)").await;
    let _ = player.eval("gRayArgs = [#maxNumberOfModels: 4, #levelOfDetail: #detailed, #modelList: gRayModels]").await;
    probe(format!("RESET ray hits={} first={} | no-args hits={}",
        ask!("gGame.Get3D().modelsUnderRay(gRayPos + vector(0,0,8000), vector(0,0,-1.0), gRayArgs).count"),
        ask!("gGame.Get3D().modelsUnderRay(gRayPos + vector(0,0,8000), vector(0,0,-1.0), gRayArgs)[1].model.name"),
        ask!("gGame.Get3D().modelsUnderRay(gRayPos + vector(0,0,8000), vector(0,0,-1.0), 4, #detailed).count")));
    // The corkscrew is tokens 14/15 (l_t_twist_*). Reproduce exactly what
    // ResetToTrack does at each token and see whether the ray finds ground.
    for tok in [2, 13, 14, 15, 16, 40, 41] {
        let _ = player.eval(&format!(
            "gP = gGame.GetTokenManager().TokenToWorld({}, 0.5, 0.0)", tok)).await;
        let _ = player.eval("gM = gGame.GetCullingManager().GetCullerBlockModels(gP)").await;
        let _ = player.eval("gA = [#maxNumberOfModels: 4, #levelOfDetail: #detailed, #modelList: gM]").await;
        probe(format!("TWIST t{} pos={} cullerModels={} rayHits={} unfiltered={} first={}",
            tok,
            ask!("gP"),
            ask!("gM.count"),
            ask!("gGame.Get3D().modelsUnderRay(gP + vector(0,0,8000), vector(0,0,-1.0), gA).count"),
            ask!("gGame.Get3D().modelsUnderRay(gP + vector(0,0,8000), vector(0,0,-1.0), 4, #detailed).count"),
            ask!("gGame.Get3D().modelsUnderRay(gP + vector(0,0,8000), vector(0,0,-1.0), gA)[1].model.name")));
    }
    for tok in [14, 15, 40, 41, 42, 43] {
        let mut misses = Vec::new();
        for i in 0..9 {
            let lon = i as f64 / 8.0;
            let _ = player.eval(&format!(
                "gP = gGame.GetTokenManager().TokenToWorld({}, {}, 0.0)", tok, lon)).await;
            let _ = player.eval("gM = gGame.GetCullingManager().GetCullerBlockModels(gP)").await;
            let _ = player.eval("gA = [#maxNumberOfModels: 4, #levelOfDetail: #detailed, #modelList: gM]").await;
            let n = ask!("gGame.Get3D().modelsUnderRay(gP + vector(0,0,8000), vector(0,0,-1.0), gA).count");
            if n == "0" { misses.push(format!("{:.2}", lon)); }
        }
        probe(format!("SWEEP t{}: {} of 9 longitudinals find NO ground{}",
            tok, misses.len(),
            if misses.is_empty() { String::new() } else { format!(" -> {}", misses.join(",")) }));
    }


    // ---- corkscrew entry: interpolatingMoveTo must apply its ROTATION ----
    //
    // `hkRigidBody.interpolatingMoveTo(position, rotation)` takes an absolute
    // orientation as [axis, angleDegrees]. dirplayer used to set only the
    // position and drop the rotation, which is how the car "fell out of the
    // world" on the corkscrew: `AgeOfSpeed2 Gameplay.ResetToTrackCallback`
    // recovers a lost car with
    //   GetChassisRB().interpolatingMoveTo(tokenPos + tokenUp * 450, [tokenUp, yaw])
    // so the car was put back over the road still in the attitude it had while
    // it was crashing — upside-down, sideways, nose-down. Its four hover rays
    // then point away from the road, it never re-seats, the out-of-track timer
    // fires again, and it resets into the same attitude forever.
    //
    // Place the car at the mouth of the twist the same way the movie does and
    // assert the heading actually came out where it was asked for.
    player.key_up("ArrowLeft", 37).await;
    let _ = player.eval("gV = gGame.GetPlayerVehicle().GetVehicle()").await;
    let _ = player.eval("gT = gGame.GetTokenManager()").await;
    probe(format!("INERTIA {}", chassis_inertia("veh_chassis_1")));
    if FORCE_DIRECTOR_INERTIA {
        probe(format!("INERTIA {}", force_director_inertia("veh_chassis_1")));
    }
    let _ = player.eval("gRB0 = gV.GetChassisRB()").await;
    probe(format!("INERTIA lingo L={} w={} mass={}",
        ask!("gRB0.angularMomentum"), ask!("gRB0.angularVelocity"), ask!("gRB0.mass")));
    let _ = player.eval("gRef = gT.GetTokenRef(#t14)").await;
    let _ = player.eval("gNP = gT.TokenToWorld3DByRef(gRef, 0.02, 0.0)").await;
    let _ = player.eval("gUp = gRef.normal").await;
    let _ = player.eval("gRes = gT.getToken(#t14, gNP.x, gNP.y, gNP, 0.0, 0.0)").await;
    let _ = player.eval("gTan = gRes[3]").await;
    // Director's convention: rotating +Y by `ang` about the token up lands on
    // the token tangent (see ResetToTrackCallback for the same three lines).
    let ang: f64 = ask!("gTan.angleBetween(vector(0.0,1.0,0.0))").parse().unwrap_or(0.0);
    let vz: f64 = ask!("gTan.crossProduct(vector(0.0,1.0,0.0)).z").parse().unwrap_or(0.0);
    let ang = if vz >= 0.0 { -ang } else { ang };
    let _ = player.eval(&format!(
        "gV.GetChassisRB().interpolatingMoveTo(gNP + (gUp * 450.0), [gUp, {}])", ang)).await;
    let _ = player.eval("gV.SetCurrentToken(#t14)").await;
    player.step_frames(1).await;
    let tan = ask!("gTan");
    let car_y = ask!("gV.pChassisMDL.transform.yAxis");
    probe(format!("ENTRY at={} up={} tan={} yaw={:.1} carY={}",
        ask!("gNP"), ask!("gUp"), tan, ang, car_y));
    let parse_v = |s: &str| -> [f64; 3] {
        let inner = s.trim_start_matches("vector(").trim_end_matches(')');
        let mut it = inner.split(',').map(|p| p.trim().parse::<f64>().unwrap_or(f64::NAN));
        [it.next().unwrap_or(f64::NAN), it.next().unwrap_or(f64::NAN), it.next().unwrap_or(f64::NAN)]
    };
    let (t, c) = (parse_v(&tan), parse_v(&car_y));
    let dot = t[0] * c[0] + t[1] * c[1] + t[2] * c[2];
    assert!(dot > 0.99,
        "interpolatingMoveTo dropped its rotation: asked for heading {} (yaw {:.1} about {}), \
         got {} (dot {:.3})", tan, ang, ask!("gUp"), car_y, dot);

    // Enter the corkscrew the way a racing car actually enters it. This matters
    // more than it looks: the twist's centreline IS the roll axis, so a car at
    // transversal 0 travels a straight line and gets NO centripetal force at
    // all — it can only fall off, at any speed. A real car enters ~1800 units
    // off-centre and is swept around the barrel; a Director capture of the same
    // corner holds that offset the whole way (x -96296 at the mouth becomes
    // z +1790 at 90 degrees of roll) at ~335 km/h, which is ~2.6 g. Reproduce
    // that entry — offset and speed — or the measurement means nothing.
    //
    // pSpeedKmh = pCurrSpeed * 0.036, so one unit is a centimetre and 335 km/h
    // is ~9330 units/s.
    let x_at_1: f64 = {
        let _ = player.eval("gEdge = gT.TokenToWorld3DByRef(gRef, 0.03, 1.0)").await;
        ask!("gEdge.x").parse().unwrap_or(f64::NAN)
    };
    let x_at_0: f64 = {
        let _ = player.eval("gMid = gT.TokenToWorld3DByRef(gRef, 0.03, 0.0)").await;
        ask!("gMid.x").parse().unwrap_or(f64::NAN)
    };
    // Director sits 1796 units off the centreline at the mouth of t14.
    let trasv = -1796.0 / (x_at_1 - x_at_0);
    let _ = player.eval(&format!(
        "gNP = gT.TokenToWorld3DByRef(gRef, 0.03, {})", trasv)).await;
    let _ = player.eval(&format!(
        "gV.GetChassisRB().interpolatingMoveTo(gNP + (gUp * 450.0), [gUp, {}])", ang)).await;
    let _ = player.eval("gV.SetCurrentToken(#t14)").await;
    // NB: assign through a variable. `gV.GetChassisRB().linearVelocity = v`
    // silently does nothing — a chained call is not a valid assignment target.
    let _ = player.eval("gRB = gV.GetChassisRB()").await;
    let _ = player.eval("gRB.linearVelocity = gTan * 9330.0").await;
    probe(format!("ENTRY2 trasv={:.3} at={} vSet={}", trasv, ask!("gNP"),
        ask!("gRB.linearVelocity")));
    player.step_frames(1).await;
    probe(format!("ENTRY2 after 1 frame v={} kmh={}",
        ask!("gRB.linearVelocity"), ask!("gV.pSpeedKmh")));
    player.key_down("ArrowUp", 38).await;
    let mut max_lon = 0.0f64;
    let mut max_bank = 0.0f64;
    let mut reached_t15 = false;
    let mut reached_t16 = false;
    // Roll error = shadowNormal . carXaxis, the exact quantity the movie's own
    // roll-alignment torque is proportional to (`lXProj` in
    // AgeOfSpeed2 Vehicle.UpdateGravity). Director holds this to a median of
    // 0.084 / p90 0.148 across two corkscrew passes; anything much larger means
    // the car is riding cocked on one edge instead of flat on the road.
    let mut roll_err: Vec<f64> = Vec::new();
    let mut pitch_err: Vec<f64> = Vec::new();
    for s in 0..80 {
        player.step_frames(2).await;
        let tok = ask!("gV.GetCurrentToken()");
        let lon: f64 = ask!("gV.GetLongitudinal()").parse().unwrap_or(0.0);
        let up = parse_v(&ask!("gV.pWorldUp"));
        if tok == "t14" {
            max_lon = max_lon.max(lon);
            // Bank = how far the car's own up has rolled off world up.
            max_bank = max_bank.max(up[2].acos().to_degrees());
        }
        if tok == "t15" { reached_t15 = true; max_bank = max_bank.max(up[2].acos().to_degrees()); }
        if tok == "t14" || tok == "t15" {
            roll_err.push(ask!("gV.GetShadowIntersectionNormal().dot(gV.pChassisMDL.transform.xAxis)")
                .parse::<f64>().unwrap_or(0.0).abs());
            pitch_err.push(ask!("gV.GetShadowIntersectionNormal().dot(gV.pChassisMDL.transform.yAxis)")
                .parse::<f64>().unwrap_or(0.0).abs());
        }
        if tok == "t16" { reached_t16 = true; }
        if s % 4 == 0 {
            probe(format!("TWIST s{} tok={}@{:.3} pos={} up={} hov={} dist={} kmh={}",
                s, tok, lon, ask!("gV.getPosition()"), ask!("gV.pWorldUp"),
                ask!("gV.pIsHoveringList"), ask!("gV.pHoverDistList"),
                ask!("gV.pSpeedKmh")));
        }
    }
    probe(format!("TWIST reached t15={} t16={}", reached_t15, reached_t16));
    let pct = |v: &mut Vec<f64>, q: f64| -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() { f64::NAN } else { v[((v.len() as f64 - 1.0) * q) as usize] }
    };
    probe(format!("TWIST roll err n={} med={:.4} p90={:.4} max={:.4} | pitch med={:.4} p90={:.4}  (Director: roll med 0.084 p90 0.148 max 0.219)",
        roll_err.len(), pct(&mut roll_err, 0.5), pct(&mut roll_err, 0.9), pct(&mut roll_err, 1.0),
        pct(&mut pitch_err, 0.5), pct(&mut pitch_err, 0.9)));
    probe(format!("TWIST reached lon={:.3} of t14, banked to {:.0} deg", max_lon, max_bank));
    // Before the fix the car left t14 backwards immediately and never banked at
    // all. Half the segment and past 45 degrees of roll is well clear of that
    // and well short of where it still comes off (~120 deg) — see
    // docs/age-of-speed2-corkscrew-reset.md.
    assert!(max_lon > 0.5,
        "the car never got into the corkscrew: furthest longitudinal on t14 was {:.3}", max_lon);
    assert!(max_bank > 45.0,
        "the car never banked with the corkscrew: max roll was {:.0} deg", max_bank);

    // ---- ResetToTrack from the corkscrew ----
    // The movie registers `ResetToTrackCallback` through
    // `SetResetToTrackCallbackData`, so the ray-based fallback in
    // `Vehicle Base.ResetToTrack` (TokenToWorld + a ray down from +8000) never
    // runs in this movie at all. The callback walks back to the last token that
    // is not a jump/twist/loop and places the car there, upright.
    probe(format!("RESETCB callback={} script={}",
        ask!("gV.pResetToTrackCallback"), ask!("gV.pResetToTrackCallbackScript")));
    assert_eq!(ask!("gV.pResetToTrackCallback"), "ResetToTrackCallback",
        "AoS2 must recover through its own callback, not the raycast fallback");
    for tok in ["#t14", "#t15", "#t41", "#t42"] {
        let _ = player.eval(&format!("gV.SetCurrentToken({})", tok)).await;
        let _ = player.eval("gV.ResetToTrack()").await;
        player.step_frames(2).await;
        let pos = parse_v(&ask!("gV.getPosition()"));
        let landed = ask!("gV.GetCurrentToken()");
        probe(format!("RESETON {} -> token={} pos={:?}", tok, landed, pos));
        // Recovery must put the car on the elevated safe token, not on the
        // ground plane under it.
        // The callback places the car at tokenUp * 450 over a token whose own z
        // is ~0, then physics settles it; anything near the ground plane means
        // the rotation or the 3D token height was lost.
        assert!(pos[2] > 250.0 && pos[2] < 1200.0,
            "ResetToTrack from {} dropped the car to z={:.0} (token {})", tok, pos[2], landed);
    }

    player.key_up("ArrowUp", 38).await;
    Ok(())
});
