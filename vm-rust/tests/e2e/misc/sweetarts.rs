use vm_rust::browser_e2e_test;
use vm_rust::player::testing_shared::{SnapshotContext, SnapshotOutput, TestConfig, TestHarness};

const CONFIG: &str = include_str!("../configs/misc_sweetarts.toml");

fn probe(msg: String) {
    web_sys::console::log_1(&format!("[PROBE] {}", msg).into());
}

fn global(name: &str) -> String {
    use vm_rust::player::datum_formatting::format_concrete_datum;
    use vm_rust::player::reserve_player_ref;
    use vm_rust::player::symbols::symbol::Symbol;
    reserve_player_ref(|p| match p.globals.get(&Symbol::from_str(name)) {
        Some(r) => format_concrete_datum(&p.get_datum(r), p),
        None => "<unset>".to_string(),
    })
}

/// The 3DPR camera position/rotation the member carries, before any script runs.
fn member_camera_info(name: &str) -> Option<((f32, f32, f32), Option<(f32, f32, f32)>)> {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case(name) { continue }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                return w3d.info.camera_position.map(|p| (p, w3d.info.camera_rotation));
            }
        }
        None
    })
}

/// (parent node name, parent-relative position) of a node in member "w".
fn node_parent_and_local(node: &str) -> Option<(String, [f32; 3])> {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case("w") { continue }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                let scene = w3d.parsed_scene.as_ref()?;
                let n = scene.nodes.iter().find(|n| n.name.as_str().eq_ignore_ascii_case(node))?;
                let t = w3d.runtime_state.node_transforms.get(&n.name).unwrap_or(&n.transform);
                return Some((n.parent_name.as_str().to_string(), [t[12], t[13], t[14]]));
            }
        }
        None
    })
}

/// (changed, total) pixels between two stage snapshots inside a crop box.
fn changed_pixels(
    a: &SnapshotOutput, b: &SnapshotOutput,
    left: u32, top: u32, right: u32, bottom: u32,
) -> Option<(usize, usize)> {
    let decode = |s: &SnapshotOutput| -> Option<image::RgbaImage> {
        match s {
            SnapshotOutput::Base64Png(b64) => {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                    .ok().map(|i| i.to_rgba8())
            }
            SnapshotOutput::Rgba { width, height, data } =>
                image::RgbaImage::from_raw(*width, *height, data.clone()),
        }
    };
    let (ia, ib) = (decode(a)?, decode(b)?);
    if ia.dimensions() != ib.dimensions() { return None; }
    let (w, h) = ia.dimensions();
    let (mut n, mut total) = (0usize, 0usize);
    for y in top.min(h)..bottom.min(h) {
        for x in left.min(w)..right.min(w) {
            let (p, q) = (ia.get_pixel(x, y).0, ib.get_pixel(x, y).0);
            let d: i32 = (0..3).map(|k| (p[k] as i32 - q[k] as i32).abs()).sum();
            total += 1;
            if d > 30 { n += 1; }
        }
    }
    Some((n, total))
}

fn as_f64<E>(v: Result<vm_rust::director::static_datum::StaticDatum, E>) -> f64 {
    use vm_rust::director::static_datum::StaticDatum;
    match v {
        Ok(StaticDatum::Float(f)) => f,
        Ok(StaticDatum::Int(i)) => i as f64,
        Ok(StaticDatum::String(s)) => s.trim().parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}


// Wonka / Nestle "SweeTarts 3D Game" - Shockwave 3D, one world member ("w") that
// is torn down and rebuilt with `resetWorld` for the menu and for each track.
//
// Frames, observed by running it: 1-2 build the world, 3-5 the intro, 6 the menu
// loop ("Frame Behavior" 72 -> `menuupkeep` + `go(the frame)`), and a click walks
// `menumode` 2 -> 3 -> `go(the frame + 1)`. Frames 8-12 rebuild the world for the
// track; frame 13 is the game loop. The intro fly-around (`cameramoving`) is
// skipped by a mouseDown, which `smoothcamera` watches for.
//
// Guards TWO engine fixes, both about geometry anchored to the CAMERA:
//
// 1. The HUD. `initnumbers` (MovieScript 47) builds "LEVEL", the level digit and
//    six score digits as #plane models, positions each at world (x, 42, 188), and
//    parents them to camera[1] with the default `addChild` - #preserveWorld, per
//    the 11.5 dictionary - so the offset they keep for the rest of the game is
//    `inverse(cameraWorld) * thatWorld`, frozen at that instant. The camera at that
//    instant is whatever the member's 3DPR block stores, and `from_info` used to
//    reinterpret a stored position with x=0 and y=0 as "unset" and replace it with
//    an extruded-3D-text style frame-the-default-rect camera. (0, 0, 250) is
//    Director's DEFAULT camera position, so this member got (300, 250, 804.7) and
//    the HUD froze at (x - 335, -208, -616.7) - behind and below the eye. Nothing
//    drew.
//
// 2. The sky. `initworld` (MovieScript 44) makes `newModelResource("skybox",
//    #cylinder, #back)` of radius 6000 with a 12000-unit ground plane parented to
//    it. The renderer used to treat ANY model whose name contains "skybox" as a
//    camera-centred, depth-mask-off backdrop drawn past the far plane. That is a
//    rescue for boxes authored beyond the far plane (Rasterwerks, unicraft); here
//    it decoupled the wall from its own caps and stopped it occluding them, so the
//    ground plane's corners - which sit outside the 6000 wall and are meant to be
//    hidden by it - drew straight over the jungle backdrop.

/// The side wall of a runtime `#cylinder` resource, as the (u, v) the shader will
/// actually sample after its CLOD remap `(u+0.5, 0.5-v)` — first vertex of the top
/// ring and the vertex a quarter of the way around it.
fn cylinder_wall_uv(resource: &str) -> Option<((f32, f32), (f32, f32))> {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case("w") { continue }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                let found = (|| {
                    let scene = w3d.parsed_scene.as_ref()?;
                    let meshes = scene.clod_meshes.iter()
                        .find(|(k, _)| k.as_str().eq_ignore_ascii_case(resource))
                        .map(|(_, v)| v)?;
                    let wall = meshes.first()?;
                    let uvs = wall.tex_coords.first()?;
                    // The top ring is emitted first, one vertex per radial step.
                    let quarter = (wall.positions.len() / 2) / 4;
                    let remap = |uv: &[f32; 2]| (uv[0] + 0.5, 0.5 - uv[1]);
                    Some((remap(uvs.first()?), remap(uvs.get(quarter)?)))
                })();
                if found.is_some() { return found }
            }
        }
        None
    })
}


/// (mesh count, first face's winding) of a runtime primitive resource.
fn primitive_meshes(resource: &str) -> Option<(usize, [u32; 3])> {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case("w") { continue }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                let found = (|| {
                    let scene = w3d.parsed_scene.as_ref()?;
                    let meshes = scene.clod_meshes.iter()
                        .find(|(k, _)| k.as_str().eq_ignore_ascii_case(resource))
                        .map(|(_, v)| v)?;
                    Some((meshes.len(), *meshes.first()?.faces.first()?))
                })();
                if found.is_some() { return found }
            }
        }
        None
    })
}

/// Every face of a resource, as a set, plus the count — so a two-sided mesh can be
/// recognised by each face having a reverse-wound twin.
fn primitive_face_windings(resource: &str) -> Option<(usize, usize)> {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case("w") { continue }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                let found = (|| {
                    let scene = w3d.parsed_scene.as_ref()?;
                    let meshes = scene.clod_meshes.iter()
                        .find(|(k, _)| k.as_str().eq_ignore_ascii_case(resource))
                        .map(|(_, v)| v)?;
                    let faces: Vec<[u32; 3]> = meshes.iter().flat_map(|m| m.faces.iter().copied()).collect();
                    // Canonical key for a face ignoring rotation, and its mirror.
                    let key = |f: [u32; 3]| { let mut v = f; v.sort(); v };
                    let mut seen: std::collections::HashMap<[u32; 3], (usize, usize)> =
                        std::collections::HashMap::new();
                    for f in &faces {
                        // orientation: even permutation of the sorted order = one winding
                        let mut v = *f; v.sort();
                        let cw = (*f == [v[0], v[1], v[2]]) || (*f == [v[1], v[2], v[0]]) || (*f == [v[2], v[0], v[1]]);
                        let e = seen.entry(key(*f)).or_insert((0, 0));
                        if cw { e.0 += 1 } else { e.1 += 1 }
                    }
                    let twinned = seen.values().filter(|(a, b)| *a > 0 && *b > 0).count();
                    Some((faces.len(), twinned))
                })();
                if found.is_some() { return found }
            }
        }
        None
    })
}

/// How many distinct colours a coarse grid over the stage sees. A frame that is
/// one flat fill — the whole symptom this guards — reads 1.
fn distinct_colors(s: &SnapshotOutput) -> usize {
    let img = match s {
        SnapshotOutput::Base64Png(b64) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(b64).ok()
                .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
                .map(|i| i.to_rgba8())
        }
        SnapshotOutput::Rgba { width, height, data } =>
            image::RgbaImage::from_raw(*width, *height, data.clone()),
    };
    let Some(img) = img else { return 0 };
    let (w, h) = img.dimensions();
    let mut seen = std::collections::HashSet::new();
    for y in (0..h).step_by(8) {
        for x in (0..w).step_by(8) {
            let p = img.get_pixel(x, y).0;
            // Quantise so gradients in the sky don't count as "content".
            seen.insert((p[0] / 24, p[1] / 24, p[2] / 24));
        }
    }
    seen.len()
}


/// A named material's diffuse colour in a 3D member.
fn material_diffuse(member: &str, material: &str) -> Option<[f32; 4]> {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case(member) { continue }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                let found = w3d.parsed_scene.as_ref()
                    .and_then(|s| s.materials.iter().find(|mm| mm.name.as_str().eq_ignore_ascii_case(material)))
                    .map(|mm| mm.diffuse);
                if found.is_some() { return found }
            }
        }
        None
    })
}


/// Mean luminance of a crop of a stage snapshot.
fn mean_luma(s: &SnapshotOutput, left: u32, top: u32, right: u32, bottom: u32) -> f64 {
    let img = match s {
        SnapshotOutput::Base64Png(b64) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(b64).ok()
                .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
                .map(|i| i.to_rgba8())
        }
        SnapshotOutput::Rgba { width, height, data } =>
            image::RgbaImage::from_raw(*width, *height, data.clone()),
    };
    let Some(img) = img else { return -1.0 };
    let (w, h) = img.dimensions();
    let (mut sum, mut n) = (0.0f64, 0u32);
    for y in top.min(h)..bottom.min(h) {
        for x in left.min(w)..right.min(w) {
            let p = img.get_pixel(x, y).0;
            sum += 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
            n += 1;
        }
    }
    if n == 0 { -1.0 } else { sum / n as f64 }
}

/// Every light in a 3D member, as (name, type).
fn lights_of(member: &str) -> Vec<(String, String)> {
    vm_rust::player::reserve_player_ref(|player| {
        use vm_rust::player::cast_member::CastMemberType;
        for cast in player.movie.cast_manager.casts.iter() {
            for (_, m) in cast.members.iter() {
                if !m.name.eq_ignore_ascii_case(member) { continue }
                let CastMemberType::Shockwave3d(w3d) = &m.member_type else { continue };
                if let Some(sc) = w3d.parsed_scene.as_ref() {
                    return sc.lights.iter()
                        .map(|l| (l.name.as_str().to_string(), format!("{:?}", l.light_type)))
                        .collect();
                }
            }
        }
        Vec::new()
    })
}

browser_e2e_test!(test_misc_sweetarts_hud, |player| async move {
    let cfg = TestConfig::from_toml(CONFIG);
    cfg.apply_external_params();
    let movie_path = player.asset_path(&cfg.movie.path);

    player.load_movie(&movie_path).await;
    player.init_movie().await;

    // The seeded camera must be the member's own stored one, verbatim.
    let (pos, rot) = member_camera_info("w")
        .ok_or_else(|| "member \"w\" has no 3DPR camera info".to_string())?;
    probe(format!("3DPR camera: pos={:?} rot={:?}", pos, rot));
    let cam_x = as_f64(player.eval_datum("string(member(\"w\").camera[1].transform.position.x)").await);
    let cam_y = as_f64(player.eval_datum("string(member(\"w\").camera[1].transform.position.y)").await);
    let cam_z = as_f64(player.eval_datum("string(member(\"w\").camera[1].transform.position.z)").await);
    probe(format!("seeded camera = ({}, {}, {})", cam_x, cam_y, cam_z));
    let seeded_matches = (cam_x - pos.0 as f64).abs() < 0.01
        && (cam_y - pos.1 as f64).abs() < 0.01
        && (cam_z - pos.2 as f64).abs() < 0.01;
    if !seeded_matches {
        return Err(format!(
            "camera[1] was seeded at ({}, {}, {}) instead of the member's own stored {:?} - \
             the frame-the-default-rect rule belongs to extruded 3D text, not to a W3D world",
            cam_x, cam_y, cam_z, pos).into());
    }

    // Menu: `menuupkeep` ramps `gtimer`/`menumode` 0 -> 1 -> 2, then waits for a click.
    player.step_frames(60).await;
    for _ in 0..12 {
        if global("menumode") == "2" { break }
        player.step_frames(20).await;
    }
    if global("menumode") != "2" {
        return Err(format!("the menu never settled (menumode={})", global("menumode")).into());
    }
    if player.current_frame() != 6 {
        return Err(format!("expected the menu loop on frame 6, got {}", player.current_frame()).into());
    }

    // --- 3. The menu fade-up ends at full brightness ---
    // `menuupkeep` mode 1 does `gtimer = gtimer + 5`, writes
    // `menubackground.shader.emissive = rgb(gtimer, gtimer, gtimer)`, and only THEN
    // tests `gtimer > 255` — so the last value it ever writes is rgb(260, 260, 260).
    // Director truncates an out-of-range component into 0..255 (Scripting Dictionary,
    // `color()`); `as u8` wrapped it to rgb(4, 4, 4), and the backdrop snapped to
    // black at the exact moment the fade finished.
    let emissive = player.eval_datum("string(member(\"w\").shader(\"background\").emissive)").await
        .map_err(|e| format!("{:?}", e))?;
    let emissive = match emissive {
        vm_rust::director::static_datum::StaticDatum::String(s) => s,
        other => format!("{:?}", other),
    };
    probe(format!("menu backdrop emissive = {}", emissive));
    if !emissive.contains("255, 255, 255") {
        return Err(format!(
            "the menu backdrop finished its fade-up at {} instead of rgb(255, 255, 255) —              an out-of-range rgb() component must clamp, not wrap", emissive).into());
    }

    // --- 3. A #front cylinder wraps its texture the other way round than a #back one ---
    // `makeRoll` builds the candy roll as `newModelResource("TartsRoll", #cylinder,
    // #front)` and wraps the wrapper art around it. IFX writes the wall's `u` per
    // radial step and then `OrientPrimitiveToBeta4` rewrites every texcoord as
    // `u = 1 - u`; the sweep's own direction comes from the primitive's FACING byte
    // (`CIFXPrimitiveGenerator::MeshBuilder`) — #back counts u UP from 0, everything
    // else counts DOWN from 1 — so the two facings come out opposite. dirplayer only
    // ever emitted the #back spelling, so the level's `#cylinder, #back` sky wall was
    // right and the roll wore "SweeTarts" mirrored.
    match cylinder_wall_uv("TartsRoll") {
        Some((first, quarter)) => {
            probe(format!("TartsRoll wall uv: first={:?} quarter={:?}", first, quarter));
            if !(first.0 > 0.99 && quarter.0 < first.0) {
                return Err(format!(
                    "the #front candy roll wraps its wrapper the #back way round: u starts at                      {:.3} and reads {:.3} a quarter turn on, where it should start at 1 and                      count down", first.0, quarter.0).into());
            }
            if !(first.1 < 0.01) {
                return Err(format!(
                    "the cylinder wall's top ring is at v={:.3}, not 0 — the wrapper is                      upside down along the tube", first.1).into());
            }
        }
        None => return Err("makeRoll never built the TartsRoll cylinder".to_string().into()),
    }

    player.key_down("3", 51).await;

    // "Frame Behavior" 72's `on mouseUp`, outside sprite 2 (the instructions
    // button), advances into the track.
    player.click(320, 300).await;
    player.step_frames(30).await;
    if player.current_frame() != 13 {
        return Err(format!(
            "clicking the menu did not reach the game loop - on frame {}",
            player.current_frame()).into());
    }
    // "Frame Behavior" 72's `on mouseUp` picks the track from `the keyPressed`:
    //     gk = value(the keyPressed)
    //     if gk > 0 then tracknum = min(gk, 6) else tracknum = 1
    // so the digit has to be HELD across the click — hence the key_down above and the
    // key_up below. Track 3 is the underwater level, whose mascot (`MCnum` follows
    // `tracknum`) is the bubble.
    player.key_up("3", 51).await;

    // --- 4. The intro fly-around can see the level ---
    // `smoothcamera` mode 1 orbits at radius 3000 and y = 700 + 3000*cos(t), so it
    // starts ABOVE the sky box looking down through its open top, with the level
    // fading in through near-black fog (`fog.near` 3000 / `fog.far` 3500, colour
    // black, both walked outward each frame).
    //
    // `initworld` builds the box's caps as `newModelResource("planepanel", #plane,
    // #front)`. `CIFXPlanePrimitive::GenerateMesh` builds ONE sheet and takes its
    // normal from the facing byte — two meshes are the no-facing case alone — but the
    // dimension-setter rebuild emitted both regardless, so setting width/length (the
    // normal thing to do) silently made every explicitly-faced plane two-sided. The
    // CEILING cap came back visible from ABOVE and the intro was a flat sheet of it:
    // the level "cut in from blue" with no fog fade at all.
    match primitive_meshes("planepanel") {
        Some((count, winding)) => {
            probe(format!("planepanel: {} mesh(es), first face {:?}", count, winding));
            if count != 1 {
                return Err(format!(
                    "the #front sky caps rebuilt as {} meshes — an explicitly faced #plane is                      one sheet, and a two-sided ceiling hides the whole intro", count).into());
            }
            // The sheet Director shows only from inside the box: visible from -Z here.
            if winding != [0, 2, 1] {
                return Err(format!(
                    "the #front sky caps came out wound {:?}, i.e. visible from the wrong side                      — the ceiling would face out of the box and the ground into it", winding).into());
            }
        }
        None => return Err("initworld never built the planepanel caps".to_string().into()),
    }
    let intro = player.snapshot_stage();
    let colors = distinct_colors(&intro);
    probe(format!("intro frame: {} distinct colours, cameramoving={}", colors, global("cameramoving")));
    if colors < 4 {
        return Err(format!(
            "the intro fly-around is a flat {}-colour fill — the camera is looking at the              inside face of something instead of down at the level", colors).into());
    }

    // `smoothcamera` ends the intro fly-around on `the mouseDown`; `gameLoop` then
    // sets `startyet` once the camera has settled behind the mascot.
    player.mouse_down(320, 300).await;
    player.step_frames(4).await;
    player.mouse_up(320, 300).await;
    for _ in 0..20 {
        if global("startyet") == "1" { break }
        player.step_frames(20).await;
    }
    if global("startyet") != "1" {
        return Err("the camera never handed over to gameplay (startyet stayed 0)".to_string().into());
    }

    // --- 5. A clone's colour writes stay on the clone ---
    // `initworld` clones the level in FIRST, so it owns plain `Material01` /
    // `Material02`; the six collectible letters clone in after it, collide, and are
    // renamed `Material01-clone1..6`. `cloneModelFromCastmember` renamed the material
    // alongside the shader but left the cloned shader pointing at the SOURCE material
    // name, so each letter's `shader.diffuse = <its colour>` walked that name and
    // repainted the TRACK. It ended up wearing the last letter's grey, and the sides
    // of the walkway — which carry no texture layer, so the material colour is all
    // there is — went flat grey instead of wood brown. (`merge.rs` remaps
    // `shader.material_name` properly; only this hand-rolled copy did not.)
    // Read the colour through the MODEL, not through a material NAME. Which clone owns
    // the plain "Material01" depends on which cloned FIRST, and a letter can beat the
    // track to it — asserting on the name made this flaky without saying anything about
    // the engine.
    let track_diffuse = player.eval_datum("string(member(\"w\").model(\"plat001\").shader.diffuse)").await
        .map_err(|e| format!("{:?}", e))?;
    let track_diffuse = match track_diffuse {
        vm_rust::director::static_datum::StaticDatum::String(s) => s,
        other => format!("{:?}", other),
    };
    let track_shader = player.eval_datum("string(member(\"w\").model(\"plat001\").shader.name)").await
        .map_err(|e| format!("{:?}", e))?;
    let track_shader = match track_shader {
        vm_rust::director::static_datum::StaticDatum::String(s) => s,
        other => format!("{:?}", other),
    };
    probe(format!("track: tracknum={} shader={} diffuse={}",
        global("tracknum"), track_shader.trim(), track_diffuse));
    // Only tracks 1 and 2 keep the geometry's AUTHORED material — `initworld`'s `case
    // tracknum` arms for 3..6 assign their own "plattext" shader over it (track 3, for
    // one, sets `diffuse = rgb(255,255,255)` deliberately). Assert the clone kept its
    // source colour only where the movie has not deliberately replaced it.
    if !track_shader.trim().eq_ignore_ascii_case("plattext") && !track_diffuse.contains("138, 96, 50") {
        return Err(format!(
            "the track is wearing {} instead of its authored rgb(138, 96, 50) — a later              clone's colour write is landing on its material", track_diffuse.trim()).into());
    }
    // ...and the letters kept their own colours rather than all sharing one material.
    let letters: Vec<[f32; 4]> = (1..=6)
        .filter_map(|i| material_diffuse("w", &format!("Material01-clone{}", i)))
        .collect();
    let distinct = letters.iter().filter(|c| {
        letters.iter().filter(|d| (0..3).map(|i| (c[i] - d[i]).abs()).sum::<f32>() < 0.01).count() == 1
    }).count();
    probe(format!("letter materials: {} of {} distinct", distinct, letters.len()));
    if letters.len() >= 2 && distinct + 1 < letters.len() {
        return Err(format!(
            "only {} of the {} renamed letter materials are distinct — the clones are              still sharing one material", distinct, letters.len()).into());
    }

    // The walkway is lit, not a silhouette. It fills the bottom-centre of the chase
    // camera's view and is the only large surface in frame that depends on real
    // lighting (the ground, sky and mascot are all emissive).
    player.step_frames(60).await;
    let lit = player.snapshot_stage();
    let track = mean_luma(&lit, 220, 380, 380, 470);
    probe(format!("walkway mean luma = {:.1}", track));
    if track < 40.0 {
        return Err(format!(
            "the walkway renders at mean luma {:.1} — it is being lit by nothing that              points at it", track).into());
    }

    // A resource authored #both must be TWO-SIDED. `constructMC` case 3 builds the
    // mascot as `newModelResource("bubble", #sphere, #both)`, and Director generates the
    // reverse-wound copy of every face; the #sphere generator here ignored the facing byte
    // entirely (unlike #cylinder and #plane) and always emitted a single shell. That is
    // visible precisely because the bubble is TRANSLUCENT: Director composites its
    // reflection through both hemispheres, so the highlights read as broad soft haloes
    // over a gently tinted disc. With one shell we composited once and got isolated hard
    // dots on an otherwise invisible sphere — measured against Director's own coverage
    // map, ours was missing most of the mid-range coverage.
    match primitive_face_windings("bubble") {
        Some((faces, twinned)) => {
            probe(format!("bubble mesh: {} faces, {} with a reverse-wound twin", faces, twinned));
            if twinned * 2 < faces / 2 {
                return Err(format!(
                    "the #both bubble came out single-sided ({} of {} faces have a \
                     reverse-wound twin) - its reflection composites through one hemisphere \
                     instead of two", twinned, faces).into());
            }
        }
        None => return Err("constructMC never built the bubble sphere".to_string().into()),
    }

    // --- 6. The #inker modifier draws a THIN rim, not a solid disc ---
    // `constructMC` case 3 builds the level-3 mascot as `newModelResource("bubble",
    // #sphere, #both)` radius 25 with a #standard shader, then MovieScript 46's
    // `addinker`: `addModifier(#inker)` / `lineColor = rgb(255,255,255)` /
    // `silhouettes = 1` / `lineOffset = -10`. In Director that is a see-through cyan
    // bubble with a ~1px white circle round it.
    //
    // Toggle the modifier off and diff the frame. The 3D member only re-renders on a
    // step, so both shots are taken one frame apart; a control crop of the sky well away
    // from the mascot proves the rest of the frame held still between them.
    //
    // This catches both ways the pass has been wrong. It drew NOTHING while the modifier
    // was never wired in, and it drew a SOLID WHITE DISC once it was: the pass expanded
    // the hull's near faces, because `cull_face(FRONT)` is this renderer's Y-flipped
    // default for ordinary geometry rather than the "draw back faces" its comment
    // claimed, so the hull covered the model at every pixel regardless of depth.
    let silhouettes = as_f64(player.eval_datum("string(member(\"w\").model(\"bubble\").inker.silhouettes)").await);
    if silhouettes != 1.0 {
        return Err(format!(
            "the bubble's #inker modifier reads silhouettes={} - `addinker` never reached \
             the model, so the rim below proves nothing", silhouettes).into());
    }
    player.step_frames(1).await;
    let with_rim = player.snapshot_stage();
    for off in ["silhouettes = 0", "boundary = 0"] {
        player.eval(&format!("member(\"w\").model(\"bubble\").inker.{}", off)).await
            .map_err(|e| format!("could not switch the inker off: {:?}", e))?;
    }
    player.step_frames(1).await;
    let without_rim = player.snapshot_stage();
    for on in ["silhouettes = 1", "boundary = 1"] {
        player.eval(&format!("member(\"w\").model(\"bubble\").inker.{}", on)).await
            .map_err(|e| format!("could not switch the inker back on: {:?}", e))?;
    }

    let (rim, stage) = changed_pixels(&with_rim, &without_rim, 0, 0, 10_000, 10_000)
        .ok_or_else(|| "could not decode the stage snapshots".to_string())?;
    // Sky, top-right: no mascot, no HUD, no walkway.
    let (sky, sky_total) = changed_pixels(&with_rim, &without_rim, 460, 60, 600, 170)
        .ok_or_else(|| "could not decode the stage snapshots".to_string())?;
    probe(format!("inker rim: {}/{} stage pixels, control sky {}/{}", rim, stage, sky, sky_total));
    if sky * 50 > sky_total {
        return Err(format!(
            "the sky moved between the two inker shots ({}/{} pixels) - the frame is not \
             static, so the rim measurement means nothing", sky, sky_total).into());
    }
    if rim == 0 {
        return Err("switching the bubble's #inker off changed nothing - the modifier \
                    draws no silhouette at all".to_string().into());
    }
    // The bubble is a radius-25 sphere a few hundred units from a chase camera: its disc
    // is a good fraction of the frame, its 1.5px rim a low single-digit percent of that
    // disc. Anything above 2% of the stage is the hull filling the silhouette instead of
    // outlining it - the solid-white-disc symptom.
    if rim * 50 > stage {
        return Err(format!(
            "the #inker changed {}/{} stage pixels - that is the bubble filled in, not a \
             thin rim around it", rim, stage).into());
    }

    // --- 1. The HUD is parented to the camera and lands on screen ---
    let fov = as_f64(player.eval_datum("string(member(\"w\").camera[1].fieldOfView)").await);
    let (sl, st, sr, sb) = player.sprite_rect(1).await?;
    let aspect = (sr - sl) as f64 / ((sb - st) as f64).max(1.0);
    let half_v = (fov.to_radians() / 2.0).tan();
    probe(format!("fov={} sprite1={}x{} aspect={:.3}", fov, sr - sl, sb - st, aspect));

    for hud in ["levelTitle", "levelnum", "scorenum1", "scorenum6"] {
        let (parent, local) = node_parent_and_local(hud)
            .ok_or_else(|| format!("{} is not in the scene - initnumbers never ran", hud))?;
        probe(format!("{}: parent={} local={:?}", hud, parent, local));
        if !parent.eq_ignore_ascii_case("DefaultView") {
            return Err(format!(
                "{} should hang off the camera (initnumbers does `c.addChild(nm)`), got parent {}",
                hud, parent).into());
        }
        // Director cameras look down their own -Z.
        let depth = -local[2] as f64;
        if depth <= 0.0 {
            return Err(format!(
                "{} sits BEHIND the camera at local z={} - the camera the HUD was frozen \
                 against is not the one the member stores", hud, local[2]).into());
        }
        let (hx, hy) = (depth * half_v * aspect, depth * half_v);
        if local[0].abs() as f64 > hx || local[1].abs() as f64 > hy {
            return Err(format!(
                "{} is off screen: local ({}, {}) at depth {:.1} vs half-extents ({:.1}, {:.1})",
                hud, local[0], local[1], depth, hx, hy).into());
        }
    }

    // --- 2. The sky cylinder is ordinary world geometry ---
    // It keeps its own hierarchy, and the ground cap reaches further than the wall
    // (half-diagonal 8485 vs radius 6000), so the wall has to occlude the corners -
    // which it can only do while it renders with depth writes and real parallax.
    for (node, expect_parent) in [("skybox", "world"), ("skytop", "skybox"), ("skybottom", "skybox")] {
        let (parent, _) = node_parent_and_local(node)
            .ok_or_else(|| format!("{} is not in the scene", node))?;
        if !parent.eq_ignore_ascii_case(expect_parent) {
            return Err(format!("{} should be parented to {}, got {}", node, expect_parent, parent).into());
        }
    }
    match cylinder_wall_uv("skybox") {
        Some((first, quarter)) => {
            probe(format!("skybox wall uv: first={:?} quarter={:?}", first, quarter));
            if !(first.0 < 0.01 && quarter.0 > first.0) {
                return Err(format!(
                    "the #back sky wall no longer counts u UP from 0 (starts {:.3}, quarter                      {:.3}) — its panorama is now mirrored", first.0, quarter.0).into());
            }
        }
        None => return Err("initworld never built the skybox cylinder".to_string().into()),
    }

    let radius = as_f64(player.eval_datum("string(member(\"w\").model(\"skybox\").resource.topRadius)").await);
    let ground = as_f64(player.eval_datum("string(member(\"w\").model(\"skybottom\").resource.width)").await);
    let yon = as_f64(player.eval_datum("string(member(\"w\").camera[1].yon)").await);
    probe(format!("sky: wall radius={} ground width={} yon={}", radius, ground, yon));
    if !(radius > 0.0) || !(ground > 0.0) {
        return Err("the sky cylinder / ground plane lost their authored dimensions".to_string().into());
    }
    if ground / 2.0 * std::f64::consts::SQRT_2 <= radius {
        return Err(format!(
            "the ground plane ({}) no longer reaches past the sky wall ({}) - this movie's \
             occlusion case is gone and the assertion below means nothing", ground, radius).into());
    }
    if radius >= yon {
        return Err(format!(
            "the sky wall (radius {}) no longer fits inside the camera's far plane ({}) - \
             the renderer's sky override would legitimately take it over", radius, yon).into());
    }

    // ...and it is actually DRAWN there. Take the two "LEVEL 1" planes out of the
    // world and the top-left corner of the frame has to change; a control crop of
    // the track below it must not. This is the whole visible symptom, and unlike a reference
    // image it does not care where the chase camera happens to be.
    let before = player.snapshot_stage();
    for hud in ["levelTitle", "levelnum"] {
        player.eval(&format!("member(\"w\").model(\"{}\").removeFromWorld()", hud)).await
            .map_err(|e| format!("could not hide {}: {:?}", hud, e))?;
    }
    player.step_frames(1).await;
    let after = player.snapshot_stage();

    let (hud_changed, hud_total) = changed_pixels(&before, &after, 4, 4, 140, 44)
        .ok_or_else(|| "could not decode the stage snapshots".to_string())?;
    let (ctrl_changed, ctrl_total) = changed_pixels(&before, &after, 200, 300, 400, 460)
        .ok_or_else(|| "could not decode the stage snapshots".to_string())?;
    probe(format!("hiding the HUD changed: corner {}/{} control {}/{}",
        hud_changed, hud_total, ctrl_changed, ctrl_total));

    if ctrl_changed * 50 > ctrl_total {
        return Err(format!(
            "hiding the HUD disturbed the track as well ({}/{} pixels) - the frame moved              between the two captures, so the corner comparison proves nothing",
            ctrl_changed, ctrl_total).into());
    }
    if hud_changed * 20 < hud_total {
        return Err(format!(
            "hiding \"LEVEL\" and the level digit changed only {}/{} pixels of the top-left              corner - the HUD planes are not being drawn there",
            hud_changed, hud_total).into());
    }

    // --- 3. The sky wall occludes what lies beyond it ---
    // Park the camera inside the cylinder at (1000, 300, 1000), aimed down the +X/+Z
    // diagonal and away from the mascot. The ground cap reaches 8485 units that way
    // and the wall only 6000, so everything past the wall has to be jungle backdrop —
    // which is only true while the wall renders with depth writes and real parallax. `snapshot_stage` re-draws the CURRENT player
    // state without stepping, so this shot carries no elapsed time and no mascot
    // motion — it is a stable reference image.
    let cx = as_f64(player.eval_datum("string(member(\"w\").camera[1].getWorldTransform().position.x)").await);
    let cy = as_f64(player.eval_datum("string(member(\"w\").camera[1].getWorldTransform().position.y)").await);
    let cz = as_f64(player.eval_datum("string(member(\"w\").camera[1].getWorldTransform().position.z)").await);
    player.eval(&format!("member(\"w\").camera[1].translate({}, {}, {}, #world)",
        1000.0 - cx, 300.0 - cy, 1000.0 - cz)).await
        .map_err(|e| format!("could not park the camera: {:?}", e))?;
    player.eval("member(\"w\").camera[1].pointAt(vector(9000, 0, 9000), vector(0, 1, 0))").await
        .map_err(|e| format!("could not aim the camera: {:?}", e))?;
    let horizon = player.snapshot_stage();
    let mut snaps = SnapshotContext::new(cfg.suite(), "sweetarts");
    snaps.pixel_tolerance = 6;
    snaps.verify("horizon", horizon)?;

    Ok(())
});
