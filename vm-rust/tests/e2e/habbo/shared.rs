use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{SnapshotContext, TestHarness};

pub async fn test_habbo_entry(
    player: &mut impl TestHarness,
    suite: &str,
    movie_asset: &str,
) -> Result<(), String> {
    let movie_path = player.asset_path(movie_asset);
    let snapshots = SnapshotContext::new(suite, "entry");

    player.load_movie(&movie_path).await;
    player.init_movie().await;
    player.step_frames(5).await;

    // By frame 5, the boot sequence should be underway
    let logo = player.eval_datum("sprite(1).member.name").await?;
    if logo != StaticDatum::String("Logo".into()) {
        return Err(format!("Expected Logo sprite, got {:?}", logo));
    }
    if player.get_global_ref("gCore").is_none() {
        return Err("gCore global should exist".into());
    }
    let castload = player.eval_datum("ilk(gCore.get(#castload_manager))").await?;
    if castload != StaticDatum::Symbol("instance".into()) {
        return Err(format!("Expected castload_manager instance, got {:?}", castload));
    }
    snapshots.verify("preload", player.snapshot_stage())?;

    // Wait until the loading screen is fully drawn
    player.step_until_sprite_visible(30.0, "login_b_login_ok", 1.0).await?;
    player.step_until_sprite_visible(30.0, "corner_element", 1.0).await?;
    let loaded_count = player.eval_datum("gCore.get(#castload_manager).pLoadedCasts.count").await?
        .as_integer().unwrap_or(0);
    if loaded_count <= 2 {
        return Err(format!("Should have loaded more than 2 casts, got {}", loaded_count));
    }

    snapshots.verify("loaded_state", player.snapshot_stage())?;

    Ok(())
}

pub async fn test_habbo_login(
    player: &mut impl TestHarness,
    suite: &str,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let snapshots = SnapshotContext::new(suite, "login");

    // --- Login form ---
    player.click_member_prefix("login_name").await?;
    player.step_frames(2).await;
    player.type_text(username).await;

    player.click_member_prefix("login_password").await?;
    player.step_frames(2).await;
    player.type_text(password).await;

    snapshots.verify("login_filled", player.snapshot_stage())?;

    // Click login button
    player.click_member("login_b_login_ok").await?;
    player.step_until_sprite_visible(30.0, "entry_bar_ownhabbo_icon_image", 1.0).await?;
    snapshots.verify("login_submitted", player.snapshot_stage())?;

    Ok(())
}
