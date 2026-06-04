use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{sprite, datum, SnapshotContext, TestHarness};

pub async fn assert_entry(
    player: &mut impl TestHarness,
    suite: &str,
    test_name: &str,
    movie_asset: &str,
    login_window: bool,
) -> Result<(), String> {
    let movie_path = player.asset_path(movie_asset);
    let mut snapshots = SnapshotContext::new(suite, test_name);
    snapshots.max_diff_ratio = 0.003;

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // Wait for the boot sequence to initialize
    player.step_until(sprite().member("Logo").visible(1.0)).await?;

    if player.get_global_ref("gCore").is_none() {
        return Err("gCore global should exist".into());
    }
    let castload = player.eval_datum("ilk(gCore.get(#castload_manager))").await?;
    if castload != StaticDatum::Symbol("instance".into()) {
        return Err(format!("Expected castload_manager instance, got {:?}", castload));
    }
    snapshots.verify("preload", player.snapshot_stage())?;

    // Wait until the loading screen is fully drawn
    if login_window {
        player.step_until(sprite().member("login_b_login_ok").visible(1.0)).await?;
    }
    player.step_until(sprite().member("corner_element").visible(1.0)).await?;
    let loaded_count = player.eval_datum("gCore.get(#castload_manager).pLoadedCasts.count").await?
        .as_integer().unwrap_or(0);
    if loaded_count <= 2 {
        return Err(format!("Should have loaded more than 2 casts, got {}", loaded_count));
    }

    snapshots.verify("loaded_state", player.snapshot_stage())?;

    Ok(())
}

pub async fn assert_login_success(
    player: &mut impl TestHarness,
    suite: &str,
    test_name: &str,
) -> Result<(), String> {
    let mut snapshots = SnapshotContext::new(suite, &test_name);
    snapshots.max_diff_ratio = 0.01;

    player.step_until(sprite().member("entry_bar_ownhabbo_icon_image").visible(1.0)).timeout(150.0).await?;
    snapshots.verify("login_submitted", player.snapshot_stage())?;
    Ok(())
}

pub async fn assert_login(
    player: &mut impl TestHarness,
    suite: &str,
    test_name: &str,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let mut snapshots = SnapshotContext::new(suite, test_name);
    snapshots.max_diff_ratio = 0.01;

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
    assert_login_success(player, suite, test_name).await?;

    Ok(())
}

pub async fn assert_navigator_visible(
    player: &mut impl TestHarness,
    suite: &str,
    test_name: &str,
) -> Result<(), String> {
    if let Err(_) = player.step_until(sprite().member("Hotel Navigator_back").visible(1.0)).timeout(5.0).await {
        // Try clicking the navigator button if it didn't appear within the timeout
        if let Err(_) = player.click_sprite(sprite().member("entry_bar_nav_icon_image")).await {
            if let Err(_) = player.click_sprite(sprite().member("Room_bar_int_nav_image")).await {
                player.click_sprite(sprite().member("RoomBarID_int_nav_image")).await?;
            }
        }
        player.step_until(sprite().member("Hotel Navigator_back").visible(1.0)).await?;
    }

    Ok(())
}

pub async fn assert_navigate_pub(
    player: &mut impl TestHarness,
    suite: &str,
    test_name: &str,
    movie_asset: &str,
) -> Result<(), String> {
    let snapshots = SnapshotContext::new(suite, test_name);

    assert_navigator_visible(player, suite, test_name).await?;
    player.click_sprite(sprite().member("Hotel Navigator_nav_tb_publicRooms")).await?;
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

pub async fn assert_navigate_private(
    player: &mut impl TestHarness,
    suite: &str,
    test_name: &str,
    movie_asset: &str,
) -> Result<(), String> {
    let snapshots = SnapshotContext::new(suite, test_name);

    assert_navigator_visible(player, suite, test_name).await?;
    player.click_sprite(sprite().member("Hotel Navigator_nav_tb_guestRooms")).await?;
    snapshots.verify(
        "navigator_private",
        player.snapshot_sprite(sprite().member("Hotel Navigator_back")).await?,
    )?;

    player.step_until(sprite().member("Hotel Navigator_nav_tab_own").visible(1.0)).await?;
    player.click_sprite(sprite().member("Hotel Navigator_nav_tab_own")).await?;

    player.step_until(sprite().member("Hotel Navigator_nav_roomlist").visible(1.0)).await?;
    player.step_frames(100).await; // Wait for the room list to populate
    player.click_sprite_at(sprite().member("Hotel Navigator_nav_roomlist"), 100, 9).await?;
    player.step_until(sprite().member("Hotel Navigator_nav_go_button").visible(1.0)).await?;
    player.click_sprite(sprite().member("Hotel Navigator_nav_go_button")).await?;
    player.step_frames(300).await; // Wait to navigate away from previous room

    player.step_until(sprite().member_prefix("puppet_hilite_").visible(0.9)).await?;
    player.step_frames(300).await; // Wait for furniture to load
    snapshots.verify("private_room_entered", player.snapshot_stage())?;
    Ok(())
}
