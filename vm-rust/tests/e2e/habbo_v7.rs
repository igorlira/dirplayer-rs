use vm_rust::player::testing::{run_test, TestPlayer};

#[test]
fn test_load_movie_and_check_frame() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;

        // Movie should start at frame 1
        assert_eq!(player.current_frame(), 1);
        assert!(player.is_playing());
    });
}
