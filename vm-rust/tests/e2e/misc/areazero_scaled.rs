use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_areazero-scaled.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

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

/// AreaZero with the stage SCALED. Investigating a reported hard lag once
/// kill text appears in-game, and unreadable rows in the controls menu.
browser_e2e_test!(test_misc_areazero_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    vm_rust::set_stage_size(1920, 1080);

    let snapshots = SnapshotContext::new("misc", "areazero_scaled");

    player.step_frames(500).await;
    let _ = snapshots.verify("01_menu", player.snapshot_stage());

    let mut started = false;
    for attempt in 0..40 {
        player.click(201, 137).await;
        player.step_frames(60).await;
        if body_count() > 0 {
            probe(format!("level started on attempt {}", attempt));
            started = true;
            break;
        }
    }
    assert!(started, "never reached the level - no PhysX bodies were created");

    player.step_frames(320).await;
    let _ = snapshots.verify("02_gameplay", player.snapshot_stage());
    player.step_frames(900).await;
    let _ = snapshots.verify("03_wave_started", player.snapshot_stage());

    Ok(())
});
