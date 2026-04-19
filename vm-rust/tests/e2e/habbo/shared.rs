use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{sprite, datum, SnapshotContext, TestHarness};

pub async fn assert_entry(
    player: &mut impl TestHarness,
    suite: &str,
    movie_asset: &str,
) -> Result<(), String> {
    let movie_path = player.asset_path(movie_asset);
    let snapshots = SnapshotContext::new(suite, "entry");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // Wait for the boot sequence to initialize
    player.step_until(datum("sprite(1).member.name").equals(StaticDatum::String("Logo".into()))).timeout(10.0).await?;

    if player.get_global_ref("gCore").is_none() {
        return Err("gCore global should exist".into());
    }
    let castload = player.eval_datum("ilk(gCore.get(#castload_manager))").await?;
    if castload != StaticDatum::Symbol("instance".into()) {
        return Err(format!("Expected castload_manager instance, got {:?}", castload));
    }
    snapshots.verify("preload", player.snapshot_stage())?;

    // Wait until the loading screen is fully drawn
    player.step_until(sprite().member("login_b_login_ok").visible(1.0)).await?;
    player.step_until(sprite().member("corner_element").visible(1.0)).await?;
    let loaded_count = player.eval_datum("gCore.get(#castload_manager).pLoadedCasts.count").await?
        .as_integer().unwrap_or(0);
    if loaded_count <= 2 {
        return Err(format!("Should have loaded more than 2 casts, got {}", loaded_count));
    }

    snapshots.verify("loaded_state", player.snapshot_stage())?;

    Ok(())
}

pub async fn assert_login(
    player: &mut impl TestHarness,
    suite: &str,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let snapshots = SnapshotContext::new(suite, "login");

    // --- Login form ---
    player.click_sprite(sprite().member_prefix("login_name")).await?;
    player.step_frames(2).await;
    player.type_text(username).await;

    player.click_sprite(sprite().member_prefix("login_password")).await?;
    player.step_frames(2).await;
    player.type_text(password).await;

    snapshots.verify("login_filled", player.snapshot_stage())?;

    // Click login button
    player.click_sprite(sprite().member("login_b_login_ok")).await?;
    player.step_until(sprite().member("entry_bar_ownhabbo_icon_image").visible(0.5)).await?;
    snapshots.verify("login_submitted", player.snapshot_stage())?;

    Ok(())
}

pub async fn assert_navigate_pub(
    player: &mut impl TestHarness,
    suite: &str,
) -> Result<(), String> {
    let snapshots = SnapshotContext::new(suite, "navigation");

    player.step_until(sprite().member("Hotel Navigator_back").visible(1.0)).await?;

    snapshots.verify("navigator_opened", player.snapshot_stage())?;
    snapshots.verify(
        "navigator_public",
        player.snapshot_sprite(sprite().member("Hotel Navigator_back")).await?,
    )?;

    player.step_until(sprite().member("Hotel Navigator_nav_roomlist").visible(1.0)).await?;
    player.click_sprite_at(sprite().member("Hotel Navigator_nav_roomlist"), 100, 9).await?;
    player.step_until(sprite().member("Hotel Navigator_nav_go_button").visible(1.0)).await?;
    player.click_sprite(sprite().member("Hotel Navigator_nav_go_button")).await?;
    player.step_until(sprite().member_prefix("puppet_hilite_sh").visible(1.0)).await?;
    snapshots.verify("room_entered", player.snapshot_stage())?;
    Ok(())
}
