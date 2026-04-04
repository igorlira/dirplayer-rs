use vm_rust::player::testing::{run_test, TestPlayer, StaticDatum};

const TEST_USERNAME: &str = "testuser";
const TEST_PASSWORD: &str = "testpass";

#[test]
fn test_loading() {
    run_test(async {
        let mut player = TestPlayer::new();
        player
            .load_movie("public/dcr_woodpecker/habbo.dcr")
            .await;
        player.init_movie().await;
        player.step_frames(5).await;

        // By frame 5, the boot sequence should be underway
        assert_eq!(
            player.eval_datum("sprite(1).member.name").await,
            StaticDatum::String("Logo".into())
        );
        assert!(player.get_global_ref("gCore").is_some(), "gCore global should exist");
        assert_eq!(
            player.eval_datum("ilk(gCore.get(#castload_manager))").await,
            StaticDatum::Symbol("instance".into())
        );
        assert_eq!(
            player.eval_datum("getStreamStatus(\"external_variables.txt\").state").await,
            StaticDatum::String("Complete".into())
        );

        player.snapshot_stage().assert_snapshot("preload", 0.0);

        // Wait until the loading screen is fully drawn
        player.step_until_sprite_visible(100, "corner_element", 1.0).await;

        assert_eq!(
            player.eval_datum("getStreamStatus(\"external_texts.txt\").state").await,
            StaticDatum::String("Complete".into())
        );
        let loaded_count = player.eval_datum("gCore.get(#castload_manager).pLoadedCasts.count").await
            .as_integer().expect("pLoadedCasts.count should be an integer");
        assert!(loaded_count > 2, "Should have loaded more than 2 casts, got {}", loaded_count);

        player.snapshot_stage().assert_snapshot("loaded_state", 0.005);

        // --- Login form ---

        // Type username
        player.click_member_prefix("login_name").await;
        player.step_frames(2).await;
        player.type_text(TEST_USERNAME).await;

        // Type password
        player.click_member_prefix("login_password").await;
        player.step_frames(2).await;
        player.type_text(TEST_PASSWORD).await;

        player.snapshot_stage().assert_snapshot("login_filled", 0.005);

        // TODO: Click login button once native WebSocket support is added
        // player.click_member("login_b_login_ok").await;
    });
}
