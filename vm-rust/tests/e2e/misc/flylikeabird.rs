use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, SnapshotOutput, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_flylikeabird.toml");

fn probe(msg: String) {
    // `println!` from wasm goes nowhere; the page console is what E2E_CONSOLE
    // forwards to the terminal.
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

/// Read one pixel. The caller must pass a DECODED snapshot (`to_rgba()`):
/// on the browser harness `SnapshotOutput::pixel` decodes the whole PNG per
/// call, so sampling a region off the raw base64 hangs the test.
/// Pull a number out of an `ask!` result (`Int(3)` / `Float(-1219.0)`).
fn num(formatted: &str) -> Option<f64> {
    let inner = formatted.strip_prefix("Float(").or_else(|| formatted.strip_prefix("Int("))?;
    inner.strip_suffix(')')?.parse().ok()
}

fn pixel(snap: &SnapshotOutput, x: u32, y: u32) -> (u8, u8, u8) {
    snap.pixel(x, y).map(|(r, g, b, _)| (r, g, b)).unwrap_or((0, 0, 0))
}

/// How much of the stage between `top` and `bottom` differs from the world's
/// background colour rgb(105,110,110). A blank 3D view returns ~0.
fn non_background_ratio(snap: &SnapshotOutput, top: u32, bottom: u32) -> f64 {
    let mut total = 0u32;
    let mut differing = 0u32;
    let mut y = top;
    while y < bottom {
        let mut x = 4u32;
        while x < 636 {
            let (r, g, b) = pixel(snap, x, y);
            total += 1;
            let d = (r as i32 - 105).abs() + (g as i32 - 110).abs() + (b as i32 - 110).abs();
            if d > 24 {
                differing += 1;
            }
            x += 8;
        }
        y += 8;
    }
    if total == 0 { 0.0 } else { differing as f64 / total as f64 }
}

/// Rows carrying any near-white text pixel. The WELCOME panel is white/yellow
/// type over a dark cityscape, so this measures the panel's vertical extent
/// without depending on exact glyph rasterisation.
fn text_rows(snap: &SnapshotOutput, x0: u32, x1: u32, y0: u32, y1: u32) -> Vec<u32> {
    (y0..y1).filter(|&y| {
        (x0..x1).step_by(3).any(|x| {
            let (r, g, b) = pixel(snap, x, y);
            // White body copy and the yellow "ENTER"/"CONTROL" emphasis both
            // have high red+green; blue separates them, so leave it free.
            r > 170 && g > 170
        })
    }).collect()
}

// Gamevial / Petesland "Fly Like A Bird" (Director 8.5, Shockwave 3D + a Flash
// logo). No Havok, no AGEIA PhysX — the flight model is Lingo arithmetic plus
// `modelsUnderRay` against the city mesh.
//
// Frame 1 is the WELCOME splash (behaviour `splashes` orbits a camera around the
// bird), frame 2 the game (`birdflies`), frame 3 GAME OVER / high score. The
// splash leaves frame 1 on `keyPressed(RETURN)`; the test jumps with `go(2)`
// rather than synthesising a held key.
//
// Six defects this movie found, all asserted below:
//
//  1. `cloneModelFromCastmember` dropped the source model's RUNTIME shader.
//     `startMovie` gives every model in member("city") a #standard shader named
//     after the model with a texture from the same-named bitmap; the game then
//     clones 100 of those buildings into member("world"). The clone read only the
//     parsed `node.shader_name`, so every building arrived wearing the file's
//     white `Material #1` and the destination member held no textures at all —
//     the whole city rendered as blown-out white blocks.
//
//  2. A Shockwave3D sprite with background-transparent ink (36) painted its
//     member background. The splash lays the bird's 3D sprite over a
//     half-blended cityscape bitmap and the title banner; the opaque clear put a
//     black rectangle across both.
//
//  3. `sprite(n).camera(i).rect` was ignored. The game's second camera is a
//     100x100 poo-cam inset at rect(530, 270, 630, 370) with clearAtRender = 0;
//     rendered full-viewport it drew its underground view over the entire game
//     and the stage was nothing but the world background colour.
//
//  4. The splash camera is ORTHOGRAPHIC (IFX view attributes bit 0, orthoHeight
//     530.79) and the parser read neither field, so the bird was drawn in
//     perspective and overflowed its sprite.
//
//  5. A stale `scrollTop` in the WELCOME panel's XMED header was applied to a
//     #adjust text member — which has no scrolling box — lifting the panel 69 px
//     and clipping its first paragraph off the top of the sprite.
//
//  6. `modelsUnderRay` read only the once-per-frame `node_transforms`, so a ray
//     cast in the same handler that had just moved the geometry answered from
//     the OLD positions. The whole city is built and then probed inside one
//     beginSprite, which left the bag of chips floating 1555 units up.
browser_e2e_test!(test_misc_flylikeabird_load, |player| async move {
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

    // --- Frame 1: the WELCOME splash -------------------------------------
    player.step_frames(20).await;
    if player.current_frame() != 1 {
        return Err(format!("expected the splash on frame 1, got {}", player.current_frame()).into());
    }
    let titles = ask!("member(\"titles\").text");
    if !titles.contains("WELCOME") {
        return Err(format!("the splash behaviour never ran — member(\"titles\") is {}", titles).into());
    }

    // `startMovie` textures member("city") before anything is cloned out of it.
    let city_shader = ask!("member(\"city\").model[1].shader.name");
    let city_tex = ask!("member(\"city\").model[1].shader.texture.name");
    probe(format!("city model1 shader={} tex={}", city_shader, city_tex));
    if city_shader.contains("Material") || city_tex.contains("default") {
        return Err(format!(
            "startMovie's per-model newShader/newTexture pass did not take: model[1] \
             wears shader {} / texture {}", city_shader, city_tex
        ).into());
    }

    // Defect 4: the bird member's authored camera.
    let proj = ask!("sprite(9).camera.projection");
    let ortho_h = ask!("sprite(9).camera.orthoHeight");
    probe(format!("splash camera: projection={} orthoHeight={} fov={}",
        proj, ortho_h, ask!("sprite(9).camera.fieldOfView")));
    if proj != "Symbol(\"orthographic\")" {
        return Err(format!(
            "the splash camera should be #orthographic (IFX view attributes 0x9), got {}", proj
        ).into());
    }
    if !ortho_h.starts_with("Float(530.78") {
        return Err(format!(
            "the splash camera's authored orthoHeight (530.7867) was not read from the \
             view node — got {}", ortho_h
        ).into());
    }

    let splash_shot = player.snapshot_stage();

    // Defect 2: sprite 9 (ink 36) sits at rect(-1, 25, 364, 300) over the
    // half-blended cityscape bitmap. Sample inside it, clear of the bird: with
    // the member background painted this is its black bgColor.
    let splash_rgba = splash_shot.to_rgba();
    let over_3d = pixel(&splash_rgba, 330, 260);
    probe(format!("splash pixel inside the ink-36 3D sprite = {:?}", over_3d));
    if over_3d.0 < 12 && over_3d.1 < 12 && over_3d.2 < 12 {
        return Err(format!(
            "the ink-36 Shockwave3D sprite painted its background: {:?} at (330, 260) \
             is the member's black bgColor, so the cityscape behind it is hidden", over_3d
        ).into());
    }

    // Defect 5: the WELCOME panel (member 29 "introtextscreen", #adjust 297x272)
    // runs from just under the sprite top (167) to near its bottom (443). An
    // honoured stale scrollTop lifted it ~69 px and clipped the first paragraph.
    let rows = text_rows(&splash_rgba, 330, 620, 100, 460);
    let first = rows.first().copied().unwrap_or(0);
    let last = rows.last().copied().unwrap_or(0);
    probe(format!("WELCOME panel text rows {}..{} ({} rows)", first, last, rows.len()));
    if first < 165 || first > 205 {
        return Err(format!(
            "the WELCOME panel's first text line is at y={}, not just below the sprite \
             top (167) — a stale scrollTop is being applied to a #adjust text member",
            first
        ).into());
    }
    if last < 380 {
        return Err(format!(
            "the WELCOME panel ends at y={}; its last line (\"Press ENTER to start \
             play.\") should sit near the sprite bottom (443)", last
        ).into());
    }

    // --- Frame 2: the game ------------------------------------------------
    // `keyPressed(RETURN)` is what the movie waits on; jump instead.
    let _ = player.eval("go(2)").await;
    // The frame loop blocks while a Ruffle instance is still loading, and the
    // gamevial logo SWF only settles once its AS-init watchdog fires (~3 s), so
    // give the playhead time to actually start ticking.
    player.step_frames(400).await;
    if player.current_frame() != 2 {
        return Err(format!("go(2) did not reach the game frame — on {}", player.current_frame()).into());
    }
    if global("lives") != "3" {
        return Err(format!("`birdflies` beginSprite never ran — lives is {}", global("lives")).into());
    }

    // `cloned` builds a 10x10 block grid plus wardens, chips, the poo and the bird.
    let world_models = ask!("member(\"world\").model.count");
    probe(format!("world models={} cameras={} textures={}",
        world_models, ask!("member(\"world\").camera.count"), ask!("member(\"world\").texture.count")));
    if world_models != "Int(110)" {
        return Err(format!("expected 110 models in the world, got {}", world_models).into());
    }

    // Defect 1: a cloned building must carry the shader the movie ASSIGNED to
    // its source model, with that shader's texture, not the file's Material #1.
    let block_shader = ask!("member(\"world\").model(\"blockx0y0\").shader.name");
    let block_tex = ask!("member(\"world\").model(\"blockx0y0\").shader.texture.name");
    probe(format!("cloned blockx0y0 shader={} tex={}", block_shader, block_tex));
    if block_shader.contains("Material") || block_tex.contains("default") {
        return Err(format!(
            "cloneModelFromCastmember dropped the source model's runtime shader: \
             blockx0y0 wears {} / {} instead of the textured shader startMovie put on \
             the city model — the whole city renders untextured white",
            block_shader, block_tex
        ).into());
    }
    let world_textures = ask!("member(\"world\").texture.count");
    if world_textures == "Int(0)" || world_textures == "Int(1)" {
        return Err(format!(
            "the clones brought no building textures into member(\"world\") (texture.count = {})",
            world_textures
        ).into());
    }

    // Defect 3: the poo-cam keeps the rect the movie gave it.
    let cam_rect = ask!("sprite(1).camera(2).rect");
    probe(format!("poo-cam name={} rect={}", ask!("sprite(1).camera(2).name"), cam_rect));
    if cam_rect != "IntRect(530, 270, 630, 370)" {
        return Err(format!(
            "sprite(1).camera(2).rect does not report the rect the movie set: {}", cam_rect
        ).into());
    }

    // …and the main view actually renders. The poo-cam looks at empty space
    // under the world, so drawing it full-viewport leaves the stage as flat
    // world bgColor rgb(105, 110, 110).
    player.step_frames(40).await;
    let ground_shot = player.snapshot_stage();
    let filled = non_background_ratio(&ground_shot.to_rgba(), 110, 470);
    probe(format!("non-background coverage of the 3D sprite = {:.3}", filled));
    if filled < 0.5 {
        return Err(format!(
            "the game view is blank ({:.3} of the 3D sprite differs from the world \
             bgColor). The second camera (the 100x100 poo-cam, clearAtRender = 0) is \
             being rendered over the whole sprite instead of into its own rect",
            filled
        ).into());
    }

    // --- The chips ---------------------------------------------------------
    // `cloned` drops the chips on the ground with a downward `modelsUnderRay`
    // from z = 500 and parks the model 5 units above the hit — all inside the
    // SAME beginSprite that has just moved the 100 buildings down to z = -1550
    // through `model.transform.position`. Against unflushed node transforms that
    // ray hit the buildings at their AUTHORED height, so the bag of chips ended
    // up 1555 units in the air with nothing under it.
    let chips_z = num(&ask!("member(\"world\").model(\"chips\").worldPosition.z"));
    let ground_z = num(&ask!(
        "member(\"world\").modelsUnderRay(member(\"world\").model(\"chips\").worldPosition + vector(0,0,50), vector(0,0,-1), 4, #detailed).getLast().isectPosition.z"));
    probe(format!("chips z={:?} ground z={:?}", chips_z, ground_z));
    match (chips_z, ground_z) {
        (Some(c), Some(g)) if (c - g).abs() <= 15.0 => {}
        (c, g) => {
            return Err(format!(
                "the bag of chips is at z={:?} but the ground under it is at z={:?}. \
                 `cloned` places it 5 units above a downward modelsUnderRay hit, so the \
                 ray answered from node transforms the same handler had already moved \
                 but that had not been flushed yet.", c, g
            ).into());
        }
    }

    // --- Flying -----------------------------------------------------------
    // The bird itself never moves: `birdflies` keeps it at the origin and
    // scrolls the world past it, steering the chase camera instead.
    let cam_before = ask!("string(sprite(1).camera.transform.position)");
    let bones_before = ask!("member(\"world\").model(\"BIRDY\").bonesPlayer.currentTime");
    player.key_down("ArrowUp", 126).await;
    let mut cam_moved = false;
    let mut anim_advanced = false;
    for i in 0..8 {
        player.step_frames(20).await;
        let cam = ask!("string(sprite(1).camera.transform.position)");
        let bones = ask!("member(\"world\").model(\"BIRDY\").bonesPlayer.currentTime");
        probe(format!("fly{} cam={} bones={} score={}", i, cam, bones, global("gscore")));
        if cam != cam_before { cam_moved = true; }
        if bones != bones_before { anim_advanced = true; }
    }
    player.key_up("ArrowUp", 126).await;

    if !cam_moved {
        return Err(format!(
            "holding UP never moved the chase camera — the exitFrame flight loop is \
             not running (camera still at {})", cam_before
        ).into());
    }
    if !anim_advanced {
        return Err(format!(
            "the bird's bonesPlayer never advanced past {} — the flap animation is \
             not being driven", bones_before
        ).into());
    }

    let snapshots = SnapshotContext::new(cfg.suite(), "flylikeabird");
    let _ = snapshots.verify_with_ratio("splash", splash_shot, 0.02);
    let _ = snapshots.verify_with_ratio("ground", ground_shot, 0.02);
    let _ = snapshots.verify_with_ratio("flight", player.snapshot_stage(), 0.02);

    Ok(())
});
