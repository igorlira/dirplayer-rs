use vm_rust::browser_e2e_test;
use vm_rust::director::static_datum::StaticDatum;
use vm_rust::player::testing_shared::{SnapshotContext, TestConfig, TestHarness, datum, sprite};

const CONFIG: &str = include_str!("../configs/cc_cokestudios.toml");

/// Coca-Cola "Coke Studios" — the online client, so this covers rather more
/// than a movie boot: the shell pulls ~9 MB of external casts, checks
/// `/sf/status`, logs the account in over a Flash AMF `NetConnection`
/// (`/sf/gateway`, authenticating with the `sw1`/`sw2` external params), is
/// handed the address of a game server, opens a socket to it and enters the
/// lobby.
///
/// The two backend endpoints and the socket are wired up by `[flash]` in the
/// config, which `apply_flash_config` installs as `window.__dirplayerFlashConfig`
/// — the same object a host page hands `flashPlayerManager.ts` through
/// `DirPlayer.configureFlash`. The account and the addresses themselves live in
/// `.env` as `COKESTUDIOS_*`; the config only names the variables.
browser_e2e_test!(test_cokestudios_load, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    cfg.apply_flash_config();
    let movie_path = player.asset_path(&cfg.movie.path);
    let mut snapshots = SnapshotContext::new(cfg.suite(), "cokestudios");

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // Boot: the shell has loaded its external casts and built its managers.
    // Reached with no backend at all, so a failure here is a player bug.
    player
        .step_until(datum("ilk(oDenizenManager)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(120.0)
        .await?;

    // The whole online chain in one condition: `oRoom` is only constructed
    // once the client has authenticated, connected to the game server it was
    // handed and been let into a room.
    player
        .step_until(datum("ilk(oRoom)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(180.0)
        .await
        .map_err(|e| {
            format!(
                "{e}\nNever entered the lobby. This test needs the backend reachable: \
                 an HTTP server answering /sf/status + /sf/gateway, and a WebSocket-to-TCP \
                 proxy in front of the game server, at the addresses the COKESTUDIOS_* \
                 variables in .env point to (see cc_cokestudios.toml and .env.example)."
            )
        })?;

    // ...and then the shell reaches frame 63, its in-room loop — `oRoom`
    // exists a good while earlier (frame 20), while the "Entering Lobby"
    // screen is still up.
    player
        .step_until(datum("_movie.frame").equals(StaticDatum::Int(63)))
        .timeout(120.0)
        .await?;
    // Frame 63 is reached with the loading screen still drawn; give the room
    // and the navigator window time to paint before the snapshot.
    player.step_frames(300).await;

    // Session state the lobby implies, asserted separately so a regression
    // says which half broke.
    assert_eq!(
        player.eval_datum("ilk(oSession)").await?,
        StaticDatum::Symbol("instance".into()),
        "session object missing after entering the lobby"
    );

    // The stage here is a live multiplayer room — other people's avatars walk
    // through it — so this snapshot is recorded for the report rather than
    // compared tightly.
    // The stage here is a live multiplayer room — other people's avatars walk
    // through it — so this snapshot is recorded for the report rather than
    // compared tightly.
    snapshots.verify_with_ratio("lobby", player.snapshot_stage(), 0.25)?;

    // Enter the first public studio (London I) by clicking the "Go!" of the
    // list's first row. `BehaviorScript 29 - roomdisplay` maps the click itself:
    //
    //   row       = (mouseV - sprite.top) / (sprite.height / pDisplayLines) + pScrollIndex
    //   bGoStudio = mouseH >= sprite.right - 20
    //
    // so "Go!" of row 1 is the top-right corner of the list sprite — anywhere
    // in the last 20px of the row. pDisplayLines is 7 over a 98px sprite, so
    // row 1 is the first 14 rows of pixels.
    player
        .click_sprite_at(sprite().member("roomdisplay"), 241, 7)
        .await?;

    // `oIsoScene` is the isometric room engine — it does not exist in the
    // lobby, so this is "a studio has been handed over and built". Reaching it
    // covers the room handshake, the studio's XML scene, its furniture and the
    // avatar update the server sends for us.
    player
        .step_until(datum("ilk(oIsoScene)").equals(StaticDatum::Symbol("instance".into())))
        .timeout(180.0)
        .await?;
    // ...and frame 90 is the in-room loop, the way 63 is the lobby's.
    player
        .step_until(datum("_movie.frame").equals(StaticDatum::Int(90)))
        .timeout(120.0)
        .await?;
    player.step_frames(150).await;

    snapshots.verify_with_ratio("studio", player.snapshot_stage(), 0.35)?;

    // --- Chat ---
    // `BehaviorScript 70 - chatinput` puts the whole protocol in one handler:
    // RETURN/ENTER hands the field's text to `sendChat`, which asks the
    // speech-switch sprite for the mode (`sendAllSprites(#getSpeechMode)` ->
    // #speak / #shout, toggled on its mouseDown) and calls
    // `oStudio.sendStudioChat(text, mode)`.
    //
    // The bubbles are NOT drawn locally — the server echoes the line back and
    // `ChatRenderer` composes it into `ochat.pOffscreenImg`, which grows by one
    // bubble each time. Waiting on that height is therefore a wait on the whole
    // round trip, and it is what these two steps assert.
    player.click_sprite(sprite().member("inputchat")).await?;
    player.step_frames(2).await;
    player.type_text("speak: testyjgpq").await;
    player.key_press("Enter", 13).await;
    player
        .step_until(datum("ochat.pOffscreenImg.height > 0").equals(StaticDatum::Int(1)))
        .timeout(60.0)
        .await?;
    let after_speak = player.eval_datum("ochat.pOffscreenImg.height").await?;

    // The switch flips #speak <-> #shout on mouseDown, and its member name
    // follows the mode (`cc.speechswitch.<mode>`), so match on the prefix.
    player
        .click_sprite(sprite().member_prefix("cc.speechswitch"))
        .await?;
    player.step_frames(2).await;
    player.click_sprite(sprite().member("inputchat")).await?;
    player.step_frames(2).await;
    player.type_text("shout: testyjgpq").await;
    player.key_press("Enter", 13).await;
    let speak_h = match after_speak {
        StaticDatum::Int(h) => h,
        other => return Err(format!("chat offscreen height was {other:?}, expected an Int")),
    };
    player
        .step_until(
            datum(&format!("ochat.pOffscreenImg.height > {speak_h}"))
                .equals(StaticDatum::Int(1)),
        )
        .timeout(60.0)
        .await?;
    // Let the composed image reach the on-stage sprite.
    player.step_frames(20).await;

    // Both bubbles up: the speaker name in bold and the message beside it.
    // `ChatRenderer` sizes each half with `charPosToLoc(len + 1).locH` and crops
    // the rendered image to exactly that, so this is also the regression test
    // for the bold measurement — a short one clipped the name to "Dreamcatch".
    snapshots.verify_with_ratio("chat", player.snapshot_stage(), 0.35)?;

    Ok(())
});
