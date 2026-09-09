use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, SnapshotOutput, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_burnin_rubber3.toml");

/// Ink rows of a baked bitmap member, as "first..last" per contiguous band —
/// where the glyphs actually sit INSIDE the image the quad wears.
fn ink_bands(member_name: &str) -> String {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case(member_name) { continue; }
                let CastMemberType::Bitmap(b) = &m.member_type else { continue };
                let Some(bmp) = player.bitmap_manager.get_bitmap(b.image_ref) else { continue };
                let (w, h) = (bmp.width as usize, bmp.height as usize);
                let bpp = (bmp.bit_depth / 8).max(1) as usize;
                let mut bands: Vec<String> = Vec::new();
                let mut run: Option<usize> = None;
                for y in 0..h {
                    let mut ink = 0usize;
                    for x in 0..w {
                        let i = (y * w + x) * bpp;
                        if i + bpp > bmp.data.len() { break; }
                        let on = if bpp >= 4 { bmp.data[i + 3] > 32 } else { bmp.data[i] > 32 };
                        if on { ink += 1; }
                    }
                    match (ink > 1, run) {
                        (true, None) => run = Some(y),
                        (false, Some(st)) => { bands.push(format!("{}..{}", st, y - 1)); run = None; }
                        _ => {}
                    }
                }
                if let Some(st) = run { bands.push(format!("{}..{}", st, h - 1)); }
                return format!("{}x{} depth={} bands=[{}]", w, h, bmp.bit_depth, bands.join(" "));
            }
        }
        "<not found>".to_string()
    })
}

/// Alpha histogram of a bitmap member, in the same buckets
/// `classify_texture_alpha` uses to decide blended pass vs alpha-tested cutout.
fn alpha_stats(member_name: &str) -> String {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case(member_name) { continue; }
                let CastMemberType::Bitmap(b) = &m.member_type else { continue };
                let Some(bmp) = player.bitmap_manager.get_bitmap(b.image_ref) else { continue };
                let bpp = (bmp.bit_depth / 8).max(1) as usize;
                if bpp < 4 {
                    return format!("{}x{} depth={} (no alpha channel)", bmp.width, bmp.height, bmp.bit_depth);
                }
                let (mut clear, mut soft, mut opaque) = (0usize, 0usize, 0usize);
                for px in bmp.data.chunks(bpp) {
                    if px.len() < 4 { break; }
                    let a = px[3];
                    if a < 16 { clear += 1; } else if a < 240 { soft += 1; } else { opaque += 1; }
                }
                let total = clear + soft + opaque;
                let visible = soft + opaque;
                return format!(
                    "{}x{} total={} clear={} soft={} opaque={} soft/total={:.4} soft/visible={:.4}",
                    bmp.width, bmp.height, total, clear, soft, opaque,
                    if total > 0 { soft as f32 / total as f32 } else { 0.0 },
                    if visible > 0 { soft as f32 / visible as f32 } else { 0.0 },
                );
            }
        }
        "<not found>".to_string()
    })
}

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// Xform / Shockwave.com "Burnin' Rubber 3" (2009) — Shockwave 3D + Havok, D11.
// Reported as not working in dirplayer, and it wasn't: it opened on a black
// screen and nothing a player clicked did anything.
//
// The whole game is one Shockwave3D sprite in channel 1 driven by a data-driven
// event engine — `gData.Events` maps an event name to TAB-separated command
// lines ("CreateObject", "Create3DText", "CreateButton", "Event", …) that
// `[M] Event Manager` dispatches — and every screen is 3D: the logo is its own
// cast member composited onto the sprite with `addCamera`, and each menu is
// extruded 3D text plus 3D button models inside the "Main" member.
//
// Boot needs two things the shipped .dcr expects from its environment, both
// configured in `misc_burnin_rubber3.toml` (see the notes there): the
// `_runMode = "Projector"` external param, and `startup_do = "gSucces = 1"`
// standing in for the BR3Preloader's shockwave.com token check.
//
// Five engine defects were in the way, each fixed alongside this test:
//
//  1. `.ilk` on a 3D object answered VOID — only the `ilk()` FUNCTION form
//     worked. `[PS] Burnin3 Menu` dispatches every press through
//     `p.PreviousButton.ilk = #model`, so no 3D button in the game could be
//     clicked: the START button took its rollover and swallowed the click.
//  2. `userData` was not an interned built-in symbol, so the 3D cast member's
//     property getter raised on it before reaching any match arm, and the group
//     / camera / light getters had no arm for it at all. `Create3DText` opens
//     every glyph with `if tmember.userData <> VOID` and closes every text block
//     with `tGroup.userData.addProp(#modelList, …)`, so the first line of 3D
//     text aborted — taking the entire menu system with it.
//  3. `list.duplicate()` deep-copied the OBJECTS a list held instead of sharing
//     them (Director copies nested LISTS and nothing else). `Interpolator` runs
//     each frame over `gSystem.InterPolatorList.duplicate()` and moves a node
//     with `entry[1].interpolateTo(entry[2], pct)`, where `entry[1]` is the
//     node's own transform — so every frame interpolated a throwaway and the six
//     main-menu lines stayed parked at the x = -1200 offset `AnimateMainIn` had
//     pushed them to, off-screen.
//  4. `keyframePlayer.queue(motion)` never started the motion when the playList
//     was empty, though the dictionary says a queued motion runs as soon as
//     nothing is ahead of it. `PlayAllAnimation` drives the whole logo intro
//     that way and never calls `play()`, so nothing in it moved.
//  5. The renderer evaluated motions AFTER building the view matrix, so an
//     animated CAMERA — the logo camera is parented under
//     "Dummy Animation Node Logo_Camera" and flown by "Logo_Camera-Key" — used
//     the previous pass's motions and sat at its bind pose, pointing away from
//     the logo.
//
// NB the blue Shockwave splash a player sees first is NOT part of this movie: it
// is a Flash asset in `BR3Preloader.dcr`, the entry point this test bypasses
// because the same movie runs the dead validateToken.jsp check.
browser_e2e_test!(test_misc_burnin_rubber3_menu, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    cfg.apply_startup_do();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player
                .eval_datum($expr)
                .await
                .map(|d| format!("{:?}", d))
                .unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    let snapshots = SnapshotContext::new("misc", "burnin_rubber3");

    // ---- Boot ---------------------------------------------------------------
    // `startMovie` fires `event("GameInitialization")`, which loads the event and
    // data tables from the Internal cast's text members, streams `Data/Cars.cct`
    // and `Data/Shared.cct` in through `[PS] Preload Casts`, and ends on
    // `BurninRubber3()` setting `gSystem.Init = 1`.
    player.step_frames(60).await;
    probe(format!(
        "boot frame={} runMode={} path={} init={} events={}",
        player.current_frame(),
        ask!("string(_player.runMode)"),
        ask!("string(_movie.path)"),
        ask!("string(gSystem.Init)"),
        ask!("string(count(gData.Events))"),
    ));
    if ask!("string(gSystem.Init)") != "String(\"1\")" {
        return Err(format!(
            "the engine never initialised (gSystem.Init = {}, gGame.succes = {}). `startMovie` \
             bounces to shockwave.com unless `gSucces` is set, and `prepareMovie` wipes the \
             globals AND erases the Internal/Menu casts for every runMode but \"Projector\".",
            ask!("string(gSystem.Init)"),
            ask!("string(gGame.succes)"),
        )
        .into());
    }
    // The content casts the menu needs are pulled over the network by
    // `[PS] Preload Casts` and linked into the placeholder libs.
    for lib in ["Internal", "Menu", "Shared", "Cars"] {
        let n = ask!(&format!("string(_movie.castLib(\"{}\").member.count)", lib));
        probe(format!("castLib {} members={}", lib, n));
        if n == "String(\"0\")" {
            return Err(format!("castLib {} came up empty at boot", lib).into());
        }
    }

    probe(format!(
        "Menu cast: count={} findEmpty={} m13={} m14={} m15={} m16={} m17={}",
        ask!("string(castLib(\"Menu\").member.count)"),
        ask!("string(castLib(\"Menu\").findEmpty())"),
        ask!("string(member(13, \"Menu\").name)"),
        ask!("string(member(14, \"Menu\").name)"),
        ask!("string(member(15, \"Menu\").name)"),
        ask!("string(member(16, \"Menu\").name)"),
        ask!("string(member(17, \"Menu\").name)"),
    ));

    // ---- The logo screen ----------------------------------------------------
    // `LogoInitialization` → `PlayLogo` (500 ms) → `ShowStartButton` (2500 ms) is
    // a timeout chain, so it needs wall-clock time and not just frames. The logo
    // itself lives in its OWN cast member, composited onto the menu sprite by
    // `AddCameraToSprite {:Sprite} [#member: "Logo", #camera: "Logo_Camera"]`,
    // and `PlayAllAnimation` flies that camera on a queued keyframe motion.
    player.step_frames(400).await;
    probe(format!(
        "logo: menu={} logoInit={} models={} cameras={} sprites={}",
        ask!("string(gGame.CurrentMenu)"),
        ask!("string(gGame.LogoInit)"),
        ask!("string(member(\"Logo\").model.count)"),
        sprite_cameras(),
        sprite_summary(),
    ));
    if !sprite_cameras().contains("Logo_Camera@Logo") {
        return Err(format!(
            "the Logo member's camera was never composited onto the menu sprite: {}",
            sprite_cameras()
        )
        .into());
    }
    let logo = player.snapshot_stage();
    let logo_colors = distinct_colors(&logo);
    probe(format!("logo colours={}", logo_colors));
    // The 2D artwork on this screen is only the grey "START" and the footer
    // caption. Anything under ~16 distinct colours means the LOGO — a lit,
    // fire-wreathed 3D model — did not draw, which is what a frozen keyframe
    // camera (defects 4 and 5) looked like.
    if logo_colors < 16 {
        return Err(format!(
            "the logo screen rendered {} distinct colours — the 3D logo behind the START \
             button never drew. Its camera is parented under \
             \"Dummy Animation Node Logo_Camera\" and flown by the queued \
             \"Logo_Camera-Key\" motion.",
            logo_colors
        )
        .into());
    }
    {
        // The logo intro is ALIVE: each letter runs its own queued "<name>-Key"
        // motion, so a second of wall clock moves a good fraction of the stage.
        // This is what caught `keyframePlayer.queue()` never starting a motion —
        // with that broken the whole screen was frozen (and black).
        let a = player.snapshot_stage();
        player.step_frames(45).await;
        let b = player.snapshot_stage();
        let d = region_diff(&a, &b);
        probe(format!("logo motion by region: {}", d));
        let whole: f64 = d
            .split("whole=")
            .nth(1)
            .and_then(|s| s.trim_end_matches('%').parse().ok())
            .unwrap_or(0.0);
        if whole < 2.0 {
            return Err(format!(
                "the logo intro is frozen — only {:.1}% of the stage changed over 45 frames ({})",
                whole, d
            )
            .into());
        }
    }
    let _ = snapshots.verify("01_logo", logo);

    // ---- START → the main menu ---------------------------------------------
    // `CreateButton Main [#model: "StartButton", #event: "Main"]`. The press has
    // to land ON the model: `[PS] Burnin3 Menu`'s `enterFrame` hit-tests with
    // `camera.modelsUnderLoc(_mouse.mouseLoc)`.
    let hit = find_model(&player, "StartButton").await;
    probe(format!("StartButton hit at {:?}", hit));
    let Some((bx, by)) = hit else {
        return Err(
            "the StartButton model is under no pixel of either menu camera — the logo \
             screen's only control never rendered"
                .to_string()
                .into(),
        );
    };
    player.mouse_move(bx, by).await;
    player.step_frames(10).await;
    let rollover =
        ask!("string(member(\"Main\").model(\"StartButton\").shader.textureList[1].name)");
    probe(format!("rollover texture={}", rollover));
    if !rollover.contains("Rollover") {
        return Err(format!(
            "hovering the START button did not swap its texture ({}) — `[PS] Burnin3 Menu`'s \
             `ButtonEnter` never saw the model",
            rollover
        )
        .into());
    }

    // NOT `click()`: the behaviour latches the press across two frames —
    // `_mouse.mouseDown` in one `enterFrame` arms `p.mouseDown`, and only a later
    // frame seeing `_mouse.mouseUp` runs `ButtonUp` → `NormalButtonUp`, which
    // fires the button's event. A press and release inside one frame is invisible
    // to it.
    player.mouse_down(bx, by).await;
    player.step_frames(4).await;
    player.mouse_up(bx, by).await;
    player.step_frames(90).await;
    probe(format!(
        "after START: menu={} scene={}/{} mainInit={} sprites={}",
        ask!("string(gGame.CurrentMenu)"),
        ask!("string(gGame.scene)"),
        ask!("string(gGame.subscene)"),
        ask!("string(gGame.MainInit)"),
        sprite_summary(),
    ));
    if !ask!("string(gGame.CurrentMenu)").contains("Main") {
        return Err(format!(
            "clicking START never reached the main menu — gGame.CurrentMenu is still {}. The \
             button's `#event: \"Main\"` is fired by `NormalButtonUp`, which `ButtonUp` only \
             reaches through `p.PreviousButton.ilk = #model`.",
            ask!("string(gGame.CurrentMenu)")
        )
        .into());
    }

    // `SetupMain` lays the menu out as 3D text: `Create3DText` makes one extruded
    // glyph MODEL per character from the member's own per-character
    // modelResources and groups them per line, with a glow twin. `AnimateMainIn`
    // then parks each line 1200 units to the LEFT and queues one `DelayCommand`
    // per line (200-700 ms apart) handing it to `Interpolator` to slide back in.
    let lines = ["main menu", "new domination", "options"];
    for line in lines {
        let children = ask!(&format!(
            "string(member(\"Main\").group(\"{}\").child.count)",
            line
        ));
        probe(format!(
            "line {:?}: children={} userData keys={}",
            line,
            children,
            ask!(&format!(
                "string(count(member(\"Main\").group(\"{}\").userData))",
                line
            )),
        ));
        if children == "String(\"0\")" || children.starts_with("<err") {
            return Err(format!(
                "the menu line {:?} has no glyph models ({}) — `Create3DText` aborted before \
                 laying it out",
                line, children
            )
            .into());
        }
    }
    // Give the slide-in timeouts their wall-clock time.
    for i in 0..6 {
        player.step_frames(60).await;
        probe(format!(
            "slide t{}: mainMenu={} options={}",
            i,
            ask!("string(member(\"Main\").group(\"main menu\").worldPosition)"),
            ask!("string(member(\"Main\").group(\"options\").worldPosition)"),
        ));
    }
    for line in lines {
        let x = ask!(&format!(
            "string(member(\"Main\").group(\"{}\").worldPosition)",
            line
        ));
        if x.contains("(-1200") {
            return Err(format!(
                "the menu line {:?} never slid in — it is still at the -1200 offset \
                 `AnimateMainIn` parked it at ({}). `Interpolator` walks \
                 `gSystem.InterPolatorList.duplicate()` and moves the node through the \
                 transform OBJECT that list holds, so `duplicate()` must share it.",
                line, x
            )
            .into());
        }
    }

    let menu = player.snapshot_stage();
    let menu_colors = distinct_colors(&menu);
    probe(format!("main menu colours={}", menu_colors));
    if menu_colors < 16 {
        return Err(format!(
            "the main menu rendered {} distinct colours — the 3D text never drew",
            menu_colors
        )
        .into());
    }
    // Which parts of the main menu are ALIVE? The 3D text is static once it has
    // slid in, the muzzle flash animates on its own, and the car turns on its
    // own keyframe motion — so a region-wise diff over a second says which of
    // those the renderer is actually driving.
    let a = player.snapshot_stage();
    player.step_frames(45).await;
    let b = player.snapshot_stage();
    // KNOWN GAP, measured here rather than asserted: this comes out all-zeros.
    // The menu car orbits because `SetupMain` runs
    // `AddToMimic Car [#object: "DefaultView", #target: "Car_Camera"]`, and the
    // `Mimic` handler copies `Car_Camera.getWorldTransform()` onto the sprite's
    // camera every frame. "Car_Camera" is driven by the Car member's keyframe
    // motion — the RENDERER has it (`motion_transforms["Car_Camera"]`) — but
    // Lingo's `getWorldTransform()` answers the static parsed pose, so the mimic
    // copies a constant and the camera stands still. Measured:
    // `Car_Camera.getWorldTransform().position` is (232.8156, 289.8636,
    // 157.4659) at t0 and unchanged 45 frames later. Fixing that getter to
    // report the animated pose is what turns this into a real assertion.
    probe(format!("menu motion by region: {}", region_diff(&a, &b)));
    let _ = snapshots.verify("02_main_menu", menu);

    // ---- NEW DOMINATION → the name entry → track selection -----------------
    // The menu lines are TEXT buttons: `[PS] Burnin3 Menu` matches the hit
    // model's PARENT group against `gGame.menu.textButtons`, so clicking any
    // glyph of the line works.
    probe(format!("textButtons: {}", ask!("string(gGame.menu.textButtons.getPropAt(1))")));
    if !press_model(&mut player, "new domination_").await {
        return Err("could not find a glyph of the \"new domination\" line to click".into());
    }
    player.step_frames(120).await;
    probe(format!(
        "after NEW DOMINATION: menu={} domInit={} scripts={} nextBtn={}",
        ask!("string(gGame.CurrentMenu)"),
        ask!("string(gGame.DomInit)"),
        ask!("string(count(gGame.scriptList))"),
        ask!("string(gGame.menu.buttons[\"NextButton\"].event)"),
    ));
    let _ = snapshots.verify("03_new_domination", player.snapshot_stage());
    // The name field is a model of its own, hit-tested by `[PS] Burnin3 Enter
    // Name` with `member("Main").camera("3D_Camera").modelUnderLoc(...)`. Typing
    // a name and pressing RETURN is what reveals the NEXT button.
    if !press_model(&mut player, "EnterNameText").await {
        return Err("the name field (\"EnterNameText\") was not clickable".into());
    }
    player.step_frames(20).await;
    for ch in ["t", "e", "s", "t", "e", "r"] {
        player.key_press(ch, 0).await;
        player.step_frames(2).await;
    }
    // RETURN commits it: `keyDown`'s `case tKey of ENTER, RETURN: me.checkNames()`.
    player.key_press("Enter", 13).await;
    player.step_frames(60).await;
    probe(format!(
        "name entry: playerName={} enterNameMode={} nextBtn={}",
        ask!("string(gGame.PlayerName)"),
        ask!("string(gGame.EnterName.p.EnterName)"),
        ask!("string(member(\"Main\").model(\"NextButton\").visibility)"),
    ));
    // [probe3] The name box's frame. Black_Material is Kd 0 / Ke 0 but Ks WHITE
    // (Menu_6_Main.mtl), so the capture's thin light rim can only be a specular on
    // bevelled geometry — or a separate model. List what actually hangs under it.
    probe(format!(
        "EnterName children={} | {} | {} | {}",
        ask!("string(member(\"Main\").model(\"EnterName\").child.count)"),
        ask!("string(member(\"Main\").model(\"EnterName\").child[1].name)"),
        ask!("string(member(\"Main\").model(\"EnterName\").child[2].name)"),
        ask!("string(member(\"Main\").model(\"EnterName\").child[3].name)"),
    ));
    probe(format!(
        "EnterName mesh: faces={} shininess={} specular={} light0={} lightCount={}",
        ask!("string(member(\"Main\").model(\"EnterName\").resource.face.count)"),
        ask!("string(member(\"Main\").model(\"EnterName\").shaderList[1].shininess)"),
        ask!("string(member(\"Main\").model(\"EnterName\").shaderList[1].specular)"),
        ask!("string(member(\"Main\").light[1].name)"),
        ask!("string(member(\"Main\").light.count)"),
    ));

    if !ask!("string(gGame.PlayerName)").contains("TESTER") {
        return Err(format!(
            "the typed name never committed — gGame.PlayerName is {}. `checkNames` runs from              `keyDown`'s ENTER/RETURN arm and only sets it when the field holds a name.",
            ask!("string(gGame.PlayerName)")
        )
        .into());
    }

    if !press_model(&mut player, "NextButton").await {
        probe(format!("models on screen: {}", visible_models(&player).await));
        return Err("NEXT was not clickable on the name-entry screen".into());
    }
    player.step_frames(240).await;
    probe(format!(
        "after NEXT: menu={} scene={}/{} tsInit={} cameras={}",
        ask!("string(gGame.CurrentMenu)"),
        ask!("string(gGame.scene)"),
        ask!("string(gGame.subscene)"),
        ask!("string(gGame.TrackSelectionInit)"),
        sprite_cameras(),
    ));
    let _ = snapshots.verify("04_track_selection", player.snapshot_stage());

    // ---- Continent → location → challenge → car select ---------------------
    // `BuildContinentButtons` makes one button MODEL per continent, named after
    // the continent itself, and `BuildLocationButtons` then drops a marker model
    // per unlocked track onto the globe.
    let continent = unquote(&ask!("string(gGame.ContinentsList.getPropAt(1))"));
    probe(format!(
        "continents: first={} states={} buttons={}",
        continent,
        ask!("string(gGame.ContinentsList[1].state)"),
        ask!("string(count(gGame.menu.buttons))"),
    ));
    if !press_model(&mut player, &continent).await {
        return Err(format!("the continent button {:?} was not clickable", continent).into());
    }
    player.step_frames(120).await;
    let location = unquote(&ask!(
        "string(gGame.ContinentsList[symbol(gGame.selectedContinent)].locations.getPropAt(1))"
    ));
    probe(format!(
        "after continent: selected={} firstLocation={} markers={}",
        ask!("string(gGame.selectedContinent)"),
        location,
        ask!("string(count(gGame.currentMarkers.buttons))"),
    ));
    let _ = snapshots.verify("05_continent", player.snapshot_stage());

    if !press_model(&mut player, &location).await {
        return Err(format!("the track marker {:?} was not clickable", location).into());
    }
    player.step_frames(120).await;
    probe(format!(
        "after location: loc={} challenges={} track={}",
        ask!("string(gGame.selectedLocation)"),
        ask!("string(count(gGame.menu.buttons))"),
        ask!("string(gGame.TrackName)"),
    ));
    let _ = snapshots.verify("06_challenges", player.snapshot_stage());

    // ---- Challenge → car select → the race ---------------------------------
    // `BuildChallengeButtons` names them "Challenge<n>Button"; the first is the
    // plain RACE. Its `#handler: "SelectChallenge"` fires `ChallengeSelected`,
    // which is what reveals NEXT (→ CarSelection).
    if !press_model(&mut player, "Challenge1Button").await {
        return Err("the first challenge button was not clickable".into());
    }
    player.step_frames(90).await;
    probe(format!(
        "after challenge: mode={} track={} pressed={} nextEvent={}",
        ask!("string(gGame.GameMode)"),
        ask!("string(gGame.TrackName)"),
        ask!("string(gGame.PressedButton)"),
        ask!("string(gGame.menu.buttons[\"NextButton\"].event)"),
    ));

    // [probe3] "PRIZE MONEY" (256x32 quad at z -405) and "$ 10,000" (256x64 at
    // z -420) overlap on screen. Their extents overlap by design, so Director must
    // put the ink lower inside the 64-tall image than we do — measure where ours is.
    for t in ["PrizeMoneyTag_Texture", "PrizeMoney_Texture", "Unlock1_Texture", "TrackInfo_Texture"] {
        probe(format!("panel ink {} = {}", t, ink_bands(t)));
    }

    if !press_model(&mut player, "NextButton").await {
        return Err("NEXT was not clickable on the challenge screen".into());
    }
    for i in 0..12 {
        player.step_frames(60).await;
        if ask!("string(gGame.CurrentMenu)").contains("CarSelection") {
            break;
        }
        probe(format!(
            "waiting for car select t{}: menu={} downloaded={}",
            i,
            ask!("string(gGame.CurrentMenu)"),
            ask!("string(gGame.carsDownloaded)"),
        ));
    }
    // [timing] The player reports a ~30 s freeze ON the car-selection screen and
    // an almost instant NEXT, which is the opposite of the capture (car select is
    // immediate, the load happens under the card after NEXT). `PreloadTrackData`
    // is explicitly ASYNC in the movie — `[PS] Preload Casts` is added to
    // `_movie.actorList` and polls `netDone`/`getStreamStatus` from `stepFrame` —
    // so the download should overlap the menu, not block it. Time both phases and
    // read the movie's own download flags.
    probe(format!(
        "download flags at car select: carsDownloaded={} trackDownloaded={} dataDownloaded={}          trackHolder={} generalHolder={} actorList={}",
        ask!("string(gGame.carsDownloaded)"),
        ask!("string(gGame.TrackDownloaded)"),
        ask!("string(gGame.DataDownloaded)"),
        ask!("string(ilk(gGame.TrackHolder))"),
        ask!("string(ilk(gGame.generalHolder))"),
        ask!("string(count(_movie.actorList))"),
    ));
    probe(format!(
        "car select: menu={} car={} cameras={}",
        ask!("string(gGame.CurrentMenu)"),
        ask!("string(gGame.Car)"),
        sprite_cameras(),
    ));
    let _ = snapshots.verify("07_car_selection", player.snapshot_stage());

    // ---- NEXT → the race ----------------------------------------------------
    // NEXT is `CheckForWeapons` → `PreStartChallenge` → `TrackInitialization`,
    // which streams `Data/Tracks/<World>/<World>.cct`, runs the track's build
    // events and spawns the cars. Each car build opens with
    // `t.sound[#Mixer] = CreateMixer()` (`new(#Mixer)`) and hangs its engine,
    // tyre and skid loops off it as sound objects.
    if !press_model(&mut player, "NextButton").await {
        return Err("NEXT was not clickable on the car-selection screen".into());
    }
    let mut racing = false;
    for i in 0..40 {
        player.step_frames(60).await;
        if ask!("string(gGame.RaceState)").contains("Racing") {
            racing = true;
            break;
        }
        if i % 5 == 0 {
            probe(format!(
                "loading t{}: scene={} menu={} state={} cars={} mixers={}",
                i,
                ask!("string(gGame.scene)"),
                ask!("string(gGame.CurrentMenu)"),
                ask!("string(gGame.RaceState)"),
                ask!("string(count(gGame.cars))"),
                ask!("string(count(gSound.Mixer))"),
            ));
        }
    }
    probe(format!(
        "race: state={} input={} cars={} mixers={} objects={}",
        ask!("string(gGame.RaceState)"),
        ask!("string(gGame.input)"),
        ask!("string(count(gGame.cars))"),
        ask!("string(count(gSound.Mixer))"),
        ask!("string(count(gGame.cars[1].sound.Mixer.getSoundObjectList()))"),
    ));
    probe(format!(
        "car model: name={} member={} shaders={} shaderName={}",
        ask!("string(gGame.cars[1].car.name)"),
        ask!("string(gGame.cars[1].car.parent.name)"),
        ask!("string(gGame.cars[1].car.shaderList.count)"),
        ask!("string(gGame.cars[1].car.shader.name)"),
    ));
    probe(format!(
        "race member: sprite1={} carShaders=[{} | {} | {} | {}]",
        ask!("string(sprite(1).member.name)"),
        ask!("string(gGame.cars[1].car.shaderList[1].name)"),
        ask!("string(gGame.cars[1].car.shaderList[2].name)"),
        ask!("string(gGame.cars[1].car.shaderList[3].name)"),
        ask!("string(gGame.cars[1].car.shaderList[4].name)"),
    ));
    probe(format!(
        "car shader1: texVoid={} texName={} texType={} texW={} texH={} diffuse={} blend={}",
        ask!("string(voidp(gGame.cars[1].car.shaderList[1].textureList[1]))"),
        ask!("string(gGame.cars[1].car.shaderList[1].textureList[1].name)"),
        ask!("string(gGame.cars[1].car.shaderList[1].textureList[1].type)"),
        ask!("string(gGame.cars[1].car.shaderList[1].textureList[1].width)"),
        ask!("string(gGame.cars[1].car.shaderList[1].textureList[1].height)"),
        ask!("string(gGame.cars[1].car.shaderList[1].diffuse)"),
        ask!("string(gGame.cars[1].car.shaderList[1].blend)"),
    ));
    // [probe3] The scoreboard plates draw flat black where the capture blends a
    // gradient. The plate art is black with an alpha ramp, so the question is
    // whether classify_texture_alpha files this atlas as a cutout.
    probe(format!("atlas alpha Interface_Texture = {}", alpha_stats("Interface_Texture")));
    probe(format!("atlas alpha Weapons_Texture = {}", alpha_stats("Weapons_Texture")));

    probe(format!(
        "race member textures: count={} names={}",
        ask!("string(sprite(1).member.texture.count)"),
        ask!("string(GetStringList(0, sprite(1).member.name, [#type: #texture, #string: \"*.*\"]))"),
    ));
    for m in ["player1BMWSkin1_Texture", "BMWSkin1_Texture", "CityStreet_Texture"] {
        probe(format!(
            "member {}: void={} type={} w={} h={} depth={}",
            m,
            ask!(&format!("string(voidp(member(\"{}\")))", m)),
            ask!(&format!("string(member(\"{}\").type)", m)),
            ask!(&format!("string(member(\"{}\").width)", m)),
            ask!(&format!("string(member(\"{}\").height)", m)),
            ask!(&format!("string(member(\"{}\").depth)", m)),
        ));
    }
    let _ = snapshots.verify("08_race", player.snapshot_stage());
    if !racing {
        return Err(format!(
            "the race never started — gGame.RaceState is {}, scene {}, {} cars built",
            ask!("string(gGame.RaceState)"),
            ask!("string(gGame.scene)"),
            ask!("string(count(gGame.cars))"),
        )
        .into());
    }

    Ok(())
});








/// Every on-stage sprite's channel, member name and placement.
fn sprite_summary() -> String {
    use vm_rust::player::reserve_player_ref;
    reserve_player_ref(|p| {
        let mut out = Vec::new();
        for ch in 1..=150i16 {
            let sprite = &p.movie.score.get_channel(ch).sprite;
            if !sprite.visible || sprite.member.is_none() || sprite.loc_v < 0 {
                continue;
            }
            let name = sprite
                .member
                .as_ref()
                .and_then(|m| p.movie.cast_manager.find_member_by_ref(m))
                .map(|m| m.name.clone())
                .unwrap_or_default();
            out.push(format!(
                "{}:{}@({},{} {}x{})",
                ch, name, sprite.loc_h, sprite.loc_v, sprite.width, sprite.height
            ));
        }
        out.join(" ")
    })
}

/// Sprite 1's Shockwave3D camera list as the RENDERER sees it: each entry's
/// camera name and the cast member that OWNS it. BR3 composites its logo world
/// onto the menu sprite by adding a camera from a DIFFERENT member, so the owner
/// is the interesting half.
fn sprite_cameras() -> String {
    use vm_rust::player::reserve_player_ref;
    reserve_player_ref(|p| {
        let sprite = &p.movie.score.get_channel(1).sprite;
        let mut out = Vec::new();
        for c in sprite.w3d_camera.iter().chain(sprite.w3d_cameras.iter()) {
            let owner = match c.member {
                Some((lib, num)) => p
                    .movie
                    .cast_manager
                    .find_member_by_ref(&vm_rust::player::cast_lib::CastMemberRef {
                        cast_lib: lib,
                        cast_member: num,
                    })
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| format!("{}:{}", lib, num)),
                None => "<own>".to_string(),
            };
            out.push(format!("{}@{}", c.name, owner));
        }
        format!("[{}]", out.join(" "))
    })
}

/// Where on the stage does `model` answer `modelsUnderLoc`?
///
/// `[PS] Burnin3 Menu`'s `enterFrame` hit-tests the menu with
/// `camera.modelsUnderLoc(_mouse.mouseLoc)` over the "3D_Camera" / "2D_Camera"
/// pair it was handed, so a click only registers ON the model — which is why this
/// asks the cameras the same question instead of hardcoding pixels.
///
/// `worldSpaceToSpriteSpace` alone is not enough: it answers the model's PIVOT,
/// and for these extruded text buttons that sits below the glyphs. So it is used
/// only as a SEED, and the search sweeps a window around it.
async fn find_model<T: TestHarness>(player: &T, model: &str) -> Option<(i32, i32)> {
    find_target(
        player,
        &format!("member(\"Main\").model(\"{}\").worldPosition", model),
        model,
        80,
    )
    .await
}

/// The same search for a 3D TEXT line, which is a GROUP of one model per glyph:
/// seed on the group and match the glyph naming `<line>_<char><n>`.
async fn find_text_button<T: TestHarness>(player: &T, line: &str) -> Option<(i32, i32)> {
    find_target(
        player,
        &format!("member(\"Main\").group(\"{}\").worldPosition", line),
        &format!("{}_", line),
        220,
    )
    .await
}


/// The middle of the contiguous run of pixels around `(x, y)` that still hit a
/// model named `prefix*` under `cam`.
///
/// Aiming at a button's centre matters because the menu's buttons are
/// alpha-tested (see `find_target`): the edge of the quad is transparent and
/// correctly ignored.
async fn centre_of_hit<T: TestHarness>(
    player: &T,
    cam: &str,
    prefix: &str,
    x: i32,
    y: i32,
) -> (i32, i32) {
    async fn hits<T: TestHarness>(player: &T, cam: &str, prefix: &str, x: i32, y: i32) -> bool {
        if !(0..640).contains(&x) || !(0..480).contains(&y) {
            return false;
        }
        let name = player
            .eval_datum(&format!(
                "string(member(\"Main\").camera(\"{}\").modelsUnderLoc(point({}, {}),                  [#maxNumberOfModels: 2, #levelOfDetail: #detailed])[1].model.name)",
                cam, x, y
            ))
            .await
            .map(|d| format!("{:?}", d))
            .unwrap_or_default();
        name.trim_start_matches("String(\"").starts_with(prefix)
    }
    // Cap the walk so a full-stage backdrop model cannot turn this into a
    // thousand evals.
    const LIMIT: i32 = 160;
    let (mut lo, mut hi) = (x, x);
    while x - lo < LIMIT && hits(player, cam, prefix, lo - 2, y).await {
        lo -= 2;
    }
    while hi - x < LIMIT && hits(player, cam, prefix, hi + 2, y).await {
        hi += 2;
    }
    let cx = (lo + hi) / 2;
    let (mut top, mut bot) = (y, y);
    while y - top < LIMIT && hits(player, cam, prefix, cx, top - 2).await {
        top -= 2;
    }
    while bot - y < LIMIT && hits(player, cam, prefix, cx, bot + 2).await {
        bot += 2;
    }
    let cy = (top + bot) / 2;
    // The middle of the quad is not always where the artwork is: the footer's
    // NEXT sits at the right-hand end of a wide strip whose centre is fully
    // transparent, and a click there is correctly ignored. Walk the run for a
    // point the movie's own alpha gate accepts.
    if alpha_opaque_at(player, cam, cx, cy).await {
        return (cx, cy);
    }
    let mut best = (cx, cy);
    let mut off = 4;
    while off <= (hi - lo) / 2 + 4 {
        for cand in [cx + off, cx - off] {
            if hits(player, cam, prefix, cand, cy).await
                && alpha_opaque_at(player, cam, cand, cy).await
            {
                return (cand, cy);
            }
            if hits(player, cam, prefix, cand, cy).await {
                best = (cand, cy);
            }
        }
        off += 4;
    }
    best
}

/// Sweep a window around `seed_expr`'s projection for a pixel where the topmost
/// model's name starts with `prefix`.
async fn find_target<T: TestHarness>(
    player: &T,
    seed_expr: &str,
    prefix: &str,
    radius: i32,
) -> Option<(i32, i32)> {
    // First point that hit the model at all, used only if no opaque one turns up.
    let mut fallback: Option<(i32, i32)> = None;
    for cam in ["2D_Camera", "3D_Camera"] {
        let seed = player
            .eval_datum(&format!(
                "string(member(\"Main\").camera(\"{}\").worldSpaceToSpriteSpace({}))",
                cam, seed_expr
            ))
            .await
            .map(|d| format!("{:?}", d))
            .unwrap_or_default();
        // A seed that projects out of view still tells us nothing useful, so fall
        // back to sweeping from the middle of the stage.
        let (sx, sy) = parse_point(&seed).unwrap_or((320, 240));
        let step = if radius > 120 { 8 } else { 5 };
        let mut dy = -radius;
        while dy <= radius {
            let mut dx = -radius;
            while dx <= radius {
                let (x, y) = (sx + dx, sy + dy);
                dx += step;
                if !(0..640).contains(&x) || !(0..480).contains(&y) {
                    continue;
                }
                let name = player
                    .eval_datum(&format!(
                        "string(member(\"Main\").camera(\"{}\").modelsUnderLoc(point({}, {}),                          [#maxNumberOfModels: 2, #levelOfDetail: #detailed])[1].model.name)",
                        cam, x, y
                    ))
                    .await
                    .map(|d| format!("{:?}", d))
                    .unwrap_or_default();
                // `name` arrives as `String("new domination_n1")`.
                if name.trim_start_matches("String(\"").starts_with(prefix) {
                    // A button created with `#alpha: TRUE` is only live where its
                    // TEXTURE is opaque: `[PS] Burnin3 Menu` gates the hit on
                    // `GetAlphaPixel(tModelList[1]) <> color(0)`, which samples
                    // `shader.textureList[1].member.image` at the hit UV. START's
                    // strip is 256x32 with the glyphs in the middle third and
                    // fully transparent edges, and the sweep's FIRST hit is
                    // always a corner of the quad — a point Director rejects too.
                    // Walk to the middle of the model's footprint, which is where
                    // a player aims and where the artwork actually is.
                    fallback.get_or_insert((x, y));
                    return Some(centre_of_hit(player, cam, prefix, x, y).await);
                }
            }
            dy += step;
        }
    }
    // The seed is the model's PIVOT and the menu cameras interpolate between
    // screens, so a window around it can miss entirely. Fall back to a coarse
    // sweep of the whole stage.
    for cam in ["2D_Camera", "3D_Camera"] {
        let mut y = 6;
        while y < 480 {
            let mut x = 6;
            while x < 640 {
                let name = player
                    .eval_datum(&format!(
                        "string(member(\"Main\").camera(\"{}\").modelsUnderLoc(point({}, {}),                          [#maxNumberOfModels: 2, #levelOfDetail: #detailed])[1].model.name)",
                        cam, x, y
                    ))
                    .await
                    .map(|d| format!("{:?}", d))
                    .unwrap_or_default();
                if name.trim_start_matches("String(\"").starts_with(prefix) {
                    return Some(centre_of_hit(player, cam, prefix, x, y).await);
                }
                x += 10;
            }
            y += 10;
        }
    }
    fallback
}

/// Press the first model whose name starts with `prefix` (a trailing `_` means
/// "a glyph of the 3D TEXT line named by the rest"), the way a player does:
/// hover it so `[PS] Burnin3 Menu` latches it, then hold the button down across a
/// frame boundary so its `enterFrame` sees `_mouse.mouseDown` and then
/// `_mouse.mouseUp`.
async fn press_model<T: TestHarness>(player: &mut T, prefix: &str) -> bool {
    // Menus arrive on staggered `DelayCommand` timeouts (the globe drops its
    // track markers -66 + i*100 ms apart) and the cameras interpolate between
    // screens, so a control can simply not be on screen yet. Retry rather than
    // race it.
    let mut hit = None;
    for _ in 0..4 {
        hit = if prefix.ends_with('_') {
            find_text_button(player, prefix.trim_end_matches('_')).await
        } else {
            find_model(player, prefix).await
        };
        if hit.is_some() {
            break;
        }
        player.step_frames(60).await;
    }
    let Some((x, y)) = hit else {
        return false;
    };

    // SWEEP the pointer there in small steps rather than teleporting, because
    // one of these screens depends on the intermediate positions.
    //
    // `[PS] Burnin3 Enter Name` latches the name field in `p.button` and clears
    // it ONLY on a frame where `camera("3D_Camera").modelUnderLoc(_mouse.mouseLoc)`
    // hits some model that is not the field:
    //
    //     if tmodel <> VOID then
    //       if tmodel.name = "EnterNameText" then p.button = tmodel
    //       else p.button = VOID
    //
    // Measured against that camera at x=300, the only geometry between the name
    // bar and the footer buttons is a 9px "EnterName" plate band:
    //     244..252 EnterName | 253..283 EnterNameText | 284..292 EnterName | 293+ nothing
    // (Director agrees — `modelUnderLoc` answers VOID for that camera all the way
    // down to the NEXT button, verified in its message window.) A pointer that
    // jumps from the field straight to a button never lands in the band, so the
    // latch survives, and the next press runs `EnterName.ButtonDown` instead of
    // the menu's — re-opening name entry and detaching Back/Next.
    {
        let (sx, sy) = (240, 253);
        let steps = ((x - sx).abs().max((y - sy).abs()) / 3).max(1);
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let mx = sx + ((x - sx) as f32 * t) as i32;
            let my = sy + ((y - sy) as f32 * t) as i32;
            player.mouse_move(mx, my).await;
            player.step_frames(1).await;
        }
    }
    player.mouse_move(x, y).await;
    player.step_frames(6).await;
    probe(format!("pressing {} at ({}, {})", prefix, x, y));

    // NOT `click()`: `[PS] Burnin3 Menu` latches the press across two frames —
    // `_mouse.mouseDown` in one `enterFrame` arms `p.mouseDown`, and only a later
    // frame seeing `_mouse.mouseUp` runs `ButtonUp` → the button's event.
    player.mouse_down(x, y).await;
    player.step_frames(4).await;
    player.mouse_up(x, y).await;
    player.step_frames(6).await;
    true
}

/// `String("America")` → `America`.
fn unquote(s: &str) -> String {
    s.trim()
        .trim_start_matches("String(")
        .trim_end_matches(')')
        .trim_matches('"')
        .to_string()
}

/// Distinct model names the menu cameras can see, on a coarse sweep — a
/// "what IS on screen" dump for when a control cannot be found.
async fn visible_models<T: TestHarness>(player: &T) -> String {
    let mut seen: Vec<String> = Vec::new();
    for cam in ["2D_Camera", "3D_Camera"] {
        let mut y = 10;
        while y < 480 {
            let mut x = 10;
            while x < 640 {
                let name = player
                    .eval_datum(&format!(
                        "string(member(\"Main\").camera(\"{}\").modelsUnderLoc(point({}, {}),                          [#maxNumberOfModels: 1, #levelOfDetail: #detailed])[1].model.name)",
                        cam, x, y
                    ))
                    .await
                    .map(|d| format!("{:?}", d))
                    .unwrap_or_default();
                if name.starts_with("String(") && !name.contains("\"\")") {
                    let n = format!("{}:{}", cam, name);
                    if !seen.contains(&n) {
                        seen.push(n);
                    }
                }
                x += 20;
            }
            y += 20;
        }
    }
    seen.join(" ")
}

/// `point(409, 241)` → (409, 241).
/// Whether `[PS] Burnin3 Menu` would accept a click at `(x, y)` under `cam`.
///
/// A button whose `gGame.menu.buttons[name].alpha` is 1 is live only where its
/// TEXTURE is opaque — `enterFrame` gates the hit on
/// `GetAlphaPixel(tModelList[1]) <> color(0)`. This is that same test. It cannot
/// call the movie's handler: `on GetAlphaPixel iSectList` declares no `me`, so a
/// method call binds the RECEIVER to `iSectList` and the intersection list is
/// dropped — the handler then answers VOID for anything.
///
/// The arithmetic is the handler's, verbatim: the face's three texture
/// coordinates become pixel positions in the source image, and #uvCoord's
/// BARYCENTRIC pair reconstructs the hit point between them.
async fn alpha_opaque_at<T: TestHarness>(player: &T, cam: &str, x: i32, y: i32) -> bool {
    let isect = format!(
        "member(\"Main\").camera(\"{}\").modelsUnderLoc(point({}, {}),          [#maxNumberOfModels: 2, #levelOfDetail: #detailed])[1]",
        cam, x, y
    );
    let ask = |e: String| async move {
        player
            .eval_datum(&e)
            .await
            .map(|d| format!("{:?}", d))
            .unwrap_or_default()
    };
    let name = unquote(&ask(format!("string({}.model.name)", isect)).await);
    if name.is_empty() {
        return false;
    }
    let model = format!("member(\"Main\").model(\"{}\")", name);
    // An untextured button (no `.member` behind its texture) has no alpha to
    // test; the movie only reaches GetAlphaPixel for `#alpha: 1` buttons.
    let img = format!("{}.shader.textureList[1].member.image", model);
    let w: f64 = unquote(&ask(format!("string({}.width)", img)).await).parse().unwrap_or(0.0);
    let h: f64 = unquote(&ask(format!("string({}.height)", img)).await).parse().unwrap_or(0.0);
    if w <= 0.0 || h <= 0.0 {
        return true;
    }
    let mesh_id = unquote(&ask(format!("string({}.meshID)", isect)).await);
    let face_id = unquote(&ask(format!("string({}.faceID)", isect)).await);
    let uv = parse_floats(&ask(format!("string({}.uvCoord)", isect)).await);
    let face = parse_floats(
        &ask(format!("string({}.meshDeform.mesh[{}].face[{}])", model, mesh_id, face_id)).await,
    );
    let coords = parse_floats(
        &ask(format!("string({}.meshDeform.mesh[{}].textureCoordinateList)", model, mesh_id)).await,
    );
    if uv.len() < 2 || face.len() < 3 || coords.len() < 6 {
        return true;
    }
    // textureCoordinateList is a flat run of [u, v] pairs; face holds 1-based
    // indices into it.
    let loc = |i: f64| -> Option<(f64, f64)> {
        let k = (i as usize).checked_sub(1)? * 2;
        Some((coords.get(k)? * w, (1.0 - coords.get(k + 1)?) * h))
    };
    let (Some(a), Some(b), Some(c)) = (loc(face[0]), loc(face[1]), loc(face[2])) else {
        return true;
    };
    let px = a.0 + (b.0 - a.0) * uv[0] + (c.0 - a.0) * uv[1];
    let py = a.1 + (b.1 - a.1) * uv[0] + (c.1 - a.1) * uv[1];
    let alpha = ask(format!(
        "string({}.extractAlpha().getPixel(point({}, {})))",
        img, px as i32, py as i32
    ))
    .await;
    !alpha.contains("color(0)") && !alpha.contains("color( 0 )")
}

/// Every number in a Lingo list/proplist rendering, in order.
fn parse_floats(s: &str) -> Vec<f64> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter_map(|t| t.parse::<f64>().ok())
        .collect()
}

fn parse_point(s: &str) -> Option<(i32, i32)> {
    let inner = s.split("point(").nth(1)?;
    let inner = inner.split(')').next()?;
    let mut it = inner.split(',');
    let x = it.next()?.trim().parse::<f64>().ok()? as i32;
    let y = it.next()?.trim().parse::<f64>().ok()? as i32;
    Some((x, y))
}

/// Percentage of pixels that differ between two stage snapshots, per region.
fn region_diff(a: &SnapshotOutput, b: &SnapshotOutput) -> String {
    let (Some(ia), Some(ib)) = (to_image(a), to_image(b)) else {
        return "<no image>".to_string();
    };
    let regions: [(&str, u32, u32, u32, u32); 6] = [
        ("carLeft", 40, 230, 240, 430),
        ("carMid", 240, 300, 440, 430),
        ("flame", 420, 60, 640, 260),
        ("textTop", 80, 160, 580, 240),
        ("textLow", 80, 260, 500, 400),
        ("whole", 0, 0, 640, 480),
    ];
    let mut out = Vec::new();
    for (name, x0, y0, x1, y1) in regions {
        let (mut diff, mut total) = (0u32, 0u32);
        for y in y0..y1.min(ia.height()) {
            for x in x0..x1.min(ia.width()) {
                total += 1;
                let pa = ia.get_pixel(x, y).0;
                let pb = ib.get_pixel(x, y).0;
                let d = (pa[0] as i32 - pb[0] as i32).abs()
                    + (pa[1] as i32 - pb[1] as i32).abs()
                    + (pa[2] as i32 - pb[2] as i32).abs();
                if d > 24 {
                    diff += 1;
                }
            }
        }
        out.push(format!("{}={:.1}%", name, 100.0 * diff as f64 / total.max(1) as f64));
    }
    out.join(" ")
}

fn to_image(snap: &SnapshotOutput) -> Option<image::RgbaImage> {
    match snap {
        SnapshotOutput::Base64Png(b64) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .ok()
                .and_then(|bytes| {
                    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()
                })
                .map(|i| i.to_rgba8())
        }
        SnapshotOutput::Rgba { width, height, data } => {
            image::RgbaImage::from_raw(*width, *height, data.clone())
        }
    }
}

/// How many DISTINCT colours the stage carries — the failure this guards is a
/// near-black stage, which no reference image would catch on its own since both
/// the logo and the menu animate continuously.
fn distinct_colors(snap: &SnapshotOutput) -> usize {
    let img = match snap {
        SnapshotOutput::Base64Png(b64) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .ok()
                .and_then(|bytes| {
                    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()
                })
                .map(|i| i.to_rgba8())
        }
        SnapshotOutput::Rgba { width, height, data } => {
            image::RgbaImage::from_raw(*width, *height, data.clone())
        }
    };
    let img = match img {
        Some(i) => i,
        None => return 0,
    };
    let mut seen: Vec<u32> = Vec::new();
    for p in img.pixels() {
        // Quantise to 5 bits/channel: an anti-aliased gradient must not count as
        // thousands of "colours", but real artwork still spreads widely.
        let k = ((p.0[0] as u32 >> 3) << 10) | ((p.0[1] as u32 >> 3) << 5) | (p.0[2] as u32 >> 3);
        if !seen.contains(&k) {
            seen.push(k);
            if seen.len() > 64 {
                break;
            }
        }
    }
    seen.len()
}
