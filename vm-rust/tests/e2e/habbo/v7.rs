use vm_rust::{browser_e2e_test, hybrid_e2e_test};
use vm_rust::player::testing_shared::TestHarness;
use super::shared;

const SUITE: &str = "habbo_v7";
const MOVIE: &str = "dcr_woodpecker/habbo.dcr";
const USERNAME: &str = "crimetime";
const PASSWORD: &str = "test123";

hybrid_e2e_test!(test_habbo_v7_entry, |player| async move {
    shared::assert_entry(&mut player, SUITE, MOVIE).await?;
    Ok(())
});

browser_e2e_test!(test_habbo_v7_login, |player| async move {
    shared::assert_entry(&mut player, SUITE, MOVIE).await?;
    shared::assert_login(&mut player, SUITE, USERNAME, PASSWORD).await?;
    shared::assert_navigate_pub(&mut player, SUITE).await?;
    Ok(())
});
