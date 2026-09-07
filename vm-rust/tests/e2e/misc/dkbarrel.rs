use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_dkbarrel.toml");

// Skyworks DK Barrel Blast — coverage for `do` / `eval` REACHING AND MUTATING
// handler locals.
//
// This movie is the only one known to exercise that path, and the suite had no
// coverage of it at all. It matters because `Scope.locals` is a
// `FxHashMap<u16, DatumRef>` keyed by name id, and the do/eval resolver in
// `eval.rs` relies on a key being ABSENT to mean "not a local in this frame",
// falling through to `me` and then globals. Any change to how locals are
// stored — a dense slot-indexed Vec, say — can silently alter that, and
// "silently" is the problem: a stale or wrongly-shadowed local produces wrong
// behaviour, not a crash.
//
// Deliberately NO snapshot. The point is to EXECUTE the path far enough that a
// broken local read shows up as a script error, a hang, or a frame that never
// arrives — not to pin down pixels. A snapshot here would only add a
// maintenance burden and a second reason to fail.
browser_e2e_test!(test_misc_dkbarrel_do_locals, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // The loader hands off to dkbmmain.dcr, so this has to run long enough to
    // get through the handoff and into the game's own scripts, which is where
    // the `do`-driven local access lives.
    player.step_frames(500).await;

    Ok(())
});
