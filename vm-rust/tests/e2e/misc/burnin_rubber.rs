use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, SnapshotOutput, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_burnin_rubber.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// Read a Lingo global as a formatted string — `eval` cannot reach bare globals.
fn global(name: &str) -> String {
    use vm_rust::player::datum_formatting::format_concrete_datum;
    use vm_rust::player::reserve_player_ref;
    use vm_rust::player::symbols::symbol::Symbol;
    reserve_player_ref(|p| match p.globals.get(&Symbol::from_str(name)) {
        Some(r) => format_concrete_datum(&p.get_datum(r), p),
        None => "<unset>".to_string(),
    })
}

/// How many DISTINCT colours the stage carries. The failure this test guards is
/// a flat single-colour stage (the 3D FBO holding nothing but its clear value),
/// which no reference image would catch on its own — the menu's fire/tunnel
/// animation is time-dependent, so a pixel-exact snapshot of it is flaky.
fn pixel_at(snap: &SnapshotOutput, x: u32, y: u32) -> (u8, u8, u8) {
    let img = match snap {
        SnapshotOutput::Base64Png(b64) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(b64).ok()
                .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
                .map(|i| i.to_rgba8())
        }
        SnapshotOutput::Rgba { width, height, data } =>
            image::RgbaImage::from_raw(*width, *height, data.clone()),
    };
    match img {
        Some(i) if x < i.width() && y < i.height() => {
            let p = i.get_pixel(x, y).0;
            (p[0], p[1], p[2])
        }
        _ => (0, 0, 0),
    }
}

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
        // Quantise to 5 bits/channel: an anti-aliased gradient should not count
        // as thousands of "colours", but real artwork still spreads widely.
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

// Xform / Shockwave.com "Burnin' Rubber" (2007) — Shockwave 3D + Havok, D11.5.
// Reported as not working in dirplayer: the game booted to a blank white stage
// and stayed there.
//
// Frame layout (from `_movie.markerList`):
//   2  "Shockwave"      splash, ~220 frames
//   5  "Xform"          publisher splash
//   10 "Preload_Menu" / 11 "Menu"      the 3D main menu
//   15 "Preload_Garage" / 16 "Garage"  car select (external cast Data/Garage.cct)
//   20 "Preload_Track"  / 21 "Track"   the race
//
// The whole game is ONE Shockwave3D sprite (channel 5, member "Menu") carrying
// TWO cameras: `CameraFire` renders the fire/tunnel background, and an
// orthographic `CameraMenu` added with `sprite.addCamera(cam, 2)` draws the UI
// over it with `colorBuffer.clearAtRender = 0`.
//
// Six defects were in the way, each fixed alongside this test:
//
//  1. `_movie.scriptExecutionStyle = 10` — the very first line of `prepareMovie`
//     — raised "Cannot set movie prop". The property was hard-coded read-only,
//     so the movie died before it ever started.
//  2. `member.cast` (the cast library OBJECT) was unimplemented, so the shared
//     `GetModelList` helper errored out during menu preload.
//  3. `Data/Garage.cct`'s cast list advertises more entries than its item table
//     holds, and the MCsL parser indexed straight into it and PANICKED.
//  4. THE BLANK SCREEN: the movie creates every texture with the one-argument
//     `newTexture(name)` form and supplies the pixels afterwards via
//     `texture(name).member = member(name)`. dirplayer only registered a texture
//     when the 3-argument `#fromCastMember` form was used, so every lookup
//     returned VOID, every assignment went nowhere, and the scene rendered
//     completely untextured — the menu's fullscreen quads with no texture, over
//     a white camera clear, are a blank white stage.
//
//  5. Fog was MEMBER-wide rather than per camera, so `camera("CameraFire").fog`
//     — which the intro drives to a white far plane of 1.0 — also fogged the
//     orthographic `CameraMenu` pass that draws the entire UI.
//  6. `_key.keyCode` was cleared the moment a key was released, so `on keyUp`
//     handlers saw 0. Director documents it as "the last key PRESSED", and the
//     whole menu is driven from `on keyUp me` → `case _key.keyCode of`.
//
//  7. `member.cast` answered the CAST LIBRARY. Director answers it with the
//     MEMBER itself — measured in Director 11.5 with castLib "Garage" linked:
//     `put member("GarageMenu").cast.name` → "GarageMenu". The Event Manager
//     defaults its cast argument from it (`if pCast = VOID then pCast =
//     pMember.cast.name`) and then does `texture.member = member(fileName,
//     pCast)`, so Director qualifies with "GarageMenu" — not a cast library
//     name — the argument is unresolvable and the lookup degrades to the
//     movie-wide by-name search that finds the bitmaps in castLib "Main".
//     Handing the movie the real name "Garage" instead correctly RESTRICTED the
//     search (a qualified miss is VOID in Director), so the garage panel's
//     textures were empty and its full-screen quads painted the showroom
//     opaque white.
//  8. A 32-bit source bitmap with `image.useAlpha` off stayed translucent when a
//     texture was filled through the `texture.member =` setter (only the
//     `newTexture(…, #fromCastMember, …)` constructor forced it opaque).
//  9. A runtime-supplied 2nd UV set SHORTER than the vertex count made WebGL
//     reject the whole draw call, so the entire showroom disappeared.
// 10. An `#add` texture layer promoted the whole SURFACE to framebuffer-additive
//     even at full opacity, washing the showroom and every car out to white.
// 11. `new(#havok)` could not create a member, so no track could start.
// 12. The Havok handler table matched hand-written spellings, missing the
//     `RigidBody` capitalisation `initializeHover` uses.
// 13. `vector < vector` / `vector > vector` had no comparison arm and were always
//     false, so the race countdown never finished and `StartRace` never ran.
// 14. `texture.type` answered a blanket "#fromFile" — not one of Director's three
//     documented values — so [PS] LightManager's `case ttexture.type of` matched
//     nothing and the cars raced unlit.
browser_e2e_test!(test_misc_burnin_rubber_menu, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
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

    let snapshots = SnapshotContext::new("misc", "burnin_rubber");

    // ---- Boot ---------------------------------------------------------------
    // `prepareMovie` runs `ImportAndLoadGlobals` (reading the internal
    // "GameGlobals" text member) and `startMovie` runs `SetupBurninRubber`.
    // gInitialized = 1 is that path having completed.
    player.step_frames(60).await;
    probe(format!(
        "boot frame={} gLocal={} gDebug={} gInitialized={} gEvents={}",
        player.current_frame(),
        global("gLocal"),
        global("gDebug"),
        global("gInitialized"),
        global("gEvents"),
    ));
    if global("gInitialized") != "1" {
        return Err(
            "startMovie's SetupBurninRubber never completed — the movie died during boot"
                .to_string()
                .into(),
        );
    }
    let _ = snapshots.verify("01_boot", player.snapshot_stage());

    // ---- Walk to the "Menu" marker (frame 11) --------------------------------
    // Two splash screens plus a 3D preload sit in front of it.
    let mut reached = false;
    for _ in 0..40 {
        player.step_frames(40).await;
        if player.current_frame() == 11 {
            reached = true;
            break;
        }
    }
    if !reached {
        return Err(format!(
            "never reached the Menu marker (frame 11) — stuck on frame {} ({})",
            player.current_frame(),
            ask!("string(_movie.frameLabel)")
        )
        .into());
    }

    // The Event Manager's `MenuTextures` event builds all 19 menu textures with
    // `newTexture(name)` + `texture.member = member(name)`. If the blank-texture
    // form is not registered, `member("Menu").texture(...)` is VOID for every
    // one of them and the scene renders untextured.
    let tex_count = ask!("string(member(\"Menu\").texture.count)");
    probe(format!(
        "menu textures={} Logo_Texture width={} models={} cameras={}",
        tex_count,
        ask!("string(member(\"Menu\").texture(\"Logo_Texture\").width)"),
        ask!("string(member(\"Menu\").model.count)"),
        ask!("string(member(\"Menu\").camera.count)"),
    ));
    if ask!("string(member(\"Menu\").texture(\"Logo_Texture\") <> VOID)") != "String(\"1\")" {
        return Err(format!(
            "the Event Manager's CreateTexture built no textures (texture.count = {}). \
             `newTexture(name)` with no type/source must register the texture so the \
             following `texture(name).member = ...` has something to fill.",
            tex_count
        )
        .into());
    }

    // The intro fade is timeout-driven: MenuFade (1800 ms) → ShowMenuAFterFade
    // (750 ms) → ShowRest (3000 ms), which is what puts the logo and the menu
    // root ("XformMain") back in the world. Step long enough for that chain.
    for _ in 0..10 {
        player.step_frames(60).await;
    }
    probe(format!(
        "menu ready: timeouts={} fog(CameraFire).far={} clear={}",
        ask!("string(the timeoutList)"),
        ask!("string(member(\"Menu\").camera(\"CameraFire\").fog.far)"),
        ask!("string(member(\"Menu\").camera(\"CameraFire\").colorBuffer.clearValue)"),
    ));

    let menu_snap = player.snapshot_stage();
    // The regression this guards: a stage that is one flat colour. The menu is
    // a photographic tyre/fire image plus a logo, so it carries dozens.
    let colors = distinct_colors(&menu_snap);
    probe(format!("menu stage distinct colours={}", colors));
    if colors < 8 {
        return Err(format!(
            "the 3D main menu rendered as a flat {}-colour stage — the scene drew nothing \
             but its camera clear value",
            colors
        )
        .into());
    }
    let _ = snapshots.verify("02_menu", menu_snap);

    // ---- "PRESS SPACE TO START" ---------------------------------------------
    // [PS] Menu's `keyHandler` reads `_key.keyCode` and treats 49 (Director's
    // code for the space bar) as the select key: the first press runs
    // `StartMenu`, which swaps the splash logo for the MainMenu model set.
    // The handler lives on the SPRITE ("[SS] [Menu] Assign & Run My ParentScript"
    // → `on keyUp me`), and dirplayer routes key events to a sprite only once it
    // holds the keyboard focus — which a click on it grants. That mirrors what a
    // browser player does before typing anything.
    //
    // The harness takes a JS keyCode (32 = space); dirplayer maps it to Director's
    // own code 49, which is what `keyHandler`'s `case _key.keyCode of` tests.
    player.click(320, 240).await;
    player.step_frames(4).await;
    player.key_down(" ", 32).await;
    player.key_up(" ", 32).await;
    // `on keyUp` reads `_key.keyCode` for the key it was called for, so the code
    // must survive the release: Director documents `keyCode` as "the last key
    // PRESSED", not the key currently held.
    probe(format!("keyCode after release={}", ask!("string(_key.keyCode)")));
    player.step_frames(60).await;
    probe(format!(
        "after space: gInput={} gMenuVisited={} frame={}",
        global("gInput"),
        global("gMenuVisited"),
        player.current_frame()
    ));
    if global("gMenuVisited") != "1" {
        return Err(
            "space did not open the main menu — [PS] Menu's `on keyUp` reads `_key.keyCode`, \
             which must still report the released key (Director: the last key pressed)"
                .to_string()
                .into(),
        );
    }
    let _ = snapshots.verify("03_main_menu", player.snapshot_stage());

    // ---- START GAME → the garage --------------------------------------------
    // The MainMenu opens on "StartGame", so a second space runs `StartGame`: it
    // points castLib "Garage" at `Data/Garage.cct`, streams `Data/GarageData.cct`
    // through [PS] Preload Cast and goes to "Preload_Garage" (15) → "Garage"
    // (16). This is the part that exercises the external casts — and the MCsL
    // parse that used to panic the player outright.
    player.key_down(" ", 32).await;
    player.key_up(" ", 32).await;
    let mut in_garage = false;
    for _ in 0..40 {
        player.step_frames(40).await;
        if player.current_frame() == 16 {
            in_garage = true;
            break;
        }
    }
    if !in_garage {
        return Err(format!(
            "START GAME never reached the Garage marker (frame 16) — stuck on frame {} ({}).              The external casts Data/Garage.cct + Data/GarageData.cct must load.",
            player.current_frame(),
            ask!("string(_movie.frameLabel)")
        )
        .into());
    }
    // Let [PS] Garage build its runtime camera and the Garage Menu overlay.
    player.step_frames(240).await;
    probe(format!(
        "garage: member models={} textures={} cameras={} spriteCams={} cam1={}",
        ask!("string(member(4, 3).model.count)"),
        ask!("string(member(4, 3).texture.count)"),
        ask!("string(member(4, 3).camera.count)"),
        ask!("string(sprite(5).cameraCount)"),
        ask!("string(sprite(5).camera(1).name)"),
    ));

    // The external cast really loaded: 33 models and 32 textures come out of
    // GarageData.cct, none of which exist in the main movie.
    if ask!("string(member(4, 3).model.count)") == "String(\"0\")" {
        return Err(
            "the Garage external cast loaded no 3D content — Data/GarageData.cct did not stream in"
                .to_string()
                .into(),
        );
    }

    let garage = player.snapshot_stage();
    let garage_colors = distinct_colors(&garage);
    probe(format!("garage distinct colours={}", garage_colors));
    if garage_colors < 8 {
        return Err(format!(
            "the car-select showroom rendered as a flat {}-colour screen — the 3D scene              behind the UI drew nothing",
            garage_colors
        )
        .into());
    }
    let _ = snapshots.verify("04_garage", garage);

    // ---- Pick the car, pick the track, race -------------------------------
    // [PS] Garage's `SelectFirstCar` already advances the selection to the
    // Nissan (gCarsUnlocked = [0,1,1,0]), so SPACE selects it straight away.
    // That runs `select()`, which hands over to the TRACK list through two
    // chained timeouts — SelectFirstTrack1 (800 ms) then SelectFirstTrack2
    // (1750 ms) — the second of which advances to "Sunshine", the first
    // unlocked track. A second SPACE then fades out with `#ToDo:
    // "PreloadSelectedTrack"`, which streams Data/SunshineTrackData.cct and
    // goes to "Preload_Track" (20) → "Track" (21).
    player.key_down(" ", 32).await;
    player.key_up(" ", 32).await;
    for _ in 0..12 {
        player.step_frames(30).await;
    }
    probe(format!(
        "car picked: gSelectedCar={} frame={}",
        global("gSelectedCar"),
        player.current_frame()
    ));
    let _ = snapshots.verify("05_track_select", player.snapshot_stage());

    player.key_down(" ", 32).await;
    player.key_up(" ", 32).await;
    let mut on_track = false;
    for _ in 0..80 {
        player.step_frames(40).await;
        if player.current_frame() == 21 {
            on_track = true;
            break;
        }
    }
    probe(format!(
        "after track select: frame={} label={} gSelectedTrack={} gReverseState={}",
        player.current_frame(),
        ask!("string(_movie.frameLabel)"),
        global("gSelectedTrack"),
        global("gReverseState"),
    ));
    if !on_track {
        return Err(format!(
            "never reached the Track marker (frame 21) — stuck on frame {} ({}).              Data/SunshineTrack.cct + Data/SunshineTrackData.cct must stream in.",
            player.current_frame(),
            ask!("string(_movie.frameLabel)")
        )
        .into());
    }

    // Let the race build: the track member streams, Havok wakes up and the
    // countdown runs before the car is under the player's control.
    player.step_frames(600).await;
    probe(format!(
        "race: state={} car={} kmh={} frame={}",
        global("gRaceStats"),
        global("gCurrentCar"),
        ask!("string(gCarsAndStats[gCurrentCar][#specifics][#speedKH])"),
        player.current_frame(),
    ));
    let track = player.snapshot_stage();
    probe(format!("track distinct colours={}", distinct_colors(&track)));
    let _ = snapshots.verify("06_track", track);

    // ---- Drive ---------------------------------------------------------------
    // `PrepareRace` holds gRaceStats[#state] on "prerace" through the intro
    // flyby and the 3-2-1 counter (the CountDownScaler timeout chain), then
    // `StartRace` flips it to "started". Only then does [PS] Player read the
    // arrow keys.
    let mut started = false;
    for _ in 0..40 {
        player.step_frames(30).await;
        if global("gRaceStats").contains("started") {
            started = true;
            break;
        }
    }
    probe(format!("race start: state={} started={}", global("gRaceStats"), started));
    if !started {
        return Err(format!(
            "the race never left the pre-race state: gRaceStats = {}. The 3-2-1-GO chain              ends on `if pmodel.transform.scale <= vector(1.0, 1.0, 1.0)`, so ordered              comparison of two vectors has to work.",
            global("gRaceStats")
        )
        .into());
    }

    // Hold the throttle. gCarsAndStats[gCurrentCar][#specifics][#speedKH] is the
    // movie's own speedometer read-out, so a rising value is the Havok vehicle
    // actually being driven rather than merely existing.
    player.key_down("ArrowUp", 38).await;
    let mut top_kmh = 0.0f64;
    for i in 0..12 {
        player.step_frames(20).await;
        let kmh = ask!("string(gCarsAndStats[gCurrentCar][#specifics][#speedKH])");
        let v: f64 = kmh.trim_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
            .parse().unwrap_or(0.0);
        if v > top_kmh { top_kmh = v; }
        if i % 4 == 0 {
            probe(format!("driving {}: kmh={} pos={}", i, kmh,
                ask!("string(gCarsAndStats[gCurrentCar][#specifics][#position])")));
        }
    }
    player.key_up("ArrowUp", 38).await;
    probe(format!("top speed reached: {} km/h", top_kmh));
    // The car's paint is lit by [PS] LightManager, which ray-casts the road,
    // samples the track's lightmap at the hit point and writes the result to the
    // car shaders' `emissive` every frame. It reaches that lightmap through
    // `case ttexture.type of … #fromCastMember`, so a texture built at runtime
    // has to report its true origin — with the old blanket "#fromFile" the case
    // matched nothing, the mapping collapsed and the cars raced unlit and flat.
    let lm = ask!("string(gCarsAndStats[gCurrentCar][#specifics][#LMColor])");
    probe(format!("LightManager colour={} car emissive={}", lm,
        ask!("string(member(gSelectedTrack).model(\"Player1Nissan\").shader.emissive)")));
    if lm.contains("rgb(0, 0, 0)") {
        return Err(
            "[PS] LightManager never lit the car — its lightmap lookup goes through              `texture.type`, which must report #fromCastMember / #fromImageObject /              #importedFromFile"
                .to_string()
                .into(),
        );
    }
    let _ = snapshots.verify("07_racing", player.snapshot_stage());
    if top_kmh < 1.0 {
        return Err(format!(
            "holding the throttle never moved the car (top speed {} km/h) — the Havok              vehicle is built but not driven",
            top_kmh
        )
        .into());
    }

    Ok(())
});
