use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_estate-scaled.toml");

// estate (Shockwave3D) with the stage SCALED (`swStretchStyle = meet`) — the
// layout fullscreen uses.
//
// Camera backdrops are positioned in the SPRITE's own coordinate space:
// `addBackdrop` takes movie pixels, and this movie places its sky at
// loc (-720, 0) with scale 4. The viewport they are drawn into, though, is the
// sprite's RENDER rect, which a scaled stage has already enlarged — so the sky
// kept its authored size and origin inside a viewport 2.8x larger, leaving a
// small band across the top with bare clear colour under it.
//
// The fix divides the 3D renderer's 2D ortho by the stage scale, so a
// movie-space quad covers the same fraction of the viewport at any scale. That
// applies to camera OVERLAYS as well, which are positioned the same way.
browser_e2e_test!(test_misc_estate_scaled, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new("misc", "estate_scaled");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    // 320x388 into 1920x1080 -> `meet` picks 1080/388 = 2.784 and letterboxes.
    vm_rust::set_stage_size(1920, 1080);

    player.step_frames(240).await;

    let (rendered, backdrops) = vm_rust::player::reserve_player_ref(|p| {
        use vm_rust::player::cast_member::CastMemberType;
        let mut n_backdrops = 0usize;
        for cast in p.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if let CastMemberType::Shockwave3d(w3d) = &m.member_type {
                    n_backdrops += w3d.runtime_state.camera_backdrops
                        .values().map(|v| v.len()).sum::<usize>();
                }
            }
        }
        (p.w3d_any_rendered, n_backdrops)
    });
    assert!(rendered, "no 3D was rendered");
    assert!(backdrops > 0, "the movie's camera backdrop is gone — the snapshot below \
        cannot show whether it is positioned correctly if it is not there at all");

    // The snapshot is the real check: the sky must fill the 3D viewport rather
    // than sit at its authored size in the corner.
    let _ = snapshots.verify("01_scaled", player.snapshot_stage());
    Ok(())
});
