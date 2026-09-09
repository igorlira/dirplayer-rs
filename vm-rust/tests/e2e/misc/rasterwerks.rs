use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_rasterwerks.toml");

/// Live particle count for a named system, from the renderer-side sim.
fn particles_alive(system: &str) -> (usize, usize, [f32; 3]) {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if let CastMemberType::Shockwave3d(w3d) = &m.member_type {
                    for (name, ps) in w3d.runtime_state.particles.iter() {
                        if name.as_str().eq_ignore_ascii_case(system) {
                            return (ps.alive.iter().filter(|a| **a).count(),
                                    ps.max_particles, ps.emitter_position);
                        }
                    }
                }
            }
        }
        (0, 0, [0.0; 3])
    })
}

fn probe(msg: String) {
    // `println!` from wasm goes nowhere; the page console is what E2E_CONSOLE
    // forwards to the terminal.
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Rasterwerks "PHOSPHOR" beta2 043 — a full Shockwave 3D deathmatch FPS (D11.5)
// with bots, a nav net, jump pads and five weapons, all driven from Lingo.
//
// The boot chain is most of what this covers: the movie reads base.ini and
// PHOSPHOR_BETA.ini through the FileIO xtra (the browser test harness never
// built a FileIO manager, so the first frame of this movie panicked on a `None`
// unwrap), then streams three external casts by assigning `castLib(n).fileName`,
// builds the world, and lands on the "Start" menu.
//
// The player SPAWNS AT A RANDOM NAV NODE, so nothing about the 3D view is stable
// between runs — only the menu is snapshotted. The muzzle-flash regression this
// test was written for is pinned deterministically by the unit tests on
// `classify_texture_alpha` in rendering_gpu/webgl2/scene3d.rs; here the fire path
// is only walked end to end.
browser_e2e_test!(test_misc_rasterwerks_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    let snapshots = SnapshotContext::new("misc", "rasterwerks");

    // `string(...)` because StaticDatum has no vector variant — every 3D value
    // this movie holds would otherwise come back Void.
    macro_rules! ask {
        ($($t:tt)*) => {{
            let e = format!($($t)*);
            match player.eval_datum(&format!("string({})", e)).await {
                Ok(vm_rust::director::static_datum::StaticDatum::String(s)) => s,
                other => format!("<{:?}>", other),
            }
        }};
    }

    // Boot: F_Load streams snd_core / act_core / map_dm_acheron_bluffs off the
    // movie path, then "Start" (F_Start) is the main menu.
    player.step_frames(600).await;
    let _ = snapshots.verify("01_menu", player.snapshot_stage());

    // "Start" is F_Start's button 1 — hotspot sprite 23, laid out at (280, 20)
    // plus the menu's centring shift for a 1024x768 stage.
    //
    // Click it until the game actually starts rather than at a fixed frame
    // count. How far the boot has got after N frames depends on how fast frames
    // are stepping, and it steps a good deal slower inside a full suite run than
    // it does for this test alone — a fixed count reached the menu in isolation
    // and landed short of it in a batch, clicking empty stage. The game
    // ANSWERING is the signal: `cPlayer` does not exist until F_Main builds it,
    // so an empty pGameState means we are still in the menu (or before it).
    let mut started = false;
    for attempt in 0..40 {
        player.click(641, 244).await;
        player.step_frames(60).await;
        if !ask!("cPlayer.pGameState").is_empty() {
            probe(format!("game started on attempt {}", attempt));
            started = true;
            break;
        }
    }
    assert!(started, "never left the Start menu — cPlayer was never built");

    // The player enters #DEAD and respawns on cGame's spawn timer.
    let mut live = false;
    for _ in 0..60 {
        if ask!("cPlayer.pGameState") == "LIVE" { live = true; break; }
        player.step_frames(30).await;
    }
    assert!(live, "never reached gameplay — cPlayer.pGameState is {}", ask!("cPlayer.pGameState"));
    probe(format!("spawned at {}", ask!("cPlayer.pvPosition")));
    // The map's own objects loaded, not just the engine: LoadJumpPads reads
    // three pads out of the map cast's DAT.JumpPad.
    let pads: i32 = ask!("cWorld.plJumpPad.count").parse().unwrap_or(0);
    assert_eq!(pads, 3, "map_dm_acheron_bluffs declares three jump pads");

    // ...and every one of them resolved to a nav node. C_NavNet.InitSpecialNodes
    // builds this table by firing a ray straight DOWN from each #JUMPSPOT nav node
    // and recording which jump-pad model it hits, through `modelsUnderRay`. A pad
    // the probe ray misses leaves a hole, and the first bot to launch off it runs
    // `cNavNet.plNavNode[0]` — an out-of-range Lingo index that kills the handler
    // ("Index out of bounds: -1") mid-game.
    //
    // This is a live tripwire on that ray: flushing the persistent transform datums
    // inside `modelsUnderRay` (which the screen-picking handlers DO need) changed
    // what these probes hit and emptied one entry. Director answers the ray from
    // the same once-per-frame node state, so the flush does not belong there.
    assert_eq!(
        ask!("cNavNet.plJumpPad2NavNode"), "[57, 18, 17]",
        "a jump pad did not resolve to a nav node — bots launching off it will die          in C_Bot.cmd(#JumpPadLaunch) on plNavNode[0]"
    );

    // Walk the weapon + fire path. C_Weapon.fire() raycasts the crosshair
    // through `camera.modelsUnderLoc`, points the muzzle group at the hit,
    // launches a missile and lights the flare — so a decrement of the clip is
    // evidence the whole chain ran rather than bailing out early.
    let _ = player.eval("cPlayer.plWeaponInv = [1, 1, 1, 1, 1]").await;
    let _ = player.eval("cPlayer.plAmmo = [0, 100, 100, 100, 100]").await;
    let _ = player.eval("cWeapon.Switch(2)").await;
    let mut ready = false;
    for _ in 0..60 {
        player.step_frames(1).await;
        if ask!("cWeapon.pCurIdx") == "2" && ask!("cWeapon.pState") == "READY" { ready = true; break; }
    }
    assert!(ready, "never switched to the pulse gun: idx {} state {}",
        ask!("cWeapon.pCurIdx"), ask!("cWeapon.pState"));

    // The kill feed / chat lines are baked by writing into the EMPTY scratch
    // member `txtArialBold12_512` and blitting its `.image` into an overlay
    // texture, so the member's authored colour is the only thing that decides
    // whether they are legible. An empty text member gets one styled span that
    // spans no characters and carries a synthesised BLACK — reading it drew
    // every kill and chat line black on the dark panel.
    assert_eq!(
        ask!("member(\"txtArialBold12_512\").color"), "rgb(204, 204, 204)",
        "the kill-feed scratch member lost its authored colour"
    );

    // ...and its box must be the authored 512x16. C_MsgBox clears the alpha of
    // each 512x16 line image by copying the member's `.image` over
    // `rect(0, 0, 512, 16)`, so a member one row short leaves the last row of
    // every line image opaque — a white rule under each kill-feed row, running
    // the full width of the image. The member is authored EMPTY, where Paige's
    // doc_bottom holds the bare font extent (15) rather than the authored
    // fixedLineSpace (16); a single line is never shorter than fixedLineSpace.
    assert_eq!(
        ask!("member(\"txtArialBold12_512\").rect"), "rect(0, 0, 512, 16)",
        "the kill-feed scratch member is not its authored 512x16 box"
    );

    // Aiming: modelsUnderLoc must pick through the camera as it is NOW, not as
    // it was at the last frame boundary. C_Camera.Step writes the view rotation
    // from the mouse and C_Weapon.Step fires through modelsUnderLoc(screenCentre)
    // later in the SAME frame, so reading the once-per-frame transform cache
    // aimed every shot one frame of mouse-look behind the crosshair. Turn the
    // camera and pick without stepping a frame: the hit must lie on the NEW
    // forward axis.
    const CENTRE_HIT_DOT: &str =
        "(-cCamera.pCam.getWorldTransform().zAxis).dot((cCamera.pCam.modelsUnderLoc(\
         cEngine.pViewportScreenCenter, 1, #detailed)[1].isectPosition \
         - cCamera.pCam.worldPosition).getNormalized())";
    let settled: f64 = ask!("{}", CENTRE_HIT_DOT).parse().unwrap_or(0.0);
    assert!(settled > 0.999, "the centre ray does not run along the camera's forward axis (dot {})", settled);
    let _ = player.eval("cCamera.pCam.transform.rotation = cCamera.pCam.transform.rotation + vector(0, 35, 0)").await;
    let turned: f64 = ask!("{}", CENTRE_HIT_DOT).parse().unwrap_or(0.0);
    assert!(turned > 0.999,
        "modelsUnderLoc picked through the camera's PREVIOUS transform: after a 35 degree          yaw with no frame step the centre hit is {:.4} off the forward axis (cos 35 = 0.819)", turned);
    let _ = player.eval("cCamera.pCam.transform.rotation = cCamera.pCam.transform.rotation - vector(0, 35, 0)").await;

    // The spawn burst is a #particle resource: a 60x60 emitter region centred on
    // the origin, on a model the actor moves to the spawn point. Two things had
    // to be true before it could ever be seen, and neither was:
    //   * `emitter.region` is in the resource's OWN space, so the burst emits at
    //     the MODEL's world position offset by the region — read as world
    //     coordinates it fired at the quad's first corner, (-30, 0, -30); and
    //   * a #stream emitter is continuous, so its particles must already be
    //     spread across every age when it starts. Waiting for `age >= lifetime`
    //     to birth them cost a full 1.2 s — the entire duration of this effect.
    let spawn_ps = "PlayerSpawnFX_01";
    let _ = player.eval(&format!(
        "cPlayer.pActor.pSpawnParSys.Place(cCamera.pCam.worldPosition          - (cCamera.pCam.getWorldTransform().zAxis * 150))")).await;
    let _ = player.eval("cPlayer.pActor.pSpawnParSys.PlayAnim()").await;
    player.step_frames(4).await;
    let (alive, total, emit) = particles_alive(spawn_ps);
    probe(format!("{}: {}/{} alive, emitting at ({:.0}, {:.0}, {:.0})",
        spawn_ps, alive, total, emit[0], emit[1], emit[2]));
    assert!(alive > 0, "the spawn burst emitted nothing ({}/{} alive)", alive, total);

    let cam: Vec<f64> = ask!("cCamera.pCam.worldPosition")
        .trim_start_matches("vector(").trim_end_matches(')')
        .split(',').filter_map(|p| p.trim().parse().ok()).collect();
    if cam.len() == 3 {
        let d = ((emit[0] as f64 - cam[0]).powi(2)
               + (emit[1] as f64 - cam[1]).powi(2)
               + (emit[2] as f64 - cam[2]).powi(2)).sqrt();
        assert!(d < 300.0,
            "the burst emitted {:.0} units from where it was placed — emitter.region is resource-local, not world coordinates", d);
    }

    let _ = player.eval("cWeapon.fire()").await;
    player.step_frames(1).await;
    assert_eq!(ask!("cPlayer.plAmmo[2]"), "99", "firing the pulse gun did not consume a round");
    assert_eq!(ask!("cWeapon.pbFlareOn"), "1", "the muzzle flare was not lit");

    // Sustained fire must not degrade the aim. `C_Weapon.fire()` runs
    // `pAimUtil.pointAt(...)` then `pAimUtil.rotate(pvAimError)` on EVERY shot,
    // and pAimUtil is ONE group shared by all five weapons (the #SWAP branch only
    // re-parents it) hanging off a weapon model scaled 0.3.
    //
    // apply_point_at used to multiply the node's current column lengths onto a
    // basis that already carried inverse(parent) — so the local scale grew 1/0.3 =
    // 3.333x per shot. Measured: 11.1 -> 7.7e8 -> 5.4e16 -> 2.2e19 (f32 saturation)
    // after ~45 rounds, at which point rotate() could no longer perturb the matrix.
    // The MachineGun stopped spraying and hit one spot, and because pAimUtil is
    // shared and never reset, every other weapon then fired 70-80 degrees off the
    // crosshair. Reproduced exactly as reported: "dauerfeuer 50 bullets of the
    // machine gun, all other weapons regressing".
    let _ = player.eval("cPlayer.plWeaponInv = [1, 1, 1, 1, 1]").await;
    let _ = player.eval("cWeapon.Switch(3)").await;
    let mut mg_ready = false;
    for _ in 0..200 {
        player.step_frames(1).await;
        let _ = player.eval("cPlayer.plAmmo = [0, 500, 500, 500, 500]").await;
        if ask!("cWeapon.pCurIdx") == "3" && ask!("cWeapon.pState") == "READY" { mg_ready = true; break; }
    }
    assert!(mg_ready, "never switched to the MachineGun");

    for _ in 0..60 {
        let _ = player.eval("cPlayer.plAmmo = [0, 500, 500, 500, 500]").await;
        let _ = player.eval("cWeapon.fire()").await;
        player.step_frames(1).await;
    }
    let scale = ask!("cWeapon.pAimUtil.transform.scale");
    assert_eq!(scale, "vector(1.0000, 1.0000, 1.0000)",
        "pAimUtil's scale drifted over a 60-round burst ({}) — pointAt is compounding          inverse(parent) into the local basis", scale);

    // The aim error must still reach the barrel: the angle between the un-errored
    // and errored fire directions equals |pvAimError|.
    let imparted: f64 = ask!("cWeapon.pvAimDirN.angleBetween(cWeapon.pvFireDirN)").parse().unwrap_or(-1.0);
    let err: Vec<f64> = ask!("cWeapon.pvAimError")
        .trim_start_matches("vector(").trim_end_matches(')')
        .split(',').filter_map(|p| p.trim().parse().ok()).collect();
    let err_mag = (err[0] * err[0] + err[1] * err[1]).sqrt();
    assert!((imparted - err_mag).abs() < 0.05,
        "after the burst the MachineGun's spray is gone: pvAimError is {:.3} deg but only          {:.3} deg reached the barrel", err_mag, imparted);

    // ...and every other weapon must still shoot at the crosshair, since they all
    // share pAimUtil.
    for idx in [4, 5, 2] {
        let _ = player.eval(&format!("cWeapon.Switch({})", idx)).await;
        let mut ok = false;
        for _ in 0..200 {
            player.step_frames(1).await;
            let _ = player.eval("cPlayer.plAmmo = [0, 500, 500, 500, 500]").await;
            if ask!("cWeapon.pCurIdx") == idx.to_string() && ask!("cWeapon.pState") == "READY" { ok = true; break; }
        }
        assert!(ok, "never switched to weapon {} after the burst", idx);
        let _ = player.eval("cWeapon.fire()").await;
        let off: f64 = ask!("(cWeapon.pvCrossHairPos - cWeapon.pvFirePos).getNormalized().angleBetween(cWeapon.pvFireDirN)")
            .parse().unwrap_or(999.0);
        let half: f64 = ask!("cWeapon.plAimErrorHalf[{}]", idx).parse().unwrap_or(0.0);
        assert!(off <= half * 1.5 + 0.1,
            "weapon {} fires {:.2} deg off the crosshair after a MachineGun burst (its              AIMERROR allows {:.2})", idx, off, half * 1.415);
    }

    Ok(())
});
