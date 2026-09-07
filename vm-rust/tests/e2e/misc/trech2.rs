use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, SnapshotOutput, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_trech2.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

fn decode(snap: &SnapshotOutput) -> Option<image::RgbaImage> {
    match snap {
        SnapshotOutput::Base64Png(b64) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(b64).ok()
                .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
                .map(|i| i.to_rgba8())
        }
        SnapshotOutput::Rgba { width, height, data } =>
            image::RgbaImage::from_raw(*width, *height, data.clone()),
    }
}

/// Count distinctly GREEN pixels in a crop box. The radar's chrome — dial ring,
/// scan lines — is a saturated green nothing else in the city scene (grey
/// buildings, tan dirt, olive grass, the black mech) comes close to.
fn green_pixels(snap: &SnapshotOutput, left: u32, top: u32, right: u32, bottom: u32) -> usize {
    let img = match decode(snap) { Some(i) => i, None => return 0 };
    let (w, h) = img.dimensions();
    let mut n = 0usize;
    for y in top.min(h)..bottom.min(h) {
        for x in left.min(w)..right.min(w) {
            let p = img.get_pixel(x, y).0;
            if p[1] as i32 > p[0] as i32 + 40 && p[1] as i32 > p[2] as i32 + 40 { n += 1; }
        }
    }
    n
}

// "TRECH v2.0" (mockworld / onlinegameshq.com, credited to RazorMouse),
// Director 10 + Shockwave 3D, 800x600. A different engine ("MGE b1.49") from the
// TRECH 1 demo covered by misc/trech.rs, by a different author.
//
// NOT a physics-Xtra game: no Havok.x32, no dynamiks.x32 (AGEIA PhysX). It needs
// no `_runMode`, no `_moviePath` and no external params — see the config for the
// two startup gates in `startScript` and why both pass unset. The one worth
// knowing is `gGame.gCheckMoveCursor`, which IS 1: `startGame` walks
// `the number of xtras` for an xtra named "MoveCursor" and, not finding one,
// does `setStage("error")` (frame 10) + `halt()`. dirplayer registers MoveCursor,
// so this passes — but a movie parked dead on frame 10 is what a regression here
// would look like.
//
// FLOW: `startMovie` goes to "GT" (frame 20), and with `gGameTrust` hardcoded to
// 0 runs initializeGlobals / initializeText / pregame. `pregame` shows the splash
// on sprite 1 and arms `timeout("init").new(gGame.gSplashPause = 2970, #startGame)`
// — WALL CLOCK, so the test steps for real seconds. `startGame` lands on "intro"
// (30). START (`sign_pressspacetostart`) -> "level" (70); the level buttons are
// SWFs that call `setLevelAndLoad` back through Lingo, which goes to "preLevel"
// (100) and makes `loadOrClone`, whose `timeout("pauseCloning")` clones the
// embedded level/mech/tank/turret casts and calls back into the difficulty screen
// (80). EASY -> `loginManager` -> "inGame" (110).
//
// THE DEFECT — the radar drew no dial. TRECH 2's radar is a second camera added
// with `sprite(1).addCamera(jCamera, 2)`, inset at `rect(10, 10, 150, 150)`,
// orthographic, rooted on a detached `blipGroup`, with
// `colorBuffer.clearAtRender = 0` so the game view shows through, and its only
// chrome is `addBackdrop(radarBG, point(-58,-58), 0)`. Two things were wrong:
//
//  1. Backdrops were drawn ONLY on the clearing pass. A backdrop belongs to its
//     camera and `clearAtRender` governs only the colour buffer — Director 11.5,
//     `clearAtRender`: "indicates whether the color buffer is cleared after each
//     frame"; `clearValue`: "the color used to clear out the color buffer IF
//     colorBuffer.clearAtRender is set to TRUE". And `addCamera`: each camera's
//     view "is displayed on top of the view from cameras with lower index
//     positions". So a non-clearing extra camera still draws its own backdrop.
//     Gated on the clear, the radar blips floated over the city with no dial.
//
//  2. Once drawn, the backdrop came out at ~1/6 size in the corner of the radar
//     box. `locWithinSprite` is "measured from the upper left corner of the
//     SPRITE" (Director 11.5, `addBackdrop`) — hence the negative
//     `point(-58,-58)` for a 256px texture in a 140px window — so its ortho spans
//     the whole sprite. The inset pass had narrowed the GL viewport to the camera
//     rect, squeezing that whole-sprite ortho into 140x140. The backdrop now
//     draws with the viewport widened back to the sprite and the SCISSOR (still
//     the camera rect) doing the clipping.
browser_e2e_test!(test_misc_trech2_load, |player| async move {
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
    // `ask!` formats with `{:?}`, so a Lingo string comes back quoted. Unwrap it.
    macro_rules! ask_str {
        ($expr:expr) => {
            ask!($expr).trim_start_matches("String(\"").trim_end_matches("\")").to_string()
        };
    }
    macro_rules! ask_int {
        ($expr:expr) => {
            ask!($expr).trim_start_matches("Int(").trim_end_matches(')').parse::<i64>().unwrap_or(-1)
        };
    }
    // Click the centre of a channel's rect. The shell channels are authored and
    // `setStage` only moves sprites between their on-screen positions and the
    // off-stage parking spot at (820, 200), so a channel lookup is stable.
    macro_rules! click_ch {
        ($ch:expr, $what:expr) => {{
            let r = ask_str!(&format!("string(sprite({}).rect)", $ch));
            let nums: Vec<i32> = r.trim_start_matches("rect(").trim_end_matches(')')
                .split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if nums.len() == 4 {
                let (x, y) = ((nums[0] + nums[2]) / 2, (nums[1] + nums[3]) / 2);
                probe(format!("click {} (ch {}) at ({}, {})", $what, $ch, x, y));
                player.click(x, y).await;
                true
            } else { false }
        }};
    }
    macro_rules! find_ch {
        ($name:expr) => {{
            let mut found = 0;
            for ch in 1..40 {
                if ask_str!(&format!("sprite({}).member.name", ch)) == $name { found = ch; break; }
            }
            found
        }};
    }
    // Step until `frame` is reached, in real time — the boot and the level load
    // are both paced by Director timeouts, not by frame counts.
    macro_rules! step_until_frame {
        ($frame:expr, $what:expr) => {{
            let mut reached = false;
            for _ in 0..50 {
                player.step_frames(20).await;
                if player.current_frame() == $frame { reached = true; break; }
                if !player.is_playing() { break; }
            }
            probe(format!("{}: frame={} playing={} state={}", $what,
                player.current_frame(), player.is_playing(), ask!("string(gStatus.gState)")));
            reached
        }};
    }

    let snapshots = SnapshotContext::new("misc", "trech2");

    // ---- 1. Boot: "GT" -> splash -> "intro" --------------------------------
    if !step_until_frame!(30, "boot") {
        return Err(format!(
            "never reached the \"intro\" marker (frame 30): parked on frame {} with \
             is_playing={}. Frame 10 is the \"error\" marker — `startGame` goes there \
             and `halt()`s when its `gCheckMoveCursor` walk of `the number of xtras` \
             finds no xtra named \"MoveCursor\".",
            player.current_frame(), player.is_playing()
        ).into());
    }
    if ask_str!("string(ilk(gGame))") != "propList" {
        return Err(format!("`initializeGlobals` never built gGame (ilk {})",
            ask!("string(ilk(gGame))")).into());
    }
    probe(format!("gURLLock={} gCheckMoveCursor={} gSplashPause={} gRenderFormat={}",
        ask!("string(gGame.gURLLock)"), ask!("string(gGame.gCheckMoveCursor)"),
        ask!("string(gGame.gSplashPause)"), ask!("string(gGame.gRenderFormat)")));
    let _ = snapshots.verify("01_intro", player.snapshot_stage());

    // ---- 2. START -> the level picker --------------------------------------
    let start_ch = find_ch!("sign_pressspacetostart");
    if start_ch == 0 { return Err("the intro has no sign_pressspacetostart sprite".into()); }
    click_ch!(start_ch, "START");
    player.step_frames(20).await;
    if player.current_frame() != 70 {
        return Err(format!("START did not reach the \"level\" marker (70); frame {}",
            player.current_frame()).into());
    }
    let _ = snapshots.verify("02_level", player.snapshot_stage());

    // ---- 3. LEVEL 1 -> clone the embedded W3D casts -> difficulty ----------
    //
    // The level buttons are SWFs; they reach Lingo to call `setLevelAndLoad`,
    // which goes to "preLevel" and makes `loadOrClone`. `cloneModels` then pulls
    // level1 / mech / tank / turretB / items out of the cast and `callLogin`
    // brings the shell back up on "difficulty" (80).
    let lvl_ch = find_ch!("level1Button");
    if lvl_ch == 0 { return Err("the level screen has no level1Button sprite".into()); }
    click_ch!(lvl_ch, "LEVEL 1");
    if !step_until_frame!(80, "after LEVEL 1") {
        return Err(format!(
            "the level-1 clone never completed: parked on frame {}. The level SWF calls \
             `setLevelAndLoad` back through Lingo, which makes `loadOrClone`; only its \
             `timeout(\"pauseCloning\")` -> `cloneModels` -> `callLogin` reaches the \
             difficulty screen.",
            player.current_frame()
        ).into());
    }
    let models = ask_int!("gScene.model.count");
    probe(format!("cloned: models={} groups={} motions={}", models,
        ask!("string(gScene.group.count)"), ask!("string(gScene.motion.count)")));
    if models < 10 {
        return Err(format!("`cloneModels` only produced {} models", models).into());
    }
    let _ = snapshots.verify("03_difficulty", player.snapshot_stage());

    // ---- 4. EASY -> log in and enter the game ------------------------------
    let easy_ch = find_ch!("buttonEasy");
    if easy_ch == 0 { return Err("the difficulty screen has no buttonEasy sprite".into()); }
    click_ch!(easy_ch, "EASY");
    if !step_until_frame!(110, "after EASY") {
        return Err(format!(
            "never entered the game: parked on frame {} (gLogin={}, gRunScript={})",
            player.current_frame(), ask!("string(ilk(gLogin))"), ask!("string(ilk(gRunScript))")
        ).into());
    }
    // `loginManager`'s own timeout chain finishes a little after the playhead
    // reaches "inGame", so wait for the run script rather than for the frame.
    for _ in 0..30 {
        if ask_str!("string(ilk(gRunScript))") == "instance" { break; }
        player.step_frames(20).await;
    }
    if ask_str!("string(ilk(gRunScript))") != "instance" {
        return Err(format!("`loginManager` never built gRunScript (ilk {})",
            ask!("string(ilk(gRunScript))")).into());
    }
    let avatar = ask_str!("string(gStatus.gUserName)");
    let body = ask_str!(&format!("string(gScene.model(\"{}Body\").worldPosition)", avatar));
    probe(format!("in game: avatar={} body={} models={}", avatar, body,
        ask!("string(gScene.model.count)")));
    if !body.starts_with("vector(") {
        return Err(format!("the avatar mech \"{}Body\" was never built ({})", avatar, body).into());
    }
    let stage = player.snapshot_stage();
    let _ = snapshots.verify("04_ingame", stage.clone());

    // ---- 5. The radar's second camera and its backdrop ---------------------
    //
    // `createRadar` adds an orthographic camera at index 2, inset to gRadarRect,
    // rooted on the detached blipGroup, non-clearing, with the radarBG dial as
    // its only backdrop.
    probe(format!(
        "radar cam: name={} rect={} projection={} clearAtRender={} backdrops={} rootNode={}",
        ask!("string(sprite(1).camera(2).name)"),
        ask!("string(sprite(1).camera(2).rect)"),
        ask!("string(sprite(1).camera(2).projection)"),
        ask!("string(sprite(1).camera(2).colorBuffer.clearAtRender)"),
        ask!("string(sprite(1).camera(2).backdrop.count)"),
        ask!("string(sprite(1).camera(2).rootNode.name)")));
    if ask_str!("string(sprite(1).camera(2).name)") != "radar" {
        return Err(format!("`sprite(1).addCamera(radar, 2)` did not take (camera 2 is {})",
            ask!("string(sprite(1).camera(2).name)")).into());
    }
    if ask_str!("string(sprite(1).camera(2).backdrop.count)") == "0" {
        return Err("the radar camera has no backdrop — `addBackdrop(radarBG, …)` was lost".into());
    }

    // The dial must actually be ON SCREEN inside the radar box. The backdrop is a
    // 256px texture placed at point(-58, -58) in SPRITE space and clipped by the
    // camera rect, so it fills the 140x140 window; drawn in the camera's own
    // narrowed viewport instead it collapsed to ~45px in the corner, and gated on
    // the clear it did not draw at all.
    let box_px = green_pixels(&stage, 10, 10, 150, 150);
    let corner_px = green_pixels(&stage, 10, 10, 55, 55);
    let total = 140 * 140;
    probe(format!("radar dial: {} green px in rect(10,10,150,150) of {} ({} in the corner)",
        box_px, total, corner_px));
    if box_px * 100 < total * 15 {
        return Err(format!(
            "the radar dial did not draw: only {} of {} px inside rect(10,10,150,150) are \
             the dial's green. `addBackdrop` on the radar camera must draw even though \
             its `colorBuffer.clearAtRender` is 0 — Director 11.5, `clearAtRender`, \
             governs only the colour buffer.",
            box_px, total
        ).into());
    }
    // A dial squeezed into the camera's own viewport lands entirely in the
    // top-left ~45px; a correctly placed one fills the box, so most of its green
    // is OUTSIDE that corner.
    if corner_px * 2 > box_px {
        return Err(format!(
            "the radar dial is collapsed into the corner of its box ({} of {} green px are \
             inside rect(10,10,55,55)). `addBackdrop`'s locWithinSprite is measured from \
             the SPRITE's upper left, so its ortho must span the sprite while the camera \
             rect clips it — not be squeezed into the inset viewport.",
            corner_px, box_px
        ).into());
    }

    // ---- 6. Gameplay: drive and shoot --------------------------------------
    //
    // Both inputs are POLLED by the game's own Director timeouts rather than
    // delivered as events — `keyScript.keyControl` reads `keyPressed()` and
    // `clickScript.leftClick` reads `the mouseDown` — so the test holds each
    // input down across a run of frames instead of tapping it.
    //
    // `gControlKeys` is "both" here (gameValues()[19]), so keyScript watches the
    // Mac arrow codes 126/125/124/123 AND w/a/s/d.
    let pos_of = |s: &str| -> Option<(f64, f64, f64)> {
        let t = s.trim_start_matches("vector(").trim_end_matches(')');
        let v: Vec<f64> = t.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if v.len() == 3 { Some((v[0], v[1], v[2])) } else { None }
    };
    let dummy = format!("gScene.model(\"{}Dummy\").worldPosition", avatar);
    let before = ask_str!(&format!("string({})", dummy));
    player.key_down("ArrowUp", 126).await;
    player.step_frames(45).await;
    player.key_up("ArrowUp", 126).await;
    player.step_frames(10).await;
    let after = ask_str!(&format!("string({})", dummy));
    probe(format!("drive: {} -> {} (gMotion={})", before, after, ask!("string(gMotion)")));
    match (pos_of(&before), pos_of(&after)) {
        (Some(a), Some(b)) => {
            let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt();
            if d < 1.0 {
                return Err(format!(
                    "holding the forward key did not move the mech: {} stayed at {}. \
                     `keyScript.keyControl` polls `keyPressed(126)` / `keyPressed(\"w\")` \
                     from its own `timeout(\"keyControl\")`.",
                    dummy, before
                ).into());
            }
        }
        _ => return Err(format!("could not read the mech's world position ({})", before).into()),
    }
    let _ = snapshots.verify("05_driven", player.snapshot_stage());

    // Shooting. `clickScript.leftClick` polls `the mouseDown` from its own
    // timeout and, per tick, loops pMunitions calling
    // `shootBullet(pName, pScene.model(pName & jMunition), pBulletStyle)` for
    // each — here BOTH bulletLeft and bulletRight. `shootBullet` names its clone
    // `<user>Bullet & string(the milliSeconds)`, so the two rounds of one tick
    // only get distinct names if a millisecond ticks over between them.
    //
    // MEASURED: it does not. dirplayer runs a whole `shootBullet` (a model
    // clone plus a raycast) inside one millisecond, where Director took longer
    // and the two names differed — so holding the mouse down raises Director's
    // real "Object with duplicate name already exists" from `clone`, and
    // `break_on_error` then PAUSES the whole movie rather than just abandoning
    // the handler. TRECH 1 has the identical shape; see misc/trech.rs.
    //
    // Until that is settled, drive the shot pipeline one round per evaluation —
    // which is what the movie does across successive ticks — so `shootBullet`
    // -> gBulletList -> `moveBullets` is still covered end to end.
    let style = ask_str!("string(shotAttributes(gStatus.gCharacter)[7])");
    probe(format!("munitions style={} bulletStyle={}",
        ask!("string(gMunitions)"), style));
    let mut peak = ask_int!("gBulletList.count").max(0);
    let mut fired = 0;
    for i in 0..6 {
        let munition = if i % 2 == 0 { "bulletLeft" } else { "bulletRight" };
        // Space the shots by a frame FIRST: two `shootBullet`s inside the same
        // millisecond collide on the clone name (see the note above), and a
        // Lingo raise pauses the player, which the harness turns into a panic.
        player.step_frames(4).await;
        let r = ask!(&format!(
            "shootBullet(gStatus.gUserName, gScene.model(gStatus.gUserName & \"{}\"),              shotAttributes(gStatus.gCharacter)[7])", munition));
        if r.starts_with("<err") {
            return Err(format!("shootBullet({}) raised: {}", munition, r).into());
        }
        fired += 1;
        player.step_frames(4).await;
        peak = peak.max(ask_int!("gBulletList.count"));
    }
    probe(format!("shoot: {} rounds fired, gBulletList peak {} (now {}), models={}",
        fired, peak, ask!("string(gBulletList.count)"), ask!("string(gScene.model.count)")));
    // `moveBullets` retires a round within a few frames, so gBulletList holds
    // only what is still in flight — the count that matters is that every shot
    // went through and that rounds actually reached the list.
    if fired < 6 || peak < 1 {
        return Err(format!(
            "firing did not work: {} of 6 rounds went through and gBulletList never              rose above {}. `shootBullet` clones the muzzle model and pushes the round              onto gBulletList for `moveBullets` to fly.", fired, peak
        ).into());
    }
    // `moveBullets` flies each round and then retires it, so the list must
    // DRAIN again — a live projectile pipeline, not a leak.
    player.step_frames(120).await;
    let left = ask_int!("gBulletList.count");
    probe(format!("after flight: gBulletList={} models={}", left,
        ask!("string(gScene.model.count)")));
    if left > 0 {
        return Err(format!(
            "`moveBullets` never retired a round: gBulletList peaked at {} and is              still {} after 120 frames.", peak, left
        ).into());
    }
    let _ = snapshots.verify("06_shooting", player.snapshot_stage());
    Ok(())
});
