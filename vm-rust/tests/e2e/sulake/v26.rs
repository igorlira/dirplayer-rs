use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness, sprite};
use super::shared;

const CONFIG: &str = include_str!("../configs/sulake_habbo_v26.toml");
const TEST_NAME: &str = "habbo_v26";

browser_e2e_test!(test_habbo_v26_login, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();

    let mut snapshots = SnapshotContext::new(cfg.suite(), TEST_NAME);
    snapshots.max_diff_ratio = 0.01;
     
    shared::assert_entry(&mut player, cfg.suite(), TEST_NAME, &cfg.movie.path, false).await?;
    shared::assert_login_success(&mut player, cfg.suite(), TEST_NAME).await?;

    // Close club gift window if it appears
    if let Ok(_) = player.step_until(sprite().member("window_clubgift_drag").visible(1.0)).await {
        player.click_sprite(sprite().member("window_clubgift_club_confirm_ok")).await?;
    }
    snapshots.verify("login_success", player.snapshot_stage())?;

    shared::assert_navigator_visible(&mut player, cfg.suite(), TEST_NAME).await?;
    snapshots.verify("navigator_opened", player.snapshot_stage())?;

    shared::assert_navigate_pub(&mut player, cfg.suite(), TEST_NAME, &cfg.movie.path).await?;
    shared::assert_navigate_private(&mut player, cfg.suite(), TEST_NAME, &cfg.movie.path).await?;
    Ok(())
});
