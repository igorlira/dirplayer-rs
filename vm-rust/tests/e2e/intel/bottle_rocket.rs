use vm_rust::browser_e2e_test;
use vm_rust::director::lingo::datum::Datum;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::datum_ref::DatumRef;
use vm_rust::player::reserve_player_ref;
use vm_rust::player::testing_shared::{datum, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/intel_bottle_rocket.toml");

/// `counter` is the movie's own animation clock (BehaviorScript 6 ticks it once
/// per exitFrame while `startanim` is set). The harness steps faster than the
/// movie's tempo, so every checkpoint below is keyed to it rather than to a
/// harness frame count.
fn counter(player: &impl TestHarness) -> i32 {
    let r: Option<DatumRef> = player.get_global_ref("counter");
    match r {
        Some(d) => reserve_player_ref(|p| match p.get_datum(&d) {
            Datum::Int(v) => *v as i32,
            Datum::Float(v) => *v as i32,
            _ => -1,
        }),
        None => -1,
    }
}

async fn num(player: &impl TestHarness, expr: &str) -> f64 {
    match player.eval_datum(expr).await {
        Ok(StaticDatum::Int(v)) => v as f64,
        Ok(StaticDatum::Float(v)) => v,
        other => panic!("{} returned {:?}, expected a number", expr, other),
    }
}

/// `StaticDatum` has no vector variant, so read the components.
async fn vec3(player: &impl TestHarness, expr: &str) -> [f64; 3] {
    [
        num(player, &format!("({}).x", expr)).await,
        num(player, &format!("({}).y", expr)).await,
        num(player, &format!("({}).z", expr)).await,
    ]
}

async fn int_of(player: &impl TestHarness, expr: &str) -> i32 {
    num(player, expr).await as i32
}

/// Step until the movie's `counter` reaches `target` (it only advances while the
/// launch animation is running, so this also proves the animation is alive).
async fn step_to_counter(
    player: &mut impl TestHarness,
    target: i32,
    max_steps: usize,
) -> Result<(), String> {
    for _ in 0..max_steps {
        if counter(player) >= target {
            return Ok(());
        }
        player.step_frame().await;
    }
    Err(format!(
        "counter never reached {} (stuck at {})",
        target,
        counter(player)
    ))
}

browser_e2e_test!(test_intel_bottle_rocket_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "bottle_rocket");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(3))).timeout(10.0).await?;

    // Only the static opening frame is gated tightly. Everything after the click
    // is particle-driven and the systems integrate against wall-clock dt, so
    // those frames differ run to run by design (measured: ~10% on the exhaust) —
    // they carry a loose tolerance that still catches a blank or grossly wrong
    // frame, while the emitter-state and change-over-time assertions below do
    // the real verification, per the project's rule for 3D movies.
    snapshots.verify("start_game", player.snapshot_stage())?;

    // ── Picking ────────────────────────────────────────────────────────────
    // BehaviorScript 4 arms the launch only when `camera.modelUnderLoc` returns
    // the Fuse model, so the click has to land on the fuse wire poking out of
    // the rocket (≈390,140 on the 750x405 stage).
    let under = player.eval_datum("string(sprite(1).camera.modelUnderLoc(point(394,130)))").await?;
    if !format!("{:?}", under).to_lowercase().contains("fuse") {
        return Err(format!("modelUnderLoc at the fuse returned {:?}, expected the Fuse model", under));
    }

    let rocket_start = vec3(&player, "sprite(1).member.model(\"BottleRocket\").worldPosition").await;
    player.click(394, 130).await;

    // ── Fuse burns (counter 1..29) ─────────────────────────────────────────
    // The click must arm `startanim`; if it didn't, `counter` never moves.
    step_to_counter(&mut player, 12, 400).await?;
    let fuse_lit = int_of(&player, "sprite(1).member.modelResource(\"FuseFire\").emitter.numParticles").await;
    if fuse_lit <= 0 {
        return Err("fuse spark emitter never turned on after clicking the fuse".into());
    }
    let fuse_mid = vec3(&player, "sprite(1).member.model(\"Fuse\").worldPosition").await;
    if fuse_mid[2] <= rocket_start[2] {
        return Err(format!("fuse did not travel up the rocket: {:?}", fuse_mid));
    }
    snapshots.verify_with_ratio("fuse_burning", player.snapshot_stage(), 0.2)?;

    // ── Ignition (counter 30): flame on, spark off ─────────────────────────
    step_to_counter(&mut player, 40, 400).await?;
    let flame = int_of(&player, "sprite(1).member.modelResource(\"RocketFire\").emitter.numParticles").await;
    let spark = int_of(&player, "sprite(1).member.modelResource(\"FuseFire\").emitter.numParticles").await;
    if flame <= 0 || spark != 0 {
        return Err(format!("at ignition expected flame on / spark off, got flame={} spark={}", flame, spark));
    }
    // The exhaust is `RocketFlame.parent = rocket`, so its emitter has to follow
    // the rocket's WORLD position, not its parent-relative offset.
    snapshots.verify_with_ratio("ignition_flame", player.snapshot_stage(), 0.3)?;

    // ── Launch (counter 57..85) ────────────────────────────────────────────
    step_to_counter(&mut player, 80, 600).await?;
    let rocket_up = vec3(&player, "sprite(1).member.model(\"BottleRocket\").worldPosition").await;
    if rocket_up[2] < rocket_start[2] + 1000.0 {
        return Err(format!("rocket did not launch: {:?} -> {:?}", rocket_start, rocket_up));
    }

    // ── Explosion (counter 86: main burst; 94+: coloured secondaries) ──────
    step_to_counter(&mut player, 96, 600).await?;
    let boom = int_of(&player, "sprite(1).member.modelResource(\"ExplosionRec\").emitter.numParticles").await;
    let mini = int_of(&player, "sprite(1).member.modelResource(\"MExplosion1\").emitter.numParticles").await;
    if boom <= 0 || mini <= 0 {
        return Err(format!("explosion emitters never fired: main={} mini1={}", boom, mini));
    }
    // The rocket is recycled back to its start transform for the next loop.
    let rocket_reset = vec3(&player, "sprite(1).member.model(\"BottleRocket\").worldPosition").await;
    if (rocket_reset[2] - rocket_start[2]).abs() > 1.0 {
        return Err(format!("rocket was not reset after the explosion: {:?}", rocket_reset));
    }
    snapshots.verify_with_ratio("explosion", player.snapshot_stage(), 0.45)?;

    // ── Reset (counter 150) ────────────────────────────────────────────────
    step_to_counter(&mut player, 149, 800).await?;
    snapshots.verify_with_ratio("explosion_fading", player.snapshot_stage(), 0.45)?;

    Ok(())
});
