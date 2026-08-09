use std::collections::VecDeque;

use log::{debug, warn};
use wasm_bindgen::JsCast;

use crate::{
    console_warn,
    director::lingo::datum::Datum,
    player::{
        cast_lib::CastMemberRef,
        cast_member::{CastMemberType, Shockwave3dMember, Text3dSource, Text3dState},
        reserve_player_mut, reserve_player_ref,
        symbols::{builtin::BuiltInSymbol, symbol::Symbol},
        DatumRef, DirPlayer, ScriptError,
    },
};

const W3D_HANDLER_LOG: bool = false;

/// Director's W3D motion collection has an implicit default motion at index 1,
/// so authored motions start at index 2. dirplayer's scene.motions holds only
/// authored motions, so the accessors below synthesise this default at index 1
/// to keep `member.motion[i]` / `.count` aligned with Director (mirrors the way
/// the camera accessor inserts DefaultView as camera[1]). Without it,
/// e.g. Rasterwerks' `m.motion[3].name` returned the wrong (3rd authored) motion
/// and every actor cloned a non-skeletal motion → T-pose.
// Director spells its built-in motion "DefaultMotion", with no space — scripts
// that compare motion[1].name against it depend on the exact spelling.
const DEFAULT_MOTION_NAME: &str = "DefaultMotion";

fn log(msg: &str) {
    if W3D_HANDLER_LOG {
        debug!("[W3D-HANDLER] {}", msg);
    }
}

pub struct Shockwave3dMemberHandlers {}

impl Shockwave3dMemberHandlers {
    /// `member(x).loadFile(fileName {, overwrite, generateUniqueNames})`
    ///
    /// Director 11.5 Scripting Dictionary, `loadFile()`: imports the assets of a
    /// W3D file into a 3D cast member. `overwrite` (default TRUE) replaces the
    /// member's assets rather than adding to them; `generateUniqueNames`
    /// (default TRUE) renames incoming elements that collide with existing ones.
    ///
    /// age_of_speed loads every level this way — `member(11 + gLevelID)
    /// .loadFile("data/level_1.W3D")` in "Frame Load Data", plus the shared
    /// materials and sky members — so with this stubbed out the track had no
    /// models at all.
    pub async fn load_file(
        member_ref: &CastMemberRef,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let (file_name, overwrite, generate_unique_names) = reserve_player_ref(|player| {
            let file_name = args
                .get(0)
                .map(|a| player.get_datum(a).string_value().unwrap_or_default())
                .unwrap_or_default();
            // Both flags default to TRUE per the dictionary.
            let flag = |i: usize| {
                args.get(i)
                    .map(|a| player.get_datum(a).int_value().unwrap_or(1) != 0)
                    .unwrap_or(true)
            };
            (file_name, flag(1), flag(2))
        });

        if file_name.is_empty() {
            return Err(ScriptError::new(
                "loadFile requires a file name".to_string(),
            ));
        }

        // Fetch through NetManager, same as importFileInto.
        let task_id = reserve_player_mut(|player| {
            player.net_manager.preload_net_thing(file_name.clone())
        });
        {
            let player = unsafe { crate::player::player_mut() };
            if !player.net_manager.is_task_done(Some(task_id)) {
                player.net_manager.await_task(task_id).await;
            }
        }
        let bytes = reserve_player_ref(|player| player.net_manager.get_task_result(Some(task_id)));
        let bytes = match bytes {
            Some(Ok(b)) if !b.is_empty() => b,
            _ => {
                warn!("loadFile: could not fetch '{}'", file_name);
                return reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Void)));
            }
        };

        let scene = match crate::director::chunks::w3d::parse_w3d(&bytes) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "loadFile: failed to parse '{}' ({} bytes): {}",
                    file_name, bytes.len(), e
                );
                return reserve_player_mut(|player| Ok(player.alloc_datum(Datum::Void)));
            }
        };
        debug!(
            "loadFile: '{}' -> {} nodes, {} model_resources, {} shaders (overwrite={}, uniqueNames={})",
            file_name, scene.nodes.len(), scene.model_resources.len(), scene.shaders.len(),
            overwrite, generate_unique_names
        );

        reserve_player_mut(|player| {
            let member = player
                .movie
                .cast_manager
                .find_mut_member_by_ref(member_ref)
                .ok_or_else(|| ScriptError::new("loadFile: member not found".to_string()))?;
            let w3d = member
                .member_type
                .as_shockwave3d_mut()
                .ok_or_else(|| {
                    ScriptError::new("loadFile: member is not a Shockwave3D member".to_string())
                })?;

            if overwrite || w3d.parsed_scene.is_none() {
                // Replace outright, and drop runtime state that named the old
                // scene's models/shaders (per-node shader overrides, animation
                // players, …) — those names no longer exist.
                let info = w3d.info.clone();
                let rc_scene = std::rc::Rc::new(scene);
                w3d.runtime_state = crate::player::cast_member::Shockwave3dRuntimeState::from_info(
                    &info,
                    Some(&rc_scene),
                );
                w3d.source_scene = Some(rc_scene.clone());
                w3d.parsed_scene = Some(rc_scene);
            } else if let Some(existing) = w3d.scene_mut() {
                existing.merge_from(scene, generate_unique_names);
            }
            Ok(player.alloc_datum(Datum::Void))
        })
    }

    fn native_text_alignment(alignment: BuiltInSymbol) -> crate::player::handlers::datum_handlers::cast_member::font::TextAlignment {
        use crate::player::handlers::datum_handlers::cast_member::font::TextAlignment;

        match alignment.to_ascii_lowercase().as_str() {
            "center" => TextAlignment::Center,
            "right" => TextAlignment::Right,
            "justify" => TextAlignment::Justify,
            _ => TextAlignment::Left,
        }
    }

    fn build_fallback_text_spans(
        text_content: &str,
        font_name: &str,
        font_size: u16,
        spans: &[crate::player::handlers::datum_handlers::cast_member::font::StyledSpan],
    ) -> Vec<crate::player::handlers::datum_handlers::cast_member::font::StyledSpan> {
        use crate::player::handlers::datum_handlers::cast_member::font::{HtmlStyle, StyledSpan};

        if !spans.is_empty() {
            return spans.to_vec();
        }

        vec![StyledSpan {
            text: text_content.to_string(),
            style: HtmlStyle {
                font_face: Some(font_name.to_string()),
                font_size: Some(font_size as i32),
                color: Some(0xFFFFFF),
                ..Default::default()
            },
        }]
    }

    /// XMED stores font cell height (ascent+descent) rather than point/em size.
    /// Use canvas font metrics to convert back to the actual point size.
    fn xmed_cell_height_to_point_size(cell_height: i32, font_face: &str) -> i32 {
        if cell_height <= 0 { return cell_height; }
        let ref_size = 100.0_f64;
        let doc = match web_sys::window().and_then(|w| w.document()) {
            Some(d) => d,
            None => return cell_height,
        };
        let canvas: web_sys::HtmlCanvasElement = match doc.create_element("canvas")
            .ok().and_then(|e| e.dyn_into().ok()) {
            Some(c) => c,
            None => return cell_height,
        };
        let ctx: web_sys::CanvasRenderingContext2d = match canvas.get_context("2d")
            .ok().flatten().and_then(|c| c.dyn_into().ok()) {
            Some(c) => c,
            None => return cell_height,
        };
        let font_str = format!("{}px {}", ref_size as i32, font_face);
        ctx.set_font(&font_str);
        let metrics = match ctx.measure_text("M") {
            Ok(m) => m,
            Err(_) => return cell_height,
        };
        let ascent = metrics.font_bounding_box_ascent();
        let descent = metrics.font_bounding_box_descent();
        let measured_height = ascent + descent;
        if measured_height <= 0.0 || measured_height <= ref_size {
            return cell_height; // metrics unavailable or font has no extra leading
        }
        let ratio = measured_height / ref_size;
        let point_size = (cell_height as f64 / ratio).round() as i32;
        point_size.max(1)
    }

    fn scale_native_spans(
        spans: &[crate::player::handlers::datum_handlers::cast_member::font::StyledSpan],
        scale: i32,
        fallback_font_size: u16,
    ) -> Vec<crate::player::handlers::datum_handlers::cast_member::font::StyledSpan> {
        let scale = scale.max(1);
        spans
            .iter()
            .cloned()
            .map(|mut span| {
                let base_size = span.style.font_size.unwrap_or(fallback_font_size as i32).max(1);
                span.style.font_size = Some(base_size * scale);
                span
            })
            .collect()
    }

    fn native_text_supersample(smoothness: u32) -> i32 {
        // The native-font path traces the rasterised glyph at this resolution, so
        // the extruded silhouette (and especially the per-edge tunnel side walls)
        // is only as smooth as the supersample: too low and curved glyphs get a
        // coarse contour whose few large side quads don't cover the wall smoothly,
        // reading as stair-steps / gaps next to the front face. The mesh is rebuilt
        // only on change (not per frame), so a high supersample is affordable.
        // Director smoothness 0..10 → 6..10.
        (6 + (smoothness as i32 / 2)).clamp(6, 10)
    }

    fn render_native_text_bitmap(
        source: &Text3dSource,
        smoothness: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        use crate::player::handlers::datum_handlers::cast_member::font::FontMemberHandlers;

        let supersample = Self::native_text_supersample(smoothness);
        let bw = (source.width as i32).max(128) * supersample;
        let bh = (source.height as i32).max(32) * supersample;
        // Correct XMED cell-height values to actual point sizes before scaling
        let corrected_spans: Vec<_> = source.spans.iter().cloned().map(|mut span| {
            if let Some(sz) = span.style.font_size {
                let font_face = span.style.font_face.as_deref().unwrap_or("Arial");
                span.style.font_size = Some(Self::xmed_cell_height_to_point_size(sz, font_face));
            }
            span
        }).collect();
        let corrected_font_size = {
            let font_face = source.spans.first()
                .and_then(|s| s.style.font_face.as_deref())
                .unwrap_or("Arial");
            Self::xmed_cell_height_to_point_size(source.font_size as i32, font_face) as u16
        };
        let scaled_spans = Self::scale_native_spans(&corrected_spans, supersample, corrected_font_size);
        let alignment = Self::native_text_alignment(source.alignment);
        let scaled_tab_stops: Vec<crate::player::cast_member::TabStop> = source
            .tab_stops
            .iter()
            .cloned()
            .map(|mut stop| {
                stop.position *= supersample;
                stop
            })
            .collect();

        let mut bitmap = crate::player::bitmap::bitmap::Bitmap::new(
            bw as u16,
            bh as u16,
            32,
            32,
            8,
            crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
                crate::player::bitmap::bitmap::get_system_default_palette(),
            ),
        );
        bitmap.use_alpha = true;
        // Bitmap::new initializes 32-bit surfaces to opaque white. For native glyph
        // extrusion we need a transparent background, otherwise the alpha-mask
        // builder sees the entire text box as solid.
        bitmap.data.fill(0);
        let _ = FontMemberHandlers::render_native_text_to_bitmap(
            &mut bitmap,
            &scaled_spans,
            0,
            0,
            bw,
            bh,
            alignment,
            bw,
            source.word_wrap,
            None,
            source.fixed_line_space.saturating_mul(supersample as u16),
            source.top_spacing.saturating_mul(supersample as i16),
            source.bottom_spacing.saturating_mul(supersample as i16),
            &scaled_tab_stops,
            &[],
            &[],
        );
        Some((bw as u32, bh as u32, bitmap.data))
    }

    fn rebuild_native_text_mesh(w3d: &mut Shockwave3dMember) {
        let (source, state) = match (&w3d.text3d_source, &w3d.text3d_state) {
            (Some(source), Some(state)) if source.native_alpha_mesh => (source.clone(), state.clone()),
            _ => return,
        };

        let Some((bw, bh, rgba)) = Self::render_native_text_bitmap(&source, state.smoothness) else {
            return;
        };

        if let Some(scene) = w3d.scene_mut() {
            let mesh = crate::director::chunks::w3d::primitives::extrude_alpha_mask_to_mesh(
                bw,
                bh,
                &rgba,
                source.width as f32,
                source.height as f32,
                state.tunnel_depth,
                state.bevel_depth,
                state.bevel_type,
                state.smoothness,
            );
            scene.clod_meshes.insert(Symbol::builtin(BuiltInSymbol::Text), vec![mesh]);
            scene.mesh_content_version += 1;
        }
    }

    /// Build an extruded 3D glyph mesh from a Text3D source + state (the same
    /// native-alpha-mask extrusion used by the #mode3D path). Shared by
    /// member.extrude3d(scene) and the resulting resource's geometry setters.
    pub(crate) fn build_text3d_mesh(
        source: &crate::player::cast_member::Text3dSource,
        state: &crate::player::cast_member::Text3dState,
    ) -> Option<crate::director::chunks::w3d::types::ClodDecodedMesh> {
        use crate::director::chunks::w3d::text3d;

        // displayFace bitmask: bit0=#front, bit1=#tunnel, bit2=#back; -1 = all.
        let df = state.display_face;
        let params = text3d::ExtrudeParams {
            depth: state.tunnel_depth.max(1.0),
            bevel_type: state.bevel_type,
            bevel_depth: state.bevel_depth.max(0.0),
            smoothness: state.smoothness,
            front: df == -1 || (df & 1) != 0,
            tunnel: df == -1 || (df & 2) != 0,
            back: df == -1 || (df & 4) != 0,
        };

        // Render at a high supersample for a smooth re-vectorised outline (the
        // marching-squares trace is only as smooth as the rasterised source).
        let (bw, bh, rgba) = Self::render_native_text_bitmap(source, state.smoothness.max(8))?;
        // No-PFR (system/native font) path: there are no embedded glyph outlines,
        // so re-vectorise the rasterised text into clean contours, then run the
        // SAME contour pipeline as the PFR path (caps-with-holes + tunnel + bevel).
        // This replaces the old drop-shadow alpha mesh, so system-font 3D text
        // extrudes properly (frog01 title). px→model maps like the alpha-mask path
        // (Y flipped to Y-up).
        let ww = (source.width.max(1)) as f32;
        let wh = (source.height.max(1)) as f32;
        let pw = ww / bw.max(1) as f32;
        let ph = wh / bh.max(1) as f32;
        // Simplify tolerance ≈ 1 model unit (supersample = bw/ww pixels per model unit).
        // supersample px per model unit; blur ~⅓ of that smooths the AA iso-edge.
        let ss = (bw as f32 / ww).max(1.0);
        let blur_radius = (ss / 3.0).round().max(1.0) as usize;
        let eps_px = ss * 0.3;
        let contours_px = text3d::vectorize_alpha(bw, bh, &rgba, 128, eps_px, 2, blur_radius);
        let contours: Vec<Vec<[f32; 2]>> = contours_px
            .iter()
            .map(|c| c.iter().map(|p| [p[0] * pw, wh - p[1] * ph]).collect())
            .collect();
        if contours.is_empty() {
            return None;
        }
        // Raster-vectorised fallback uses the same extrude params built above.
        let (positions, normals, faces) = text3d::extrude_glyph(&contours, &params);
        if positions.is_empty() {
            return None;
        }
        let mut mesh = crate::director::chunks::w3d::types::ClodDecodedMesh::default();
        mesh.name = Symbol::from_str(&"Text".to_string());
        mesh.positions = positions;
        mesh.normals = normals;
        mesh.faces = faces;
        Some(mesh)
    }

    /// Re-extrude an extrude3d resource (keyed by `resname`) into `member`'s
    /// scene from the retained source + state.
    pub(crate) fn rebuild_extruded_text(player: &mut DirPlayer, member_ref: &CastMemberRef, resname: Symbol) {
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                if let Some((source, state)) = w3d.runtime_state.text3d_resources.get(&resname).cloned() {
                    if let Some(mut mesh) = Self::build_text3d_mesh(&source, &state) {
                        mesh.name = resname;
                        if let Some(scene) = w3d.scene_mut() {
                            scene.clod_meshes.insert(resname, vec![mesh]);
                            scene.mesh_content_version += 1;
                        }
                    }
                }
            }
        }
    }

    /// Apply a Text3D geometry property (tunnelDepth/bevelDepth/bevelType/
    /// smoothness) to an extrude3d resource and re-extrude its mesh. Returns
    /// true if `resname` is an extrude3d resource (so the prop was consumed),
    /// false otherwise (caller can fall back to other handling).
    pub(crate) fn set_extruded_text_param(
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        resname: Symbol,
        prop: &str,
        value: &Datum,
    ) -> bool {
        let mut found = false;
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                if let Some((_src, state)) = w3d.runtime_state.text3d_resources.get_mut(&resname) {
                    found = true;
                    match prop.to_ascii_lowercase().as_str() {
                        "tunneldepth" => {
                            state.tunnel_depth = value.float_value()
                                .or_else(|_| value.int_value().map(|v| v as f64))
                                .unwrap_or(state.tunnel_depth as f64).max(1.0) as f32;
                        }
                        "beveldepth" => {
                            state.bevel_depth = value.float_value()
                                .or_else(|_| value.int_value().map(|v| v as f64))
                                .unwrap_or(state.bevel_depth as f64) as f32;
                        }
                        "smoothness" => {
                            state.smoothness = value.int_value().unwrap_or(state.smoothness as i32) as u32;
                        }
                        "beveltype" => {
                            state.bevel_type = match value.string_value().unwrap_or_default().trim_start_matches('#') {
                                "miter" => 1,
                                "round" => 2,
                                _ => 0,
                            };
                        }
                        _ => {}
                    }
                }
            }
        }
        if found {
            Self::rebuild_extruded_text(player, member_ref, resname);
        }
        found
    }

    fn scale_text3d_mesh_depth(
        scene: &mut crate::director::chunks::w3d::types::W3dScene,
        old_depth: f32,
        new_depth: f32,
    ) {
        let old_depth = old_depth.max(1.0);
        let new_depth = new_depth.max(1.0);
        let scale = new_depth / old_depth;

        if let Some(meshes) = scene.clod_meshes.get_mut(&Symbol::builtin(BuiltInSymbol::Text)) {
            for mesh in meshes.iter_mut() {
                for pos in mesh.positions.iter_mut() {
                    pos[2] *= scale;
                }
            }
        }
        scene.mesh_content_version += 1;
    }

    fn apply_text3d_display_face(
        runtime_state: &mut crate::player::cast_member::Shockwave3dRuntimeState,
        display_face: i32,
    ) {
        // extrude_glyph now generates exactly the requested faces (front cap /
        // back cap / tunnel) per displayFace, so this only toggles overall
        // visibility and keeps backface culling OFF (mode 3) so every generated
        // face — including the inward tunnel and hole walls — renders.
        let any = display_face == -1 || (display_face & 7) != 0;
        runtime_state
            .node_visibility
            .insert(Symbol::from_str(&"Text".to_string()), if any { 3u8 } else { 0u8 });
    }

    /// Lazily initialize the embedded 3D world for text members.
    /// Builds 3D extruded text mesh from PFR glyph outlines when available,
    /// or falls back to an alpha-mask-derived glyph mesh for native/system fonts.
    fn ensure_text3d(player: &mut DirPlayer, member_ref: &CastMemberRef) {
        use crate::director::chunks::w3d::types::*;

        // Check if this is a text member that needs 3D initialization
        let text_info = {
            let member = player.movie.cast_manager.find_member_by_ref(member_ref);
            match member {
                Some(m) => match &m.member_type {
                    CastMemberType::Text(text) => {
                        let tex_member_name = text.info.as_ref()
                            .filter(|i| i.texture_type == 2) // 2 = #member
                            .map(|i| i.texture_member.clone())
                            .filter(|s| !s.is_empty() && s != "NoTexture");
                        Some((
                            text.text.clone(),
                            text.font.clone(),
                            text.font_size,
                            text.width,
                            text.height,
                            text.alignment.clone(),
                            text.word_wrap,
                            text.html_styled_spans.clone(),
                            text.fixed_line_space,
                            text.top_spacing,
                            text.bottom_spacing,
                            text.tab_stops.clone(),
                            text.info.as_ref().map(|i| i.tunnel_depth).unwrap_or(10),
                            tex_member_name,
                        ))
                    }
                    _ => None,
                },
                None => None,
            }
        };
        let (text_content, font_name, font_size, tw, th, alignment, word_wrap, spans, fls, ts, bs, tab_stops, tunnel_depth, tex_member_name) = match text_info {
            Some(info) => info,
            None => return,
        };
        let spans = Self::build_fallback_text_spans(&text_content, &font_name, font_size, &spans);

        // Look up PFR glyph outlines from font cast members
        let glyph_data = {
            let mut result = None;
            for cast_lib in &player.movie.cast_manager.casts {
                for member in cast_lib.members.values() {
                    if let CastMemberType::Font(font_member) = &member.member_type {
                        if font_member.font_info.name.eq_ignore_ascii_case(&font_name) {
                            if let Some(ref pfr) = font_member.pfr_parsed {
                                result = Some((
                                    pfr.glyphs.clone(),
                                    pfr.physical_font.outline_resolution,
                                ));
                                break;
                            }
                        }
                    }
                }
                if result.is_some() { break; }
            }
            result
        };

        let has_pfr = glyph_data.is_some();

        log(&format!(
            "[Text3D] text='{}' font='{}' size={} has_pfr={} spans={} w={} h={} tex_member={:?}",
            text_content, font_name, font_size, has_pfr, spans.len(), tw, th, tex_member_name
        ));

        let texture_bitmap: Option<(u32, u32, Vec<u8>)> = if let Some(ref tex_name) = tex_member_name {
            // Look up the texture cast member by name and get its RGBA data
            let mut tex_result = None;
            let tex_ref = player.movie.cast_manager.find_member_ref_by_name(tex_name);
            if let Some(tref) = tex_ref {
                if let Some(tmember) = player.movie.cast_manager.find_member_by_ref(&tref) {
                    if let CastMemberType::Bitmap(bm) = &tmember.member_type {
                        if let Some(bmp) = player.bitmap_manager.get_bitmap(bm.image_ref) {
                            let w = bmp.width;
                            let h = bmp.height;
                            let palettes = player.movie.cast_manager.palettes();
                            let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
                            for y in 0..h as usize {
                                for x in 0..w as usize {
                                    let (r, g, b, a) = bmp.get_pixel_color_with_alpha(&palettes, x as u16, y as u16);
                                    let idx = (y * w as usize + x) * 4;
                                    rgba[idx] = r;
                                    rgba[idx + 1] = g;
                                    rgba[idx + 2] = b;
                                    rgba[idx + 3] = a;
                                }
                            }
                            log(&format!(
                                "[Text3D] texture from member '{}': {}x{} rgba_len={}",
                                tex_name, w, h, rgba.len()
                            ));
                            tex_result = Some((w as u32, h as u32, rgba));
                        }
                    }
                }
            }
            if tex_result.is_none() {
                log(&format!(
                    "[Text3D] texture member '{}' not found or not a bitmap", tex_name
                ));
            }
            tex_result
        } else {
            None
        };

        let glyph_bitmap: Option<(u32, u32, Vec<u8>)> = if !has_pfr && !spans.is_empty() {
            let source = Text3dSource {
                spans: spans.clone(),
                font_size,
                width: tw,
                height: th,
                alignment: alignment.clone(),
                word_wrap,
                fixed_line_space: fls,
                top_spacing: ts,
                bottom_spacing: bs,
                tab_stops: tab_stops.clone(),
                native_alpha_mesh: true,
            };
            Self::render_native_text_bitmap(&source, 10)
        } else {
            None
        };

        // Convert Text member → Shockwave3d member
        // Build the 3D scene, add mesh + texture, then replace the member type entirely.
        // This ensures the member goes through the exact same rendering path as regular 3D.
        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(member_ref) {
            if let CastMemberType::Text(ref mut text) = member.member_type {
                text.ensure_w3d();
                let depth = tunnel_depth.max(1) as f32;

                // Take the w3d out of the text member
                let mut w3d_member = match text.w3d.take() {
                    Some(boxed) => *boxed,
                    None => return,
                };

                // Add mesh to the scene
                if let Some((glyphs, outline_res)) = glyph_data {
                    let bevel_depth = w3d_member.text3d_state.as_ref().map(|s| s.bevel_depth).unwrap_or(1.0);
                    let bevel_type = w3d_member.text3d_state.as_ref().map(|s| s.bevel_type).unwrap_or(1);
                    let smoothness = w3d_member.text3d_state.as_ref().map(|s| s.smoothness).unwrap_or(10);
                    let display_face = w3d_member.text3d_state.as_ref().map(|s| s.display_face).unwrap_or(-1);
                    let mesh = crate::director::chunks::w3d::primitives::extrude_text_to_mesh(
                        &text_content, &glyphs, outline_res, font_size as f32, depth,
                        bevel_type, bevel_depth, smoothness, display_face,
                    );
                    if !mesh.positions.is_empty() {
                        if let Some(scene) = w3d_member.scene_mut() {
                            scene.clod_meshes.insert(Symbol::builtin(BuiltInSymbol::Text), vec![mesh]);
                            scene.mesh_content_version += 1;
                        }
                    }
                } else if let Some((bw, bh, rgba)) = glyph_bitmap {
                    let bevel_depth = w3d_member.text3d_state.as_ref().map(|s| s.bevel_depth).unwrap_or(1.0);
                    let bevel_type = w3d_member.text3d_state.as_ref().map(|s| s.bevel_type).unwrap_or(0);
                    let smoothness = w3d_member.text3d_state.as_ref().map(|s| s.smoothness).unwrap_or(10);
                    if let Some(scene) = w3d_member.scene_mut() {
                        let mesh = crate::director::chunks::w3d::primitives::extrude_alpha_mask_to_mesh(
                            bw,
                            bh,
                            &rgba,
                            tw as f32,
                            th as f32,
                            depth,
                            bevel_depth,
                            bevel_type,
                            smoothness,
                        );
                        if let Some((tex_w, tex_h, tex_rgba)) = texture_bitmap.as_ref() {
                            let mut tex_data = Vec::with_capacity(8 + tex_rgba.len());
                            tex_data.extend_from_slice(&tex_w.to_le_bytes());
                            tex_data.extend_from_slice(&tex_h.to_le_bytes());
                            tex_data.extend_from_slice(tex_rgba);
                            scene.texture_images.insert(Symbol::from_str("TextBitmap"), tex_data);
                            if !scene.texture_infos.iter().any(|t| t.name == Symbol::from_str("TextBitmap")) {
                                scene.texture_infos.push(W3dTextureInfo {
                                    name: Symbol::from_str("TextBitmap"),
                                    render_format: 0, mip_mode: 0, mag_filter: 0, image_type: 0,
                                });
                            }
                            if let Some(shader) = scene.shaders.first_mut() {
                                if !shader.texture_layers.iter().any(|l| l.name == Symbol::from_str("TextBitmap")) {
                                    shader.texture_layers.push(W3dTextureLayer {
                                        name: Symbol::from_str("TextBitmap"),
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                        scene.clod_meshes.insert(Symbol::builtin(BuiltInSymbol::Text), vec![mesh]);
                        scene.mesh_content_version += 1;
                    }
                }

                w3d_member.converted_from_text = true;
                if let Some(state) = w3d_member.text3d_state.as_mut() {
                    state.tunnel_depth = depth.max(1.0);
                } else {
                    w3d_member.text3d_state = Some(Text3dState {
                        tunnel_depth: depth.max(1.0),
                        smoothness: 10,
                        bevel_depth: 1.0,
                        // Director's default bevelType is #miter (1), not #none. A flat
                        // #none slab has no lit edge so letters/hole-rims read poorly;
                        // the miter chamfer catches light and makes depth + holes pop.
                        bevel_type: 1,
                        display_face: -1,
                        display_mode: 1,
                        diffuse_color: (0, 0, 0),
                    });
                }
                if let Some(state) = w3d_member.text3d_state.as_ref() {
                    Self::apply_text3d_display_face(&mut w3d_member.runtime_state, state.display_face);
                }
                w3d_member.text3d_source = Some(Text3dSource {
                    spans: spans.clone(),
                    font_size,
                    width: tw,
                    height: th,
                    alignment,
                    word_wrap,
                    fixed_line_space: fls,
                    top_spacing: ts,
                    bottom_spacing: bs,
                    tab_stops: tab_stops.clone(),
                    native_alpha_mesh: !has_pfr,
                });
                member.member_type = CastMemberType::Shockwave3d(w3d_member);
            }
        }
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: Symbol,
    ) -> Result<Datum, ScriptError> {
        Self::ensure_text3d(player, cast_member_ref);
        // Clone info and scene data upfront to avoid borrow conflicts with player.alloc_datum
        let (info, scene_data, text3d_state) = {
            let member = player
                .movie
                .cast_manager
                .find_member_by_ref(cast_member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
            let w3d = member.member_type.as_shockwave3d()
                .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
            (w3d.info.clone(), w3d.parsed_scene.clone(), w3d.text3d_state.clone())
        };

        use crate::director::chunks::w3d::types::W3dNodeType;

        let prop_builtin = prop.into_builtin_or_error()?;
        match prop_builtin {
            // ─── Member-level properties ───
            BuiltInSymbol::DirectToStage => Ok(Datum::Int(if info.direct_to_stage { 1 } else { 0 })),
            BuiltInSymbol::Preload => Ok(Datum::Int(if info.preload { 1 } else { 0 })),
            BuiltInSymbol::Duration => Ok(Datum::Int(info.duration as i32)),

            BuiltInSymbol::RegPoint => {
                Ok(Datum::Point([info.reg_point.0 as f64, info.reg_point.1 as f64], 0))
            }
            BuiltInSymbol::Rect => {
                let r = info.default_rect;
                Ok(Datum::Rect([r.0 as f64, r.1 as f64, r.2 as f64, r.3 as f64], 0))
            }
            BuiltInSymbol::Width => Ok(Datum::Int(info.default_rect.2 - info.default_rect.0)),
            BuiltInSymbol::Height => Ok(Datum::Int(info.default_rect.3 - info.default_rect.1)),

            // ─── Scene collection properties ───
            // These return lists of Shockwave3dObjectRefs, supporting .count and [index]
            BuiltInSymbol::Model | BuiltInSymbol::ModelCount | BuiltInSymbol::ModelResource | BuiltInSymbol::ModelResourceCount
            | BuiltInSymbol::Shader | BuiltInSymbol::ShaderCount | BuiltInSymbol::Texture | BuiltInSymbol::TextureCount
            | BuiltInSymbol::Light | BuiltInSymbol::LightCount | BuiltInSymbol::Camera | BuiltInSymbol::CameraCount
            | BuiltInSymbol::Group | BuiltInSymbol::GroupCount | BuiltInSymbol::Motion | BuiltInSymbol::MotionCount => {
                use crate::director::lingo::datum::{Shockwave3dObjectRef, DatumType};
                let prop_str = prop_builtin.as_str();
                let collection = Symbol::from_str(prop_str.trim_end_matches("Count")).into_builtin();
                let names: Vec<Symbol> = if let Some(scene) = &scene_data {
                    match collection {
                        Some(BuiltInSymbol::Model) => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).map(|n| n.name).collect(),
                        Some(BuiltInSymbol::ModelResource) => scene.model_resources.keys().copied().collect(),
                        Some(BuiltInSymbol::Shader) => scene.shaders.iter().map(|s| s.name).collect(),
                        Some(BuiltInSymbol::Texture) => scene.texture_images.keys().copied().collect(),
                        Some(BuiltInSymbol::Light) => scene.lights.iter().map(|l| l.name).collect(),
                        Some(BuiltInSymbol::Camera) => {
                            let mut cams = Vec::new();
                            if let Some(dv) = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View && n.name == Symbol::builtin(BuiltInSymbol::DefaultView)) {
                                cams.push(dv.name);
                            }
                            for n in &scene.nodes {
                                if n.node_type == W3dNodeType::View && n.name != Symbol::builtin(BuiltInSymbol::DefaultView) {
                                    cams.push(n.name);
                                }
                            }
                            cams
                        }
                        Some(BuiltInSymbol::Group) => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).map(|n| n.name.clone()).collect(),
                        Some(BuiltInSymbol::Motion) => {
                            // Default motion at index 1, then authored motions (see DEFAULT_MOTION_NAME).
                            let mut v = vec![Symbol::from_str(DEFAULT_MOTION_NAME)];
                            v.extend(scene.motions.iter().map(|m| m.name));
                            v
                        }
                        _ => vec![],
                    }
                } else {
                    vec![]
                };
                // If prop ends with "Count", return just the count
                if prop_str.ends_with("Count") {
                    return Ok(Datum::Int(names.len() as i32));
                }
                // Return a list of Shockwave3dObjectRefs
                let items: VecDeque<_> = names.iter().map(|name| {
                    player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: cast_member_ref.cast_lib,
                        cast_member: cast_member_ref.cast_member,
                        object_type: collection.unwrap(),
                        name: *name,
                    }))
                }).collect();
                Ok(Datum::List(DatumType::List, items, false))
            }

            // ─── State ───
            BuiltInSymbol::State => Ok(Datum::Int(4)), // 4 = loaded
            BuiltInSymbol::PercentStreamed => Ok(Datum::Int(100)),
            BuiltInSymbol::AnimationEnabled => Ok(Datum::Int(if info.animation_enabled { 1 } else { 0 })),
            BuiltInSymbol::Loop => Ok(Datum::Int(if info.loops { 1 } else { 0 })),

            // ─── Rendering ───
            BuiltInSymbol::Image => {
                // Force a sync of runtime shader-list mutations into scene data
                // before reading. Per-frame draw_frame() does this, but world.image
                // is often called inside a Lingo handler that just modified
                // textureList/textureModeList — without this, the first read returns
                // stale scene state and the avatar reflection is missing.
                crate::player::handlers::datum_handlers::shockwave3d_object::sync_shader_texture_lists(player);
                // Re-clone scene_data to pick up the just-applied sync.
                let scene_data = {
                    let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                        .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                    let w3d = member.member_type.as_shockwave3d()
                        .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
                    w3d.parsed_scene.clone()
                };

                // member("3d").image returns the rendered 3D world as a bitmap.
                let w = (info.default_rect.2 - info.default_rect.0).max(1) as u32;
                let h = (info.default_rect.3 - info.default_rect.1).max(1) as u32;

                // Try cached frame first (from sprite rendering), then offscreen render
                let key = (cast_member_ref.cast_lib, cast_member_ref.cast_member);
                // Opt this member into the per-frame FBO capture from now on. Until
                // a script asks, the renderer skips that readback entirely.
                player.w3d_image_requested.insert(key);
                if let Some(&bitmap_ref) = player.w3d_frame_buffers.get(&key) {
                    return Ok(Datum::BitmapRef(bitmap_ref));
                }

                // No cached frame — render offscreen
                let runtime_state = {
                    let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                        .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                    let w3d = member.member_type.as_shockwave3d()
                        .ok_or_else(|| ScriptError::new("Not 3D".to_string()))?;
                    w3d.runtime_state.clone()
                };

                let rgba_data = render_3d_to_rgba(&scene_data, &runtime_state, w, h);

                let mut bitmap = crate::player::bitmap::bitmap::Bitmap::new(
                    w as u16, h as u16, 32, 32, 8,
                    crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
                        crate::player::bitmap::bitmap::get_system_default_palette()
                    ),
                );
                bitmap.data = rgba_data;
                bitmap.use_alpha = true;
                // Offscreen 3D snapshot — not stored in w3d_frame_buffers, so
                // it has no other owner. Refcount through the DatumRef so the
                // snapshot is freed once the script releases it. The cached
                // path above (w3d_frame_buffers) returns an anchored bitmap
                // and bypasses this branch.
                let bitmap_ref = player.bitmap_manager.add_ephemeral_bitmap(bitmap);
                Ok(Datum::BitmapRef(bitmap_ref))
            }
            BuiltInSymbol::BackgroundColor => {
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(50, 50, 50)))
            }
            BuiltInSymbol::AmbientColor => {
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(25, 25, 25)))
            }
            BuiltInSymbol::Renderer | BuiltInSymbol::RendererDeviceList => Ok(Datum::Symbol(BuiltInSymbol::OpenGL.into())),
            BuiltInSymbol::ColorBufferDepth => Ok(Datum::Int(32)),
            BuiltInSymbol::DepthBufferDepth => Ok(Datum::Int(24)),
            BuiltInSymbol::AntiAliasingEnabled => Ok(Datum::Int(0)),
            BuiltInSymbol::StreamSize => Ok(Datum::Int(0)),
            // Text3D properties (stub values after Text→Shockwave3d conversion)
            BuiltInSymbol::Smoothness => Ok(Datum::Int(text3d_state.as_ref().map(|s| s.smoothness as i32).unwrap_or(10))),
            BuiltInSymbol::TunnelDepth => Ok(Datum::Float(text3d_state.as_ref().map(|s| s.tunnel_depth as f64).unwrap_or(10.0))),
            BuiltInSymbol::BevelDepth => Ok(Datum::Float(text3d_state.as_ref().map(|s| s.bevel_depth as f64).unwrap_or(1.0))),
            BuiltInSymbol::BevelType => Ok(Datum::Symbol(match text3d_state.as_ref().map(|s| s.bevel_type).unwrap_or(0) {
                1 => BuiltInSymbol::Miter.into(),
                2 => BuiltInSymbol::Round.into(),
                _ => BuiltInSymbol::None.into(),
            })),
            BuiltInSymbol::DisplayFace => Ok(Datum::Int(text3d_state.as_ref().map(|s| s.display_face).unwrap_or(-1))),
            BuiltInSymbol::DisplayMode => Ok(Datum::Symbol(if text3d_state.as_ref().map(|s| s.display_mode).unwrap_or(1) == 1 {
                BuiltInSymbol::Mode3d.into()
            } else {
                BuiltInSymbol::Normal.into()
            })),
            BuiltInSymbol::DiffuseColor => {
                let (r, g, b) = text3d_state.as_ref().map(|s| s.diffuse_color).unwrap_or((0, 0, 0));
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)))
            }
            BuiltInSymbol::DirectionalPreset => {
                // Read current preset from runtime state (default 2 = #topCenter)
                let preset = {
                    let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .map(|w3d| w3d.runtime_state.directional_preset)
                        .unwrap_or(2)
                };
                let symbol: Symbol = match preset {
                    1 => BuiltInSymbol::TopLeft.into(),
                    2 => BuiltInSymbol::TopCenter.into(),
                    3 => BuiltInSymbol::TopRight.into(),
                    4 => BuiltInSymbol::MiddleLeft.into(),
                    5 => BuiltInSymbol::MiddleCenter.into(),
                    6 => BuiltInSymbol::MiddleRight.into(),
                    7 => BuiltInSymbol::BottomLeft.into(),
                    8 => BuiltInSymbol::BottomCenter.into(),
                    9 => BuiltInSymbol::BottomRight.into(),
                    _ => BuiltInSymbol::None.into(),
                };
                Ok(Datum::Symbol(symbol.into()))
            }

            BuiltInSymbol::Text => {
                // A Text member with displayMode #mode3D is represented here as a
                // converted Shockwave3D member; Director keeps its .text live. Read
                // it back from the retained 3D-text spans.
                let s = player.movie.cast_manager.find_member_by_ref(cast_member_ref)
                    .and_then(|m| m.member_type.as_shockwave3d())
                    .and_then(|w3d| w3d.text3d_source.as_ref())
                    .map(|src| src.spans.iter().map(|sp| sp.text.as_str()).collect::<String>())
                    .unwrap_or_default();
                Ok(Datum::String(s))
            }
            _ => {
                Err(ScriptError::new(format!(
                    "Cannot get Shockwave3D property '{}'", prop
                )))
            }
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &str,
        value: &Datum,
    ) -> Result<(), ScriptError> {
        match prop {
            "diffuseColor" | "diffusecolor" => {
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(cast_member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                        if let Some(state) = w3d.text3d_state.as_mut() {
                            if let Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)) = value {
                                state.diffuse_color = (*r, *g, *b);
                                if let Some(scene) = w3d.scene_mut() {
                                    if let Some(mat) = scene.materials.iter_mut().find(|m| m.name == Symbol::builtin(BuiltInSymbol::TextMaterial)) {
                                        mat.diffuse = [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0];
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            "directToStage" | "preLoad" | "preload" | "loop" | "animationEnabled"
            | "smoothness" | "tunnelDepth" | "tunneldepth" | "bevelDepth" | "beveldepth"
            | "bevelType" | "beveltype" | "displayFace" | "displayface"
            | "displayMode" | "displaymode" => {
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(cast_member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                        let mut pending_depth_update: Option<(f32, f32)> = None;
                        let mut needs_rebuild = false;
                        if let Some(state) = w3d.text3d_state.as_mut() {
                            match prop {
                                "smoothness" => {
                                    state.smoothness = value.int_value()? as u32;
                                    needs_rebuild = true;
                                }
                                "tunnelDepth" | "tunneldepth" => {
                                    let new_depth = value
                                        .float_value()
                                        .or_else(|_| value.int_value().map(|v| v as f64))? as f32;
                                    let new_depth = new_depth.max(1.0);
                                    pending_depth_update = Some((state.tunnel_depth.max(1.0), new_depth));
                                    state.tunnel_depth = new_depth;
                                }
                                "bevelDepth" | "beveldepth" => {
                                    state.bevel_depth = value
                                        .float_value()
                                        .or_else(|_| value.int_value().map(|v| v as f64))? as f32;
                                    needs_rebuild = true;
                                }
                                "bevelType" | "beveltype" => {
                                    state.bevel_type = match value.string_value()?.trim_start_matches('#') {
                                        "miter" => 1,
                                        "round" => 2,
                                        _ => 0,
                                    };
                                    needs_rebuild = true;
                                }
                                "displayFace" | "displayface" => state.display_face = value.int_value()?,
                                "displayMode" | "displaymode" => {
                                    state.display_mode = match value.string_value()?.trim_start_matches('#') {
                                        "mode3d" => 1,
                                        _ => 0,
                                    };
                                }
                                _ => {}
                            }
                        }
                        if let Some((old_depth, new_depth)) = pending_depth_update {
                            if let Some(scene) = w3d.scene_mut() {
                                Self::scale_text3d_mesh_depth(scene, old_depth, new_depth);
                            }
                        }
                        if needs_rebuild {
                            Self::rebuild_native_text_mesh(w3d);
                        }
                        if let Some(state) = w3d.text3d_state.as_ref() {
                            Self::apply_text3d_display_face(&mut w3d.runtime_state, state.display_face);
                        }
                    }
                }
                Ok(())
            }
            "directionalPreset" | "directionalpreset" => {
                // Parse the symbol into preset 0..9 (0 = #None, 2 = #topCenter default).
                let preset: u32 = match value {
                    Datum::Symbol(s) => match s.into_builtin() {
                        Some(BuiltInSymbol::TopLeft) => 1,
                        Some(BuiltInSymbol::TopCenter) => 2,
                        Some(BuiltInSymbol::TopRight) => 3,
                        Some(BuiltInSymbol::MiddleLeft) => 4,
                        Some(BuiltInSymbol::MiddleCenter) => 5,
                        Some(BuiltInSymbol::MiddleRight) => 6,
                        Some(BuiltInSymbol::BottomLeft) => 7,
                        Some(BuiltInSymbol::BottomCenter) => 8,
                        Some(BuiltInSymbol::BottomRight) => 9,
                        Some(BuiltInSymbol::None) => 0,
                        _ => 0,
                    },
                    Datum::Int(i) => (*i as u32).min(9),
                    _ => 0,
                };

                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(cast_member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                        w3d.runtime_state.directional_preset = preset;

                        // Compute the light-node transform for this preset.
                        // preset=0 (#None) keeps whatever the scene already has.
                        if preset >= 1 && preset <= 9 {
                            let t = crate::player::cast_member::TextMember::directional_preset_to_transform_3d(preset);

                            // Update the scene's DefaultDirectional light node transform (authoritative)
                            // and also the runtime_state.node_transforms so the renderer picks it up.
                            if let Some(scene) = w3d.scene_mut() {
                                if let Some(light_node) = scene.nodes.iter_mut()
                                    .find(|n| n.name == Symbol::builtin(BuiltInSymbol::DefaultDirectional))
                                {
                                    light_node.transform = t;
                                }
                            }
                            w3d.runtime_state.node_transforms.insert(Symbol::builtin(BuiltInSymbol::DefaultDirectional), t);
                        }
                    }
                }
                Ok(())
            }
            "text" => {
                // member("x").text = "..." on a Text member that was promoted to a
                // 3D-text member (displayMode #mode3D). Director keeps the text live
                // and re-extrudes; replace the retained span content (preserving the
                // first span's style) and rebuild the mesh. Returning Ok here is what
                // stops PacMan3D crashing on its score-popup updates.
                if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(cast_member_ref) {
                    if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                        let new_text = value.string_value().unwrap_or_default();
                        if let Some(source) = w3d.text3d_source.as_mut() {
                            if let Some(first) = source.spans.first().cloned() {
                                source.spans = vec![crate::player::handlers::datum_handlers::cast_member::font::StyledSpan {
                                    text: new_text,
                                    style: first.style,
                                }];
                            }
                        }
                        Self::rebuild_native_text_mesh(w3d);
                    }
                }
                Ok(())
            }
            // Cast member property: the default rectangle used to size new
            // sprites / the rendered 3D image (Director dict: `defaultRect`,
            // e.g. `member.defaultRect = rect(0, 0, 300, 300)`). Stored as the
            // member's default_rect, which drives width/height/`.image` size.
            // `rect` is accepted as an alias since the getter maps it to the
            // same field. (defaultRectMode→#fixed isn't tracked for 3D members.)
            "defaultRect" | "defaultrect" | "rect" => {
                if let Datum::Rect([l, t, r, b], _) = value {
                    let new_rect = (*l as i32, *t as i32, *r as i32, *b as i32);
                    if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(cast_member_ref) {
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.info.default_rect = new_rect;
                        }
                    }
                }
                Ok(())
            }
            _ => {
                Err(ScriptError::new(format!(
                    "Cannot set Shockwave3D property '{}'", prop
                )))
            }
        }
    }

    // ─── Call handlers for Shockwave3D member methods ───
    // (moved from cast_member_ref.rs to consolidate 3D code)
    pub fn call(
        datum: &DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        let handler_name_builtin = handler_name.into_builtin_or_error()?;
        // Lazily init 3D world for text members before any 3D operation
        reserve_player_mut(|player| {
            let member_ref = match player.get_datum(datum) {
                Datum::CastMember(r) => r.to_owned(),
                _ => return Ok(()),
            };
            Self::ensure_text3d(player, &member_ref);
            Ok(())
        })?;

        match handler_name_builtin {
            BuiltInSymbol::GetPropRef => {
                // member("x").model[1] → getPropRef(#model, 1)
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    let collection = player.get_datum(&args[0]).symbol_value()?;
                    let index = if args.len() > 1 {
                        player.get_datum(&args[1]).int_value()? as usize
                    } else {
                        1
                    };
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                let obj_name = Self::get_3d_object_name_by_index(scene, collection, index)
                                    .unwrap_or_default();
                                if !obj_name.is_empty() {
                                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                                    return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                        cast_lib: cast_member_ref.cast_lib,
                                        cast_member: cast_member_ref.cast_member,
                                        object_type: collection.into_builtin_or_error()?,
                                        name: obj_name,
                                    })));
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            BuiltInSymbol::Count => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    if args.is_empty() {
                        return Err(ScriptError::new("count requires 1 argument".to_string()));
                    }
                    let count_of = player.get_datum(&args[0]).symbol_value()?;
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                let count = Self::get_3d_collection_count(scene, count_of);
                                return Ok(player.alloc_datum(Datum::Int(count)));
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Int(0)))
                })
            }
            // Shockwave 3D collection accessors & mutators
            BuiltInSymbol::Model | BuiltInSymbol::ModelResource | BuiltInSymbol::Shader | BuiltInSymbol::Texture | BuiltInSymbol::Light | BuiltInSymbol::Camera | BuiltInSymbol::Group | BuiltInSymbol::Motion
            | BuiltInSymbol::ResetWorld | BuiltInSymbol::RevertToWorldDefaults
            | BuiltInSymbol::NewTexture | BuiltInSymbol::NewShader | BuiltInSymbol::NewModel | BuiltInSymbol::NewModelResource | BuiltInSymbol::NewLight | BuiltInSymbol::NewCamera | BuiltInSymbol::NewGroup | BuiltInSymbol::NewMotion | BuiltInSymbol::NewMesh
            | BuiltInSymbol::DeleteTexture | BuiltInSymbol::DeleteShader | BuiltInSymbol::DeleteModel | BuiltInSymbol::DeleteModelResource | BuiltInSymbol::DeleteLight | BuiltInSymbol::DeleteCamera | BuiltInSymbol::DeleteGroup | BuiltInSymbol::DeleteMotion
            | BuiltInSymbol::CloneModelFromCastmember | BuiltInSymbol::CloneMotionFromCastmember | BuiltInSymbol::CloneDeep
            | BuiltInSymbol::LoadFile | BuiltInSymbol::Extrude3d | BuiltInSymbol::GetPref | BuiltInSymbol::SetPref
            | BuiltInSymbol::RegisterForEvent | BuiltInSymbol::RegisterScript | BuiltInSymbol::UnregisterAllEvents
            | BuiltInSymbol::Image => {
                reserve_player_mut(|player| {
                    let member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    let cast_member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                        .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
                    let w3d = cast_member.member_type.as_shockwave3d()
                        .ok_or_else(|| {
                            ScriptError::new(format!(
                                "Cannot call .{}() on non-Shockwave3D member (type: {:?})",
                                handler_name, cast_member.member_type.member_type_id()
                            ))
                        })?;

                    if handler_name_builtin == BuiltInSymbol::UnregisterAllEvents {
                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.registered_events.clear();
                        }
                        return Ok(player.alloc_datum(Datum::Void));
                    }
                    if handler_name_builtin == BuiltInSymbol::RegisterForEvent || handler_name_builtin == BuiltInSymbol::RegisterScript {
                        // member.registerForEvent(eventName, handlerName, scriptObject {, begin, period, repetitions})
                        // For #timeMS this drives the dispatcher in `dispatch_w3d_timer_events`.
                        // Other event names are stored but never fired (their producers —
                        // collision callbacks, animation start/end notifications — aren't wired).
                        let event_name = args.get(0)
                            .map(|a| {
                                let d = player.get_datum(a);
                                d.symbol_value().unwrap_or_else(|_| Symbol::empty())
                            })
                            .unwrap_or_default();
                        let handler_sym = args.get(1)
                            .map(|a| {
                                let d = player.get_datum(a);
                                d.symbol_value().unwrap_or_else(|_| Symbol::empty())
                            })
                            .unwrap_or_default();
                        let script_instance = args.get(2).and_then(|a| {
                            match player.get_datum(a) {
                                Datum::ScriptInstanceRef(r) => Some(r.clone()),
                                _ => None,
                            }
                        });
                        let begin_ms = args.get(3)
                            .and_then(|a| player.get_datum(a).int_value().ok())
                            .map(|v| v.max(0) as u32)
                            .unwrap_or(0);
                        let period_ms = args.get(4)
                            .and_then(|a| player.get_datum(a).int_value().ok())
                            .map(|v| v.max(0) as u32)
                            .unwrap_or(0);
                        let repetitions = args.get(5)
                            .and_then(|a| player.get_datum(a).int_value().ok())
                            .map(|v| v.max(0) as u32)
                            .unwrap_or(0);
                        let now_ms = crate::player::testing_shared::now_ms();
                        let event = crate::player::cast_member::RegisteredW3dEvent {
                            event_name,
                            handler_name,
                            script_instance,
                            begin_ms,
                            period_ms,
                            repetitions,
                            registered_at_ms: now_ms,
                            fires_so_far: 0,
                            last_fire_ms: now_ms,
                        };
                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            w3d.runtime_state.registered_events.push(event);
                        }
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    if handler_name == "resetWorld" {
                        use std::sync::atomic::{AtomicU64, Ordering};
                        // Monotonic generation so each resetWorld stamps a brand-new
                        // content version — guarantees the renderer's per-member GPU
                        // mesh cache is rebuilt. Without this, a deterministic restart
                        // (game over → resetWorld → rebuild the same models) can land
                        // back on a cached version and leave the PREVIOUS game's models
                        // on screen (stale models, maze rebuilt over old transforms).
                        static RESET_GEN: AtomicU64 = AtomicU64::new(1_000_000);
                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            // resetWorld: restore to the state from when the member was
                            // first loaded. Deep-clone the retained source so runtime
                            // edits never leak back into it, and stamp a fresh version.
                            if let Some(ref source) = w3d.source_scene {
                                let mut fresh = (**source).clone();
                                let reset_gen = RESET_GEN.fetch_add(1, Ordering::Relaxed);
                                fresh.mesh_content_version = reset_gen;
                                fresh.texture_content_version = reset_gen;
                                w3d.parsed_scene = Some(std::rc::Rc::new(fresh));
                            }
                            w3d.runtime_state = crate::player::cast_member::Shockwave3dRuntimeState::from_info(&w3d.info, w3d.parsed_scene.as_deref());
                        }
                        return Ok(player.alloc_datum(Datum::Void));
                    }
                    if handler_name == BuiltInSymbol::RevertToWorldDefaults {
                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            // revertToWorldDefaults: restore to state when member was first created
                            // (re-parse from original W3D data)
                            if !w3d.w3d_data.is_empty() {
                                match crate::director::chunks::w3d::parse_w3d(&w3d.w3d_data) {
                                    Ok(scene) => {
                                        w3d.parsed_scene = Some(std::rc::Rc::new(scene));
                                    }
                                    Err(_) => {
                                        w3d.parsed_scene = Some(std::rc::Rc::new(
                                            crate::player::cast_member::CastMember::create_empty_w3d_scene()
                                        ));
                                    }
                                }
                            } else {
                                w3d.parsed_scene = Some(std::rc::Rc::new(
                                    crate::player::cast_member::CastMember::create_empty_w3d_scene()
                                ));
                            }
                            w3d.runtime_state = crate::player::cast_member::Shockwave3dRuntimeState::from_info(&w3d.info, w3d.parsed_scene.as_deref());
                        }
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // cloneModelFromCastmember / cloneMotionFromCastmember / cloneDeep
                    if handler_name == BuiltInSymbol::CloneModelFromCastmember || handler_name == BuiltInSymbol::CloneMotionFromCastmember || handler_name == BuiltInSymbol::CloneDeep {
                        let obj_name = if !args.is_empty() {
                            player.get_datum(&args[0]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let source_model_name = if args.len() > 1 {
                            player.get_datum(&args[1]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let source_member_ref = if args.len() > 2 {
                            match player.get_datum(&args[2]) {
                                Datum::CastMember(r) => Some(r.clone()),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let obj_type = if handler_name_builtin == BuiltInSymbol::CloneMotionFromCastmember {
                            BuiltInSymbol::Motion
                        } else {
                            BuiltInSymbol::Model
                        };

                        // Look up source model's shader/transform/resource from source member's scene
                        let identity = [1.0f32,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
                        let (source_shader_name, source_transform, source_resource_name, source_model_resource_name, src_motion_tracks, src_child_nodes) = if let Some(ref src_ref) = source_member_ref {
                            let src_member = player.movie.cast_manager.find_member_by_ref(src_ref);
                            if let Some(sm) = src_member {
                                if let Some(sw3d) = sm.member_type.as_shockwave3d() {
                                    if let Some(ref scene) = sw3d.parsed_scene {
                                        let node = scene.nodes.iter().find(|n| n.name == Symbol::from_str(&source_model_name));
                                        let (sn, st, sr, smr) = if let Some(n) = node {
                                            (n.shader_name, n.transform, n.resource_name, n.model_resource_name)
                                        } else {
                                            (Symbol::empty(), identity, Symbol::empty(), Symbol::empty())
                                        };
                                        // cloneMotionFromCastmember(newName, sourceMotionName, member)
                                        // must clone the SPECIFIC motion named sourceMotionName
                                        // (= source_model_name, arg1). The old code grabbed the
                                        // motion with the most tracks instead, so every cloned
                                        // motion in a member collapsed onto the last/largest one
                                        // (ties resolve to the last) — On the Run's bonus got the
                                        // gate's "SbarraChiusura" swing instead of "BonusRotazione",
                                        // and all train/jeep motions became "JeepCPU02". For model
                                        // clones (no specific motion requested) keep the
                                        // most-tracks heuristic (a skeletal model's main motion).
                                        let motion_tracks = if obj_type == "motion" {
                                            // Among motions matching the requested name, take the one
                                            // with the most tracks (the skeletal animation, vs an
                                            // empty/stub of the same name) — robust to duplicate names;
                                            // a no-op when the name is unique (On the Run).
                                            scene.motions.iter()
                                                .filter(|m| m.name.eq_ignore_ascii_case(&source_model_name))
                                                .max_by_key(|m| m.tracks.len())
                                                .map(|m| m.tracks.clone())
                                                .unwrap_or_default()
                                        } else {
                                            scene.motions.iter()
                                                .max_by_key(|m| m.tracks.len())
                                                .map(|m| m.tracks.clone())
                                                .unwrap_or_default()
                                        };
                                        // Collect all descendant nodes of the source model recursively
                                        // Use case-insensitive matching (Director is case-insensitive)
                                        let child_nodes = {
                                            let mut descendants = Vec::new();
                                            // Expand each parent name at most once. Without this
                                            // the walk is unbounded whenever a name repeats in the
                                            // scene: every pop re-scans ALL nodes and re-pushes the
                                            // same child names, so `descendants` grows without
                                            // limit and the handler never returns — a hang with
                                            // memory climbing into the gigabytes.
                                            //
                                            // Duplicate names are normal, not exotic. A node is
                                            // matched by NAME here, and `loadFile` merges a world
                                            // into the existing one, so loading the same .w3d twice
                                            // (Agent Free Ride loads level.w3d twice during its
                                            // level setup) gives every node a same-named twin. A
                                            // self-parented node or any parent cycle does it too.
                                            let mut visited: std::collections::HashSet<String> =
                                                std::collections::HashSet::new();
                                            // Index children by parent name ONCE. The walk used to
                                            // rescan every node for each parent it popped, an
                                            // O(subtree x scene) sweep of case-insensitive string
                                            // compares — with ~2000 nodes in a merged level world
                                            // that is millions of comparisons per clone, and this
                                            // game clones 40 models during level setup.
                                            let mut children_by_parent: std::collections::HashMap<
                                                String,
                                                Vec<&crate::director::chunks::w3d::types::W3dNode>,
                                            > = std::collections::HashMap::new();
                                            for n in &scene.nodes {
                                                children_by_parent
                                                    .entry(n.parent_name.to_lowercase())
                                                    .or_default()
                                                    .push(n);
                                            }
                                            let mut stack = vec![source_model_name.to_string()];
                                            while let Some(parent) = stack.pop() {
                                                let key = parent.to_lowercase();
                                                if !visited.insert(key.clone()) {
                                                    continue;
                                                }
                                                if let Some(kids) = children_by_parent.get(&key) {
                                                    for n in kids {
                                                        descendants.push((*n).clone());
                                                        stack.push(n.name.clone().to_string());
                                                    }
                                                }
                                            }
                                            descendants
                                        };
                                        (sn, st, sr, smr, motion_tracks, child_nodes)
                                    } else { (Symbol::empty(), identity, Symbol::empty(), Symbol::empty(), vec![], vec![]) }
                                } else { (Symbol::empty(), identity, Symbol::empty(), Symbol::empty(), vec![], vec![]) }
                            } else { (Symbol::empty(), identity, Symbol::empty(), Symbol::empty(), vec![], vec![]) }
                        } else {
                            (Symbol::empty(), identity, Symbol::empty(), Symbol::empty(), vec![], vec![])
                        };

                        // Track shader name remapping for -clone suffix creation
                        let mut shader_name_map: std::collections::HashMap<Symbol, Symbol> = std::collections::HashMap::new();
                        // Track texture name remapping for -clone suffix creation. Director
                        // renames a colliding texture to "<name>-clone<N>": two models can both
                        // ship a generically-named texture (e.g. the base map AND the Shield item
                        // both export "Map #19"); the second model's copy becomes "Map #19-clone1"
                        // and that model's shaders are repointed at the clone, so neither hijacks
                        // the other. dirplayer previously kept only the first → the map wall showed
                        // the shield's sunset texture.
                        let mut texture_name_map: std::collections::HashMap<Symbol, Symbol> = std::collections::HashMap::new();

                        // Namespace prefix for cloned NODE names (must be unique per
                        // clone). Resource/mesh names instead follow loadFile's merge
                        // rule: keep the incoming name unless it collides, so bare-name
                        // lookups like `modelResource("bonus_money")` still resolve after
                        // a same-named clone. `res_rename` maps lowercase source
                        // resource/mesh name -> destination name (populated below).
                        let ns = format!("{}_", obj_name);
                        let mut res_rename: std::collections::HashMap<Symbol, Symbol> =
                            std::collections::HashMap::new();

                        // Copy source shaders, model resources, meshes, and textures that don't exist in target scene
                        if let Some(ref src_ref) = source_member_ref {
                            let (src_shaders, src_materials, src_model_resources, src_clod_meshes, src_raw_meshes, src_textures, src_lights, src_light_nodes, src_skeletons, src_motions) = {
                                let src_member = player.movie.cast_manager.find_member_by_ref(src_ref);
                                let scene = src_member.and_then(|sm| sm.member_type.as_shockwave3d())
                                    .and_then(|sw3d| sw3d.parsed_scene.as_ref());

                                // Director 11.5 (`cloneModelFromCastmember`): this copies "the model
                                // resources, shaders, and textures used by the model and its
                                // children" — not the source member's whole 3D world. Working the
                                // GEOMETRY subset out here, against a borrow of the source scene,
                                // keeps the copy off the hot path: pulling every table out by value
                                // first meant each clone dragged the entire source world across
                                // before the filters below ever ran. Agent Free Ride clones 40
                                // models out of a 247-mesh level during setup and spent minutes
                                // there with RSS climbing into the gigabytes, while the Lingo datum
                                // arena stayed flat at ~5.2k — none of that growth was script data.
                                //
                                // A node's resource_name may name a model resource, a CLOD mesh or
                                // a raw mesh (they share one namespace), so all three are filtered
                                // by the same key set. An empty set means we couldn't attribute
                                // anything, and we fall back to copying everything as before.
                                //
                                // Shaders and textures are deliberately NOT filtered here. A model
                                // can pick up a shader the source file never bound to its resource
                                // (a runtime shaderList assignment, for one), so attributing them
                                // from the file alone under-collects: filtering the track's clones
                                // this way stripped their textures and rendered the whole slope
                                // black. The copy loops below already skip unused shaders/textures
                                // when they can attribute them, and falling back to the full tables
                                // costs little — the load stays ~30s either way, because the meshes
                                // and model resources were what made it expensive.
                                let mut used_res: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
                                for name in [source_resource_name, source_model_resource_name] {
                                    if !name.as_str().is_empty() { used_res.insert(name); }
                                }
                                for child in &src_child_nodes {
                                    for name in [child.resource_name, child.model_resource_name] {
                                        if !name.as_str().is_empty() { used_res.insert(name); }
                                    }
                                }
                                let filter_res = !used_res.is_empty();
                                let wants_res = |name: Symbol| !filter_res || used_res.contains(&name);

                                let shaders: Vec<_> = scene.map(|s| s.shaders.clone()).unwrap_or_default();
                                let materials: Vec<_> = scene.map(|s| s.materials.clone()).unwrap_or_default();
                                let resources: Vec<_> = scene.map(|s| s.model_resources.iter()
                                    .filter(|(k, _)| wants_res(**k))
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let meshes: Vec<_> = scene.map(|s| s.clod_meshes.iter()
                                    .filter(|(k, _)| wants_res(**k))
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let raw: Vec<_> = scene.map(|s| s.raw_meshes.iter()
                                    .filter(|m| wants_res(m.name))
                                    .cloned().collect()).unwrap_or_default();
                                let textures: Vec<_> = scene.map(|s| s.texture_images.iter()
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let lights: Vec<_> = scene.map(|s| s.lights.clone()).unwrap_or_default();
                                let light_nodes: Vec<_> = scene.map(|s| s.nodes.iter()
                                    .filter(|n| n.node_type == crate::director::chunks::w3d::types::W3dNodeType::Light)
                                    .cloned().collect()).unwrap_or_default();
                                let skeletons: Vec<_> = scene.map(|s| s.skeletons.clone()).unwrap_or_default();
                                // A skeleton without its motions is just a bind pose. Agent Free
                                // Ride clones "player" out of member 5 into the member 1 scene it
                                // actually renders, and the rider stood in his T-pose because the
                                // clip stayed behind in the source member.
                                let motions: Vec<_> = scene.map(|s| s.motions.clone()).unwrap_or_default();
                                (shaders, materials, resources, meshes, raw, textures, lights, light_nodes, skeletons, motions)
                            };

                            debug!(
                                "[W3D-CLONE] {}(\"{}\") src_model=\"{}\" src_member={:?}: \
                                 {} shaders, {} model_resources, {} clod_meshes(keys={:?}), {} raw_meshes(names={:?}), {} textures, \
                                 src_res=\"{}\", src_mres=\"{}\"",
                                handler_name, obj_name, source_model_name, source_member_ref,
                                src_shaders.len(), src_model_resources.len(),
                                src_clod_meshes.len(), src_clod_meshes.iter().map(|(k,_)| k.clone()).collect::<Vec<Symbol>>(),
                                src_raw_meshes.len(), src_raw_meshes.iter().map(|m| m.name.clone()).collect::<Vec<Symbol>>(),
                                src_textures.len(),
                                source_resource_name, source_model_resource_name,
                            );

                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        // Determine which shaders are USED by the model being cloned.
                                        // Director docs: "copies shaders...used by the model and its children"
                                        let mut used_shader_names: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
                                        // From model resource shader bindings
                                        let res_key = if !source_model_resource_name.as_str().is_empty() {
                                            source_model_resource_name
                                        } else {
                                            source_resource_name
                                        };
                                        for (rname, rinfo) in &src_model_resources {
                                            if *rname == res_key {
                                                for binding in &rinfo.shader_bindings {
                                                    for shader_name in &binding.mesh_bindings {
                                                        used_shader_names.insert(*shader_name);
                                                    }
                                                }
                                            }
                                        }
                                        // From node shader_name
                                        if !source_shader_name.is_empty() {
                                            used_shader_names.insert(source_shader_name);
                                        }
                                        // Also collect shaders from CHILD model resources
                                        for child in &src_child_nodes {
                                            let child_res = if !child.model_resource_name.is_empty() {
                                                child.model_resource_name
                                            } else {
                                                child.resource_name
                                            };
                                            for (rname, rinfo) in &src_model_resources {
                                                if *rname == child_res {
                                                    for binding in &rinfo.shader_bindings {
                                                        for shader_name in &binding.mesh_bindings {
                                                            used_shader_names.insert(*shader_name);
                                                        }
                                                    }
                                                }
                                            }
                                            if !child.shader_name.is_empty() {
                                                used_shader_names.insert(child.shader_name);
                                            }
                                        }

                                        // Collect texture names used by the used shaders
                                        let mut used_texture_names: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
                                        for shader in &src_shaders {
                                            if used_shader_names.contains(&shader.name) {
                                                for layer in &shader.texture_layers {
                                                    if !layer.name.is_empty() {
                                                        used_texture_names.insert(layer.name);
                                                    }
                                                }
                                            }
                                        }

                                        // If no specific shaders identified, fall back to copying all
                                        // (handles cases where shader bindings are empty/unknown)
                                        let filter_shaders = !used_shader_names.is_empty();

                                        // Pre-pass: detect texture-name collisions. A texture whose
                                        // name already exists in the target scene with DIFFERENT
                                        // pixels is a real collision (generic exporter names); record
                                        // a "-clone<N>" rename so the shaders below point at it. Same
                                        // bytes under the same name are genuinely shared — left as-is.
                                        for (tex_name, tex_data) in &src_textures {
                                            if filter_shaders && !used_texture_names.contains(tex_name) { continue; }
                                            if let Some(existing) = scene.texture_images.get(tex_name) {
                                                if existing != tex_data {
                                                    let mut n = 1;
                                                    loop {
                                                        let cand = Symbol::from_str(&format!("{}-clone{}", tex_name, n));
                                                        if !scene.texture_images.contains_key(&cand)
                                                            && !texture_name_map.values().any(|v| *v == cand)
                                                        {
                                                            texture_name_map.insert(*tex_name, cand);
                                                            break;
                                                        }
                                                        n += 1;
                                                    }
                                                }
                                            }
                                        }

                                        // Shaders: only copy those used by the model.
                                        // If name conflicts, create -clone<N> copy (Director behavior).
                                        // DefaultShader is built-in to every cast member — never copy it.
                                        for shader in &src_shaders {
                                            if shader.name == Symbol::builtin(BuiltInSymbol::DefaultShader) {
                                                continue;
                                            }
                                            if filter_shaders && !used_shader_names.contains(&shader.name) {
                                                continue; // Skip shaders not used by this model
                                            }
                                            let mut cloned = shader.clone();
                                            // Repoint any texture layers whose texture was renamed
                                            // on collision (Director's -clone<N> behavior).
                                            if !texture_name_map.is_empty() {
                                                for layer in &mut cloned.texture_layers {
                                                    if let Some(nt) = texture_name_map.get(&layer.name) {
                                                        layer.name = Symbol::from_str(&nt.clone().as_str());
                                                    }
                                                }
                                            }
                                            if scene.shaders.iter().any(|s| s.name == shader.name) {
                                                // Name conflict — create a -clone<N> copy
                                                let mut n = 1;
                                                loop {
                                                    let clone_name = Symbol::from_str(&format!("{}-clone{}", shader.name, n));
                                                    if !scene.shaders.iter().any(|s| s.name == clone_name) {
                                                        shader_name_map.insert(shader.name.clone(), clone_name.clone());
                                                        cloned.name = clone_name;
                                                        scene.shaders.push(cloned);
                                                        break;
                                                    }
                                                    n += 1;
                                                }
                                            } else {
                                                scene.shaders.push(cloned);
                                            }
                                        }
                                        // Copy materials referenced by copied shaders.
                                        // Check both shader.material_name and shader.name as material key,
                                        // since the renderer falls back to finding materials by shader name.
                                        for shader in &src_shaders {
                                            if !used_shader_names.contains(&shader.name) { continue; }
                                            for mat in &src_materials {
                                                if (!shader.material_name.is_empty() && mat.name == shader.material_name)
                                                    || mat.name == shader.name
                                                {
                                                    let target_name = shader_name_map.get(&shader.name)
                                                        .map(|mapped| {
                                                            // If shader was renamed (conflict), rename material too
                                                            let mut m = mat.clone();
                                                            m.name = Symbol::from_str(&*mapped.as_str());
                                                            m
                                                        });
                                                    let mat_to_push = target_name.unwrap_or_else(|| mat.clone());
                                                    if !scene.materials.iter().any(|m| m.name == mat_to_push.name) {
                                                        scene.materials.push(mat_to_push);
                                                    }
                                                }
                                            }
                                        }

                                        // Log ALL shaders that were just copied
                                        log(&format!(
                                            "[CLONE-SHADERS] '{}' used_shaders={:?} used_textures={:?} shader_map={:?}",
                                            obj_name, used_shader_names, used_texture_names, shader_name_map
                                        ));
                                        // Plan resource/mesh renames like loadFile's merge:
                                        // keep each incoming name unless it collides with an
                                        // existing destination name, in which case give it a
                                        // unique "-clone<N>" suffix. Model resources, CLOD
                                        // meshes and raw meshes share one namespace (a node's
                                        // resource_name may name any of them).
                                        {
                                            let mut taken: std::collections::HashSet<Symbol> =
                                                scene.model_resources.keys().copied().collect();
                                            taken.extend(scene.raw_meshes.iter().map(|m| m.name));
                                            taken.extend(scene.clod_meshes.keys().copied());
                                            let mut src_names: Vec<Symbol> =
                                                src_model_resources.iter().map(|(k, _)| *k).collect();
                                            src_names.extend(src_clod_meshes.iter().map(|(k, _)| *k));
                                            src_names.extend(src_raw_meshes.iter().map(|m| m.name));
                                            for name in src_names {
                                                if name.as_str().is_empty() { continue; }
                                                if res_rename.contains_key(&name) { continue; }
                                                let dst = if taken.contains(&name) {
                                                    let mut n = 1;
                                                    loop {
                                                        let cand = Symbol::from_str(&format!("{}-clone{}", name, n));
                                                        if !taken.contains(&cand) { break cand; }
                                                        n += 1;
                                                    }
                                                } else {
                                                    name
                                                };
                                                taken.insert(dst);
                                                res_rename.insert(name, dst);
                                            }
                                        }
                                        // Helper: resolve a source resource/mesh name through the map.
                                        let map_res = |name: Symbol| -> Symbol {
                                            res_rename.get(&name).copied().unwrap_or(name)
                                        };
                                        // Model resources: insert under their (collision-renamed) names.
                                        for (res_name, res_info) in &src_model_resources {
                                            let new_name = map_res(*res_name);
                                            if !scene.model_resources.contains_key(&new_name) {
                                                let mut cloned_res = res_info.clone();
                                                for binding in &mut cloned_res.shader_bindings {
                                                    for mesh_shader in &mut binding.mesh_bindings {
                                                        if let Some(renamed) = shader_name_map.get(mesh_shader) {
                                                            *mesh_shader = *renamed;
                                                        }
                                                    }
                                                }
                                                scene.model_resources.insert(new_name, cloned_res);
                                            }
                                        }
                                        // CLOD meshes: insert under their (collision-renamed) names.
                                        for (mesh_name, mesh_data) in &src_clod_meshes {
                                            let new_name = map_res(*mesh_name);
                                            if !scene.clod_meshes.contains_key(&new_name) {
                                                scene.clod_meshes.insert(new_name, mesh_data.clone());
                                            }
                                        }
                                        // Textures: only copy those used by copied shaders.
                                        // A collided texture lands under its "-clone<N>" name.
                                        for (tex_name, tex_data) in &src_textures {
                                            if filter_shaders && !used_texture_names.contains(tex_name) {
                                                continue;
                                            }
                                            let target = texture_name_map.get(tex_name).cloned()
                                                .unwrap_or_else(|| Symbol::from_str(&tex_name.clone().to_string()));
                                            if !scene.texture_images.contains_key(&Symbol::from_str(&target.as_str())) {
                                                scene.texture_images.insert(Symbol::from_str(&target.as_str()), tex_data.clone());
                                                scene.texture_content_version += 1;
                                            }
                                        }
                                        // Raw meshes: insert under their (collision-renamed) names.
                                        for raw_mesh in &src_raw_meshes {
                                            let new_name = map_res(raw_mesh.name);
                                            if !scene.raw_meshes.iter().any(|m| m.name == new_name) {
                                                let mut cloned = raw_mesh.clone();
                                                cloned.name = new_name;
                                                scene.raw_meshes.push(cloned);
                                            }
                                        }
                                        // Copy lights from source scene
                                        for light in &src_lights {
                                            if !scene.lights.iter().any(|l| l.name == light.name) {
                                                scene.lights.push(light.clone());
                                            }
                                        }
                                        // Copy light nodes from source scene
                                        for node in &src_light_nodes {
                                            if !scene.nodes.iter().any(|n| n.name == node.name) {
                                                scene.nodes.push(node.clone());
                                            }
                                        }
                                        // Copy the skeleton that BELONGS to the source model
                                        // (named after its resource), not just the first skeleton
                                        // in the source scene — which may be a DIFFERENT model's
                                        // rig (e.g. a 3-bone hair skeleton). Copying the wrong one
                                        // left the cloned biped with too few bones, so bone[6]
                                        // (the head) was out of range: the hair fell back to the
                                        // model transform and the skeletal animation couldn't pose.
                                        let skel_key = if !source_model_resource_name.as_str().is_empty() {
                                            map_res(source_model_resource_name)
                                        } else if !source_resource_name.as_str().is_empty() {
                                            map_res(source_resource_name)
                                        } else { Symbol::empty() };
                                        let src_skel = src_skeletons.iter().find(|s|
                                                s.name == source_model_resource_name
                                                || s.name == source_resource_name)
                                            .or_else(|| src_skeletons.first());
                                        if let Some(skeleton) = src_skel {
                                            if !skel_key.is_empty() && !scene.skeletons.iter().any(|s| s.name == skel_key) {
                                                let mut cloned = skeleton.clone();
                                                cloned.name = Symbol::from_str(&skel_key.as_str());
                                                scene.skeletons.push(cloned);

                                                // Bring the rig's clips across too — a skeleton with
                                                // no motion is just a bind pose. Keep each clip's
                                                // ORIGINAL name even though the skeleton is renamed
                                                // to the clone's key: scripts play motions by the
                                                // source name (Agent Free Ride clones "player" in as
                                                // "veh_player_1" and then calls play("player")).
                                                // Motion tracks address bones by name, so a copied
                                                // clip drives the renamed skeleton unchanged.
                                                for motion in &src_motions {
                                                    if scene.motions.iter().any(|m| m.name == motion.name) {
                                                        continue;
                                                    }
                                                    scene.motions.push(motion.clone());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Add the cloned object to the target scene. Resolve the root
                        // model's resource pointers through the same collision-only
                        // rename map used when the resources were inserted above.
                        let map_res_out = |name: Symbol| -> Symbol {
                            Symbol::from_str(&res_rename.get(&name).copied()
                                .unwrap_or_else(|| Symbol::from_str(&name.to_string())).to_string())
                        };
                        let mapped_resource = if !source_resource_name.as_str().is_empty() {
                            map_res_out(source_resource_name)
                        } else { source_resource_name };
                        let mapped_model_resource = if !source_model_resource_name.as_str().is_empty() {
                            map_res_out(source_model_resource_name)
                        } else { source_model_resource_name };

                        // Don't propagate "DefaultShader" as the node-level shader —
                        // it overrides the model resource's per-mesh shader bindings
                        // (which have the correct materials with proper colors).
                        let effective_shader_name = if source_shader_name == Symbol::builtin(BuiltInSymbol::DefaultShader) || source_shader_name.is_empty() {
                            Symbol::empty()
                        } else {
                            shader_name_map.get(&source_shader_name)
                                .copied()
                                .unwrap_or(Symbol::from_str(&source_shader_name.to_string()))
                        };

                        // Copy the source member's keyframe MOTIONS so the cloned
                        // model's keyframePlayer.play(name) can find them — clone
                        // otherwise copies geometry/shaders/meshes but NOT motions.
                        // (Splat: pac-man feet footA/footB clones play "footA-Key"/
                        // "footB-Key"; the motion's track is named after the source
                        // node "footA"/"footB", which matches the same-named clone.)
                        let src_motions: Vec<crate::director::chunks::w3d::types::W3dMotion> = if obj_type == "model" {
                            source_member_ref.as_ref()
                                .and_then(|sr| player.movie.cast_manager.find_member_by_ref(sr))
                                .and_then(|sm| sm.member_type.as_shockwave3d())
                                .and_then(|sw| sw.parsed_scene.as_ref())
                                .map(|sc| sc.motions.clone())
                                .unwrap_or_default()
                        } else { Vec::new() };

                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    use crate::director::chunks::w3d::types::*;
                                    if obj_type == "model" {
                                        // Bring over any source motions not already present (by
                                        // name) so keyframePlayer.play() resolves them.
                                        for m in &src_motions {
                                            if !scene.motions.iter().any(|em| em.name.eq_ignore_ascii_case(&m.name.as_str())) {
                                                scene.motions.push(m.clone());
                                            }
                                        }
                                        scene.nodes.push(W3dNode {
                                            name: Symbol::from_str(&obj_name), node_type: W3dNodeType::Model,
                                            parent_name: Symbol::builtin(BuiltInSymbol::World),
                                            resource_name: mapped_resource,
                                            model_resource_name: mapped_model_resource,
                                            shader_name: effective_shader_name,
                                            visibility: 1,
                                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                            screen_width: 640, screen_height: 480,
                                            transform: source_transform,
                                        });
                                        // Namespace every descendant's name to avoid collisions
                                        // with prior clones from the same source.
                                        let mut node_name_map: std::collections::HashMap<Symbol, Symbol> =
                                            std::collections::HashMap::new();
                                        for child in &src_child_nodes {
                                            let new_name = Symbol::from_str(&format!("{}{}", ns, child.name));
                                            node_name_map.insert(child.name, new_name);
                                        }

                                        // Clone child nodes from source scene, re-parenting
                                        // the direct children of source_model to obj_name and
                                        // rewiring deeper parent links to the namespaced names.
                                        for child in &src_child_nodes {
                                            let mut cloned = child.clone();
                                            // Rename the node itself
                                            if let Some(new_name) = node_name_map.get(&cloned.name) {
                                                cloned.name = *new_name;
                                            }
                                            // Re-parent: direct child of source_model → obj_name;
                                            // otherwise remap to the namespaced descendant name.
                                            if cloned.parent_name == Symbol::from_str(&source_model_name) {
                                                cloned.parent_name = Symbol::from_str(&obj_name);
                                            } else if let Some(new_parent) = node_name_map.get(&cloned.parent_name) {
                                                cloned.parent_name = *new_parent;
                                            }
                                            // Repoint child resource names to the (collision-
                                            // renamed) keys the cloned mesh data was inserted under.
                                            if !cloned.resource_name.is_empty() {
                                                cloned.resource_name = map_res_out(cloned.resource_name);
                                            }
                                            if !cloned.model_resource_name.is_empty() {
                                                cloned.model_resource_name = map_res_out(cloned.model_resource_name);
                                            }
                                            // Remap shader name if it was renamed during clone
                                            if let Some(new_shader) = shader_name_map.get(&cloned.shader_name) {
                                                cloned.shader_name = Symbol::from_str(&*new_shader.as_str());
                                            }
                                            // Names are now unique per clone — push unconditionally
                                            scene.nodes.push(cloned);
                                        }
                                    } else if obj_type == BuiltInSymbol::Motion {
                                        scene.motions.push(W3dMotion {
                                            name: Symbol::from_str(&obj_name),
                                            tracks: src_motion_tracks.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        use crate::director::lingo::datum::Shockwave3dObjectRef;
                        return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib,
                            cast_member: member_ref.cast_member,
                            object_type: obj_type,
                            name: Symbol::from_str(&obj_name),
                        })));
                    }

                    // newTexture/newShader/newModel/etc. — create and return a ref
                    let handler_name_str = handler_name.as_str();
                    if handler_name_str.starts_with("new") || handler_name_str.starts_with("delete") {
                        let obj_type = match handler_name_builtin {
                            BuiltInSymbol::NewTexture | BuiltInSymbol::DeleteTexture => BuiltInSymbol::Texture,
                            BuiltInSymbol::NewShader | BuiltInSymbol::DeleteShader => BuiltInSymbol::Shader,
                            BuiltInSymbol::NewModel | BuiltInSymbol::DeleteModel => BuiltInSymbol::Model,
                            BuiltInSymbol::NewModelResource | BuiltInSymbol::DeleteModelResource | BuiltInSymbol::NewMesh => BuiltInSymbol::ModelResource,
                            BuiltInSymbol::NewLight | BuiltInSymbol::DeleteLight => BuiltInSymbol::Light,
                            BuiltInSymbol::NewCamera | BuiltInSymbol::DeleteCamera => BuiltInSymbol::Camera,
                            BuiltInSymbol::NewGroup | BuiltInSymbol::DeleteGroup => BuiltInSymbol::Group,
                            BuiltInSymbol::NewMotion | BuiltInSymbol::DeleteMotion => BuiltInSymbol::Motion,
                            _ => BuiltInSymbol::Unknown,
                        };
                        let obj_name = if !args.is_empty() {
                            player.get_datum(&args[0]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };

                        if handler_name_str.starts_with("delete") {
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        let obj_sym = Symbol::from_str(&obj_name);
                                        match obj_type {
                                            BuiltInSymbol::Model | BuiltInSymbol::Group | BuiltInSymbol::Camera => {
                                                // Director deleteModel removes the model AND its child
                                                // subtree. Without that, children added via addChild are
                                                // orphaned (parent gone) and keep rendering at a fixed
                                                // local position. frog01 builds an invisible "car1"
                                                // template with the body (acar) + 4 wheels as children,
                                                // clones it 16×, then deleteModel("car1") — leaving the
                                                // original acar behind as a static, misplaced car. The
                                                // clones (children of car2..car17) are NOT in car1's
                                                // subtree, so they're unaffected.
                                                let mut doomed: std::collections::HashSet<String> =
                                                    std::collections::HashSet::new();
                                                doomed.insert(obj_name.to_ascii_lowercase());
                                                let mut changed = true;
                                                while changed {
                                                    changed = false;
                                                    for n in scene.nodes.iter() {
                                                        let nl = n.name.to_ascii_lowercase();
                                                        if !doomed.contains(&nl)
                                                            && doomed.contains(&n.parent_name.to_ascii_lowercase())
                                                        {
                                                            doomed.insert(nl);
                                                            changed = true;
                                                        }
                                                    }
                                                }
                                                scene.nodes.retain(|n| !doomed.contains(&n.name.to_ascii_lowercase()));
                                            }
                                            BuiltInSymbol::Light => {
                                                // Lights live in two places: the scene
                                                // graph (scene.nodes) and a separate
                                                // scene.lights Vec the renderer uses
                                                // for per-frame uniform setup. Without
                                                // dropping the lights entry, deleted
                                                // lights keep contributing to lighting
                                                // and are still visible via
                                                // sp.light.count.
                                                scene.nodes.retain(|n| n.name != obj_sym);
                                                scene.lights.retain(|l| l.name != obj_sym);
                                            }
                                            BuiltInSymbol::Shader => {
                                                // DefaultShader cannot be deleted (Director behavior)
                                                if obj_sym != Symbol::builtin(BuiltInSymbol::DefaultShader) {
                                                    scene.shaders.retain(|s| s.name != obj_sym);
                                                }
                                            }
                                            BuiltInSymbol::Motion => {
                                                scene.motions.retain(|m| m.name != obj_sym);
                                            }
                                            BuiltInSymbol::Texture => {
                                                scene.texture_images.remove(&obj_sym);
                                                scene.texture_content_version += 1;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            return Ok(player.alloc_datum(Datum::Void));
                        }

                        // Pre-read args for newMesh before mutable borrow
                        let mesh_num_faces = if handler_name.eq_builtin(BuiltInSymbol::NewMesh) && args.len() >= 2 {
                            player.get_datum(&args[1]).int_value().unwrap_or(0) as u32
                        } else { 0 };

                        // Pre-read model resource name for newModel(name, modelResource)
                        let new_model_resource_name = if handler_name.eq_builtin(BuiltInSymbol::NewModel) && args.len() >= 2 {
                            match player.get_datum(&args[1]) {
                                Datum::Shockwave3dObjectRef(r) if r.object_type == BuiltInSymbol::ModelResource => r.name,
                                _ => Symbol::empty(),
                            }
                        } else { Symbol::empty() };

                        // Pre-read type arg for newModelResource(name, #type, #facing), newLight(name, #type),
                        // newShader(name, #type)
                        let new_res_type = if (handler_name.eq_builtin(BuiltInSymbol::NewModelResource)
                            || handler_name.eq_builtin(BuiltInSymbol::NewMesh)
                            || handler_name.eq_builtin(BuiltInSymbol::NewLight)
                            || handler_name.eq_builtin(BuiltInSymbol::NewShader)) && args.len() >= 2
                        {
                            // Lowercase at the source: this comes from a SYMBOL, whose
                            // string form is the first-interned spelling, and every
                            // downstream compare uses lowercase literals.
                            player.get_datum(&args[1]).string_value().unwrap_or_default().to_ascii_lowercase()
                        } else { String::new() };
                        let new_res_facing = if handler_name.eq_builtin(BuiltInSymbol::NewModelResource) && args.len() >= 3 {
                            player.get_datum(&args[2]).string_value().unwrap_or_default().to_ascii_lowercase()
                        } else { String::new() };

                        let obj_sym = Symbol::from_str(&obj_name);
                        // Add to parsed scene
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    use crate::director::chunks::w3d::types::*;
                                    let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
                                    match obj_type {
                                        BuiltInSymbol::Model => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_sym, node_type: W3dNodeType::Model,
                                                parent_name: Symbol::builtin(BuiltInSymbol::World),
                                                resource_name: Symbol::empty(),
                                                model_resource_name: new_model_resource_name,
                                                shader_name: Symbol::empty(),
                                                visibility: 1,
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        BuiltInSymbol::Group => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_sym, node_type: W3dNodeType::Group,
                                                parent_name: Symbol::builtin(BuiltInSymbol::World),
                                                resource_name: Symbol::empty(), model_resource_name: Symbol::empty(),
                                                shader_name: Symbol::empty(),
                                                visibility: 1,
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        BuiltInSymbol::Camera => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_sym, node_type: W3dNodeType::View,
                                                parent_name: Symbol::builtin(BuiltInSymbol::World),
                                                resource_name: Symbol::empty(), model_resource_name: Symbol::empty(),
                                                shader_name: Symbol::empty(),
                                                visibility: 1,
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
                                            });
                                        }
                                        BuiltInSymbol::Light => {
                                            let light_type = match_ci!(new_res_type.as_str(), {
                                                "ambient" => W3dLightType::Ambient,
                                                "directional" => W3dLightType::Directional,
                                                "spot" => W3dLightType::Spot,
                                                _ => W3dLightType::Point,
                                            });
                                            log(&format!(
                                                "[W3D-NEWLIGHT] name=\"{}\" type_arg=\"{}\" → {:?}",
                                                obj_name, new_res_type, light_type
                                            ));
                                            scene.lights.push(W3dLight {
                                                name: obj_sym,
                                                light_type,
                                                color: [191.0/255.0, 191.0/255.0, 191.0/255.0], // Director default: color(191,191,191)
                                                attenuation: [1.0, 0.0, 0.0],
                                                spot_angle: 90.0, // Director default
                                                enabled: true,
                                                ..Default::default()
                                            });
                                            scene.nodes.push(W3dNode {
                                                name: obj_sym, node_type: W3dNodeType::Light,
                                                parent_name: Symbol::builtin(BuiltInSymbol::World),
                                                resource_name: Symbol::empty(), model_resource_name: Symbol::empty(),
                                                shader_name: Symbol::empty(),
                                                visibility: 1,
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        BuiltInSymbol::Shader => {
                                            // newShader(name, #type). The type arg was
                                            // dropped, so a #normalMap shader was read with
                                            // #standard layer order and rendered its NORMAL
                                            // map as the base texture — AreaZero's menu robot
                                            // came out blue/purple.
                                            let shader_type = match_ci!(new_res_type.as_str(), {
                                                "normalmap" => W3dShaderType::NormalMap,
                                                "painter" => W3dShaderType::Painter,
                                                "inker" => W3dShaderType::Inker,
                                                "engraver" => W3dShaderType::Engraver,
                                                "newsprint" => W3dShaderType::Newsprint,
                                                _ => W3dShaderType::LitTexture,
                                            });
                                            scene.shaders.push(W3dShader {
                                                name: obj_sym,
                                                shader_type,
                                                ..Default::default()
                                            });
                                        }
                                        BuiltInSymbol::ModelResource => {
                                            // Generate primitive geometry based on type
                                            // For #plane default: both front+back (2 meshes). #front/#back = single mesh.
                                            let want_front = new_res_facing.is_empty() || new_res_facing == "front" || new_res_facing == "both";
                                            let want_back = new_res_facing.is_empty() || new_res_facing == "back" || new_res_facing == "both";
                                            // For plane, default facing generates both sides; for others, default is #front only
                                            let (plane_front, plane_back) = if new_res_type == "plane" {
                                                (want_front, want_back)
                                            } else {
                                                let f = new_res_facing.is_empty() || new_res_facing == "front" || new_res_facing == "both";
                                                let b = new_res_facing == "back" || new_res_facing == "both";
                                                (f, b)
                                            };

                                            let mut meshes: Vec<ClodDecodedMesh> = Vec::new();
                                            let (positions, normals, tex_coords, faces) = match new_res_type.as_str() {
                                                "plane" => {
                                                    // 1x1 quad centered at origin
                                                    // Front face: normal +Z; Back face: normal -Z (reversed winding)
                                                    if plane_front {
                                                        meshes.push(ClodDecodedMesh {
                                                            name: obj_sym,
                                                            positions: vec![[-0.5,-0.5,0.0],[0.5,-0.5,0.0],[0.5,0.5,0.0],[-0.5,0.5,0.0]],
                                                            normals: vec![[0.0,0.0,1.0]; 4],
                                                            tex_coords: vec![vec![[0.0,1.0],[1.0,1.0],[1.0,0.0],[0.0,0.0]]],
                                                            faces: vec![[0,1,2],[0,2,3]],
                                                            diffuse_colors: vec![], specular_colors: vec![],
                                                            bone_indices: vec![], bone_weights: vec![],
                                                        });
                                                    }
                                                    if plane_back {
                                                        meshes.push(ClodDecodedMesh {
                                                            name: obj_sym,
                                                            positions: vec![[-0.5,-0.5,0.0],[0.5,-0.5,0.0],[0.5,0.5,0.0],[-0.5,0.5,0.0]],
                                                            normals: vec![[0.0,0.0,-1.0]; 4],
                                                            tex_coords: vec![vec![[1.0,1.0],[0.0,1.0],[0.0,0.0],[1.0,0.0]]],
                                                            faces: vec![[0,2,1],[0,3,2]],
                                                            diffuse_colors: vec![], specular_colors: vec![],
                                                            bone_indices: vec![], bone_weights: vec![],
                                                        });
                                                    }
                                                    // Return empty tuple — meshes already pushed above
                                                    (vec![], vec![], vec![vec![]], vec![])
                                                },
                                                "particle" => {
                                                    // Particle resources use a single quad billboard
                                                    let p = vec![
                                                        [-0.5, -0.5, 0.0_f32],
                                                        [ 0.5, -0.5, 0.0],
                                                        [ 0.5,  0.5, 0.0],
                                                        [-0.5,  0.5, 0.0],
                                                    ];
                                                    let n = vec![[0.0, 0.0, 1.0_f32]; 4];
                                                    let uv = vec![vec![[0.0, 1.0_f32], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]];
                                                    let f = vec![[0u32, 1, 2], [0, 2, 3]];
                                                    (p, n, uv, f)
                                                },
                                                "box" => {
                                                    // Unit cube centered at origin
                                                    let p = vec![
                                                        // Front face
                                                        [-0.5, -0.5,  0.5_f32], [ 0.5, -0.5,  0.5], [ 0.5,  0.5,  0.5], [-0.5,  0.5,  0.5],
                                                        // Back face
                                                        [ 0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5,  0.5, -0.5], [ 0.5,  0.5, -0.5],
                                                        // Top face
                                                        [-0.5,  0.5,  0.5], [ 0.5,  0.5,  0.5], [ 0.5,  0.5, -0.5], [-0.5,  0.5, -0.5],
                                                        // Bottom face
                                                        [-0.5, -0.5, -0.5], [ 0.5, -0.5, -0.5], [ 0.5, -0.5,  0.5], [-0.5, -0.5,  0.5],
                                                        // Right face
                                                        [ 0.5, -0.5,  0.5], [ 0.5, -0.5, -0.5], [ 0.5,  0.5, -0.5], [ 0.5,  0.5,  0.5],
                                                        // Left face
                                                        [-0.5, -0.5, -0.5], [-0.5, -0.5,  0.5], [-0.5,  0.5,  0.5], [-0.5,  0.5, -0.5],
                                                    ];
                                                    let n = vec![
                                                        [0.0, 0.0, 1.0_f32], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0],
                                                        [0.0, 0.0, -1.0], [0.0, 0.0, -1.0], [0.0, 0.0, -1.0], [0.0, 0.0, -1.0],
                                                        [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0],
                                                        [0.0, -1.0, 0.0], [0.0, -1.0, 0.0], [0.0, -1.0, 0.0], [0.0, -1.0, 0.0],
                                                        [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0],
                                                        [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0],
                                                    ];
                                                    let face_uv = vec![[0.0, 1.0_f32], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
                                                    let mut uv_all = Vec::with_capacity(24);
                                                    for _ in 0..6 { uv_all.extend_from_slice(&face_uv); }
                                                    let uv = vec![uv_all];
                                                    let f = vec![
                                                        [0u32,1,2],[0,2,3], [4,5,6],[4,6,7], [8,9,10],[8,10,11],
                                                        [12,13,14],[12,14,15], [16,17,18],[16,18,19], [20,21,22],[20,22,23],
                                                    ];
                                                    (p, n, uv, f)
                                                },
                                                "sphere" => {
                                                    // UV sphere matching Director's default tessellation
                                                    let segments = 8u32;
                                                    let rings = 6u32;
                                                    let mut p = Vec::new();
                                                    let mut n = Vec::new();
                                                    let mut uv_data = Vec::new();
                                                    let mut f = Vec::new();
                                                    // UV scale 4× tiles the 2×2 checker into a dense grid matching Director
                                                    let uv_scale = 1.0f32;
                                                    for j in 0..=rings {
                                                        let v = j as f32 / rings as f32;
                                                        let phi = v * std::f32::consts::PI;
                                                        for i in 0..=segments {
                                                            let u = i as f32 / segments as f32;
                                                            let theta = u * 2.0 * std::f32::consts::PI;
                                                            let x = phi.sin() * theta.cos();
                                                            let y = phi.cos();
                                                            let z = phi.sin() * theta.sin();
                                                            // let x = phi.sin() * theta.cos();
                                                            // let y = phi.sin() * theta.sin();
                                                            // let z = phi.cos();
                                                            p.push([x * 0.5, y * 0.5, z * 0.5]);
                                                            n.push([x, y, z]);
                                                            uv_data.push([u * uv_scale, v * uv_scale]);
                                                        }
                                                    }
                                                    for j in 0..rings {
                                                        for i in 0..segments {
                                                            let a = j * (segments + 1) + i;
                                                            let b = a + 1;
                                                            let c = a + segments + 1;
                                                            let d = c + 1;
                                                            f.push([a, c, d]);
                                                            f.push([a, d, b]);
                                                        }
                                                    }
                                                    (p, n, vec![uv_data], f)
                                                },
                                                "cylinder" => {
                                                    // Simple cylinder (8 segments, height 1)
                                                    let segments = 8u32;
                                                    let mut p = Vec::new();
                                                    let mut normals = Vec::new();
                                                    let mut uv_data = Vec::new();
                                                    let mut f = Vec::new();
                                                    // Side vertices
                                                    for j in 0..=1u32 {
                                                        let y = j as f32 - 0.5;
                                                        for i in 0..=segments {
                                                            let u = i as f32 / segments as f32;
                                                            let theta = u * 2.0 * std::f32::consts::PI;
                                                            let x = theta.cos();
                                                            let z = theta.sin();
                                                            p.push([x * 0.5, y, z * 0.5]);
                                                            normals.push([x, 0.0, z]);
                                                            uv_data.push([u, j as f32]);
                                                        }
                                                    }
                                                    for i in 0..segments {
                                                        let a = i;
                                                        let b = a + 1;
                                                        let c = a + segments + 1;
                                                        let d = c + 1;
                                                        f.push([a, c, d]);
                                                        f.push([a, d, b]);
                                                    }
                                                    (p, normals, vec![uv_data], f)
                                                },
                                                _ => {
                                                    // Unknown type or newMesh — empty geometry
                                                    (vec![], vec![], vec![vec![]], vec![])
                                                }
                                            };

                                            // For non-plane types, build a single mesh from the returned geometry
                                            if !positions.is_empty() && !faces.is_empty() {
                                                meshes.push(ClodDecodedMesh {
                                                    name: obj_sym,
                                                    positions,
                                                    normals,
                                                    tex_coords,
                                                    faces,
                                                    diffuse_colors: vec![],
                                                    specular_colors: vec![],
                                                    bone_indices: vec![],
                                                    bone_weights: vec![],
                                                });
                                            }

                                            let total_faces: u32 = meshes.iter().map(|m| m.faces.len() as u32).sum();
                                            let num_faces = if total_faces > 0 { total_faces } else { mesh_num_faces };
                                            let mut mesh_info = ClodMeshInfo::default();
                                            mesh_info.num_faces = num_faces;
                                            // Store primitive type so dimension setters can regenerate
                                            let prim_type = if !new_res_type.is_empty() {
                                                Some(new_res_type.clone())
                                            } else { None };
                                            // Create a default shader + material for the
                                            // new resource so the renderer can bind it
                                            // (Director shows a red/white checkerboard on
                                            // untextured primitives).
                                            let shader_name = Symbol::from_str(&format!("{}_Shader", obj_name));
                                            let material_name = Symbol::from_str(&format!("{}_Material", obj_name));
                                            scene.materials.push(W3dMaterial {
                                                name: material_name,
                                                // Director's default for new primitives: ambient = diffuse = white
                                                ambient: [1.0, 1.0, 1.0, 1.0],
                                                ..Default::default()
                                            });
                                            scene.shaders.push(W3dShader {
                                                name: shader_name,
                                                material_name,
                                                ..Default::default()
                                            });
                                            let num_meshes = meshes.len().max(1);
                                            scene.model_resources.insert(obj_sym, ModelResourceInfo {
                                                name: obj_sym,
                                                mesh_infos: vec![mesh_info],
                                                shader_bindings: vec![ModelShaderBinding {
                                                    name: shader_name,
                                                    mesh_bindings: vec![Symbol::empty(); num_meshes],
                                                }],
                                                primitive_type: prim_type,
                                                primitive_width: 1.0,
                                                primitive_length: 1.0,
                                                primitive_height: 1.0,
                                                primitive_radius: 1.0,
                                                primitive_top_radius: 1.0,
                                                primitive_resolution: 0, // 0 → default tessellation
                                                primitive_start_angle: 0.0,
                                                primitive_end_angle: 360.0,
                                                // Spec lists topCap default FALSE / bottomCap TRUE, but every
                                                // capped cylinder in these movies (Coke can, maze pipes) relies on
                                                // BOTH caps without setting topCap, and Shockwave renders them
                                                // sealed — so default both TRUE. Models that want an open tube set
                                                // the flag explicitly (the ghost body sets topCap=0 AND bottomCap=0).
                                                primitive_top_cap: true,
                                                primitive_bottom_cap: true,
                                                // #back/#both (e.g. skybox cylinder) → render two-sided
                                                // so the inward-facing surface isn't backface-culled.
                                                primitive_facing: new_res_facing.clone(),
                                                ..Default::default()
                                            });

                                            // Store generated mesh geometry so the renderer can upload it
                                            if !meshes.is_empty() {
                                                scene.clod_meshes.insert(obj_sym, meshes);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        // For newTexture(name, #fromImageObject/#fromCastMember, source)
                        if handler_name.eq_builtin(BuiltInSymbol::NewTexture) && args.len() >= 3 {
                            let tex_type = player.get_datum(&args[1]).string_value().unwrap_or_default();
                            if tex_type == "fromCastMember" {
                                let source_member_ref = match player.get_datum(&args[2]) {
                                    Datum::CastMember(r) => Some(r.clone()),
                                    _ => None,
                                };
                                if let Some(src_ref) = source_member_ref {
                                    // Off-screen Flash → 3D texture (frog01 environment): capture the
                                    // SWF bytes + native dims so we can kick off a Ruffle render below.
                                    // The synchronous bitmap path produces None for Flash members.
                                    let flash_dispatch: Option<(Vec<u8>, u32, u32)> = {
                                        let src_member = player.movie.cast_manager.find_member_by_ref(&src_ref);
                                        match src_member.map(|m| &m.member_type) {
                                            Some(CastMemberType::Flash(flash)) => {
                                                let (l, t, r, b) = flash.effective_rect();
                                                let mut fw = (r - l).max(1) as u32;
                                                let mut fh = (b - t).max(1) as u32;
                                                // Director renders a Flash member into the texture at the SWF's OWN
                                                // stage size, NOT the cast member's display rect. frog01's
                                                // `front`/`back` banner have a TALL display rect (640×1320) but a
                                                // WIDE swf stage (≈1320×640); using the rect letterboxed the wide
                                                // banner into a tall frame → mostly-black. Parse the frame RECT from
                                                // the (uncompressed FWS) SWF header for the real stage size.
                                                //
                                                // Do NOT round to power-of-2: an earlier POT rounding (to match
                                                // Director's reported 1024×512) DISTORTED other members — the
                                                // bark/wood log textures came out the wrong size, so the logs looked
                                                // gappy / "open caps". NPOT textures are fine in WebGL2; keep the raw
                                                // stage size.
                                                if flash.data.len() >= 9 && &flash.data[0..3] == b"FWS" {
                                                    let bits = &flash.data[8..];
                                                    let mut bitpos = 0usize;
                                                    let mut read = |n: usize| -> u32 {
                                                        let mut v = 0u32;
                                                        for _ in 0..n {
                                                            let byte = bits.get(bitpos >> 3).copied().unwrap_or(0);
                                                            v = (v << 1) | ((byte >> (7 - (bitpos & 7))) & 1) as u32;
                                                            bitpos += 1;
                                                        }
                                                        v
                                                    };
                                                    let nbits = read(5) as usize;
                                                    if nbits > 0 && nbits <= 31 {
                                                        let _xmin = read(nbits);
                                                        let xmax = read(nbits);
                                                        let _ymin = read(nbits);
                                                        let ymax = read(nbits);
                                                        let w = (xmax / 20).max(1); // twips → px
                                                        let h = (ymax / 20).max(1);
                                                        if w > 1 && h > 1 { fw = w; fh = h; }
                                                    }
                                                }
                                                Some((flash.data.clone(), fw, fh))
                                            }
                                            _ => None,
                                        }
                                    };
                                    let rgba_data = {
                                        let src_member = player.movie.cast_manager.find_member_by_ref(&src_ref);
                                        src_member.and_then(|m| {
                                            match &m.member_type {
                                                CastMemberType::Bitmap(bmp_member) => {
                                                    let bmp = player.bitmap_manager.get_bitmap(bmp_member.image_ref)?;
                                                    let w = bmp.width;
                                                    let h = bmp.height;
                                                    let palettes = player.movie.cast_manager.palettes();
                                                    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
                                                    let mut any_opaque = false;
                                                    for y in 0..h as usize {
                                                        for x in 0..w as usize {
                                                            let (r, g, b, a) = bmp.get_pixel_color_with_alpha(&palettes, x as u16, y as u16);
                                                            let idx = (y * w as usize + x) * 4;
                                                            rgba[idx] = r;
                                                            rgba[idx + 1] = g;
                                                            rgba[idx + 2] = b;
                                                            rgba[idx + 3] = a;
                                                            if a != 0 { any_opaque = true; }
                                                        }
                                                    }
                                                    // A 32-bit cast bitmap with no real alpha channel (use_alpha
                                                    // off) or whose alpha bytes are all 0 must render OPAQUE as a
                                                    // 3D texture — Director ignores texture alpha for #standard
                                                    // shaders. Without this, frog01's car-colour textures cc2-cc5
                                                    // (32-bit, alpha 0) made the car bodies fully transparent
                                                    // (invisible); cc1 happened to have alpha 255 so it showed.
                                                    if !bmp.use_alpha || !any_opaque {
                                                        for px in 0..(w as usize) * (h as usize) {
                                                            rgba[px * 4 + 3] = 255;
                                                        }
                                                    }
                                                    log(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): {}x{} from member {}:{} '{}' (forced_opaque={})",
                                                        obj_name, w, h, src_ref.cast_lib, src_ref.cast_member, m.name,
                                                        !bmp.use_alpha || !any_opaque
                                                    ));
                                                    Some((w, h, rgba))
                                                }
                                                // Flash members are rendered off-screen via Ruffle
                                                // (flash_dispatch above), not the synchronous bitmap path.
                                                CastMemberType::Flash(_) => None,
                                                _ => {
                                                    console_warn!(
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): member {}:{} '{}' is {} not Bitmap",
                                                        obj_name, src_ref.cast_lib, src_ref.cast_member,
                                                        m.name, m.member_type.type_string()
                                                    );
                                                    None
                                                }
                                            }
                                        })
                                    };
                                    if let Some((w, h, rgba)) = rgba_data {
                                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref);
                                        if let Some(member) = member {
                                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                                if let Some(scene) = w3d.scene_mut() {
                                                    let mut tex_data = Vec::with_capacity(8 + rgba.len());
                                                    tex_data.extend_from_slice(&(w as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&(h as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&rgba);
                                                    scene.texture_images.insert(obj_sym, tex_data);
                                                    scene.texture_content_version += 1;
                                                    log(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): stored {}x{} RGBA",
                                                        obj_name, w, h
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    // Flash source: start an off-screen Ruffle render and route its
                                    // captured frames into this texture (update_flash_frame consults
                                    // player.flash_texture_targets). The synthetic sprite number is
                                    // NEGATIVE so it never collides with an on-stage channel, and
                                    // deterministic per source member so a movie restart reuses the
                                    // same Ruffle instance instead of leaking a new one.
                                    if let Some((swf, fw, fh)) = flash_dispatch {
                                        let synthetic = {
                                            let raw = 2000i32
                                                + src_ref.cast_lib.max(0) * 1000
                                                + src_ref.cast_member.max(0);
                                            -(raw.min(30000)) as i16
                                        };
                                        player.flash_texture_targets
                                            .insert(synthetic, (member_ref.clone(), obj_name.clone()));
                                        // Transparent 1×1 placeholder so the plane isn't the default
                                        // checker (primitive fallback) until the first Flash frame lands.
                                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                                if let Some(scene) = w3d.scene_mut() {
                                                    if !scene.texture_images.contains_key(&Symbol::from_str(&obj_name)) {
                                                        let mut ph = Vec::with_capacity(12);
                                                        ph.extend_from_slice(&1u32.to_le_bytes());
                                                        ph.extend_from_slice(&1u32.to_le_bytes());
                                                        ph.extend_from_slice(&[0u8, 0, 0, 0]);
                                                        scene.texture_images.insert(Symbol::from_str(&obj_name.clone()), ph);
                                                        scene.texture_content_version += 1;
                                                    }
                                                }
                                            }
                                        }
                                        crate::js_api::JsApi::dispatch_flash_member_loaded(
                                            synthetic as i32, src_ref.cast_lib, src_ref.cast_member,
                                            &swf, fw, fh, true, -1,
                                        );
                                        log(&format!(
                                            "[W3D] newTexture(\"{}\"): Flash member {}:{} -> off-screen Ruffle (sprite {}, {}x{})",
                                            obj_name, src_ref.cast_lib, src_ref.cast_member, synthetic, fw, fh
                                        ));
                                    }
                                }
                            } else if tex_type == "fromImageObject" {
                                if let Ok(bitmap_ref) = player.get_datum(&args[2]).to_bitmap_ref() {
                                    let rgba_data = if let Some(bmp) = player.bitmap_manager.get_bitmap(*bitmap_ref) {
                                        let w = bmp.width;
                                        let h = bmp.height;
                                        let palettes = player.movie.cast_manager.palettes();
                                        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
                                        for y in 0..h as usize {
                                            for x in 0..w as usize {
                                                let (r, g, b, a) = bmp.get_pixel_color_with_alpha(&palettes, x as u16, y as u16);
                                                let idx = (y * w as usize + x) * 4;
                                                rgba[idx] = r;
                                                rgba[idx + 1] = g;
                                                rgba[idx + 2] = b;
                                                rgba[idx + 3] = a;
                                            }
                                        }
                                        // Post-process: when bitmap has use_alpha, trailing rows of fully
                                        // opaque white (255,255,255,255) are unfilled padding from power-of-2
                                        // texture sizing. Make them transparent so 3D overlays don't show
                                        // white blocks below the actual content.
                                        if bmp.use_alpha {
                                            let w_usize = w as usize;
                                            let h_usize = h as usize;
                                            // Scan from bottom row upward: stop at first row that isn't all white-opaque
                                            for y in (0..h_usize).rev() {
                                                let row_start = y * w_usize * 4;
                                                let row_all_white_opaque = (0..w_usize).all(|x| {
                                                    let i = row_start + x * 4;
                                                    rgba[i] == 255 && rgba[i+1] == 255 && rgba[i+2] == 255 && rgba[i+3] == 255
                                                });
                                                if !row_all_white_opaque { break; }
                                                // Make this row transparent
                                                for x in 0..w_usize {
                                                    let i = row_start + x * 4;
                                                    rgba[i + 3] = 0;
                                                }
                                            }
                                        }
                                        Some((w, h, rgba))
                                    } else {
                                        None
                                    };

                                    if let Some((w, h, rgba)) = rgba_data {
                                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref);
                                        if let Some(member) = member {
                                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                                if let Some(scene) = w3d.scene_mut() {
                                                    let mut tex_data = Vec::with_capacity(8 + rgba.len());
                                                    tex_data.extend_from_slice(&(w as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&(h as u32).to_le_bytes());
                                                    tex_data.extend_from_slice(&rgba);
                                                    scene.texture_images.insert(obj_sym, tex_data);
                                                    scene.texture_content_version += 1;
                                                    // Log pixel stats
                                                    let total = rgba.len() / 4;
                                                    let alpha_lt255 = rgba.chunks(4).filter(|p| p[3] < 255).count();
                                                    let alpha_eq0 = rgba.chunks(4).filter(|p| p[3] == 0).count();
                                                    let first_lt255 = rgba.chunks(4).enumerate().find(|(_, p)| p[3] < 255)
                                                        .map(|(i, p)| format!("px{}=({},{},{},{})", i, p[0], p[1], p[2], p[3]))
                                                        .unwrap_or("none".to_string());
                                                    log(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromImageObject): {}x{} alpha<255={}/{} alpha=0={} first_partial={}",
                                                        obj_name, w, h, alpha_lt255, total, alpha_eq0, first_lt255
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        use crate::director::lingo::datum::Shockwave3dObjectRef;
                        return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                            cast_lib: member_ref.cast_lib,
                            cast_member: member_ref.cast_member,
                            object_type: obj_type,
                            name: obj_sym,
                        })));
                    }

                    // image — return the rendered 3D world as a bitmap ref
                    if handler_name_builtin == BuiltInSymbol::Image {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // getPref, setPref — stubs. (`loadFile` is handled on the
                    // async path in cast_member_ref.rs; it must fetch the W3D.)
                    if handler_name == "getPref" || handler_name == "setPref" {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // extrude3d(targetScene): extrude THIS text member's glyphs into a
                    // model resource inside the target 3D scene member (arg 0) and
                    // return that resource. `member_ref` was promoted to a 3D-text
                    // member by ensure_text3d at call() entry, so its retained
                    // text3d_source/state hold the glyphs + extrude params.
                    if handler_name == "extrude3d" {
                        let (source, state) = {
                            let m = player.movie.cast_manager.find_member_by_ref(&member_ref);
                            let w3d = m.and_then(|m| m.member_type.as_shockwave3d());
                            match (
                                w3d.and_then(|w| w.text3d_source.clone()),
                                w3d.and_then(|w| w.text3d_state.clone()),
                            ) {
                                (Some(s), Some(st)) => (s, st),
                                _ => return Ok(player.alloc_datum(Datum::Void)),
                            }
                        };
                        let target_ref = match args.get(0).map(|a| player.get_datum(a)) {
                            Some(Datum::CastMember(r)) => r.clone(),
                            _ => return Ok(player.alloc_datum(Datum::Void)),
                        };
                        let src_name = player.movie.cast_manager.find_member_by_ref(&member_ref)
                            .map(|m| m.name.clone()).unwrap_or_else(|| "text".to_string());
                        // Each extrude3d call returns a DISTINCT model resource — Director does
                        // too, even when the SAME text member is reused (set text → extrude →
                        // newModel → repeat), which frog01's title screen does. Keying the mesh
                        // by member name alone made every call overwrite the one "<name>_extrude3d"
                        // mesh, so every model showed the LAST text ("www.jellygames.com"). Make
                        // the name unique per call via the target scene's current resource count.
                        let seq = player.movie.cast_manager.find_member_by_ref(&target_ref)
                            .and_then(|m| m.member_type.as_shockwave3d())
                            .map(|w| w.runtime_state.text3d_resources.len())
                            .unwrap_or(0);
                        let resname = format!("{}_extrude3d_{}", src_name, seq);
                        let mut mesh = match Self::build_text3d_mesh(&source, &state) {
                            Some(m) => m,
                            None => return Ok(player.alloc_datum(Datum::Void)),
                        };
                        mesh.name = Symbol::from_str(&resname.clone());
                        let num_faces = mesh.faces.len() as u32;
                        if let Some(target) = player.movie.cast_manager.find_mut_member_by_ref(&target_ref) {
                            if let Some(w3d_t) = target.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d_t.scene_mut() {
                                    use crate::director::chunks::w3d::types::*;
                                    scene.clod_meshes.insert(Symbol::from_str(&resname.clone()), vec![mesh]);
                                    let mut mi = ClodMeshInfo::default();
                                    mi.num_faces = num_faces;
                                    scene.model_resources.insert(Symbol::from_str(&resname.clone()), ModelResourceInfo {
                                        name: Symbol::from_str(&resname.clone()),
                                        mesh_infos: vec![mi],
                                        shader_bindings: vec![ModelShaderBinding {
                                            name: Symbol::from_str(&String::new()),
                                            mesh_bindings: vec![Symbol::from_str(&String::new())],
                                        }],
                                        ..Default::default()
                                    });
                                    scene.mesh_content_version += 1;
                                }
                                w3d_t.runtime_state.text3d_resources.insert(Symbol::from_str(&resname.clone()), (source, state));
                            }
                        }
                        use crate::director::lingo::datum::Shockwave3dObjectRef;
                        return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                            cast_lib: target_ref.cast_lib,
                            cast_member: target_ref.cast_member,
                            object_type: BuiltInSymbol::ModelResource,
                            name: Symbol::from_str(&resname),
                        })));
                    }

                    // If no parsed scene exists, create a minimal empty scene
                    if w3d.parsed_scene.is_none() {
                        use crate::director::chunks::w3d::types::*;
                        use std::collections::HashMap;
                        let mut empty_scene = W3dScene {
                            materials: Vec::new(), shaders: Vec::new(), nodes: Vec::new(),
                            lights: Vec::new(), texture_images: HashMap::new(), texture_infos: Vec::new(),
                            skeletons: Vec::new(), motions: Vec::new(), model_resources: HashMap::new(),
                            clod_meshes: HashMap::new(), clod_decoders: HashMap::new(), raw_meshes: Vec::new(),
                            mesh_content_version: 0,
                            texture_content_version: 0,
                            model_root_com: HashMap::new(),
                        };
                        empty_scene.nodes.push(W3dNode {
                            name: Symbol::builtin(BuiltInSymbol::World),
                            node_type: W3dNodeType::Group,
                            parent_name: Symbol::empty(),
                            resource_name: Symbol::empty(),
                            model_resource_name: Symbol::empty(),
                            shader_name: Symbol::empty(),
                            visibility: 1,
                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                            screen_width: player.movie.rect.right as i32,
                            screen_height: player.movie.rect.bottom as i32,
                            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
                        });
                        empty_scene.nodes.push(W3dNode {
                            name: Symbol::builtin(BuiltInSymbol::DefaultView),
                            node_type: W3dNodeType::View,
                            parent_name: Symbol::builtin(BuiltInSymbol::World),
                            resource_name: Symbol::empty(),
                            model_resource_name: Symbol::empty(),
                            shader_name: Symbol::empty(),
                            visibility: 1,
                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                            screen_width: player.movie.rect.right as i32,
                            screen_height: player.movie.rect.bottom as i32,
                            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,500.0,1.0],
                        });
                        empty_scene.shaders.push(W3dShader {
                            name: Symbol::builtin(BuiltInSymbol::DefaultShader),
                            ..Default::default()
                        });
                        // Built-in "defaultmodel" plane resource (used by overlay scripts)
                        let member_mut = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d_mut) = member_mut.member_type.as_shockwave3d_mut() {
                            w3d_mut.parsed_scene = Some(std::rc::Rc::new(empty_scene));
                        }
                    }
                    // Re-fetch after potential mutation
                    let cast_member = player.movie.cast_manager.find_member_by_ref(&member_ref)
                        .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                    let w3d = cast_member.member_type.as_shockwave3d()
                        .ok_or_else(|| ScriptError::new("Not a 3D member".to_string()))?;
                    let scene = w3d.parsed_scene.as_ref().unwrap();

                    // Resolve name from argument (string or int index)
                    let obj_name = if args.is_empty() {
                        let count = Self::get_3d_collection_count(scene, handler_name);
                        return Ok(player.alloc_datum(Datum::Int(count)));
                    } else {
                        let arg = player.get_datum(&args[0]).clone();
                        match arg {
                            Datum::String(s) => Symbol::from_str(&s),
                            Datum::Int(idx) => {
                                Self::get_3d_object_name_by_index(scene, handler_name, idx as usize)
                                    .unwrap_or_default()
                            }
                            _ => Symbol::from_str(&arg.string_value().unwrap_or_default()),
                        }
                    };

                    if obj_name.is_empty() {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // Check if the named object actually exists in the scene.
                    // Per Director docs: "If no [object] exists for the specified parameter, returns void."
                    // Symbol equality is already case-insensitive (all strings interned as lowercase).
                    use crate::director::chunks::w3d::types::W3dNodeType;
                    let resolved_name: Option<Symbol> = match handler_name_builtin {
                        BuiltInSymbol::ModelResource => scene.model_resources.keys()
                            .find(|k| **k == obj_name).copied(),
                        BuiltInSymbol::Model => scene.nodes.iter()
                            .find(|n| n.node_type == W3dNodeType::Model && n.name == obj_name)
                            .map(|n| n.name),
                        BuiltInSymbol::Shader => scene.shaders.iter()
                            .find(|s| s.name == obj_name)
                            .map(|s| s.name),
                        BuiltInSymbol::Texture => scene.texture_images.keys()
                            .find(|k| **k == obj_name).copied(),
                        BuiltInSymbol::Light => scene.lights.iter()
                            .find(|l| l.name == obj_name)
                            .map(|l| l.name),
                        BuiltInSymbol::Camera => scene.nodes.iter()
                            .find(|n| n.node_type == W3dNodeType::View && n.name == obj_name)
                            .map(|n| n.name),
                        BuiltInSymbol::Group => scene.nodes.iter()
                            .find(|n| n.node_type == W3dNodeType::Group && n.name == obj_name)
                            .map(|n| n.name)
                            // "World" is the implicit scene root in W3D — there's
                            // no actual Group node for it, but Director scripts
                            // routinely do `sp.group("World").addChild(child)` to
                            // unparent a node back to the root. Synthesize the
                            // name so the ref is valid; addChild's setter side
                            // already handles "World" by clearing the child's
                            // parent reference.
                            .or_else(|| if obj_name == BuiltInSymbol::World {
                                Some(Symbol::builtin(BuiltInSymbol::World))
                            } else { None }),
                        BuiltInSymbol::Motion => scene.motions.iter()
                            .find(|m| m.name == obj_name)
                            .map(|m| m.name),
                        _ => Some(obj_name), // Unknown collection types pass through
                    };
                    let resolved_name = match resolved_name {
                        Some(name) => name,
                        None => return Ok(player.alloc_datum(Datum::Void)),
                    };

                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib,
                        cast_member: member_ref.cast_member,
                        object_type: handler_name_builtin,
                        name: resolved_name,
                    })))
                })
            }
            BuiltInSymbol::ModelsUnderRay => {
                reserve_player_mut(|player| {
                    let member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    if args.len() < 2 {
                        return Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, VecDeque::new(), false,
                        )));
                    }
                    let origin = player.get_datum(&args[0]).to_vector()?;
                    let direction = player.get_datum(&args[1]).to_vector()?;

                    // Director's modelsUnderRay accepts EITHER the positional form
                    //   (loc, dir, maxNumber, #detailed [, modelList])
                    // OR the documented options-list form (Director 11.5 dictionary)
                    //   (loc, dir, optionsList)
                    // where optionsList is a property list with #maxNumberOfModels,
                    // #levelOfDetail (#simple default / #detailed), #modelList and
                    // #maxDistance. Rasterwerks' C_MissilePhysics uses the proplist form;
                    // parsing it as a positional int silently dropped #modelList/#detailed,
                    // so missiles hit the firer's own proxy/walls -> bots shot themselves.
                    use crate::player::handlers::datum_handlers::prop_list::PropListUtils;
                    let mut max_models: i32 = 100;
                    let mut detailed = false;
                    let mut max_dist: f32 = 100000.0;
                    // #modelList: a list of model REFERENCES to restrict the cast to. An
                    // empty/absent list means "no restriction" (test all), matching the
                    // dictionary's "if omitted, all models" wording.
                    let mut model_whitelist: std::collections::HashSet<Symbol> = std::collections::HashSet::new();

                    let is_proplist = args.len() > 2 && matches!(player.get_datum(&args[2]), Datum::PropList(..));
                    if is_proplist {
                        let (map, map_sorted) = {
                            let (m, srt) = player.get_datum(&args[2]).to_map_tuple()?;
                            (m.clone(), srt)
                        };
                        let v = PropListUtils::get_by_concrete_key(&map, &Datum::Symbol(Symbol::from_str(&"maxNumberOfModels".to_owned())), &player.allocator, map_sorted)?;
                        if let Ok(n) = player.get_datum(&v).int_value() { max_models = n; }
                        let v = PropListUtils::get_by_concrete_key(&map, &Datum::Symbol(Symbol::from_str(&"levelOfDetail".to_owned())), &player.allocator, map_sorted)?;
                        if player.get_datum(&v).string_value().unwrap_or_default().eq_ignore_ascii_case("detailed") { detailed = true; }
                        let v = PropListUtils::get_by_concrete_key(&map, &Datum::Symbol(Symbol::from_str(&"maxDistance".to_owned())), &player.allocator, map_sorted)?;
                        match player.get_datum(&v) {
                            Datum::Int(i) => max_dist = *i as f32,
                            Datum::Float(f) => max_dist = *f as f32,
                            _ => {}
                        }
                        let v = PropListUtils::get_by_concrete_key(&map, &Datum::Symbol(Symbol::from_str(&"modelList".to_owned())), &player.allocator, map_sorted)?;
                        let items = match player.get_datum(&v) { Datum::List(_, items, _) => items.clone(), _ => VecDeque::new() };
                        for item in &items {
                            if let Datum::Shockwave3dObjectRef(r) = player.get_datum(item) { model_whitelist.insert(r.name); }
                        }
                    } else {
                        if args.len() > 2 { max_models = player.get_datum(&args[2]).int_value().unwrap_or(100); }
                        if args.len() > 3 { detailed = player.get_datum(&args[3]).string_value().unwrap_or_default().eq_ignore_ascii_case("detailed"); }
                        // Optional positional #modelList at args[4].
                        if args.len() > 4 {
                            let items = match player.get_datum(&args[4]) { Datum::List(_, items, _) => items.clone(), _ => VecDeque::new() };
                            for item in &items {
                                if let Datum::Shockwave3dObjectRef(r) = player.get_datum(item) { model_whitelist.insert(r.name); }
                            }
                        }
                    }
                    if max_models <= 0 { max_models = 100; }

                    let scene = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                        member.and_then(|m| m.member_type.as_shockwave3d())
                            .and_then(|w3d| w3d.parsed_scene.clone())
                    };

                    // Get runtime node transforms and build exclusion set for invisible/detached models
                    let (node_transforms, excluded_nodes) = {
                        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
                        if let Some(w3d) = member.and_then(|m| m.member_type.as_shockwave3d()) {
                            let transforms = w3d.runtime_state.node_transforms.clone();
                            let mut excluded = std::collections::HashSet::new();
                            // NOTE: do NOT exclude models by visibility. Per the Director
                            // 11.5 dictionary, modelsUnderRay filters only by the optional
                            // #modelList and maxDistance — `visibility = #none` is a
                            // render-only flag and such models are still hit by the ray.
                            // Invisible collision geometry (e.g. the estate explore's
                            // `model("invisible")` boundary, hidden with visibility=#none
                            // but used by touch() to block the camera) depends on this;
                            // excluding it let the player walk off the map. Only
                            // removeFromWorld (detached_nodes) removes a model from raycasts.
                            for name in &w3d.runtime_state.detached_nodes {
                                excluded.insert(*name);
                            }
                            if let Some(ref scene) = w3d.parsed_scene {
                                for node in &scene.nodes {
                                    if excluded.contains(&node.name) { continue; }
                                    let mut parent = &node.parent_name;
                                    for _ in 0..10 {
                                        if parent.is_empty() {
                                            excluded.insert(node.name);
                                            break;
                                        }
                                        if *parent == BuiltInSymbol::World { break; }
                                        if w3d.runtime_state.detached_nodes.contains(parent) {
                                            excluded.insert(node.name);
                                            break;
                                        }
                                        if let Some(pn) = scene.nodes.iter().find(|n| n.name == *parent) {
                                            parent = &pn.parent_name;
                                        } else { break; }
                                    }
                                }
                            }
                            (Some(transforms), excluded)
                        } else {
                            (None, std::collections::HashSet::new())
                        }
                    };

                    let mut results = Vec::new();
                    if let Some(scene) = scene {
                        use crate::director::chunks::w3d::raycast::{Ray, raycast_scene_multi};
                        // Normalize direction to ensure unit vector — some models may have
                        // scaled world transforms that produce non-unit axis vectors.
                        let dir_len = ((direction[0]*direction[0] + direction[1]*direction[1] + direction[2]*direction[2]) as f64).sqrt();
                        let norm_dir = if dir_len > 1e-10 {
                            [(direction[0] / dir_len) as f32, (direction[1] / dir_len) as f32, (direction[2] / dir_len) as f32]
                        } else {
                            [0.0f32, 0.0, -1.0] // Default downward
                        };
                        let ray = Ray {
                            origin: [origin[0] as f32, origin[1] as f32, origin[2] as f32],
                            direction: norm_dir,
                        };
                        // Director parameterizes the ray as origin + t*direction with
                        // t in [0, maxDistance], so maxDistance is measured in units of
                        // the DIRECTION VECTOR's length, not world units. The world reach
                        // is therefore maxDistance * |direction|. We cast with a unit
                        // direction, so scale the world cutoff by |direction| to match.
                        // SweeTarts' snake ground-snap casts vector(0,-15,0) with
                        // maxDistance 100 → 1500 units of reach; treating it as 100 world
                        // units fell ~7 units short of the platform 107 below the spawn
                        // origin, so the snake never seated ("can't move before it jumps").
                        let world_max_dist = if dir_len > 1e-10 {
                            max_dist * dir_len as f32
                        } else {
                            max_dist
                        };
                        let excluded_ref = if excluded_nodes.is_empty() { None } else { Some(&excluded_nodes) };
                        let included_ref = if model_whitelist.is_empty() { None } else { Some(&model_whitelist) };
                        let hits = raycast_scene_multi(
                            &ray, &scene, world_max_dist, max_models as usize,
                            node_transforms.as_ref(),
                            excluded_ref,
                            included_ref,
                        );
                        for hit in &hits {
                            if detailed {
                                let model_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::Model.into()));
                                let model_val = player.alloc_datum(Datum::Shockwave3dObjectRef(
                                    crate::director::lingo::datum::Shockwave3dObjectRef {
                                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                        object_type: BuiltInSymbol::Model.into(),
                                        name: Symbol::from_str(&hit.model_name),
                                    }
                                ));
                                let dist_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::Distance.into()));
                                let dist_val = player.alloc_datum(Datum::Float(hit.distance as f64));
                                let pos_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::IsectPosition.into()));
                                let pos_val = player.alloc_datum(Datum::Vector([
                                    hit.position[0] as f64, hit.position[1] as f64, hit.position[2] as f64,
                                ]));
                                let norm_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::IsectNormal.into()));
                                let norm_val = player.alloc_datum(Datum::Vector([
                                    hit.normal[0] as f64, hit.normal[1] as f64, hit.normal[2] as f64,
                                ]));
                                let mesh_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::MeshID.into()));
                                let mesh_val = player.alloc_datum(Datum::Int(hit.mesh_id as i32));
                                let face_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::FaceID.into()));
                                let face_val = player.alloc_datum(Datum::Int(hit.face_index as i32 + 1));
                                let vert_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::Vertices.into()));
                                let mut vert_items = VecDeque::new();
                                for vtx in &hit.vertices {
                                    vert_items.push_back(player.alloc_datum(Datum::Vector([
                                        vtx[0] as f64, vtx[1] as f64, vtx[2] as f64,
                                    ])));
                                }
                                let vert_val = player.alloc_datum(Datum::List(
                                    crate::director::lingo::datum::DatumType::List, vert_items, false,
                                ));
                                let uv_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::UvCoord.into()));
                                let u_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::U.into()));
                                let u_val = player.alloc_datum(Datum::Float(hit.uv_coord[0] as f64));
                                let v_key = player.alloc_datum(Datum::Symbol(BuiltInSymbol::V.into()));
                                let v_val = player.alloc_datum(Datum::Float(hit.uv_coord[1] as f64));
                                let uv_val = player.alloc_datum(Datum::PropList(
                                    VecDeque::from(vec![(u_key, u_val), (v_key, v_val)]), false,
                                ));

                                let hit_proplist = player.alloc_datum(Datum::PropList(VecDeque::from(vec![
                                    (model_key, model_val), (dist_key, dist_val),
                                    (pos_key, pos_val), (norm_key, norm_val),
                                    (mesh_key, mesh_val), (face_key, face_val),
                                    (vert_key, vert_val), (uv_key, uv_val),
                                ]), false));
                                results.push(hit_proplist);
                            } else {
                                results.push(player.alloc_datum(Datum::Shockwave3dObjectRef(
                                    crate::director::lingo::datum::Shockwave3dObjectRef {
                                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                        object_type: BuiltInSymbol::Model,
                                        name: Symbol::from_str(&hit.model_name),
                                    }
                                )));
                            }
                        }
                    }

                    Ok(player.alloc_datum(Datum::List(
                        crate::director::lingo::datum::DatumType::List, VecDeque::from(results), false,
                    )))
                })
            }
            BuiltInSymbol::ModelsUnderLoc | BuiltInSymbol::ModelUnderLoc => {
                reserve_player_mut(|player| {
                    if handler_name == BuiltInSymbol::ModelUnderLoc {
                        Ok(player.alloc_datum(Datum::Void))
                    } else {
                        Ok(player.alloc_datum(Datum::List(
                            crate::director::lingo::datum::DatumType::List, VecDeque::new(), false,
                        )))
                    }
                })
            }
            _ => Err(ScriptError::new(format!(
                "No Shockwave3D member handler for '{}'", handler_name
            ))),
        }
    }

    pub fn get_3d_collection_count(scene: &crate::director::chunks::w3d::types::W3dScene, collection: Symbol) -> i32 {
        use crate::director::chunks::w3d::types::W3dNodeType;
        match collection.as_lower_str() {
            "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).count() as i32,
            "modelresource" => scene.model_resources.len() as i32,
            "shader" => scene.shaders.len() as i32,
            "texture" => scene.texture_images.len() as i32,
            "light" => scene.lights.len() as i32,
            "camera" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::View).count() as i32,
            "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).count() as i32,
            // +1 for the implicit default motion at index 1 (see DEFAULT_MOTION_NAME).
            "motion" => scene.motions.len() as i32 + 1,
            _ => 0,
        }
    }

    pub fn get_3d_object_name_by_index(scene: &crate::director::chunks::w3d::types::W3dScene, collection: Symbol, index: usize) -> Option<Symbol> {
        use crate::director::chunks::w3d::types::W3dNodeType;
        if index == 0 { return None; }
        let idx = index - 1; // 1-based to 0-based
        match collection.into_builtin() {
            Some(BuiltInSymbol::Model) => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).nth(idx).map(|n| n.name),
            Some(BuiltInSymbol::ModelResource) => scene.model_resources.keys().nth(idx).copied(),
            Some(BuiltInSymbol::Shader) => scene.shaders.get(idx).map(|s| s.name),
            Some(BuiltInSymbol::Texture) => scene.texture_images.keys().nth(idx).copied(),
            Some(BuiltInSymbol::Light) => scene.lights.get(idx).map(|l| l.name),
            Some(BuiltInSymbol::Camera) => {
                // Director puts DefaultView as camera[1], then other cameras in scene order
                let mut cams: Vec<Symbol> = Vec::new();
                // DefaultView first
                if let Some(dv) = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View && n.name == Symbol::builtin(BuiltInSymbol::DefaultView)) {
                    cams.push(dv.name);
                }
                // Then other cameras in scene order
                for n in &scene.nodes {
                    if n.node_type == W3dNodeType::View && n.name != Symbol::builtin(BuiltInSymbol::DefaultView) {
                        cams.push(n.name);
                    }
                }
                cams.get(idx).copied()
            }
            Some(BuiltInSymbol::Group) => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).nth(idx).map(|n| n.name.clone()),
            // motion[1] = implicit default; authored motions follow at 2.. (see DEFAULT_MOTION_NAME).
            Some(BuiltInSymbol::Motion) => {
                if idx == 0 {
                    Some(Symbol::from_str(&DEFAULT_MOTION_NAME.to_string()))
                } else {
                    scene.motions.get(idx - 1).map(|m| m.name.clone())
                }
            }
            _ => None,
        }
    }
}

/// Public wrapper for render_3d_to_rgba (used by text3D software rendering path)
pub fn render_3d_to_rgba_pub(
    scene_data: &Option<std::rc::Rc<crate::director::chunks::w3d::types::W3dScene>>,
    runtime_state: &crate::player::cast_member::Shockwave3dRuntimeState,
    width: u32,
    height: u32,
) -> Vec<u8> {
    render_3d_to_rgba(scene_data, runtime_state, width, height)
}

/// Render a Shockwave3D scene to RGBA pixels using a temporary offscreen WebGL2 context.
fn render_3d_to_rgba(
    scene_data: &Option<std::rc::Rc<crate::director::chunks::w3d::types::W3dScene>>,
    runtime_state: &crate::player::cast_member::Shockwave3dRuntimeState,
    width: u32,
    height: u32,
) -> Vec<u8> {
    use wasm_bindgen::JsCast;
    use web_sys::WebGl2RenderingContext;

    let scene = match scene_data {
        Some(s) => s,
        None => return vec![128u8; (width * height * 4) as usize], // grey fallback
    };

    // Create offscreen canvas
    let document = match web_sys::window().and_then(|w| w.document()) {
        Some(d) => d,
        None => return vec![128u8; (width * height * 4) as usize],
    };
    let canvas = match document.create_element("canvas") {
        Ok(el) => el,
        Err(_) => return vec![128u8; (width * height * 4) as usize],
    };
    let canvas: web_sys::HtmlCanvasElement = match canvas.dyn_into() {
        Ok(c) => c,
        Err(_) => return vec![128u8; (width * height * 4) as usize],
    };
    canvas.set_width(width);
    canvas.set_height(height);

    let mut context_attrs = web_sys::WebGlContextAttributes::new();
    context_attrs.alpha(true);
    context_attrs.depth(true);
    context_attrs.preserve_drawing_buffer(true); // needed for readPixels

    let gl: WebGl2RenderingContext = match canvas.get_context_with_context_options("webgl2", &context_attrs) {
        Ok(Some(ctx)) => match ctx.dyn_into() {
            Ok(gl) => gl,
            Err(_) => return vec![128u8; (width * height * 4) as usize],
        },
        _ => return vec![128u8; (width * height * 4) as usize],
    };

    let context = match crate::rendering_gpu::webgl2::context::WebGL2Context::new(gl.clone()) {
        Ok(c) => c,
        Err(_) => return vec![128u8; (width * height * 4) as usize],
    };

    // Render directly to the default framebuffer (the offscreen canvas), not to FBO
    let mut renderer = crate::rendering_gpu::webgl2::scene3d::Scene3dRenderer::new();
    match renderer.render_to_default_framebuffer(&context, (0, 0), scene, width, height, Some(runtime_state)) {
        Ok(_) => {}
        Err(e) => {
            console_warn!("[W3D] render_3d_to_rgba failed: {:?}", e);
            return vec![200u8; (width * height * 4) as usize];
        }
    }

    // Read pixels from the default framebuffer
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let _ = gl.read_pixels_with_opt_u8_array(
        0, 0, width as i32, height as i32,
        WebGl2RenderingContext::RGBA,
        WebGl2RenderingContext::UNSIGNED_BYTE,
        Some(&mut pixels),
    );

    // Return pixels directly (no flip needed — Director bitmaps are top-to-bottom
    // which matches WebGL's bottom-to-top readPixels when used as a texture source)
    pixels
}
