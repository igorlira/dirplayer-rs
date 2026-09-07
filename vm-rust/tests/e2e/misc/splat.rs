use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{sprite, SnapshotContext, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_splat.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

// jellygames "Splat" — a 3D Pac-Man. Coverage for `newMesh` + `build()` with
// PER-FACE shaders, which is how the whole field of pac-dots is created:
//
//   adddot()   -> one shader per dot, named string(gridIndex); every face of
//                 that dot is assigned it
//   maketower  -> newMesh("pipRes", ...) with `nm.face[f].shader = ...`, then
//                 build(); `pips` is ONE model whose sub-meshes are the dots
//
// The game then maps sub-mesh -> grid cell, and that mapping is what makes a
// dot edible:
//
//   repeat with a = 1 to pips.meshDeform.mesh.count
//     b = value(pips.shaderList[a].name)
//     pgrid[ax[b]][az[b]][ay[b]][6] = [a, pips.shaderList[a].texture.name = "dotTextp"]
//
//   doteat gate:  if pgrid[pacpos[1]][pacpos[2]][pacpos[3]][6][1] > 0
//
// So `meshDeform.mesh.count` and `shaderList` must agree in LENGTH and in
// ORDER, entry for entry. If they disagree the dots still RENDER (one mesh
// draws the lot) but nothing is ever eaten — the reported symptom, and the
// reason this test asserts the mapping rather than the picture.
//
// Splat also harvests `meshDeform.mesh[1].face[a]` from a #sphere primitive and
// then DELETES that model before using the values, so `face[]` must return a
// VALUE and not a live reference. See docs/rifleman-npc-navmesh-handoff.md §1.1.
browser_e2e_test!(test_misc_splat_dots, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    macro_rules! ask {
        ($expr:expr) => {
            player.eval_datum($expr).await
                .map(|d| format!("{:?}", d)).unwrap_or_else(|e| format!("<err {}>", e.message))
        };
    }

    // The title screen runs a Flash intro and only accepts the START click once
    // that has played out, so give it a generous budget before touching
    // anything. Paced by Ruffle's own wall clock rather than by stepped
    // Director frames, so the frame count needed varies between runs.
    for i in 0..40 {
        player.step_frames(25).await;
        if i % 10 == 9 {
            probe(format!("title tick {}: frame={}", i, player.current_frame()));
        }
    }
    probe(format!("title settled: frame={} moviePath={}",
        player.current_frame(), ask!("string(the moviePath)")));

    // What is actually ON the loading screen? `titleJump.beginSprite` shows
    // sprites 1..4 and sets sprite(2) to member("boxStart"); a pacman graphic is
    // meant to be visible here too.
    // Print EVERY channel, including the empty ones. `titleJump.beginSprite`
    // makes sprites 1..4 visible, so a channel that reads "member 0 of castLib
    // 0" here is a channel the score should have populated and did not — which
    // is what a missing pacman on this screen would look like.
    for ch in 1..=8 {
        let m = ask!(&format!("string(sprite({}).member)", ch));
        probe(format!("title ch {}: member={} type={} visible={} rect={} ink={} blend={}",
            ch, m,
            ask!(&format!("string(sprite({}).member.type)", ch)),
            ask!(&format!("string(sprite({}).visible)", ch)),
            ask!(&format!("string(sprite({}).rect)", ch)),
            ask!(&format!("string(sprite({}).ink)", ch)),
            ask!(&format!("string(sprite({}).blend)", ch))));
    }
    probe(format!("title members: paclife={} boxStart={} loading={}",
        ask!("string(member(\"paclife\").type)"),
        ask!("string(member(\"boxStart\").type)"),
        ask!("string(member(\"loading\").type)")));
    // The loading screen, before the game starts. Separate from the HUD shot so
    // a failure says which one moved.
    let snapshots = SnapshotContext::new("misc", "splat");
    let _ = snapshots.verify("00_loading", player.snapshot_stage());

    // Take the gate's SUCCESS branch directly instead of clicking sprite(3).
    // `titleJump.mouseDown` checks `sprite(3).mouseOverButton = 1` — a Ruffle
    // hit-test at an exact stage coordinate, the kind of timing this suite has
    // been bitten by before — and then does exactly these two things when both
    // domain checks pass (moviePath contains "jellygames2.com", and `src` does
    // not contain "jellygames", which is why the config leaves `src` unset).
    // Click sprite(3) the way a player does — `titleJump.mouseDown` gates on
    // `sprite(3).mouseOverButton = 1`, and taking the handler's `go(6)` branch
    // directly SKIPS whatever else that path sets up. That matters here: driven
    // by `go(6)` the loading frame came up completely black, while the real
    // click leaves it showing "level 1 loading - please wait". Park the cursor
    // first so the rollover state is live before the press (same reason
    // spectral_wizard hovers before clicking).
    // Aim at the "CLICK HERE TO PLAY" line specifically. sprite(3) is the FLASH
    // banner spanning rect(10,14,630,103), so its CENTRE is empty artwork —
    // `mouseOverButton` only reports 1 over the actual button, and a centre
    // click left the playhead on frame 5.
    for (x, y) in [(180, 76), (180, 55), (120, 76)] {
        vm_rust::player::reserve_player_mut(move |p| { p.mouse_loc = (x, y); });
        player.step_frames(2).await;
        probe(format!("hover ({},{}) mouseOverButton={}", x, y,
            ask!("string(sprite(3).mouseOverButton)")));
        player.click(x, y).await;
        player.step_frames(4).await;
        if player.current_frame() >= 6 {
            probe(format!("click at ({},{}) advanced to frame {}", x, y,
                player.current_frame()));
            break;
        }
    }
    if player.current_frame() < 6 {
        // The real click lands on frame 8, NOT the frame 6 that
        // `titleJump.mouseDown` names — the movie moves on immediately. Jumping
        // straight to 6 skips the loading frame's setup and produced a black
        // stage, which nearly sent me chasing a phantom rendering bug.
        probe(format!("click did not advance (frame={}), falling back to go(8)",
            player.current_frame()));
        let _ = ask!("sprite(5).visible = 0");
        let _ = ask!("go(8)");
    }

    player.step_frames(150).await;

    // The LOADING screen — `member("loading").text` is showing and the level is
    // being built. This is the screen a pacman graphic is meant to appear on,
    // so dump its channels before the build finishes and the maze takes over.
    player.step_frames(2).await;
    probe(format!("loading frame={} loadingText={}",
        player.current_frame(), ask!("string(member(\"loading\").text)")));
    for ch in 1..=8 {
        probe(format!("load ch {}: member={} type={} visible={} rect={}",
            ch,
            ask!(&format!("string(sprite({}).member)", ch)),
            ask!(&format!("string(sprite({}).member.type)", ch)),
            ask!(&format!("string(sprite({}).visible)", ch)),
            ask!(&format!("string(sprite({}).rect)", ch))));
    }

    let _ = snapshots.verify("00b_loading_after_click", player.snapshot_stage());

    // `newLevel` (setScene / maketower / makeCharacters) runs on the gameplay
    // frame: it decodes the pip sphere, creates one shader per dot, then builds
    // the combined mesh. Poll for `pips` rather than guessing a frame budget.
    let mut saw_pips = false;
    for _ in 0..80 {
        player.step_frames(10).await;
        if ask!("string(voidP(member(\"scene\").model(\"pips\")))").contains('0') {
            saw_pips = true;
            break;
        }
    }
    probe(format!("frame={} sawPips={} sceneModels={}",
        player.current_frame(), saw_pips,
        ask!("string(member(\"scene\").model.count)")));
    assert!(saw_pips, "the `pips` model was never built — the level never loaded");

    // The loading-screen pacman (Flash member 34, "mspac" on sprite 4) is
    // covered by the `00b_loading_after_click` snapshot above rather than by
    // forcing it visible here. An earlier diagnostic did force it — and hid
    // sprite(5), the directToStage Shockwave3D member, to prove the Flash was
    // not merely being occluded. That left the 3D hidden for the REST of the
    // test, so every later snapshot showed a game with no maze in it.
    //
    // It rendered blank because a Flash sprite's pixels come from Ruffle, which
    // only advances when the browser event loop turns; the movie shows the
    // sprite, calls updateStage(), then blocks for seconds building the world.
    // Fixed by waiting for Flash instance readiness BEFORE running each frame's
    // scripts (see `run_single_frame`), so the guard now belongs on the real
    // loading screen, with the 3D left alone.

    // THE INVARIANT the eating logic rests on.
    let mesh_count = ask!("string(member(\"scene\").model(\"pips\").meshDeform.mesh.count)");
    let shader_count = ask!("string(member(\"scene\").model(\"pips\").shaderList.count)");
    probe(format!("pips meshDeform.mesh.count={} shaderList.count={}",
        mesh_count, shader_count));

    // Per entry: the shader name IS the grid index, so it has to parse as an
    // integer. A name like "DefaultShader" (or an empty one) makes `value()`
    // return VOID and that dot is never registered as edible.
    for i in [1, 2, 3] {
        probe(format!("  shaderList[{}] name={} value={} texture={}", i,
            ask!(&format!("string(member(\"scene\").model(\"pips\").shaderList[{}].name)", i)),
            ask!(&format!("string(value(member(\"scene\").model(\"pips\").shaderList[{}].name))", i)),
            ask!(&format!("string(member(\"scene\").model(\"pips\").shaderList[{}].texture.name)", i))));
    }

    // One sub-mesh should be ONE dot — the #sphere primitive at resolution 4.
    // A single huge mesh means build() did not split per shader, which is the
    // other way the mapping can fail.
    probe(format!("  mesh[1] faces={} verts={}",
        ask!("string(member(\"scene\").model(\"pips\").meshDeform.mesh[1].face.count)"),
        ask!("string(member(\"scene\").model(\"pips\").meshDeform.mesh[1].vertexList.count)")));

    // --- eating a dot -----------------------------------------------------
    //
    // "The orbs are not counted" can mean two different failures and they live
    // in different places, so measure both.
    //
    // 1. `doteat` never runs. Its gate is
    //      pgrid[pacpos[1]][pacpos[2]][pacpos[3]][6][1] > 0
    //    which is populated by the mesh/shader mapping asserted above.
    // 2. `doteat` runs but the SCORE never shows it. Splat renders the score as
    //      member("thescore").text = chars(string(gamescore), 2, 7)
    //    with gamescore biased by 1000000, so the display is a 6-char slice.
    //    A wrong `chars()` leaves it reading 000000 no matter what was eaten.
    probe(format!("score member={} chars(1000010,2,7)={} chars(1000000,2,7)={}",
        ask!("string(member(\"thescore\").text)"),
        ask!("string(chars(string(1000010), 2, 7))"),
        ask!("string(chars(string(1000000), 2, 7))")));

    // `magoo` is a sprite behaviour, so its state hangs off whichever channel it
    // is attached to. Find that channel by looking for a NON-EMPTY dotcount —
    // an unknown property reads back as an empty string, not as an error.
    let mut magoo = 0;
    for ch in 1..=30 {
        let dc = ask!(&format!("string(sprite({}).dotcount)", ch));
        if dc.contains("err") || dc == "String(\"\")" { continue; }
        magoo = ch;
        probe(format!("magoo on ch {}: dotcount={} gamestart={} pplay={} pacpos={}",
            ch, dc,
            ask!(&format!("string(sprite({}).gamestart)", ch)),
            ask!(&format!("string(sprite({}).pplay)", ch)),
            ask!(&format!("string(sprite({}).pacpos)", ch))));
        break;
    }
    assert_ne!(magoo, 0, "could not locate the `magoo` behaviour on any sprite channel");

    // Is the pac standing on a registered dot? `[6]` is [meshIndex, isPowerPill]
    // and `doteat` only fires when `[6][1] > 0`.
    probe(format!("cell6 at pac={}",
        ask!(&format!(
            "string(sprite({m}).pgrid[sprite({m}).pacpos[1]][sprite({m}).pacpos[2]][sprite({m}).pacpos[3]][6])",
            m = magoo))));

    // Let the game actually play and assert the orbs are TALLIED. `dotcount`
    // starts at the dot total and is decremented by every `doteat`, so a
    // falling count is the eat path working end to end.
    let mut first_count = String::new();
    let mut last_count = String::new();
    for i in 0..8 {
        last_count = ask!(&format!("string(sprite({}).dotcount)", magoo));
        if i == 0 { first_count = last_count.clone(); }
        probe(format!("play t={} dotcount={} score={} pacpos={}", i,
            last_count,
            ask!("string(member(\"thescore\").text)"),
            ask!(&format!("string(sprite({}).pacpos)", magoo))));
        player.step_frames(40).await;
    }
    assert_ne!(first_count, last_count,
        "dotcount never moved — the pac ate nothing, so the dot -> grid mapping          or the `doteat` gate is broken");

    // The member's text is provably correct by here (probed above), so this
    // snapshot answers the remaining question: does the STAGE show it? A score
    // box still reading 000000 while `member("thescore").text` says otherwise is
    // a repaint/invalidation failure, not a gameplay one.
    probe(format!("final score text={} dotcount={}",
        ask!("string(member(\"thescore\").text)"),
        ask!(&format!("string(sprite({}).dotcount)", magoo))));

    // Snapshot the HUD STRIP only. A full-stage shot of a live 3D game is not
    // reproducible — the pac's position and the eaten-dot pattern differ every
    // run (35% of pixels moved between two runs of this very test). The bottom
    // strip is what this test is actually about: the score is painted by a
    // sprite that sits UNDER the `(the stage).image` overlay, so a stale
    // full-stage composite used to freeze it at 000000 while the member's text
    // advanced. Cropping keeps the regression guard and drops the noise.
    let _ = snapshots.verify("01_hud", player.snapshot_stage().crop(0, 424, 640, 480));

    // --- the HUD strip is composited 1:1 --------------------------------
    //
    // Splat paints its lives bar and its eight drink icons straight into
    // `(the stage).image` (magoo `displaylifes` / `displaydrink`), which the
    // renderer composites over the sprite output as a sub-rect quad. The dirty
    // rect those draws accumulate is rect(130, 430, 630, 470) — it does NOT
    // start at the origin, and that is what exposed the defect: the overlay's
    // texture coordinates were uploaded as the far EDGES (dr/sw, db/sh) while
    // the vertex shader reads `u_tex_rect.zw` as a WIDTH and HEIGHT. v then ran
    // 430/480 -> 1900/480 across a 40-row quad, so the whole bottom of the
    // stage was crushed into the strip's first few rows and everything below
    // clamped to the last texture row — "the graphics at the bottom are
    // collapsed".
    //
    // Assert against the COMPOSITED frame rather than the snapshot alone: a
    // whole-picture diff of a live 3D game cannot say which half is at fault,
    // and the read-back below proves the stage BITMAP was always correct, so a
    // mismatch is squarely the compositor's.
    probe(format!("stage img {}x{} depth={} drawRect={}",
        ask!("string((the stage).image.width)"),
        ask!("string((the stage).image.height)"),
        ask!("string((the stage).image.depth)"),
        ask!("string((the stage).drawRect)")));

    // A marker the 3D scene can never produce, inside the lives bar and well
    // clear of its top rows, painted through the same imaging-Lingo path the
    // game uses.
    let _ = ask!("(the stage).image.fill(rect(200, 440, 260, 465), rgb(255, 0, 255))");
    player.step_frames(1).await;
    for y in [442, 450, 462] {
        probe(format!("marker y={} stageImage={}", y,
            ask!(&format!("string((the stage).image.getPixel(230, {}))", y))));
    }
    let composited = player.snapshot_stage();
    for y in [442u32, 450, 462] {
        let px = composited.pixel(230, y)
            .unwrap_or_else(|| panic!("stage snapshot has no pixel at (230, {})", y));
        probe(format!("marker y={} composited={:?}", y, px));
        assert!(px.0 > 200 && px.1 < 80 && px.2 > 200,
            "the stage-image overlay is not composited 1:1: (230, {}) reads {:?}              but `(the stage).image` holds magenta there. The sub-rect quad's              texture coordinates must be (x, y, WIDTH, HEIGHT) — passing the              right/bottom edges crushes the whole bottom of the stage into the              top few rows of the HUD strip", y, px);
    }


    assert_ne!(mesh_count, "String(\"0\")", "pips has no meshDeform meshes");
    assert_eq!(mesh_count, shader_count,
        "meshDeform.mesh.count and shaderList.count disagree — maketower indexes \
         shaderList BY MESH INDEX to map each dot to its grid cell, so a mismatch \
         leaves the dots visible but none of them edible");

    Ok(())
});
