use vm_rust::player::testing::{run_test, TestPlayer};

#[test]
fn test_load_movie() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;

        assert_eq!(player.current_frame(), 1);
        assert!(player.is_playing());
    });
}

#[test]
fn test_initial_stage() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;

        player.snapshot_stage().assert_snapshot("habbo_v7__initial_stage", 0.0);
    });
}

#[test]
fn test_after_init() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;
        player.init_movie().await;

        player.snapshot_stage().assert_snapshot("habbo_v7__after_init", 0.0);
    });
}


#[test]
fn test_after_5_frames() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;
        player.init_movie().await;
        player.step_frames(5).await;

        player.snapshot_stage().assert_snapshot("habbo_v7__after_5_frames", 0.0);
    });
}

#[test]
fn test_after_30_frames() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;
        player.init_movie().await;
        player.step_frames(30).await;

        player.snapshot_stage().assert_snapshot("habbo_v7__after_30_frames", 0.0);
    });
}

#[test]
fn test_after_100_frames() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;
        player.init_movie().await;
        player.step_frames(100).await;

        // Allow small tolerance — loading animation timing varies between runs
        player.snapshot_stage().assert_snapshot("habbo_v7__after_100_frames", 0.005);
    });
}
