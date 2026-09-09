use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/dpt_fps_test.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[TEMPO] {}", msg).into());
}

/// (puppet_tempo, current_frame_tempo, effective_tempo)
fn tempo_state() -> (u32, u32, u32) {
    vm_rust::player::reserve_player_ref(|p| {
        (
            p.movie.puppet_tempo,
            p.current_frame_tempo,
            p.movie.get_effective_tempo(),
        )
    })
}

// `puppetTempo` must take effect on a HELD frame.
//
// `current_frame_tempo` is the value the frame loop paces from, and it used to be
// refreshed only by `begin_all_sprites` — which `run_single_frame` skips whenever
// the playhead stays on the same frame. Holding a frame with `go the frame` is the
// standard Director idiom (every game in the corpus does it), so `puppetTempo()`
// updated `movie.puppet_tempo`, `get_effective_tempo()` reported the new value,
// and the frame loop went on pacing from a tempo frozen at frame entry.
//
// Measured with `docs/fps-probe` before the fix: this movie sat at 1.00 fps
// (1002 ms/frame) through puppetTempo 30 / 60 / 120 / 999 — its authored
// frame_rate is 1 — where Shockwave gives 30 / 60 / 120 / 1000.
browser_e2e_test!(test_dpt_puppet_tempo_on_held_frame, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    player.step_frames(30).await;

    let frame_before = vm_rust::player::reserve_player_ref(|p| p.movie.current_frame);

    for tempo in [30u32, 60, 120, 999] {
        player
            .eval(&format!("puppetTempo({})", tempo))
            .await
            .map_err(|e| format!("puppetTempo({}) failed: {:?}", tempo, e))?;

        // Give the frame loop a bounded chance to pick it up, rather than
        // assuming a fixed number of harness steps: `step_frames` yields rAF
        // ticks (~16 ms), and the loop refreshes the tempo at the TOP of each
        // iteration, so at tempo 30 a single 33 ms frame spans more than two
        // ticks. This stays a real assertion — with the bug it fixes,
        // `current_frame_tempo` never converges on a held frame no matter how
        // long you wait, so the loop below just runs out and the assert fires.
        for _ in 0..60 {
            if tempo_state().1 == tempo {
                break;
            }
            player.step_frames(2).await;
        }

        let (puppet, cached, effective) = tempo_state();
        probe(format!(
            "puppetTempo({}) -> puppet={} cached={} effective={}",
            tempo, puppet, cached, effective
        ));

        assert_eq!(
            puppet, tempo,
            "puppetTempo({}) did not reach movie.puppet_tempo", tempo
        );
        assert_eq!(
            effective, tempo,
            "get_effective_tempo() ignored puppetTempo({})", tempo
        );
        assert_eq!(
            cached, tempo,
            "current_frame_tempo (what the frame loop paces from) is stale at {} \
             after puppetTempo({}) — the tempo is not being re-read on a held frame",
            cached, tempo
        );
    }

    // The whole point: none of the above advanced the playhead.
    let frame_after = vm_rust::player::reserve_player_ref(|p| p.movie.current_frame);
    assert_eq!(
        frame_before, frame_after,
        "movie was expected to HOLD one frame; it advanced {} -> {}, so this test \
         no longer covers the held-frame case",
        frame_before, frame_after
    );

    Ok(())
});
