use vm_rust::browser_e2e_test;
use vm_rust::director::lingo::datum::Datum;
use vm_rust::player::testing_shared::{SnapshotContext, SnapshotOutput, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_burnin_rubber2.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

/// One key out of the engine's single `g` property-list global. `eval` cannot
/// reach bare globals, and the whole `g` dump runs to thousands of characters
/// (it carries every track's unlock table), so probes read individual keys.
fn gprop(key: &str) -> String {
    use vm_rust::player::datum_formatting::format_concrete_datum;
    use vm_rust::player::reserve_player_ref;
    use vm_rust::player::symbols::symbol::Symbol;
    reserve_player_ref(|p| {
        let Some(r) = p.globals.get(&Symbol::from_str("g")) else {
            return "<no g>".to_string();
        };
        match p.get_datum(r) {
            Datum::PropList(pairs, _) => pairs
                .iter()
                .find(|(k, _)| {
                    format_concrete_datum(&p.get_datum(k), p)
                        .trim_start_matches('#')
                        .eq_ignore_ascii_case(key)
                })
                .map(|(_, v)| format_concrete_datum(&p.get_datum(v), p))
                .unwrap_or_else(|| "<unset>".to_string()),
            _ => "<g not a propList>".to_string(),
        }
    })
}

/// How many DISTINCT colours the stage carries. The failure this guards is a
/// flat single-colour stage (the 3D FBO holding nothing but its clear value),
/// which no reference image would catch on its own — the menu and the race are
/// both time-dependent, so a pixel-exact snapshot of either is flaky.
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
        // Quantise to 5 bits/channel: an anti-aliased gradient must not count
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

// Xform / Shockwave.com "Burnin' Rubber 2" (2008) — Shockwave 3D + Havok, D11.
// Reported as not working in dirplayer: the movie died on its very first frame.
//
// Frame layout (from `_movie.markerList`):
//   1  Preloader / 2 Downloader     the shipped Preloader.dcr's landing frames
//   3  ShockwaveIntro / 4 LogoIntro splashes
//   5  Menu                         the main menu
//   6  TrackIntro / 7 TrackSelection
//   8  CarIntro   / 9 CarSelection
//   20 Race                         the race itself
//
// Everything is driven by a data-driven event engine: `g.Events` maps an event
// name to a list of TAB-separated command lines ("AddScript", "AddModule",
// "GoTo", "LinkKey", …) which `[M] Event Manager` dispatches to handlers in
// `[M] Event Manager Functions` / `… 3D Functions`. The whole game is one
// Shockwave3D sprite in channel 1 carrying a `[PS] 3D Script Manager` behaviour;
// cars, cameras and HUD modules hang off that as nested parent-script instances.
//
// Five defects were in the way, each fixed alongside this test:
//
//  1. `_movie.markerlist()` — the PARENTHESISED form of a read-only movie
//     property — raised "No handler markerlist for datum <_movie>". Every single
//     `GoTo` event command validates its marker through it, so the movie never
//     left frame 1. (Director's dot syntax accepts empty parens on a property.)
//  2. `_movie.castLib("Engine").member.count` — the cast library's member
//     COLLECTION — raised "Cannot get castLib property member". Both `AddScript`
//     and `AddModule` open with it to locate the named script in the Engine cast.
//  3. `sprite(n).scriptInstanceList.count` compiles to `count(sprite, #prop)`,
//     an objcall on the SPRITE. There was no arm for it, so it fell through to
//     the global `count()` and raised "Cannot get count of non-list (type:
//     sprite)". `AddModule` gates its whole dispatch on that count, so the race
//     ran with NONE of its per-car modules — no speedometer, no lightmap
//     lighting, no shadow, no checkpoint manager.
//  4. `cloneModelFromCastmember` namespaced the cloned CHILD nodes
//     ("FinishLine_FinishLineDummy_LeftTop"), but Director copies them under
//     their own names. `SetCheckPoints` measures the finish line from those
//     dummies by bare name, so it found a loose world-parented copy instead and
//     put the finish rect on the origin — on top of the start line. Once the
//     checkpoint manager was running (defect 3) that ended the race a few metres
//     after the lights went out.
//  5. Timeout objects relayed the system events (prepareFrame / exitFrame) to
//     EVERY target, not just child-object targets. `DelayEvent` builds timeouts
//     whose target is the event NAME — a string — so each frame logged two
//     "No handler prepareFrame for string datum" failures.
browser_e2e_test!(test_misc_burnin_rubber2_race, |player| async move {
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

    let snapshots = SnapshotContext::new("misc", "burnin_rubber2");

    // ---- Boot ---------------------------------------------------------------
    // `_player.runMode` defaults to "Plugin", which is what puts `InitGlobals`
    // into `g.local = 0` — the network path. `[PS] Download Casts MAIN` then
    // pulls all seven content casts with
    // `preloadNetThing(_movie.path & "Data/<name>.cct")` and links them into the
    // placeholder libs eShared / eCars / eTrack.
    player.step_frames(60).await;
    probe(format!(
        "boot frame={} label={} runMode={} gInit={} gLocal={} downloaded={} map={}",
        player.current_frame(),
        ask!("string(_movie.frameLabel)"),
        ask!("string(_player.runMode)"),
        gprop("Init"),
        gprop("local"),
        gprop("DownloadedCasts"),
        gprop("map"),
    ));
    if gprop("Init") != "1" {
        return Err(format!(
            "the engine never initialised (g.Init = {}) — `BurninRubber2()` runs from \
             `startMovie` once the GameInitialization event has loaded the globals",
            gprop("Init")
        )
        .into());
    }
    let _ = snapshots.verify("01_boot", player.snapshot_stage());

    // ---- Walk to the "Menu" marker (frame 5) --------------------------------
    // Two splashes and a timeout-driven LogoIntro sit in front of it. Every hop
    // is a `GoTo` event command, which validates its marker against
    // `_movie.markerlist()`.
    let mut reached = false;
    for _ in 0..40 {
        player.step_frames(40).await;
        if ask!("string(_movie.frameLabel)").contains("Menu") {
            reached = true;
            break;
        }
    }
    if !reached {
        return Err(format!(
            "never reached the Menu marker (frame 5) — stuck on frame {} ({}). Every `GoTo` \
             event command walks `_movie.markerlist()` to validate its marker first.",
            player.current_frame(),
            ask!("string(_movie.frameLabel)")
        )
        .into());
    }
    let menu = player.snapshot_stage();
    let menu_colors = distinct_colors(&menu);
    probe(format!("menu: frame={} colours={}", player.current_frame(), menu_colors));
    if menu_colors < 8 {
        return Err(format!(
            "the main menu rendered as a flat {}-colour stage",
            menu_colors
        )
        .into());
    }
    let _ = snapshots.verify("02_menu", menu);

    // ---- START GAME → track select ------------------------------------------
    // The footer buttons carry "[SS] BurninRubber2 Button Script"; its
    // `on mouseUp` fires event("TrackIntro"), and the Event Manager chains
    // TrackIntro → TrackSelection.
    probe(format!("menu buttons: {}", button_rects()));
    click_button(&mut player, "StartGameButton").await;
    for _ in 0..30 {
        player.step_frames(20).await;
        if ask!("string(_movie.frameLabel)").contains("TrackSelection") {
            break;
        }
    }
    probe(format!(
        "track select: frame={} label={} track={} world={}",
        player.current_frame(),
        ask!("string(_movie.frameLabel)"),
        gprop("CurrentTrack"),
        gprop("CurrentWorld"),
    ));
    if !ask!("string(_movie.frameLabel)").contains("TrackSelection") {
        return Err(format!(
            "START GAME never reached the TrackSelection marker (frame 7) — stuck on frame {} ({})",
            player.current_frame(),
            ask!("string(_movie.frameLabel)")
        )
        .into());
    }
    let _ = snapshots.verify("03_track_select", player.snapshot_stage());

    // ---- CONTINUE → car select ----------------------------------------------
    click_button(&mut player, "ContinueButton").await;
    for _ in 0..30 {
        player.step_frames(20).await;
        if ask!("string(_movie.frameLabel)").contains("CarSelection") {
            break;
        }
    }
    probe(format!(
        "car select: frame={} label={} car={}",
        player.current_frame(),
        ask!("string(_movie.frameLabel)"),
        gprop("CurrentCar"),
    ));
    if !ask!("string(_movie.frameLabel)").contains("CarSelection") {
        return Err(format!(
            "CONTINUE never reached the CarSelection marker (frame 9) — stuck on frame {} ({})",
            player.current_frame(),
            ask!("string(_movie.frameLabel)")
        )
        .into());
    }
    let _ = snapshots.verify("04_car_select", player.snapshot_stage());

    // ---- CONTINUE → TrackInitialization → the race --------------------------
    // The Event Manager's TrackInitialization streams `Data/<World>.cct` through
    // [PS] Download Casts SUB and lands on the "Race" marker (20).
    click_button(&mut player, "ContinueButton").await;
    let mut racing = false;
    for _ in 0..60 {
        player.step_frames(30).await;
        if ask!("string(_movie.frameLabel)").contains("Race") {
            racing = true;
            break;
        }
    }
    if !racing {
        return Err(format!(
            "CONTINUE never reached the Race marker (frame 20) — stuck on frame {} ({}). \
             Data/<World>.cct must stream in through [PS] Download Casts SUB.",
            player.current_frame(),
            ask!("string(_movie.frameLabel)")
        )
        .into());
    }
    // Let the track member stream in, Havok wake up and the interface build.
    player.step_frames(600).await;
    probe(format!(
        "race: state={} track={} physics={} checkpoints={}",
        gprop("RaceState"),
        gprop("CurrentTrack"),
        gprop("Physics"),
        gprop("CheckPoints"),
    ));

    // The per-car modules come from the event data's `AddModule` lines, which
    // AddModule dispatches through `sprite(1).scriptInstanceList.count` and the
    // Engine cast's member collection. With either of those broken it added
    // NOTHING and the race ran with no speedometer, no lighting and no
    // checkpoints — silently, because the engine's own PutError is muted.
    // scriptList[2] of the 3D Script Manager is the PlayerCar; [PS] Car itself
    // contributes 2 ([PSM] Car Player Input + Car Collision Manager) and the
    // event data adds 7 more.
    let modules =
        ask!("string(sprite(1).scriptInstanceList[1].p.scriptList[2].p.scriptList.count)");
    probe(format!("player car modules={}", modules));
    if modules == "String(\"2\")" || modules.starts_with("<err") {
        return Err(format!(
            "the player car carries only its own two scripts (module count = {}) — every \
             `AddModule` event line was a no-op",
            modules
        )
        .into());
    }

    // The finish line has to sit at the END of the track, not on the origin:
    // `SetCheckPoints` moves the cloned FinishLine model to the last TrackRoot
    // group and then measures its child dummies' worldPosition.
    let checkpoints = gprop("CheckPoints");
    if checkpoints.contains("#finish: [#rect: rect(-15") {
        return Err(format!(
            "the finish rect landed on the start line: {}. `cloneModelFromCastmember` must \
             copy a model's children under their OWN names so `FinishLineDummy_LeftTop` \
             resolves to the child of the cloned FinishLine, not a loose world-parented copy.",
            checkpoints
        )
        .into());
    }

    let track = player.snapshot_stage();
    let track_colors = distinct_colors(&track);
    probe(format!("track distinct colours={}", track_colors));
    if track_colors < 8 {
        return Err(format!(
            "the race rendered as a flat {}-colour stage — the 3D scene drew nothing",
            track_colors
        )
        .into());
    }
    let _ = snapshots.verify("05_race", track);

    // ---- "PRESS SPACE TO START", then drive ---------------------------------
    // The race arms `LinkKey [#key: " ", #event: "StartGame"]`; the key reaches
    // it through `[PS] 3D Script Manager`'s `on keyUp me` → `KeyHandler(_key.key)`,
    // and dirplayer routes key events to a sprite only once it holds the
    // keyboard focus — which a click on it grants, as a browser player does.
    probe(format!("race keys={} input={}", gprop("Keys"), gprop("GameInput")));
    player.click(320, 240).await;
    player.step_frames(4).await;
    player.key_down(" ", 32).await;
    player.key_up(" ", 32).await;
    player.step_frames(120).await;
    if !gprop("Keys").contains("KeyEvent: []") {
        return Err(
            "SPACE did not fire the track's StartGame event — the `LinkKey [#key: \" \"]` entry \
             is consumed by `[PS] 3D Script Manager`'s `on keyUp me` → `KeyHandler(_key.key)`"
                .to_string()
                .into(),
        );
    }

    // [PSM] Car Player Input reads the arrow keys through `keyPressed(126)`
    // (Director's Mac key code for cursor-up) every frame once `g.GameInput` is
    // 1; `#speedMS` is the movie's own metres-per-second read-out, so a rising
    // value is the Havok vehicle actually being driven.
    player.key_down("ArrowUp", 38).await;
    let mut top_speed = 0.0f64;
    for i in 0..15 {
        player.step_frames(20).await;
        let v = car_speed();
        if v > top_speed {
            top_speed = v;
        }
        if i % 5 == 0 {
            probe(format!(
                "driving {}: speed={} input={} state={}",
                i,
                v,
                gprop("GameInput"),
                gprop("RaceState")
            ));
        }
    }
    player.key_up("ArrowUp", 38).await;
    probe(format!(
        "top speed = {} m/s, input={}, state={}",
        top_speed,
        gprop("GameInput"),
        gprop("RaceState")
    ));
    // `StartGame` opens a timeout chain — CountDown3 → CountDown2 → CountDown1 →
    // GO — and only the last of those hands the player the controls by setting
    // `g.GameInput = 1`.
    if gprop("GameInput") != "1" {
        return Err(format!(
            "the 3-2-1-GO countdown never handed over the controls: g.GameInput = {}",
            gprop("GameInput")
        )
        .into());
    }
    // A race that has already "finished" a few metres off the line means the
    // finish checkpoint is sitting on the start line (see defect 4).
    if !gprop("RaceState").contains("Pending") {
        return Err(format!(
            "the race left the Pending state while still on the first straight: {}",
            gprop("RaceState")
        )
        .into());
    }
    if top_speed < 1.0 {
        return Err(format!(
            "holding the throttle never moved the car (top speed {} m/s) — the Havok vehicle \
             is built but not driven",
            top_speed
        )
        .into());
    }

    // [PSM] Speedo swaps each digit model's `resource` to `Font<N>` every frame.
    // It is the cheapest proof that the AddModule-attached HUD modules actually
    // tick — they showed a frozen "000" while AddModule was a no-op.
    let mut speedo = Vec::new();
    for i in 1..=3 {
        speedo.push(ask!(&format!(
            "string(member(\"Interface\").model(\"Speedo{}\").resource.name)",
            i
        )));
    }
    probe(format!("speedo digits = {:?}", speedo));
    if !speedo.iter().any(|d| d.contains("Font")) {
        return Err(format!(
            "the speedometer never left its authored digits ({:?}) — [PSM] Speedo's \
             `model.resource = modelResource(\"Font\" & n)` never ran",
            speedo
        )
        .into());
    }
    let _ = snapshots.verify("06_driving", player.snapshot_stage());

    Ok(())
});

/// `g.cars[1][#specifics][#speedMS]` — the player car's own speedometer.
fn car_speed() -> f64 {
    let cars = gprop("cars");
    // The prop list prints as `[#player1BMW: [… #speedMS: 12.3400, …], …]`; the
    // FIRST #speedMS in it is the player's, which is all this needs.
    cars.split("#speedMS:")
        .nth(1)
        .and_then(|t| t.split(',').next())
        .and_then(|t| t.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Every on-stage sprite's channel, member name and placement — the menus are
/// built from named button bitmaps, so this locates a button without hard-coding
/// pixel coordinates.
fn button_rects() -> String {
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
            if name.is_empty() {
                continue;
            }
            out.push(format!(
                "{}:{}@({},{} {}x{})",
                ch, name, sprite.loc_h, sprite.loc_v, sprite.width, sprite.height
            ));
        }
        out.join(" ")
    })
}

/// Click the centre of the sprite whose member name starts with `prefix`.
async fn click_button<T: TestHarness>(player: &mut T, prefix: &str) {
    use vm_rust::player::reserve_player_ref;
    let hit = reserve_player_ref(|p| {
        for ch in 1..=150i16 {
            let sprite = &p.movie.score.get_channel(ch).sprite;
            if !sprite.visible {
                continue;
            }
            let name = sprite
                .member
                .as_ref()
                .and_then(|m| p.movie.cast_manager.find_member_by_ref(m))
                .map(|m| m.name.clone())
                .unwrap_or_default();
            if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                return Some((sprite.loc_h, sprite.loc_v));
            }
        }
        None
    });
    match hit {
        Some((x, y)) => {
            probe(format!("clicking {} at ({}, {})", prefix, x, y));
            player.click(x as i32, y as i32).await;
        }
        None => probe(format!("button {} NOT FOUND on stage", prefix)),
    }
}
