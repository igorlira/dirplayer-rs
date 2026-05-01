use vm_rust::{browser_e2e_test, hybrid_e2e_test};
use vm_rust::player::testing_shared::{TestConfig};
use super::shared;

const CONFIG: &str = include_str!("../configs/sulake_habbo_v7.toml");

hybrid_e2e_test!(test_habbo_v7_entry, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    shared::assert_entry(&mut player, cfg.suite(), &cfg.movie.path).await?;
    Ok(())
});

browser_e2e_test!(test_habbo_v7_login, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    shared::assert_entry(&mut player, cfg.suite(), &cfg.movie.path).await?;
    shared::assert_login(&mut player, cfg.suite(), &cfg.movie.path, cfg.param("username"), cfg.param("password")).await?;
    shared::assert_navigate_pub(&mut player, cfg.suite(), &cfg.movie.path).await?;
    Ok(())
});
