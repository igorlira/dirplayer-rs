use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_agent_freeride.toml");

fn probe(msg: String) {
    // `println!` from wasm goes nowhere; the page console is what E2E_CONSOLE
    // forwards to the terminal.
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Miniclip: Agent Free Ride — Shockwave 3D + AGEIA PhysX (Dynamiks.x32), D11.5.
//
// Both PLAY buttons live INSIDE the full-stage Flash sprite, so they are clicked
// by stage coordinate rather than by sprite. After the second click the movie
// streams its track (2162 models) and builds the culling manager before gameplay
// starts, which is why the load walk below is long.
//
// The guard at the end is the boarder's footing, which its hover rig maintains
// with four downward rays. The opening of the run is legitimately AIRBORNE: the
// boarder spawns above the course, drops to it and does a trick, so ground
// distance reaches ~1950 with all four rays missing ([0,0,0,0]) before it settles
// — roughly frame 250 of the 300 below. So neither a single sample nor "did it
// ever come near the ground" says anything; a build that drops the boarder
// straight PAST the track would satisfy both. What must hold is that it ends the
// window SETTLED on the track, so the check looks at the last samples.
browser_e2e_test!(test_misc_agent_freeride_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    // The config's `[movie] startup_do` — the projector's `--do` argument. This
    // movie is entered through `wrapper_silentbaystudios.dcr`, which redirects
    // to whatever `member("gameUrl").text` holds; the .dcr ships that member
    // holding a "game-name/game_name.dcr" PLACEHOLDER, so without this the
    // wrapper resolves the placeholder and the mount 404s.
    cfg.apply_startup_do();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    // The wrapper -> gameloader -> agent_freeride hand-off takes a few hundred
    // stepped frames; `gGame` only exists once the GAME movie is mounted and has
    // run its "Frame Initialize". Wait for it rather than guessing a count.
    let mut mounted = false;
    for _ in 0..40 {
        player.step_frames(20).await;
        // A bare `gGame` parses as a call; ask for its string form instead —
        // VOID stringifies to EMPTY.
        if let Ok(vm_rust::director::static_datum::StaticDatum::String(s)) =
            player.eval_datum("string(gGame)").await
        {
            if !s.is_empty() {
                mounted = true;
                break;
            }
        }
    }
    probe(format!("game movie mounted={} frame={}", mounted, player.current_frame()));

    let snapshots = SnapshotContext::new("misc", "agent_freeride");
    let _ = snapshots.verify("01_title", player.snapshot_stage());

    // The two PLAY buttons live inside the full-stage Flash sprite, and the menu
    // SWF's ActionScript does not come up under Ruffle here (its AS-init wait
    // times out), so clicking stage coordinates does nothing. What the SWF would
    // do is call ONE movie handler — `flash_start` in
    // `MovieScript 62 - Flash start` — which seeds level/player/score, the audio
    // and tutorial flags, and then `go(13)`. Call it directly so the test reaches
    // gameplay regardless of the Flash menu. `me` is NOT auto-prepended for this
    // movie-script handler, so the leading argument is the dummy the SWF's
    // `getURL("lingo:…")` call supplies.
    let started = ask!("flash_start(0, \"1\", \"1\", \"0\", \"1\", \"0\", \"0\")");
    probe(format!("flash_start -> {} frame={} level={}",
        started, player.current_frame(), ask!("gGame.GetLevelId()")));
    player.step_frames(30).await;
    probe(format!("after flash_start frame={}", player.current_frame()));

    let _ = snapshots.verify("02_after_clicks", player.snapshot_stage());

    // Stream the track and build the level.
    player.step_frames(400).await;
    player.step_frames(75).await;
    probe(format!("gameplay frame={} status={}",
        player.current_frame(), ask!("string(gGame.GetGameStatus())")));

    let _ = snapshots.verify("03_gameplay", player.snapshot_stage());

    // --- laser gates ---------------------------------------------------------
    // `Snowboard LaserPort` builds each gate with
    // `cloneModelFromCastmember(name, "bo2c18_01"/"bo1c18_07", THE SAME member)`,
    // duplicates `mes_fx_laserdoor_1_mat` per gate, hands the copy to the texture
    // shifter and toggles `shader.blend` 0 <-> 100 to open and close the fence.
    // Cloning inside one member is what exposed the skeleton fallback in
    // `cloneModelFromCastmember`: the level member carries ~30 character rigs, so
    // every gate was filed under a foreign skeleton, drawn skinned by bones that
    // have nothing to do with a 4-vertex quad, and no laser wall ever appeared.
    // Park the boarder on a gate and photograph it closed and open.
    {
        let idx = 1;
        let m = format!("gGame.pLaserPortList[{}].GetMdl()", idx);
        probe(format!("LASERGATE n={} mdl={} faces={} shader={} tex={}",
            ask!("gGame.pLaserPortList.count"),
            ask!(&format!("string({}.name)", m)),
            ask!(&format!("string({}.resource.face.count)", m)),
            ask!(&format!("string({}.shaderList[1].name)", m)),
            ask!(&format!("string({}.shaderList[1].texture)", m))));
        let _ = ask!(&format!("gGame.pLaserPortList[{}].SetBlinking(0)", idx));
        let _ = ask!(&format!(
            "gGame.GetPlayerVehicle().GetVehicle().setPosition(gGame.pLaserPortList[{}].getPosition() + vector(0.0, -3000.0, 400.0))", idx));
        for (tag, state) in [("closed", 1), ("open", 0)] {
            let _ = ask!(&format!("gGame.pLaserPortList[{}].SetPortState({})", idx, state));
            player.step_frames(8).await;
            let _ = ask!(&format!("gGame.pLaserPortList[{}].SetPortState({})", idx, state));
            player.step_frames(2).await;
            probe(format!("LASERGATE {} blend={}", tag,
                ask!(&format!("{}.shaderList[1].blend", m))));
            let _ = snapshots.verify(&format!("07_laser_{}", tag), player.snapshot_stage());
        }
    }

    // --- diagnostics ---------------------------------------------------------
    // Keep this block SHORT: the movie's timers run on wall-clock milliseconds, so
    // a long probe loop between stepped frames burns level time and the run ends on
    // "TIME'S UP" before it has ridden anywhere.
    probe(format!("COIN n={} shader={} layer1={} layer2={}",
        ask!("gGame.GetBonusManager().GetBonusArray().count"),
        ask!("gGame.Get3D().model(\"bonus_money_cam\").shaderList[1].name"),
        ask!("string(gGame.Get3D().model(\"bonus_money_cam\").shaderList[1].textureList[1])"),
        ask!("string(gGame.Get3D().model(\"bonus_money_cam\").shaderList[1].textureList[2])")));
    probe(format!("LASER n={} mdl={} blend={} pos={}",
        ask!("gGame.pLaserPortList.count"),
        ask!("gGame.pLaserPortList[1].GetMdl().name"),
        ask!("gGame.pLaserPortList[1].GetMdl().shaderList[1].blend"),
        ask!("string(gGame.pLaserPortList[1].GetMdl().transform.position)")));
    probe(format!("TREE n={} mdl={} children={} kf={}",
        ask!("gGame.GetTrapManager().pTreeTrapList.count"),
        ask!("gGame.GetTrapManager().pTreeTrapList[1].GetMdl().name"),
        ask!("gGame.GetTrapManager().pTreeTrapList[1].GetMdl().child.count"),
        ask!("gGame.GetTrapManager().pTreeTrapList[1].GetKeyFramedList().count")));
    // Fire one trap by hand and confirm the keyframed trunk actually rotates and
    // then HOLDS at the end of the fall clip (the tree stays down, it doesn't
    // vanish or snap back upright).
    let before = ask!("string(gGame.GetTrapManager().pTreeTrapList[1].GetMdl().child[1].getWorldTransform().rotation)");
    let _ = ask!("gGame.GetTrapManager().pTreeTrapList[1].ChangeState(#TrapOn, gGame.GetTimeManager().GetTime())");
    player.step_frames(40).await;
    probe(format!("TREE fall rot before={} after={} animTime={}",
        before,
        ask!("string(gGame.GetTrapManager().pTreeTrapList[1].GetMdl().child[1].getWorldTransform().rotation)"),
        ask!("string(gGame.GetTrapManager().pTreeTrapList[1].GetMdl().child[1].keyframePlayer.currentTime)")));

    // --- respawn ("ResetToTrack") -------------------------------------------
    // `Vehicle Base.ResetToTrack` re-seats the boarder by casting a ray from
    // 8000 units ABOVE the track token and dropping him to `isectPosition.z + 200`.
    // If that ray reports the wrong hit point he is left thousands of units up and
    // falls for seconds — the "teleported way up" the player sees. Run the movie's
    // own query and check the hit is on the track, not back at the ray origin.
    const RAY: &str = "gGame.Get3D().modelsUnderRay(\
        gGame.GetPlayerVehicle().getPosition() + vector(0.0, 0.0, 8000.0), \
        vector(0.0, 0.0, -1.0), \
        [#maxNumberOfModels: 4, #levelOfDetail: #detailed, \
         #modelList: gGame.GetCullingManager().GetCullerBlockModels(gGame.GetPlayerVehicle().getPosition())])";
    probe(format!("RESET ray hits={} playerZ={} hit1={} isect={} dist={}",
        ask!(&format!("{}.count", RAY)),
        ask!("string(gGame.GetPlayerVehicle().getPosition().z)"),
        ask!(&format!("{}[1].model.name", RAY)),
        ask!(&format!("string({}[1].isectPosition)", RAY)),
        ask!(&format!("string({}[1].distance)", RAY))));
    // Respawn ("ResetToTrack") regression guard. The movie re-seats the boarder by
    // casting a ray DOWN from 8000 above the current track token; the token plan is
    // deliberately flat at z=0 (`Token Manager.InitializeTrackTokens` zeroes it), so
    // the height comes entirely from that ray, and a miss silently leaves him at
    // z=0. The course descends past -125000, so the drop can be ~80000 units — the
    // "teleported way up, falls for ages" bug. It only showed up deep in the course,
    // because the ray was being truncated at a made-up 100000-unit default reach.
    //
    // Drop the player onto a token, push him off the map, and require the game to
    // put him back ON the track.
    for n in [8usize, 16, 24, 32] {
        let tok = format!("gGame.GetTokenManager().TokenToWorld(symbol(\"t\" & {}), 0.5, 0.0)", n);
        let _ = ask!(&format!("gGame.GetPlayerVehicle().GetVehicle().setPosition({} + vector(0.0, 0.0, 500.0))", tok));
        player.step_frames(30).await;
        let _ = ask!("gGame.GetPlayerVehicle().GetVehicle().setPosition(gGame.GetPlayerVehicle().getPosition() + vector(120000.0, 0.0, 0.0))");
        // Wait for the reset to SETTLE rather than sampling one frame. The reset is
        // gated on the camera fade completing, and the culling grid needs a moment
        // to page the target block back in after a 120000-unit jump, so a single
        // reading can land mid-sequence — which made this guard fail in a full-suite
        // run while passing standalone.
        let mut gd = f64::NAN;
        for _ in 0..10 {
            player.step_frames(30).await;
            gd = match player.eval_datum("gGame.GetPlayerVehicle().GetVehicle().GetGroundDistance()").await {
                Ok(vm_rust::director::static_datum::StaticDatum::Float(f)) => f,
                Ok(vm_rust::director::static_datum::StaticDatum::Int(i)) => i as f64,
                _ => f64::NAN,
            };
            if gd.is_finite() && gd < 2000.0 { break; }
        }
        probe(format!("RESPAWN t{} groundDistance={:.1}", n, gd));
        assert!(
            gd.is_finite() && gd < 2000.0,
            "respawn at token t{} left the boarder {:.0} units above the track —              ResetToTrack's ray failed to reach the ground and fell back to the              token plan's z=0", n, gd
        );
    }

    let z_before = ask!("string(gGame.GetPlayerVehicle().getPosition().z)");
    let _ = ask!("gGame.GetPlayerVehicle().GetVehicle().ResetToTrack()");
    probe(format!("RESET zBefore={} zAfter={} gdist={}",
        z_before,
        ask!("string(gGame.GetPlayerVehicle().getPosition().z)"),
        ask!("gGame.GetPlayerVehicle().GetVehicle().GetGroundDistance()")));

    // Ride.
    player.step_frames(120).await;
    probe(format!("ride gdist={} hover={}",
        ask!("gGame.GetPlayerVehicle().GetVehicle().GetGroundDistance()"),
        ask!("string(gGame.GetPlayerVehicle().GetVehicle().GetIsHoveringList())")));

    // Sample the whole ride so a failure shows the shape of it — the spawn drop,
    // the trick, then the settle.
    use vm_rust::director::static_datum::StaticDatum;
    let mut series: Vec<f64> = Vec::new();
    // The reset ray, evaluated at the player's CURRENT token the way
    // `Vehicle Base.ResetToTrack` does it. Zero hits here is the bug: the movie
    // then leaves `lNewPosition.z` at TokenToWorld's height (the token plan is
    // FLAT at z=0) and the boarder is dropped from z=0 onto track thousands of
    // units below.
    const CUR: &str = "gGame.GetTokenManager().TokenToWorld(        gGame.GetPlayerVehicle().GetVehicle().GetCurrentToken(),         gGame.GetPlayerVehicle().GetVehicle().GetLongitudinal(), 0.0)";
    for i in 0..10 {
        player.step_frames(30).await;
        let ray = format!(
            "gGame.Get3D().modelsUnderRay({} + vector(0.0, 0.0, 8000.0), vector(0.0, 0.0, -1.0),              [#maxNumberOfModels: 4, #levelOfDetail: #detailed,               #modelList: gGame.GetCullingManager().GetCullerBlockModels({})])", CUR, CUR);
        let hits = ask!(&format!("{}.count", ray));
        let mut listing = String::new();
        for h in 1..=4 {
            let name = ask!(&format!("{}[{}].model.name", ray, h));
            if name.starts_with("<err") { break; }
            listing.push_str(&format!(" [{} z={}]", name,
                ask!(&format!("string({}[{}].isectPosition.z)", ray, h))));
        }
        probe(format!("RESET@ride {} token={} playerZ={} hits={}{}",
            i,
            ask!("string(gGame.GetPlayerVehicle().GetVehicle().GetCurrentToken())"),
            ask!("string(gGame.GetPlayerVehicle().getPosition().z)"),
            hits, listing));
        series.push(
            player
                .eval_datum("gGame.GetPlayerVehicle().GetVehicle().GetGroundDistance()")
                .await
                .ok()
                .and_then(|d| match d {
                    StaticDatum::Float(f) => Some(f),
                    StaticDatum::Int(i) => Some(i as f64),
                    _ => None,
                })
                .unwrap_or(f64::NAN),
        );
    }

    let _ = snapshots.verify("04_ride", player.snapshot_stage());

    probe(format!(
        "ground distance over the ride: {}",
        series.iter().map(|g| format!("{:.0}", g)).collect::<Vec<_>>().join(" ")
    ));

    // Riding = settled at the end. Two of the last three tolerates one sample
    // landing mid-trick without accepting a boarder that never came down.
    let tail = &series[series.len() - 3..];
    let grounded = tail.iter().filter(|g| g.is_finite() && **g < 500.0).count();
    assert!(
        grounded >= 2,
        "boarder is not riding the course — last three ground distances {:?} \
         (spawn drop never settled, or the hover rays are missing)", tail
    );

    // --- end-of-level scene camera ------------------------------------------
    // Reaching the finish hands over to `Snowboard EndScene`, which pins the
    // vehicle, swaps the boarder for the paraglider rig (`obs_final1_main`, with
    // the animated `player_fake` under it) and puts `Snowboard Camera` in its
    // #EndScene state. That state seats the camera ONCE, at a fixed offset in the
    // source model's own axes, and then re-aims it at the target every frame:
    //
    //   pos = src.worldPosition + src.transform.xAxis*2000 + yAxis*2500 + zAxis*100
    //   pointAt(pos + normalize(target.worldPosition - pos)*100, vector(0,0,1))
    //
    // So the whole shot depends on `player_fake.worldPosition` FOLLOWING the
    // paraglider clip. It did not: IFX extracts the root bone's translation and
    // adds it to the model node, we kept it inside the skeleton, and the boarder
    // flew ~16000 units out of frame while the camera — aimed perfectly, dot
    // -1.0000 every frame — stared at the launch point. That is the reported
    // "camera points the wrong way at the end of the level".
    //
    // Drive it the way the movie's own debug key "k" does.
    let _ = ask!("gGame.GetSequenceManager().StartSequence(\"LevelEndGood\")");
    let _ = ask!("gGame.pEndScene.pCharacterRef.ChangeState(#EndScene, gGame.GetTimeManager().GetTime())");
    player.step_frames(5).await;

    const CAM: &str = "gGame.GetCamera().GetCameraNode()";
    const TGT: &str = "gGame.GetCamera().pEndTargetRef";
    // Camera-space vector to the target, normalized. The camera looks down its own
    // -Z, so `dot` is -1 when the target is dead centre.
    let aim = format!(
        "string({}.getWorldTransform().zAxis.dot(({}.worldPosition - {}.worldPosition).getNormalized()))",
        CAM, TGT, CAM);
    probe(format!("ENDSCENE state={} tgt={} tgtWorld={} camWorld={} dot={}",
        ask!("string(gGame.pEndScene.pCharacterRef.GetState())"),
        ask!(&format!("{}.name", TGT)),
        ask!(&format!("string({}.worldPosition)", TGT)),
        ask!(&format!("string({}.worldPosition)", CAM)),
        ask!(&aim)));
    let _ = snapshots.verify("05_endscene", player.snapshot_stage());

    // The paraglider canopy. `obs_parachute_main` is pinned to the character rig
    // and its keyframed child `parachute` carries the flight AND the canopy
    // inflating (the clip ramps scale ~100x over its first two seconds, against a
    // node authored at ~1/100). It must end the shot near the boarder, not
    // hundreds of thousands of units away.
    const PARA: &str = "gGame.pEndScene.pParachute";
    probe(format!("ENDSCENE canopy name={} vis={} faces={} shader={} bsphere={}",
        ask!(&format!("string({}.child[1].name)", PARA)),
        ask!(&format!("string({}.child[1].visibility)", PARA)),
        ask!(&format!("string({}.child[1].resource.face.count)", PARA)),
        ask!(&format!("string({}.child[1].shaderList[1].name)", PARA)),
        ask!(&format!("string({}.child[1].boundingSphere)", PARA))));

    // Track the flight. Two things must hold together: the target model actually
    // TRAVELS (root motion reaches the node), and the camera stays locked on it.
    let flight_start = player.eval_datum(&format!("{}.worldPosition.x", TGT)).await
        .ok().and_then(|d| match d {
            StaticDatum::Float(f) => Some(f), StaticDatum::Int(i) => Some(i as f64), _ => None })
        .unwrap_or(f64::NAN);
    let mut travelled = 0.0f64;
    for i in 0..6 {
        player.step_frames(20).await;
        let x = player.eval_datum(&format!("{}.worldPosition.x", TGT)).await
            .ok().and_then(|d| match d {
                StaticDatum::Float(f) => Some(f), StaticDatum::Int(i) => Some(i as f64), _ => None })
            .unwrap_or(f64::NAN);
        travelled = (x - flight_start).abs();
        probe(format!("ENDSCENE canopy {} kfT={} canopyWorld={} canopyR={}",
            i,
            ask!(&format!("{}.child[1].keyframePlayer.currentTime", PARA)),
            ask!(&format!("string({}.child[1].worldPosition)", PARA)),
            ask!(&format!("string({}.child[1].boundingSphere[2])", PARA))));
        probe(format!("ENDSCENE flight {} animT={} tgtWorld={} travelled={:.0} dot={}",
            i,
            ask!("gGame.pEndScene.pCharacterRef.pCharacterMdl.bonesPlayer.currentTime"),
            ask!(&format!("string({}.worldPosition)", TGT)),
            travelled,
            ask!(&aim)));
    }
    let _ = snapshots.verify("06_endscene_flight", player.snapshot_stage());

    assert!(
        travelled.is_finite() && travelled > 1000.0,
        "the end-scene paraglider never left the launch point ({:.0} units) — the \
         clip's root motion is stuck inside the skeleton instead of reaching \
         `player_fake`'s model node, so the camera has nothing to follow", travelled
    );
    let dot = player.eval_datum(&format!(
        "{}.getWorldTransform().zAxis.dot(({}.worldPosition - {}.worldPosition).getNormalized())",
        CAM, TGT, CAM)).await
        .ok().and_then(|d| match d {
            StaticDatum::Float(f) => Some(f), StaticDatum::Int(i) => Some(i as f64), _ => None })
        .unwrap_or(f64::NAN);
    assert!(
        dot.is_finite() && dot < -0.99,
        "the end-scene camera is not looking at the paraglider (dot {:.4}, want ~-1)", dot
    );

    Ok(())
});
