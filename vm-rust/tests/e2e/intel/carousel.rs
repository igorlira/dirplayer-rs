use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{
    datum, log_test_action, sprite, SnapshotContext, TestConfig, TestHarness,
};

const CONFIG: &str = include_str!("../configs/intel_carousel.toml");

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

/// Intel's Carousel — one of the sample movies shipped with the Shockwave 3D
/// asset Intel itself authored, so its script comments document the intended
/// behaviour directly.
///
/// The movie is a three-frame boot: frame 1 spins until every streaming member
/// reports `mediaReady`, frame 2 waits for `member("Carousel").state = 4` and
/// then runs `init`, and frame 3 calls `spin` once per `exitFrame` forever.
/// `init` is the interesting part — it clones 11 horses off the imported
/// master, parents each to a pole, builds the carousel hierarchy under `floor`,
/// and generates the surrounding environment (a `#cylinder` wall and ground)
/// from scratch. Almost every assertion below is a fact that only holds if that
/// whole chain ran.
browser_e2e_test!(test_intel_carousel_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "carousel");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // Frame 3 is only reached once BOTH gates pass: all seven members report
    // mediaReady, and the 3D member finishes loading (state 4). Stalling at
    // frame 1 or 2 is the movie's own way of reporting a boot failure.
    player
        .step_until(datum("_movie.frame").equals(StaticDatum::Int(3)))
        .timeout(30.0)
        .await?;

    let state = num(&player, "member(\"Carousel\").state").await;
    if state != 4.0 {
        return Err(format!("3D member state is {state}, expected 4 (loaded)"));
    }

    // ── init: the cloned horses ────────────────────────────────────────────
    // `clone` builds horseList[1..12]: horse1 is the imported master, 2..12 are
    // clones. Each is parented to its own pole, which is what carries them
    // around when `floor` rotates — a horse left on the world root would stay
    // put while the carousel turned.
    let horses = num(&player, "count(horseList)").await as i32;
    if horses != 12 {
        return Err(format!("horseList has {horses} entries, expected 12"));
    }
    for i in 1..=12 {
        let parent = text_of(&player, &format!("string(horseList[{i}].parent)")).await;
        let expected = format!("pole {i}");
        if !parent.to_lowercase().contains(&expected.to_lowercase()) {
            return Err(format!(
                "horse {i} is parented to {parent:?}, expected model(\"{expected}\")"
            ));
        }
    }

    // The two alternating horse shaders/textures, and the motion every horse
    // queues. `duration` proves the motion resource itself parsed, not just its
    // name — `quartTime` is derived from it to stagger the horses.
    for expr in [
        "mem.shader(\"OneShade\")",
        "mem.shader(\"TwoShade\")",
        "mem.texture(\"OneTex\")",
        "mem.texture(\"TwoTex\")",
        "mem.motion(\"horse1-key\")",
    ] {
        let v = text_of(&player, &format!("string({expr})")).await;
        if v.is_empty() || v == "<Void>" {
            return Err(format!("{expr} is VOID after init"));
        }
    }
    let duration = num(&player, "mem.motion(\"horse1-key\").duration").await;
    if duration <= 0.0 {
        return Err(format!("horse1-key duration is {duration}, expected > 0"));
    }

    // ── init: the environment `ce()` generates at runtime ──────────────────
    // A #cylinder resource with both caps off for the sky wall, and a second
    // flattened cylinder for the ground. Neither exists in the imported W3D.
    for expr in [
        "mem.modelResource(\"RoundWallsRec\")",
        "mem.model(\"RoundWalls\")",
        "mem.modelResource(\"groundRec\")",
        "mem.model(\"ground\")",
        "mem.texture(\"wallPaint\")",
    ] {
        let v = text_of(&player, &format!("string({expr})")).await;
        if v.is_empty() || v == "<Void>" {
            return Err(format!("{expr} was not created by ce()"));
        }
    }

    // `gInitCameraTrans` is captured in `init` via `transform.clone()`, which
    // the Reset View and Zoom buttons both depend on. It used to raise
    // "No handler 'clone' for transform", killing init on its fourth line.
    let cam = text_of(&player, "string(gInitCameraTrans)").await;
    if !cam.starts_with("transform(") {
        return Err(format!("gInitCameraTrans is {cam:?}, expected a transform"));
    }

    // The four push buttons are one row of identical controls in Director. They
    // are the regression guard for button sprites taking their size from the
    // score rather than from a stale member initialRect.
    let mut heights = Vec::new();
    for n in 2..=5 {
        heights.push(num(&player, &format!("sprite({n}).height")).await as i32);
    }
    if heights.iter().any(|h| *h != heights[0]) {
        return Err(format!(
            "the four button sprites have heights {heights:?}, expected one uniform row"
        ));
    }
    log_test_action(&format!("[carousel] button row height {}", heights[0]));

    snapshots.verify_with_ratio("idle", player.snapshot_stage(), 0.02)?;

    // ── Start: the carousel accelerates ────────────────────────────────────
    // The Start button toggles `startFlag`, after which `spin` ramps `speed` up
    // by gSpeedIncrement each frame and rotates `floor`. Everything on the
    // carousel is parented under `floor`, so the horses have to travel with it.
    let horse_before = num(&player, "mem.model(\"horse1\").worldPosition.x").await;
    player.click_sprite(sprite().number(2)).await?;
    // The member's `on mouseUp` is dispatched on the next step, not inside the
    // click itself, so the toggle is only observable after a frame.
    player.step_frames(3).await;
    if text_of(&player, "string(member(\"Start-Stop\").text)").await != "Stop" {
        return Err("the Start button did not toggle to Stop".into());
    }

    player.step_frames(120).await;

    let rot = num(&player, "mem.model(\"floor\").transform.rotation.z").await;
    if rot == 0.0 {
        return Err("floor never rotated after Start was pressed".into());
    }
    let horse_after = num(&player, "mem.model(\"horse1\").worldPosition.x").await;
    if (horse_after - horse_before).abs() < 1.0 {
        return Err(format!(
            "horse 1 did not travel with the carousel: {horse_before} -> {horse_after}"
        ));
    }
    log_test_action(&format!(
        "[carousel] floor rotation.z {rot:.2}, horse1 x {horse_before:.1} -> {horse_after:.1}"
    ));

    Ok(())
});
