use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{log_test_action, sprite, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/gorillaz_finaldrive.toml");

/// FinalDrive traction probe — the offline equivalent of the manual
/// Director-vs-dirplayer log comparison this movie was previously diagnosed with.
///
/// The RaycastCar behaviour gates ALL drive, steering and grip on
/// `pPowerCoeff = (wheels within pHoverDist) / 4`, so the single number that
/// predicts whether the car can climb the sloped road is how often all four wheels
/// keep contact. Measured against real Director on this movie: hover=4 on 397/397
/// frames (100%). dirplayer has measured between 35% and 66%.
///
/// This exists so that metric can be iterated on without a human driving both
/// engines and pasting logs.
browser_e2e_test!(test_finaldrive_traction, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // The movie opens on a menu; sprite 8 enters the 3D scene. Without this the
    // RaycastCar behaviour never begins and everything below measures nothing.
    player.step_frames(20).await;
    player.click_sprite(sprite().number(8)).await?;

    // Let the car spawn, fall and settle on its wheels.
    player.step_frames(220).await;

    fn hovering(d: &StaticDatum) -> usize {
        match d {
            StaticDatum::List(items) => items
                .iter()
                .filter(|i| matches!(i, StaticDatum::Int(1)))
                .count(),
            _ => usize::MAX, // sentinel: shape not as expected
        }
    }
    fn as_f64(d: &StaticDatum) -> Option<f64> {
        match d {
            StaticDatum::Float(f) => Some(*f),
            StaticDatum::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    // Sanity: confirm we're actually in the 3D scene with a live car before
    // measuring, so a menu-still-showing run fails loudly instead of silently
    // reporting 0%.
    let probe = player.eval_datum("sprite(1).pIsHoveringList").await;
    log_test_action(&format!("FINALDRIVE pIsHoveringList probe = {:?}", probe));

    // Hold the accelerator (arrow-up = keyCode 126, per getKeys).
    player.key_down("", 126).await;

    let mut hover_counts = [0u32; 5];
    let mut unexpected = 0u32;
    let mut max_speed = 0.0f64;
    for _ in 0..300 {
        player.step_frames(1).await;
        if let Ok(d) = player.eval_datum("sprite(1).pIsHoveringList").await {
            let n = hovering(&d);
            if n <= 4 { hover_counts[n] += 1; } else { unexpected += 1; }
        }
        if let Ok(d) = player
            .eval_datum("gHavok.rigidBody(\"chassis\").linearVelocity.length")
            .await
        {
            if let Some(f) = as_f64(&d) {
                if f > max_speed { max_speed = f; }
            }
        }
    }
    player.key_up("", 126).await;

    let total: u32 = hover_counts.iter().sum();
    let pct = if total > 0 { 100 * hover_counts[4] / total } else { 0 };
    let summary = format!(
        "hover {:?} (unexpected {})  four-wheel {}/{} ({}%)  max|v| {:.2}   [Director: 100%]",
        hover_counts, unexpected, hover_counts[4], total, pct, max_speed
    );
    log_test_action(&format!("FINALDRIVE {}", summary));

    // Assert rather than just log: the browser runner surfaces only pass/fail and
    // the error string, so this is how the measurement actually reaches the console.
    // Director keeps all four wheels in contact on 100% of frames; the threshold is
    // deliberately below that so it tracks progress instead of demanding parity.
    if total == 0 {
        return Err("no hover samples — did the sprite(8) click reach the 3D scene?".into());
    }
    if pct < 90 {
        return Err(summary);
    }
    Ok(())
});
