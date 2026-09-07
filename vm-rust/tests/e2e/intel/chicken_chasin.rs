use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{log_test_action, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/intel_chicken_chasin.toml");

async fn num(player: &impl TestHarness, expr: &str) -> f64 {
    match player.eval_datum(expr).await {
        Ok(StaticDatum::Int(v)) => v as f64,
        Ok(StaticDatum::Float(v)) => v,
        other => panic!("{} returned {:?}, expected a number", expr, other),
    }
}

async fn text_of(player: &impl TestHarness, expr: &str) -> String {
    match player.eval_datum(expr).await {
        Ok(StaticDatum::String(s)) => s,
        other => format!("{:?}", other),
    }
}

/// Intel's ChickenChasin — another sample shipped with the Shockwave 3D asset
/// Intel authored, so its script comments document the intent directly.
///
/// It is the most demanding of the Intel set because almost nothing is authored
/// into the movie: `SetScene` runs a six-step load state machine that pulls in
/// six external `.w3d` files one at a time (each one only issued once the member
/// reports `state = 4` again), and the ground is GENERATED at runtime — a 64x64
/// greyscale member is read pixel by pixel into a `newMesh` with 4096 vertices
/// and 7938 faces, then `build()`. The assertions below are the facts that only
/// hold if that whole chain ran.
browser_e2e_test!(test_intel_chicken_chasin_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "chicken_chasin");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    player.step_frames(160).await;

    // `gEnvironmentLoaded` is set by the behaviour only when the load state
    // machine reaches step 7, i.e. every external .w3d came back.
    if text_of(&player, "string(gEnvironmentLoaded)").await != "1" {
        return Err("the load state machine never completed (gEnvironmentLoaded is false)".into());
    }

    // Each external file contributes models the movie then reaches by name.
    for expr in [
        "gScene.model(\"terrainmesh\")", // generated, not loaded
        "gScene.model(\"kid_boy\")",     // kid_boy.w3d
        "gScene.model(\"chicken\")",     // chicken.w3d
        "gScene.group(\"envgroup\")",    // built in the pYard step over yard.w3d
        "gScene.group(\"feathers\")",    // feathers.w3d
    ] {
        let v = text_of(&player, &format!("string({expr})")).await;
        if v.is_empty() || v == "<Void>" {
            return Err(format!("{expr} is VOID — its .w3d never loaded"));
        }
    }

    // ── The generated terrain ──────────────────────────────────────────────
    // 64x64 heightmap → 64*64 vertices and 2*(64-1)*(64-1) faces. The vertex
    // count is the regression guard for `newMesh` reserving its vertexList and
    // for that list surviving element-by-element assignment: it read 0 while the
    // faces read 7938, so `build()` produced a resource with no geometry at all.
    let verts = num(&player, "count(gScene.modelResource(\"terrainmeshres\").vertexList)").await as i32;
    let faces = num(&player, "count(gScene.modelResource(\"terrainmeshres\").face)").await as i32;
    if verts != 4096 || faces != 7938 {
        return Err(format!(
            "terrain resource has {verts} vertices / {faces} faces, expected 4096 / 7938"
        ));
    }

    // The heights come from `getPixel(x, y, #integer)`, which returned a color
    // object on this overload and made the movie's `float(...)` raise. A terrain
    // of all-zero heights would still have 4096 vertices, so check the geometry
    // actually varies and spans the authored 3000-unit extent.
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    let mut max_x: f64 = 0.0;
    for i in [1, 500, 1000, 2000, 3000, 4096] {
        let y = num(&player, &format!("gScene.modelResource(\"terrainmeshres\").vertexList[{i}].y")).await;
        let x = num(&player, &format!("gScene.modelResource(\"terrainmeshres\").vertexList[{i}].x")).await;
        min_y = min_y.min(y);
        max_y = max_y.max(y);
        max_x = max_x.max(x);
    }
    if (max_y - min_y) < 1.0 {
        return Err(format!(
            "terrain is flat (y spans {min_y}..{max_y}) — the heightmap never reached the mesh"
        ));
    }
    if max_x < 1000.0 {
        return Err(format!("terrain x extent is only {max_x}, expected ~3000"));
    }
    log_test_action(&format!(
        "[chicken] terrain {verts} verts / {faces} faces, height {min_y:.1}..{max_y:.1}"
    ));

    // The bounding sphere is the whole-mesh summary — it was radius 54 around
    // the origin when the vertex list was empty, and is ~2000 once it is filled.
    let radius = num(&player, "gScene.model(\"terrainmesh\").boundingSphere[2]").await;
    if radius < 1000.0 {
        return Err(format!("terrain boundingSphere radius is {radius}, expected > 1000"));
    }

    // The characters stand ON the generated ground, so their height is only
    // right if the terrain sampling works (Navigation asks the terrain script
    // for getHeight at the character's x/z every frame).
    let boy_y = num(&player, "gScene.model(\"kid_boy\").worldPosition.y").await;
    if boy_y <= 0.0 {
        return Err(format!("kid_boy is at y={boy_y}, expected to stand on the terrain"));
    }

    if text_of(&player, "string(member(\"score\").text)").await != "Score: 0" {
        return Err("the score member was not initialised by startMovie".into());
    }

    for e in [
        "string(gScoreUp)", "string(gScore)",
        "string(member(\"score\").text)", "string(member(\"score\").mediaReady)",
        "string(member(4))", "string(member(4).name)",
        "string(member(4).texture(\"mytex\"))",
        "sprite(1).camera.overlay.count",
        "string(sprite(1).camera.overlay[1].source)",
        "string(sprite(1).camera.overlay[1].scale)",
        "string(sprite(1).camera.overlay[1].blend)",
        "string(sprite(1).camera.name)",
        "gScene.motion.count",
    ] {
        let v = player.eval_datum(e).await;
        log_test_action(&format!("[chicken] {e} => {v:?}"));
    }
    for e in ["string(gWalkMotion)", "string(gWaveMotion)", "string(gCheerMotion)"] {
        let v = player.eval_datum(e).await;
        log_test_action(&format!("[chicken] {e} => {v:?}"));
    }
    let mc = num(&player, "gScene.motion.count").await as i32;
    log_test_action(&format!("[chicken] motion.count = {mc}"));
    for i in 1..=mc.min(20) {
        let n = text_of(&player, &format!("string(gScene.motion({i}).name)")).await;
        let d = text_of(&player, &format!("string(gScene.motion({i}).duration)")).await;
        log_test_action(&format!("[chicken] motion {i}: {n} dur={d}"));
    }

    // The score is drawn as a CAMERA OVERLAY, not a sprite: ChangeScore builds a
    // texture from the "score" text member and adds it to the camera. Both halves
    // were broken — newTexture(#fromCastMember) refused a text member outright,
    // and once it accepted one, forcing the texture opaque drew a black bar.
    if text_of(&player, "string(sprite(1).camera.overlay[1].source)").await.is_empty() {
        return Err("the score overlay has no source texture".into());
    }

    // Six .w3d files load into one member, and two of them (kid_boy_wave,
    // kid_boy_cheer) carry a clip named for the same rig as kid_boy.w3d. With
    // generateUniqueNames the incoming clip must be RENAMED and added, not
    // dropped on top of the existing one — Intel reads the newest clip back as
    // `motion(motion.count)`, so a swallowed clip silently hands the movie the
    // previous file's animation.
    let motions = num(&player, "gScene.motion.count").await as i32;
    if motions != 16 {
        return Err(format!("expected 16 motions after all six loads, got {motions}"));
    }
    let walk = text_of(&player, "string(gWalkMotion)").await;
    let wave = text_of(&player, "string(gWaveMotion)").await;
    let cheer = text_of(&player, "string(gCheerMotion)").await;
    if wave == walk || cheer == walk || wave == cheer {
        return Err(format!(
            "the wave/cheer clips collapsed onto another motion: walk={walk} wave={wave} cheer={cheer}"
        ));
    }

    // KNOWN DEFECT — a thin bright-green wedge crosses the hillside where
    // Director's is smooth. It is NOT a hole and NOT missing geometry:
    //
    //   * The wedge pixels are rgb(96,255,82) — the grass diffuse rgb(64,192,54)
    //     scaled about 1.5x with GREEN CLIPPED at 255 — against rgb(25,75,22)
    //     (~0.39x) on the surrounding slope. So it is a blown-out highlight of
    //     the same material, roughly 4x the surrounding intensity.
    //   * The built mesh is a perfect manifold: 12033 edges, exactly 252 used
    //     once (the 4x63 perimeter), 11781 used twice, none used more, and no
    //     coincident vertices. An .obj export of the same mesh renders solid.
    //   * The terrain is drawn ONCE (one mesh_info, one shader binding, always
    //     the "grass" override), so it is not z-fighting with a second draw.
    //   * Ruled out by rendering with each disabled: the sky model, the grass
    //     shader's specular, and area- vs unit-weighted smooth normals.
    //
    // Most likely cause, not yet fixed: we light the scene with TWO lights
    // Director does not have. Ours are DefaultAmbient rgb(76,76,76) and
    // DefaultDirectional rgb(191,191,191) — dirplayer's empty-scene fallbacks —
    // on top of the movie's own UIAmbient/UIDirectional/Omni01/Omni02/max light.
    // Director's list for this member is seven lights, none of them named
    // Default*. That extra ~1.05 of intensity is the right order to push a lit
    // ridge crest from ~0.5 to the ~1.5 measured here. Note the terrain really
    // does have sharp ridges (210-unit steps across a 47-unit grid spacing), so
    // a crest highlight belongs there — it is the MAGNITUDE that is wrong.
    //
    // Careful when testing this: `light.color = rgb(0,0,0)` from Lingo does not
    // reach the renderer (setting every light black leaves the frame identical),
    // and `light.attenuation` reads empty, so neither can be used to bisect the
    // lighting from a test. Both are separate gaps.
    snapshots.verify_with_ratio("start_game", player.snapshot_stage(), 0.05)?;

    Ok(())
});
