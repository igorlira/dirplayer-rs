use vm_rust::{browser_e2e_test, hybrid_e2e_test};
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness};
use super::shared;

const CONFIG: &str = include_str!("../configs/sulake_habbo_v7.toml");
const TEST_NAME: &str = "habbo_v7";

hybrid_e2e_test!(test_habbo_v7_entry, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    shared::assert_entry(&mut player, cfg.suite(), TEST_NAME, &cfg.movie.path, true).await?;
    Ok(())
});

browser_e2e_test!(test_habbo_v7_login, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let mut snapshots = SnapshotContext::new(cfg.suite(), TEST_NAME);
    snapshots.max_diff_ratio = 0.01;

    shared::assert_entry(&mut player, cfg.suite(), TEST_NAME, &cfg.movie.path, true).await?;
    shared::assert_login(&mut player, cfg.suite(), TEST_NAME, cfg.param("username"), cfg.param("password")).await?;
    
    shared::assert_navigator_visible(&mut player, cfg.suite(), TEST_NAME).await?;
    snapshots.verify("navigator_opened", player.snapshot_stage())?;
    
    shared::assert_navigate_pub(&mut player, cfg.suite(), TEST_NAME, &cfg.movie.path).await?;
    shared::assert_navigate_private(&mut player, cfg.suite(), TEST_NAME, &cfg.movie.path).await?;
    Ok(())
});
