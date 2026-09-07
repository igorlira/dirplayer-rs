use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_areazero.toml");

fn probe(msg: String) {
    // `println!` from wasm goes nowhere; the page console is what E2E_CONSOLE
    // forwards to the terminal.
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// (wants_pointer_lock, cursor_is_hidden) — the mouselook pointer-lock gate.
fn lock_state() -> (bool, bool) {
    vm_rust::player::reserve_player_ref(|p| (p.wants_pointer_lock, p.cursor_is_hidden))
}

/// Every PhysX body in the movie, across all members.
fn body_count() -> usize {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        player.movie.cast_manager.casts.iter()
            .flat_map(|c| c.members.iter())
            .filter_map(|(_, m)| match &m.member_type {
                CastMemberType::PhysXPhysics(p) => Some(p.state.bodies.len()),
                _ => None,
            })
            .sum()
    })
}

/// World position of the player's collision proxy (AreaZero is Z-up, metric).
fn player_pos() -> [f64; 3] {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        player.movie.cast_manager.casts.iter()
            .flat_map(|c| c.members.iter())
            .filter_map(|(_, m)| match &m.member_type {
                CastMemberType::PhysXPhysics(p) => p.state.bodies.iter()
                    .find(|b| b.name.as_str().eq_ignore_ascii_case("FPSPlayer1Stand"))
                    .map(|b| b.position),
                _ => None,
            })
            .next()
            .unwrap_or([f64::NAN; 3])
    })
}

fn player_z() -> f64 { player_pos()[2] }

/// Skeletons across every Shockwave3D member whose name matches `needle`,
/// as (member-scene skeleton name, bone count).
///
/// This is the frog guard's FIRST assertion and the one that fails when the
/// BONES attribute-gated-field fix is reverted: `RobotFrog` constrains six
/// bones, the old fixed-size bone reader derailed on the first of them, and the
/// member ended up with NO skeleton at all. A rig with skin weights and no
/// skeleton draws in a fixed pose forever, which is exactly "the crawlers glide
/// around without ever animating".
fn skeletons_matching(needle: &str) -> Vec<(String, usize)> {
    let needle = needle.to_ascii_lowercase();
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        let mut out = Vec::new();
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                let Some(scene) = w3d.parsed_scene.as_ref() else { continue };
                for sk in scene.skeletons.iter() {
                    if sk.name.as_str().to_ascii_lowercase().contains(&needle) {
                        out.push((sk.name.as_str().to_string(), sk.bones.len()));
                    }
                }
            }
        }
        out
    })
}

/// Every bonesPlayer whose MODEL name matches `needle`, as
/// (model, current motion, playhead). Reported across all 3D members because
/// AreaZero spawns each robot as a clone into the level member.
fn bones_players_matching(needle: &str) -> Vec<(String, String, f32)> {
    let needle = needle.to_ascii_lowercase();
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        let mut out = Vec::new();
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                for (model, bp) in w3d.runtime_state.bones_players.iter() {
                    if model.as_str().to_ascii_lowercase().contains(&needle) {
                        out.push((
                            model.as_str().to_string(),
                            bp.current_motion.map(|s| s.as_str().to_string())
                                .unwrap_or_else(|| "<none>".to_string()),
                            bp.animation_time,
                        ));
                    }
                }
            }
        }
        out
    })
}



/// Live datums in the refcounted arena, plus the per-node/per-resource runtime
/// maps that `deleteModel` / `deleteModelResource` are supposed to release.
fn leak_counters() -> (usize, usize, usize, usize, usize, usize) {
    let arena = vm_rust::player::reserve_player_ref(|p| p.allocator.datums.len());
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case("Level1") { continue; }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                let rs = &w3d.runtime_state;
                let res = w3d.parsed_scene.as_ref().map(|s| s.model_resources.len()).unwrap_or(0);
                return (arena, rs.node_transforms.len(), rs.node_transform_datums.len(),
                        rs.user_data.len(), res, rs.mesh_build_data.len());
            }
        }
        (arena, 0, 0, 0, 0, 0)
    })
}

// Xform / AddictingGames "AreaZero" — Shockwave 3D + AGEIA PhysX, D11.5.
// Port notes, menu/level architecture and open render defects: docs/areazero/README.md
//
// Covers the in-game HUD and the player's footing. The gameplay snapshot is the
// regression guard for two defects that shared one root cause — the meshDeform
// `textureCoordinateList` accessors mirrored V, so a script doing arithmetic on
// the UVs it read back addressed the wrong edge of the texture:
//   * the ammo clip bar (`InterfaceBulletsClip`, hardcoded V 0..0.0625 over the
//     strip along the BOTTOM of Interface_Texture) sampled the transparent top
//     edge and drew nothing at all; and
//   * every baked HUD string ("Score", "Level") lost its top third, because
//     `GetAndSetTagTextureSize` rounds the text texture up to a power of two and
//     compensates with `v * (size/POT) + (1 - size/POT)`, which anchors the
//     content at V = 1 — the image top.
browser_e2e_test!(test_misc_areazero_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    let snapshots = SnapshotContext::new("misc", "areazero");

    // Boot: external casts are fetched at runtime from Data/Libs/*.cct, then the
    // event tables build the menu.
    player.step_frames(500).await;
    let _ = snapshots.verify("01_menu", player.snapshot_stage());

    // Click PLAY until the level actually loads. The menu ANIMATES in (splash →
    // branding → main), so clicking at a fixed frame count is a coin flip — one
    // build's pacing lands on the menu and another's lands before it exists.
    // Wait on the physics scene instead: the level's bodies appearing IS the
    // signal that the game began.
    //
    // Be generous with the attempt count. The menu's pacing is driven by
    // WALL-CLOCK timeouts (`DelayEvent`), not by frames, so how many frames it
    // takes to reach a clickable PLAY depends on how fast frames are stepping.
    // 12 attempts was enough for a filtered run and not enough for this test's
    // slot in the full suite, where it failed with "no PhysX bodies".
    let mut started = false;
    for attempt in 0..40 {
        player.click(201, 137).await;
        player.step_frames(60).await;
        if body_count() > 0 {
            probe(format!("level started on attempt {} with {} bodies",
                attempt, body_count()));
            started = true;
            break;
        }
    }
    assert!(started, "never reached the level — no PhysX bodies were created");

    // `[PS] FPS.new` places FPSPlayer1Stand on the Level1 group "PlayerSpawn"
    // (0, 24.24, 3.51) and creates its proxy body four lines later, in the same
    // handler. `model.transform.position = v` mutates the node's persistent
    // Transform3d datum in place and that datum was only flushed into
    // `node_transforms` once per FRAME, so createRigidBody cooked the body from
    // the pre-write transform — the player spawned at the world origin, in the
    // middle of the hangar, facing a locker. Every other body in the level is
    // authored, which is why only the player was misplaced.
    let spawn = player_pos();
    probe(format!("player spawn: ({:.2}, {:.2}, {:.2})", spawn[0], spawn[1], spawn[2]));
    assert!(
        (spawn[0] - 0.0).abs() < 1.0 && (spawn[1] - 24.24).abs() < 1.0,
        "player did not spawn at the level's PlayerSpawn group (0.00, 24.24): got ({:.2}, {:.2}, {:.2})",
        spawn[0], spawn[1], spawn[2]
    );

    let z_start = player_z();
    player.step_frames(320).await;
    let z_mid = player_z();

    let _ = snapshots.verify("02_gameplay", player.snapshot_stage());

    // `StartLevel1` is behind a 6 s DelayEvent, so the wave counter reads "00"
    // until then — the level really has not begun yet at frame 320. Run on so
    // the snapshot covers a started level (counter "01", first robot spawned)
    // and so the footing check has a settled sample to compare against.
    player.step_frames(900).await;
    let z_end = player_z();
    probe(format!("player z: {:.2} -> {:.2} -> {:.2}", z_start, z_mid, z_end));

    let wave = player.eval_datum("string(gGame.UserWave)").await;
    probe(format!("gGame.UserWave once the level started = {:?}", wave));

    let _ = snapshots.verify("03_wave_started", player.snapshot_stage());





    // ── PERMANENT COVERAGE: the crawling robots ("frog") animate ────────────
    //
    // The first session's probe for this was deleted when it finished, leaving
    // the fix with no standing guard. The failure mode is a crawler GLIDING in
    // a static pose, which no single frame can distinguish from a correct one,
    // so this asserts on state that must CHANGE — plus the structural fact
    // underneath it.
    //
    // Frogs first spawn in Wave 2, which is unreachable inside the harness, so
    // spawn one directly the way `[PS] Spawner` does.
    probe(format!("spawn frog -> {:?}",
        player.eval_datum(
            "AddScript(0, 0, [#script: \"[PS] Robot Frog\", \
             #data: [#member: \"Level1\", #Spawner: \"E2EProbe\"]])").await));
    player.step_frames(30).await;

    // 1. Structural: the rig must have a SKELETON. `RobotFrog` constrains six
    //    bones (`Head` is attrs=0x12), and before the BONES attribute-gated
    //    optional-field fix in `parse_bones_block` the fixed-size reader ran off
    //    the rails at the first of them and the member came out with no skeleton
    //    at all. Reverting that parser hunk makes THIS assertion fail.
    let skels = skeletons_matching("robotfrog");
    probe(format!("RobotFrog skeletons: {:?}", skels));
    assert!(
        skels.iter().any(|(_, bones)| *bones > 1),
        "RobotFrog has no usable skeleton — the BONES block reader derailed on \
         its constrained bones, so the crawlers can only glide in a fixed pose \
         (see docs/areazero/animation-fixes-2026-08-18.md §1.1). found: {:?}",
        skels
    );

    // 2. Behavioural: a walk clip must be BOUND and its playhead must ADVANCE.
    player.step_frames(60).await;
    let frogs_a = bones_players_matching("robotfrog");
    probe(format!("frog bonesPlayers @a: {:?}", frogs_a));
    assert!(!frogs_a.is_empty(),
        "no RobotFrog bonesPlayer — the frog never spawned, so this guard \
         proves nothing; fix the spawn before trusting it");
    // A clip must be BOUND. Deliberately not "the walk clip specifically at this
    // frame": the robot's state machine is physics-paced, so which clip is up at
    // a given step varies run to run. "Something is bound" does not.
    assert!(
        frogs_a.iter().any(|(_, motion, _)| motion != "<none>"),
        "no RobotFrog has any motion bound: {:?}", frogs_a);

    player.step_frames(60).await;
    let frogs_b = bones_players_matching("robotfrog");
    probe(format!("frog bonesPlayers @b: {:?}", frogs_b));
    let advanced = frogs_a.iter().any(|(model, _, t_a)| {
        frogs_b.iter().any(|(m2, _, t_b)| m2 == model && (t_b - t_a).abs() > 1e-3)
    });
    assert!(advanced,
        "the RobotFrog walk playhead did not advance between samples — the \
         crawlers are frozen: {:?} -> {:?}", frogs_a, frogs_b);

    // ── PERMANENT COVERAGE: the F-key blade ("Punch") ───────────────────────
    //
    // KNOWN-BAD RENDER as of 2026-08-19. `[M] FPS Weapon.setup_Punch` clones the
    // 54-bone "Punch" rig into FPSPlayerView out of a member that holds the rig
    // and none of its clips, so nothing strips the biped COM out of its skin and
    // the blade draws as a large near-black hard-edged slab sweeping the view
    // instead of a lit, textured arm/gauntlet with the additive PunchEnergy
    // slash ring. The attempted fix — a draw-time posed-root strip for any clone
    // — is REVERTED: it rotated Agent Free Ride's riders and Rifleman's
    // soldiers, and it contradicts the clone-hop contract documented on
    // `Shockwave3dRuntimeState::clone_hop_count`, measured in real Director 11.5.
    // See §1.5 / §B / §C of docs/areazero/animation-fixes-2026-08-18.md.
    //
    // What is asserted here is only the STRUCTURE — a parse fact, identical in
    // every run: the Punch rig must come through with a real skeleton. Be
    // honest about its strength: that skeleton belongs to the SOURCE member, so
    // the assertion would also hold if the melee never fired; it is a guard
    // against the rig failing to parse (the same class of defect the BONES fix
    // addressed for RobotFrog), not against the blade being mis-posed. The
    // bonesPlayer probe below shows whether the swing actually bound. The POSE —
    // the actual defect — is not assertable at all today; see the coverage-limit
    // note at the end of this test.
    player.key_down("f", 70).await;
    player.step_frames(6).await;
    // TEMP PROBE (robots-and-blade): capture the blade mid-swing for a by-eye check.
    let _ = snapshots.verify("04_blade_probe", player.snapshot_stage());
    let punch_skels = skeletons_matching("punch");
    let punch_players = bones_players_matching("punch");
    probe(format!("blade: skeletons={:?} bonesPlayers={:?}", punch_skels, punch_players));
    assert!(
        punch_skels.iter().any(|(_, bones)| *bones > 1),
        "the F-key melee produced no usable Punch skeleton: {:?}", punch_skels);

    // Guards the in-game footing. The player used to slide down through the
    // hangar floor forever, because:
    //   * its collision proxy was centred on the body ORIGIN, but the authored
    //     proxy has its origin at the FEET (half-height 1.0, geometry AABB
    //     centre +1.0), so it spawned half-buried; and
    //   * `rayCastClosest` tested cooked meshes as their bounding BOX, so the
    //     ground probe (`feet + 0.1`, -Z, needs a hit within 0.2) never found a
    //     surface and it reported "airborne" forever.
    // The drop from z_start to z_mid is the real one — the spawn group hangs
    // ~1 unit above the floor. What must NOT happen is continued sinking.
    assert!(
        z_end.is_finite() && z_end > z_mid - 0.5,
        "player sank through the hangar floor: z {:.2} -> {:.2} -> {:.2}", z_start, z_mid, z_end
    );



    // ── PERMANENT COVERAGE: deleting 3D objects RELEASES their runtime state ──
    //
    // `deleteModel` used to remove the node from `scene.nodes` and nothing else,
    // and `deleteModelResource` fell through to a no-op arm entirely. Every
    // per-node map (`node_transforms`, `node_transform_datums`, `user_data`, …)
    // and every per-resource one (`mesh_build_data`, the "face:<res>" list) kept
    // its entry forever — and two of those hold a `DatumRef`, so the arena datums
    // behind them could never be reclaimed either.
    //
    // This movie is the worst case: every robot spawns ~8 nodes and every wave
    // destroys them, and `[PS] Rocket.CreateTrail` builds a fresh `newMesh`
    // resource with a 40-face list for EVERY rocket fired. Measured before the
    // fix, 10 create/delete cycles leaked ~4,030 arena datums and never gave any
    // of it back; a long session took Chrome to 25 GB.
    //
    // No pixel or state assertion can see this, so it needs its own guard. The
    // test is a FIXED POINT: create, delete, and every counter must come back to
    // where it started — checked twice, because a one-round check cannot tell a
    // real release from a one-off allocation that simply hasn't repeated yet.
    let base = leak_counters();
    probe(format!("leak baseline (datums, nt, ntd, ud, res, mbd) = {:?}", base));
    let mut after_rounds = Vec::new();
    for round in 0..2 {
        for i in 0..8 {
            let n = round * 8 + i;
            let _ = player.eval(&format!(
                "gAZLeak = member(\"Level1\").newModel(\"azLeakGuard{}\")", n)).await;
            let _ = player.eval(
                "gAZLeak.resource = member(\"Level1\").modelResource(\"RobotTank\")").await;
            let _ = player.eval("gAZLeak.transform.position = vector(1,2,3)").await;
            let _ = player.eval("gAZLeak.userData.addProp(#azLeakGuard, 1)").await;
            let _ = player.eval(&format!(
                "gAZLeakM = member(\"Level1\").newMesh(\"azLeakTrail{}\", 40, 42, 42, 0, 42, 2)", n)).await;
            let _ = player.eval(&format!(
                "gAZLeakF = member(\"Level1\").modelResource(\"azLeakTrail{}\").face[1]", n)).await;
        }
        player.step_frames(4).await;
        for i in 0..8 {
            let n = round * 8 + i;
            let _ = player.eval(&format!(
                "member(\"Level1\").deleteModel(\"azLeakGuard{}\")", n)).await;
            let _ = player.eval(&format!(
                "member(\"Level1\").deleteModelResource(\"azLeakTrail{}\")", n)).await;
        }
        player.step_frames(4).await;
        let now = leak_counters();
        probe(format!("leak after round {} = {:?}", round, now));
        after_rounds.push(now);
    }
    // Compare round 1 against round 0, NOT against the baseline. The guard runs
    // during live gameplay, where the movie is spawning and destroying its own
    // robots the whole time, so a counter can legitimately drift either way by a
    // few between samples. A LEAK is not subtle and is not noise: it adds exactly
    // one entry per object created, so 8 per round, every round, forever.
    let created_per_round = 8i64;
    let names = ["node_transforms", "node_transform_datums", "user_data",
                 "model_resources", "mesh_build_data"];
    let r0 = [after_rounds[0].1, after_rounds[0].2, after_rounds[0].3,
              after_rounds[0].4, after_rounds[0].5];
    let r1 = [after_rounds[1].1, after_rounds[1].2, after_rounds[1].3,
              after_rounds[1].4, after_rounds[1].5];
    for i in 0..5 {
        let growth = r1[i] as i64 - r0[i] as i64;
        assert!(
            growth < created_per_round,
            "`{}` grew by {} across an identical create/delete round ({} -> {}).              Deleting a model or a model resource must release its runtime state;              a leak here also pins arena datums forever, because              node_transform_datums and user_data hold DatumRefs.",
            names[i], growth, r0[i], r1[i]);
    }
    // And the arena itself must stop growing. Round 1 vs round 0 is again the
    // honest comparison: the first round also interns the names and allocates the
    // globals used above, which is one-time — a leak repeats every round.
    let growth = after_rounds[1].0 as i64 - after_rounds[0].0 as i64;
    probe(format!("arena datums round0={} round1={} growth={}",
        after_rounds[0].0, after_rounds[1].0, growth));
    assert!(growth < 2000,
        "the datum arena grew by {} across an identical create/delete round —          something is still holding DatumRefs for deleted 3D objects (measured at          ~4,030 per 10 cycles before the fix, and unbounded)", growth);

    // ── KNOWN COVERAGE LIMITS (deliberate, not forgotten) ───────────────────
    //
    // Pixel snapshots are NOT used as the oracle for any of this movie's 3D.
    // The rendering is physics-paced (AGEIA PhysX) and its frame-to-frame state
    // depends on how fast frames step, so a reference image is not reproducible
    // — that is why `reference/misc/browser/areazero/` was intentionally left
    // empty, and it is what `intel_mrm_lod`'s standing 2.6859% diff looks like.
    // Everything above is therefore either a PARSE fact (deterministic) or a
    // change-over-time check (timing-independent).
    //
    // Consequently these two things remain uncovered and must be checked by eye
    // in the app after `npm run build-vm-dev`:
    //   * the blade's POSE. The defect is a root displacement of ~105 units and
    //     a +90 degree Z fold applied inside the renderer's skinning path; no
    //     Lingo-visible value changes when it happens (position, transform and
    //     aim all still read correctly), and the engine exposes no post-skin
    //     bounds accessor a test could read. Asserting it would need a new
    //     accessor for the posed-skin AABB — that is the missing piece, not the
    //     assertion itself.
    //   * the same is true of Agent Free Ride's rider rotation and Rifleman's
    //     soldier aiming: those regressions were also renderer-internal skin
    //     transforms with no Lingo-visible signature.

    // ── PERMANENT COVERAGE: `cursor -1` ends mouselook ─────────────────────
    //
    // AreaZero hides the cursor for mouselook with `cursor 200` and hands the
    // pointer back with `cursor -1` when it leaves the world for a menu (the
    // death/summary screen after three lives, pause, options). Director 11.5
    // Scripting Dictionary, `cursor` value table: "-1, 0 Arrow" / "200 Blank
    // (hides cursor)" — only 200 hides, and -1 is documented as the way to
    // "reset the cursor to the regular arrow cursor".
    //
    // dirplayer read -1 as hidden too, so `wants_pointer_lock` never dropped and
    // those menus stayed pointer-captured and unclickable. Neither existing
    // release path can save this movie: its menus are THEMSELVES 3D (so the
    // "last 3D sprite left the stage" release never fires) and it never sets a
    // visible non-(-1) cursor. This asserts the documented contract directly.
    let (locked_before, hidden_before) = lock_state();
    probe(format!("mouselook engaged: wants_lock={} cursor_hidden={}",
        locked_before, hidden_before));
    assert!(locked_before && hidden_before,
        "expected mouselook to hold the pointer lock in-level (cursor 200)");

    let _ = player.eval("cursor -1").await;
    player.step_frames(4).await;
    let (locked_after, hidden_after) = lock_state();
    probe(format!("after `cursor -1`: wants_lock={} cursor_hidden={}",
        locked_after, hidden_after));
    assert!(!hidden_after,
        "`cursor -1` is Director's Arrow, it must not read as a hidden cursor");
    assert!(!locked_after,
        "`cursor -1` must release the mouselook pointer lock, or every menu          this movie shows over its 3D world stays unclickable");

    Ok(())
});
