use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/groove_dora_soccer.toml");

/// Dora Soccer is the Groove movie that first exercised the `.3GM` `gSca`
/// chunk. Its `ba.3gm` (soccer ball) is a static shape whose `gSca = 0.1` has
/// to survive `ScaleObject(gBall, 200, …)`, while `da.3gm` (Dora, 115 frames)
/// carries the same `gSca` and must NOT honour it. Get either wrong and the
/// pitch is either swallowed by a ten-times-oversized ball or missing its
/// exploded, ten-times-undersized heroine.
browser_e2e_test!(test_dora_soccer_practice, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "dora_soccer");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    player.step_frames(300).await;

    // Frame 7 is the 3D pitch; gamemode 1 is the easy/practice round.
    let _ = player.eval("gamemode = 1").await;
    let _ = player.eval("go(7)").await;
    player.step_frames(60).await;

    snapshots.verify("practice", player.snapshot_stage())?;

    // Let the intro camera settle onto Dora, then force the HUD number overlays
    // (o1..o5 = LoadSprite "one".."five", the arrowhud `numup` handler) visible.
    // They are keyed cut-out sprites; this guards their transparency compositing.
    player.step_frames(200).await;
    let _ = player.eval("sendSprite(1, #numup, 1)").await;
    let _ = player.eval("sendSprite(1, #numup, 2)").await;
    let _ = player.eval("sendSprite(1, #numup, 3)").await;
    let _ = player.eval("sendSprite(1, #numup, 4)").await;
    let _ = player.eval("sendSprite(1, #numup, 5)").await;
    player.step_frames(4).await;
    snapshots.verify("hud", player.snapshot_stage())?;

    // The five field dots (bshad.3gm, textures d1.s..d5.s) share their shape and
    // each carry a d?.a alpha mask. The mask must NOT consume its own texture id
    // or `dbstart = dotstart + 7` lands on d1.a instead of the first blink
    // texture bd1.s, and the active target renders as a grey circle on an
    // unkeyed black square. Assert the fusion: the seven `.s` dots stay
    // consecutive and bd1.s follows immediately, while d1.a has no id of its own.
    let d1s = player.eval_datum("TextureID(\"d1.s\")").await.ok();
    let d7s = player.eval_datum("TextureID(\"d7.s\")").await.ok();
    let bd1s = player.eval_datum("TextureID(\"bd1.s\")").await.ok();
    let d1a = player.eval_datum("TextureID(\"d1.a\")").await.ok();
    use vm_rust::director::static_datum::StaticDatum::Int;
    match (&d1s, &d7s, &bd1s) {
        (Some(Int(a)), Some(Int(g)), Some(Int(b))) => {
            assert_eq!(*g, a + 6, "d1.s..d7.s must be 7 consecutive ids");
            assert_eq!(*b, a + 7, "bd1.s (dbstart) must follow d7.s — the .a masks take no ids");
        }
        _ => panic!("dot texture ids not resolved: {d1s:?} {d7s:?} {bd1s:?}"),
    }
    assert_eq!(d1a, Some(Int(-1)), "d1.a must fuse into d1.s, not hold its own id");

    // Line the dots up in front of a fixed camera so all five read clearly.
    let _ = player.eval("MoveCamera(0, -400, 120)").await;
    let _ = player.eval("CameraLookAt(0, 200, 0)").await;
    for i in 1..=5 {
        let _ = player.eval(&format!("MoveObject(dotlist[{i}], {}, 200, 4)", -320 + (i - 1) * 160)).await;
        let _ = player.eval(&format!("ScaleObject(dotlist[{i}], 400, 400, 400)")).await;
    }
    player.step_frames(3).await;
    snapshots.verify("dots_row", player.snapshot_stage())?;

    Ok(())
});
