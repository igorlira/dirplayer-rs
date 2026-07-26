use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{datum, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/intel_sub_div_surfaces.toml");

browser_e2e_test!(test_intel_sub_div_surfaces_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "sub_div_surfaces");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(3))).timeout(10.0).await?;

    snapshots.verify("start_game", player.snapshot_stage())?;

    Ok(())
});

// Drive the Depth/Tension sliders and the Wireframe/Shaded toggle to pin the
// `#sds` subdivision + `renderStyle` output against Director's own reference
// grid. Director's depth 0..4 × tension {0,100} matrix is the ground truth:
//   - tension 100 → butterfly `w = 0` → clean, even (linear) subdivision
//   - tension 0   → butterfly `w = 0.2` → overshoot (the lumpy belly)
//   - depth 0     → base cage (no subdivision)
// Driving via `sendAllSprites(#sliderChange, ...)` exercises the real behavior
// path (`sds.depth = integer(ratio/25)`), and each cell reads `sds.depth`/
// `sds.tension` back so a silent eval failure fails loudly instead of snapshotting
// a stale mesh.
browser_e2e_test!(test_intel_sub_div_surfaces_grid, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "sub_div_surfaces");
    snapshots.max_diff_ratio = 0.09;
    snapshots.pixel_tolerance = 30;

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    player.step_until(datum("_movie.frame").equals(StaticDatum::Int(3))).timeout(10.0).await?;

    const MODEL: &str = "member(\"Pitcher\").model(\"Pot\")";
    // Screen positions of the Wireframe / Shaded toggle buttons. Driven by click
    // (the button behaviors run `shaderList[1].renderstyle = #wire/#fill` in real
    // script context — eval doesn't route a chained lvalue assignment there).
    const WIREFRAME_BTN: (i32, i32) = (423, 378);
    const SHADED_BTN: (i32, i32) = (423, 420);

    // depth 0 — base cage, no subdivision (wireframe makes it unambiguous).
    player.eval("sendAllSprites(#sliderChange, \"Depth Slider\", 0)").await?;
    let d = player.eval_datum(&format!("{MODEL}.sds.depth")).await?;
    assert_eq!(d, StaticDatum::Int(0), "depth ratio 0 → sds.depth 0");
    player.click(WIREFRAME_BTN.0, WIREFRAME_BTN.1).await;
    player.step_frames(2).await;
    snapshots.verify("grid_depth0_wire", player.snapshot_stage())?;

    // depth 4 / tension 0 — max butterfly smoothing → overshoot (lumpy).
    player.eval("sendAllSprites(#sliderChange, \"Depth Slider\", 100)").await?;
    player.eval("sendAllSprites(#sliderChange, \"Tension Slider\", 0)").await?;
    let d = player.eval_datum(&format!("{MODEL}.sds.depth")).await?;
    assert_eq!(d, StaticDatum::Int(4), "depth ratio 100 → sds.depth 4");
    player.step_frames(2).await;
    snapshots.verify("grid_depth4_tension0_wire", player.snapshot_stage())?;
    player.click(SHADED_BTN.0, SHADED_BTN.1).await;
    player.step_frames(2).await;
    snapshots.verify("grid_depth4_tension0_shaded", player.snapshot_stage())?;

    // depth 4 / tension 100 — w = 0 → clean, even linear subdivision.
    player.eval("sendAllSprites(#sliderChange, \"Tension Slider\", 100)").await?;
    player.click(WIREFRAME_BTN.0, WIREFRAME_BTN.1).await;
    player.step_frames(2).await;
    snapshots.verify("grid_depth4_tension100_wire", player.snapshot_stage())?;
    player.click(SHADED_BTN.0, SHADED_BTN.1).await;
    player.step_frames(2).await;
    snapshots.verify("grid_depth4_tension100_shaded", player.snapshot_stage())?;

    // depth 2 / tension 65 — the movie's mild default midpoint, shaded.
    player.eval("sendAllSprites(#sliderChange, \"Depth Slider\", 50)").await?;
    player.eval("sendAllSprites(#sliderChange, \"Tension Slider\", 65)").await?;
    let d = player.eval_datum(&format!("{MODEL}.sds.depth")).await?;
    assert_eq!(d, StaticDatum::Int(2), "depth ratio 50 → sds.depth 2");
    player.step_frames(2).await;
    snapshots.verify("grid_depth2_tension65_shaded", player.snapshot_stage())?;

    Ok(())
});
