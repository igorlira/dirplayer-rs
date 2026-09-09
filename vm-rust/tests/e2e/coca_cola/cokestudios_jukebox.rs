use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{
    SnapshotContext, TestConfig, TestHarness, datum, log_test_action, sprite,
};

const CONFIG: &str = include_str!("../configs/cc_cokestudios.toml");

/// Coke Studios' in-studio Infinite Jukebox, and specifically what a burst of
/// taps does to its catalogue.
///
/// `BehaviorScript 177 - catalogdisplay` reads `the doubleClick` on every
/// mouseUp and sends the row either to `hiliteline` (single) or `lineclicked`
/// (double). `lineclicked` descends a level on the CLICK alone —
/// `pCatalogLevel` becomes `#songs` immediately, while `pContentlist` keeps the
/// artist strings until the Flash round trip comes back three frames later. So
/// one extra spurious double click descends two levels at once and the late
/// `getArtistsByGenre_result` renders artist STRINGS through the `#songs`
/// branch of `createimg`, whose `pContentlist[n].songName` then raises
/// "Invalid string built-in property songName".
///
/// The engine's part in that is `the doubleClick`: it has to pair clicks the
/// way the platform does — inside the double-click rectangle as well as the
/// time, and one pair at a time, so a third tap is a single click again. This
/// test taps one row three times and requires the catalogue to have gone down
/// exactly one level.
browser_e2e_test!(test_cokestudios_jukebox, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    cfg.apply_flash_config();
    let movie_path = player.asset_path(&cfg.movie.path);
    let snapshots = SnapshotContext::new(cfg.suite(), "cokestudios_jukebox");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // Boot: the shell has loaded its external casts and built its managers.
    // Reached with no backend at all, so a failure here is a player bug, not a
    // server one — same split `cokestudios.rs` makes.
    player
        .step_until(datum("ilk(oDenizenManager)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(120.0)
        .await?;
    player
        .step_until(datum("ilk(oRoom)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(180.0)
        .await
        .map_err(|e| format!("{e}\nNever entered the lobby (see cc_cokestudios.toml)."))?;
    player
        .step_until(datum("_movie.frame").equals(StaticDatum::Int(63)))
        .timeout(120.0)
        .await?;

    // Frame 63 is the in-room loop, but it is reached with the loading screen
    // still drawn and the navigator not yet built — `oRoom` exists a good while
    // earlier. Two waits, because the sprite and its CONTENT arrive separately:
    //
    //  * the room list sprite has to exist at all before it can be clicked. How
    //    long that takes is a property of the live backend, not of the player,
    //    so it is waited for rather than slept through. The 60 frames this
    //    replaces put the click before the sprite existed whenever the server
    //    was even slightly slow — "No sprite with member 'roomdisplay' found",
    //    with the console showing the click landing right after "--> logging
    //    in".
    //
    //  * ...and then the list has to be POPULATED. Waiting only for the sprite
    //    is not enough: measured, it appears 83 frames in, and clicking 20
    //    frames later hits an empty row, so the movie stays in the lobby and
    //    the `oIsoScene` wait below times out. 300 frames is what
    //    `cokestudios.rs` settles for before the same click, and that test
    //    enters London I reliably.
    player
        .step_until(sprite().member("roomdisplay").exists())
        .timeout(120.0)
        .await
        .map_err(|e| {
            format!("{e}\nThe navigator's room list never appeared after entering the lobby.")
        })?;
    player.step_frames(300).await;

    // Enter the first public studio (London I) — see `cokestudios.rs` for how
    // the "Go!" of row 1 maps onto the room list sprite.
    player
        .click_sprite_at(sprite().member("roomdisplay"), 241, 7)
        .await?;
    player
        .step_until(datum("ilk(oIsoScene)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(180.0)
        .await?;
    player
        .step_until(datum("_movie.frame").equals(StaticDatum::Int(90)))
        .timeout(120.0)
        .await?;
    player.step_frames(150).await;

    // What the toolbar's jukebox button does on mouseUp (`BehaviorScript 99 -
    // jukeboxbtn`): the FTM check comes back and its result opens the catalog
    // window, which asks the backend for the genres.
    player.eval("oDenizenManager.isFTMmember()").await?;
    player
        .step_until(datum("ilk(ElementMgr.oJukebox)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(60.0)
        .await?;

    let cl = "ElementMgr.oJukebox.getOpenWindow().pScrollingLists.cataloglist";
    player
        .step_until(datum(&format!("{cl}.pContentlist.count > 0")).equals(StaticDatum::Int(1)))
        .timeout(60.0)
        .await?;
    let genres = player.eval_datum(&format!("string({cl}.pContentlist)")).await?;
    log_test_action(&format!("Jukebox genres: {genres:?}"));
    player.step_frames(20).await;
    snapshots.verify_with_ratio("catalog_genres", player.snapshot_stage(), 0.35)?;

    // Three taps on the first row, back to back — a double click and then a
    // single one, not two double clicks.
    for _ in 0..3 {
        player.click_sprite_at(sprite().member("catalogdisplay"), 40, 5).await?;
        player.step_frames(2).await;
    }
    player.step_frames(60).await;

    let level = player.eval_datum(&format!("{cl}.pCatalogLevel")).await?;
    if level != StaticDatum::Symbol("Artists".into()) {
        return Err(format!(
            "three taps on one catalogue row left the jukebox at {level:?}; expected #Artists \
             (a spurious double click descended a second level, which renders the level \
             above's strings through createimg's #songs branch)"
        ));
    }
    let artists = player.eval_datum(&format!("string({cl}.pContentlist)")).await?;
    log_test_action(&format!("Jukebox artists: {artists:?}"));
    snapshots.verify_with_ratio("catalog_artists", player.snapshot_stage(), 0.35)?;
    if artists == genres {
        return Err("the artist list never replaced the genres".to_string());
    }

    // Two taps on DIFFERENT rows are two single clicks, not a double click, so
    // the level must not move at all.
    player.click_sprite_at(sprite().member("catalogdisplay"), 40, 5).await?;
    player.click_sprite_at(sprite().member("catalogdisplay"), 40, 47).await?;
    player.step_frames(60).await;
    let level = player.eval_datum(&format!("{cl}.pCatalogLevel")).await?;
    if level != StaticDatum::Symbol("Artists".into()) {
        return Err(format!(
            "two taps on different catalogue rows were taken as a double click \
             (level is {level:?}, expected #Artists)"
        ));
    }

    // Down to the songs, which is the level the reference art shows: the song
    // name at the left of each row and "mp3" right-aligned against a tab stop,
    // with the selected row inverted.
    player.click_sprite_at(sprite().member("catalogdisplay"), 40, 5).await?;
    player.step_frames(2).await;
    player.click_sprite_at(sprite().member("catalogdisplay"), 40, 5).await?;
    player
        .step_until(datum(&format!("{cl}.pCatalogLevel")).equals(StaticDatum::Symbol("songs".into())))
        .timeout(60.0)
        .await?;
    player.step_frames(90).await;
    let songs = player.eval_datum(&format!("string({cl}.pContentlist)")).await?;
    log_test_action(&format!("Jukebox songs: {songs:?}"));
    snapshots.verify_with_ratio("catalog_songs", player.snapshot_stage(), 0.35)?;

    // ...and a single tap to select one, which inverts the row.
    player.click_sprite_at(sprite().member("catalogdisplay"), 40, 5).await?;
    player.step_frames(30).await;
    snapshots.verify_with_ratio("catalog_songs_selected", player.snapshot_stage(), 0.35)?;

    // The playlist window (`BehaviorScript 179 - jukebox.playlist.btn`).
    player.eval("ElementMgr.oJukebox.openWindow(\"cc.infinite_jukebox.playlist.window\")").await?;
    player.step_frames(120).await;
    snapshots.verify_with_ratio("playlist", player.snapshot_stage(), 0.35)?;

    Ok(())
});
