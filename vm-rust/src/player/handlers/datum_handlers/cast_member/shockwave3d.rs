use std::collections::VecDeque;

use log::debug;
use wasm_bindgen::JsCast;

use crate::{
    director::lingo::datum::Datum,
    player::{
        cast_lib::CastMemberRef,
        cast_member::{CastMemberType, Shockwave3dMember, Text3dSource, Text3dState},
        reserve_player_mut,
        DatumRef, DirPlayer, ScriptError,
    },
    console_warn,
};

const W3D_HANDLER_LOG: bool = false;

fn log(msg: &str) {
    if W3D_HANDLER_LOG {
        debug!("[W3D-HANDLER] {}", msg);
    }
}

pub struct Shockwave3dMemberHandlers {}

impl Shockwave3dMemberHandlers {
    fn native_text_alignment(alignment: &str) -> crate::player::handlers::datum_handlers::cast_member::font::TextAlignment {
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
        (2 + (smoothness as i32 / 4)).clamp(2, 5)
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
        let alignment = Self::native_text_alignment(&source.alignment);
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
            scene.clod_meshes.insert("Text".to_string(), vec![mesh]);
            scene.mesh_content_version += 1;
        }
    }

    fn scale_text3d_mesh_depth(
        scene: &mut crate::director::chunks::w3d::types::W3dScene,
        old_depth: f32,
        new_depth: f32,
    ) {
        let old_depth = old_depth.max(1.0);
        let new_depth = new_depth.max(1.0);
        let scale = new_depth / old_depth;

        if let Some(meshes) = scene.clod_meshes.get_mut("Text") {
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
        let front = display_face == -1 || (display_face & 1) != 0;
        let tunnel = display_face == -1 || (display_face & 2) != 0;
        let back = display_face == -1 || (display_face & 4) != 0;

        let mode = if !front && !back && !tunnel {
            Some(0u8)
        } else if tunnel || (front && back) {
            // Tunnel faces need culling disabled to read as extruded text.
            Some(3u8)
        } else if back && !front {
            Some(2u8)
        } else {
            Some(1u8)
        };

        match mode {
            Some(1) => {
                runtime_state.node_visibility.remove("Text");
            }
            Some(mode) => {
                runtime_state.node_visibility.insert("Text".to_string(), mode);
            }
            None => {}
        }
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
                    let mesh = crate::director::chunks::w3d::primitives::extrude_text_to_mesh(
                        &text_content, &glyphs, outline_res, font_size as f32, depth,
                    );
                    if !mesh.positions.is_empty() {
                        if let Some(scene) = w3d_member.scene_mut() {
                            scene.clod_meshes.insert("Text".to_string(), vec![mesh]);
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
                            scene.texture_images.insert("TextBitmap".to_string(), tex_data);
                            if !scene.texture_infos.iter().any(|t| t.name == "TextBitmap") {
                                scene.texture_infos.push(W3dTextureInfo {
                                    name: "TextBitmap".to_string(),
                                    render_format: 0, mip_mode: 0, mag_filter: 0, image_type: 0,
                                });
                            }
                            if let Some(shader) = scene.shaders.first_mut() {
                                if !shader.texture_layers.iter().any(|l| l.name == "TextBitmap") {
                                    shader.texture_layers.push(W3dTextureLayer {
                                        name: "TextBitmap".to_string(),
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                        scene.clod_meshes.insert("Text".to_string(), vec![mesh]);
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
                        bevel_type: 0,
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
        prop: &str,
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

        match prop {
            // ─── Member-level properties ───
            "directToStage" => Ok(Datum::Int(if info.direct_to_stage { 1 } else { 0 })),
            "preLoad" | "preload" => Ok(Datum::Int(if info.preload { 1 } else { 0 })),
            "duration" => Ok(Datum::Int(info.duration as i32)),

            "regPoint" => {
                Ok(Datum::Point([info.reg_point.0 as f64, info.reg_point.1 as f64], 0))
            }
            "rect" => {
                let r = info.default_rect;
                Ok(Datum::Rect([r.0 as f64, r.1 as f64, r.2 as f64, r.3 as f64], 0))
            }
            "width" => Ok(Datum::Int(info.default_rect.2 - info.default_rect.0)),
            "height" => Ok(Datum::Int(info.default_rect.3 - info.default_rect.1)),

            // ─── Scene collection properties ───
            // These return lists of Shockwave3dObjectRefs, supporting .count and [index]
            "model" | "modelCount" | "modelResource" | "modelResourceCount"
            | "shader" | "shaderCount" | "texture" | "textureCount"
            | "light" | "lightCount" | "camera" | "cameraCount"
            | "group" | "groupCount" | "motion" | "motionCount" => {
                use crate::director::lingo::datum::{Shockwave3dObjectRef, DatumType};
                let collection = prop.trim_end_matches("Count");
                let names: Vec<String> = if let Some(scene) = &scene_data {
                    match collection {
                        "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).map(|n| n.name.clone()).collect(),
                        "modelResource" => scene.model_resources.keys().cloned().collect(),
                        "shader" => scene.shaders.iter().map(|s| s.name.clone()).collect(),
                        "texture" => scene.texture_images.keys().cloned().collect(),
                        "light" => scene.lights.iter().map(|l| l.name.clone()).collect(),
                        "camera" => {
                            let mut cams = Vec::new();
                            if let Some(dv) = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View && n.name.eq_ignore_ascii_case("defaultview")) {
                                cams.push(dv.name.clone());
                            }
                            for n in &scene.nodes {
                                if n.node_type == W3dNodeType::View && !n.name.eq_ignore_ascii_case("defaultview") {
                                    cams.push(n.name.clone());
                                }
                            }
                            cams
                        }
                        "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).map(|n| n.name.clone()).collect(),
                        "motion" => scene.motions.iter().map(|m| m.name.clone()).collect(),
                        _ => vec![],
                    }
                } else {
                    vec![]
                };
                // If prop ends with "Count", return just the count
                if prop.ends_with("Count") {
                    return Ok(Datum::Int(names.len() as i32));
                }
                // Return a list of Shockwave3dObjectRefs
                let items: VecDeque<_> = names.iter().map(|name| {
                    player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: cast_member_ref.cast_lib,
                        cast_member: cast_member_ref.cast_member,
                        object_type: collection.to_string(),
                        name: name.clone(),
                    }))
                }).collect();
                Ok(Datum::List(DatumType::List, items, false))
            }

            // ─── State ───
            "state" => Ok(Datum::Int(4)), // 4 = loaded
            "percentStreamed" => Ok(Datum::Int(100)),
            "animationEnabled" => Ok(Datum::Int(if info.animation_enabled { 1 } else { 0 })),
            "loop" => Ok(Datum::Int(if info.loops { 1 } else { 0 })),

            // ─── Rendering ───
            "image" => {
                // member("3d").image returns the rendered 3D world as a bitmap.
                let w = (info.default_rect.2 - info.default_rect.0).max(1) as u32;
                let h = (info.default_rect.3 - info.default_rect.1).max(1) as u32;

                // Try cached frame first (from sprite rendering), then offscreen render
                let key = (cast_member_ref.cast_lib, cast_member_ref.cast_member);
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
                let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                Ok(Datum::BitmapRef(bitmap_ref))
            }
            "backgroundColor" => {
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(50, 50, 50)))
            }
            "ambientColor" => {
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(25, 25, 25)))
            }
            "renderer" | "rendererDeviceList" => Ok(Datum::Symbol("openGL".to_string())),
            "colorBufferDepth" => Ok(Datum::Int(32)),
            "depthBufferDepth" => Ok(Datum::Int(24)),
            "antiAliasingEnabled" => Ok(Datum::Int(0)),
            "streamSize" => Ok(Datum::Int(0)),
            // Text3D properties (stub values after Text→Shockwave3d conversion)
            "smoothness" => Ok(Datum::Int(text3d_state.as_ref().map(|s| s.smoothness as i32).unwrap_or(10))),
            "tunnelDepth" | "tunneldepth" => Ok(Datum::Float(text3d_state.as_ref().map(|s| s.tunnel_depth as f64).unwrap_or(10.0))),
            "bevelDepth" | "beveldepth" => Ok(Datum::Float(text3d_state.as_ref().map(|s| s.bevel_depth as f64).unwrap_or(1.0))),
            "bevelType" | "beveltype" => Ok(Datum::Symbol(match text3d_state.as_ref().map(|s| s.bevel_type).unwrap_or(0) {
                1 => "miter".to_string(),
                2 => "round".to_string(),
                _ => "none".to_string(),
            })),
            "displayFace" | "displayface" => Ok(Datum::Int(text3d_state.as_ref().map(|s| s.display_face).unwrap_or(-1))),
            "displayMode" | "displaymode" => Ok(Datum::Symbol(if text3d_state.as_ref().map(|s| s.display_mode).unwrap_or(1) == 1 {
                "mode3d".to_string()
            } else {
                "normal".to_string()
            })),
            "diffuseColor" | "diffusecolor" => {
                let (r, g, b) = text3d_state.as_ref().map(|s| s.diffuse_color).unwrap_or((0, 0, 0));
                Ok(Datum::ColorRef(crate::player::sprite::ColorRef::Rgb(r, g, b)))
            }
            "directionalPreset" | "directionalpreset" => {
                // Read current preset from runtime state (default 2 = #topCenter)
                let preset = {
                    let member = player.movie.cast_manager.find_member_by_ref(cast_member_ref);
                    member.and_then(|m| m.member_type.as_shockwave3d())
                        .map(|w3d| w3d.runtime_state.directional_preset)
                        .unwrap_or(2)
                };
                let symbol = match preset {
                    1 => "topLeft",
                    2 => "topCenter",
                    3 => "topRight",
                    4 => "middleLeft",
                    5 => "middleCenter",
                    6 => "middleRight",
                    7 => "bottomLeft",
                    8 => "bottomCenter",
                    9 => "bottomRight",
                    _ => "None",
                };
                Ok(Datum::Symbol(symbol.to_string()))
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
                                    if let Some(mat) = scene.materials.iter_mut().find(|m| m.name == "TextMaterial") {
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
                    Datum::Symbol(s) => match s.trim_start_matches('#').to_ascii_lowercase().as_str() {
                        "topleft" => 1,
                        "topcenter" => 2,
                        "topright" => 3,
                        "middleleft" => 4,
                        "middlecenter" => 5,
                        "middleright" => 6,
                        "bottomleft" => 7,
                        "bottomcenter" => 8,
                        "bottomright" => 9,
                        "none" => 0,
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
                                    .find(|n| n.name.eq_ignore_ascii_case("DefaultDirectional"))
                                {
                                    light_node.transform = t;
                                }
                            }
                            w3d.runtime_state.node_transforms.insert("DefaultDirectional".to_string(), t);
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
        handler_name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        // Lazily init 3D world for text members before any 3D operation
        reserve_player_mut(|player| {
            let member_ref = match player.get_datum(datum) {
                Datum::CastMember(r) => r.to_owned(),
                _ => return Ok(()),
            };
            Self::ensure_text3d(player, &member_ref);
            Ok(())
        })?;

        match handler_name {
            "getPropRef" => {
                // member("x").model[1] → getPropRef(#model, 1)
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    let collection = player.get_datum(&args[0]).string_value()?;
                    let index = if args.len() > 1 {
                        player.get_datum(&args[1]).int_value()? as usize
                    } else {
                        1
                    };
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                let obj_name = Self::get_3d_object_name_by_index(scene, &collection, index)
                                    .unwrap_or_default();
                                if !obj_name.is_empty() {
                                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                                    return Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                                        cast_lib: cast_member_ref.cast_lib,
                                        cast_member: cast_member_ref.cast_member,
                                        object_type: collection,
                                        name: obj_name,
                                    })));
                                }
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Void))
                })
            }
            "count" => {
                reserve_player_mut(|player| {
                    let cast_member_ref = match player.get_datum(datum) {
                        Datum::CastMember(r) => r.to_owned(),
                        _ => return Err(ScriptError::new("Expected cast member ref".to_string())),
                    };
                    if args.is_empty() {
                        return Err(ScriptError::new("count requires 1 argument".to_string()));
                    }
                    let count_of = player.get_datum(&args[0]).string_value()?;
                    let member = player.movie.cast_manager.find_member_by_ref(&cast_member_ref);
                    if let Some(m) = member {
                        if let Some(w3d) = m.member_type.as_shockwave3d() {
                            if let Some(ref scene) = w3d.parsed_scene {
                                let count = Self::get_3d_collection_count(scene, &count_of);
                                return Ok(player.alloc_datum(Datum::Int(count)));
                            }
                        }
                    }
                    Ok(player.alloc_datum(Datum::Int(0)))
                })
            }
            // Shockwave 3D collection accessors & mutators
            "model" | "modelResource" | "shader" | "texture" | "light" | "camera" | "group" | "motion"
            | "resetWorld" | "revertToWorldDefaults"
            | "newTexture" | "newShader" | "newModel" | "newModelResource" | "newLight" | "newCamera" | "newGroup" | "newMotion" | "newMesh"
            | "deleteTexture" | "deleteShader" | "deleteModel" | "deleteModelResource" | "deleteLight" | "deleteCamera" | "deleteGroup" | "deleteMotion"
            | "cloneModelFromCastmember" | "cloneMotionFromCastmember" | "cloneDeep"
            | "loadFile" | "extrude3d" | "getPref" | "setPref"
            | "registerForEvent" | "registerScript"
            | "image" => {
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

                    // registerForEvent / registerScript — stub (event system not implemented)
                    if handler_name == "registerForEvent" || handler_name == "registerScript" {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    if handler_name == "resetWorld" {
                        let member = player.movie.cast_manager.find_mut_member_by_ref(&member_ref)
                            .ok_or_else(|| ScriptError::new("Member not found".to_string()))?;
                        if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                            // resetWorld: restore to state when member was first loaded into memory
                            if let Some(ref source) = w3d.source_scene {
                                w3d.parsed_scene = Some(source.clone());
                            }
                            w3d.runtime_state = crate::player::cast_member::Shockwave3dRuntimeState::from_info(&w3d.info, w3d.parsed_scene.as_deref());
                        }
                        return Ok(player.alloc_datum(Datum::Void));
                    }
                    if handler_name == "revertToWorldDefaults" {
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
                    if handler_name == "cloneModelFromCastmember" || handler_name == "cloneMotionFromCastmember" || handler_name == "cloneDeep" {
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
                        let obj_type = if handler_name == "cloneMotionFromCastmember" {
                            "motion"
                        } else {
                            "model"
                        };

                        // Look up source model's shader/transform/resource from source member's scene
                        let identity = [1.0f32,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
                        let (source_shader_name, source_transform, source_resource_name, source_model_resource_name, src_motion_tracks, src_child_nodes) = if let Some(ref src_ref) = source_member_ref {
                            let src_member = player.movie.cast_manager.find_member_by_ref(src_ref);
                            if let Some(sm) = src_member {
                                if let Some(sw3d) = sm.member_type.as_shockwave3d() {
                                    if let Some(ref scene) = sw3d.parsed_scene {
                                        let node = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&source_model_name));
                                        let (sn, st, sr, smr) = if let Some(n) = node {
                                            (n.shader_name.clone(), n.transform, n.resource_name.clone(), n.model_resource_name.clone())
                                        } else {
                                            (String::new(), identity, String::new(), String::new())
                                        };
                                        let motion_tracks = scene.motions.iter()
                                            .max_by_key(|m| m.tracks.len())
                                            .map(|m| m.tracks.clone())
                                            .unwrap_or_default();
                                        // Collect all descendant nodes of the source model recursively
                                        // Use case-insensitive matching (Director is case-insensitive)
                                        let child_nodes = {
                                            let mut descendants = Vec::new();
                                            let mut stack = vec![source_model_name.to_string()];
                                            while let Some(parent) = stack.pop() {
                                                for n in &scene.nodes {
                                                    if n.parent_name.eq_ignore_ascii_case(&parent) {
                                                        descendants.push(n.clone());
                                                        stack.push(n.name.clone());
                                                    }
                                                }
                                            }
                                            descendants
                                        };
                                        (sn, st, sr, smr, motion_tracks, child_nodes)
                                    } else { (String::new(), identity, String::new(), String::new(), vec![], vec![]) }
                                } else { (String::new(), identity, String::new(), String::new(), vec![], vec![]) }
                            } else { (String::new(), identity, String::new(), String::new(), vec![], vec![]) }
                        } else {
                            (String::new(), identity, String::new(), String::new(), vec![], vec![])
                        };

                        // Track shader name remapping for -clone suffix creation
                        let mut shader_name_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

                        // Copy source shaders, model resources, meshes, and textures that don't exist in target scene
                        if let Some(ref src_ref) = source_member_ref {
                            let (src_shaders, src_materials, src_model_resources, src_clod_meshes, src_raw_meshes, src_textures, src_lights, src_light_nodes, src_skeletons) = {
                                let src_member = player.movie.cast_manager.find_member_by_ref(src_ref);
                                let scene = src_member.and_then(|sm| sm.member_type.as_shockwave3d())
                                    .and_then(|sw3d| sw3d.parsed_scene.as_ref());
                                let shaders: Vec<_> = scene.map(|s| s.shaders.clone()).unwrap_or_default();
                                let materials: Vec<_> = scene.map(|s| s.materials.clone()).unwrap_or_default();
                                let resources: Vec<_> = scene.map(|s| s.model_resources.iter()
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let meshes: Vec<_> = scene.map(|s| s.clod_meshes.iter()
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let raw: Vec<_> = scene.map(|s| s.raw_meshes.clone()).unwrap_or_default();
                                let textures: Vec<_> = scene.map(|s| s.texture_images.iter()
                                    .map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                                let lights: Vec<_> = scene.map(|s| s.lights.clone()).unwrap_or_default();
                                let light_nodes: Vec<_> = scene.map(|s| s.nodes.iter()
                                    .filter(|n| n.node_type == crate::director::chunks::w3d::types::W3dNodeType::Light)
                                    .cloned().collect()).unwrap_or_default();
                                let skeletons: Vec<_> = scene.map(|s| s.skeletons.clone()).unwrap_or_default();
                                (shaders, materials, resources, meshes, raw, textures, lights, light_nodes, skeletons)
                            };

                            debug!(
                                "[W3D-CLONE] {}(\"{}\") src_model=\"{}\" src_member={:?}: \
                                 {} shaders, {} model_resources, {} clod_meshes(keys={:?}), {} raw_meshes(names={:?}), {} textures, \
                                 src_res=\"{}\", src_mres=\"{}\"",
                                handler_name, obj_name, source_model_name, source_member_ref,
                                src_shaders.len(), src_model_resources.len(),
                                src_clod_meshes.len(), src_clod_meshes.iter().map(|(k,_)| k.clone()).collect::<Vec<String>>(),
                                src_raw_meshes.len(), src_raw_meshes.iter().map(|m| m.name.clone()).collect::<Vec<String>>(),
                                src_textures.len(),
                                source_resource_name, source_model_resource_name,
                            );

                            // Namespace prefix to avoid name collisions
                            let ns = format!("{}_", obj_name);

                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        // Determine which shaders are USED by the model being cloned.
                                        // Director docs: "copies shaders...used by the model and its children"
                                        let mut used_shader_names: std::collections::HashSet<String> = std::collections::HashSet::new();
                                        // From model resource shader bindings
                                        let res_key = if !source_model_resource_name.is_empty() {
                                            source_model_resource_name.as_str()
                                        } else {
                                            source_resource_name.as_str()
                                        };
                                        for (rname, rinfo) in &src_model_resources {
                                            if rname == res_key {
                                                for binding in &rinfo.shader_bindings {
                                                    for shader_name in &binding.mesh_bindings {
                                                        used_shader_names.insert(shader_name.clone());
                                                    }
                                                }
                                            }
                                        }
                                        // From node shader_name
                                        if !source_shader_name.is_empty() {
                                            used_shader_names.insert(source_shader_name.clone());
                                        }
                                        // Also collect shaders from CHILD model resources
                                        for child in &src_child_nodes {
                                            let child_res = if !child.model_resource_name.is_empty() {
                                                child.model_resource_name.as_str()
                                            } else {
                                                child.resource_name.as_str()
                                            };
                                            for (rname, rinfo) in &src_model_resources {
                                                if rname == child_res {
                                                    for binding in &rinfo.shader_bindings {
                                                        for shader_name in &binding.mesh_bindings {
                                                            used_shader_names.insert(shader_name.clone());
                                                        }
                                                    }
                                                }
                                            }
                                            if !child.shader_name.is_empty() {
                                                used_shader_names.insert(child.shader_name.clone());
                                            }
                                        }

                                        // Collect texture names used by the used shaders
                                        let mut used_texture_names: std::collections::HashSet<String> = std::collections::HashSet::new();
                                        for shader in &src_shaders {
                                            if used_shader_names.contains(&shader.name) {
                                                for layer in &shader.texture_layers {
                                                    if !layer.name.is_empty() {
                                                        used_texture_names.insert(layer.name.clone());
                                                    }
                                                }
                                            }
                                        }

                                        // If no specific shaders identified, fall back to copying all
                                        // (handles cases where shader bindings are empty/unknown)
                                        let filter_shaders = !used_shader_names.is_empty();

                                        // Shaders: only copy those used by the model.
                                        // If name conflicts, create -clone<N> copy (Director behavior).
                                        // DefaultShader is built-in to every cast member — never copy it.
                                        for shader in &src_shaders {
                                            if shader.name == "DefaultShader" {
                                                continue;
                                            }
                                            if filter_shaders && !used_shader_names.contains(&shader.name) {
                                                continue; // Skip shaders not used by this model
                                            }
                                            if scene.shaders.iter().any(|s| s.name == shader.name) {
                                                // Name conflict — create a -clone<N> copy
                                                let mut n = 1;
                                                loop {
                                                    let clone_name = format!("{}-clone{}", shader.name, n);
                                                    if !scene.shaders.iter().any(|s| s.name == clone_name) {
                                                        shader_name_map.insert(shader.name.clone(), clone_name.clone());
                                                        let mut cloned = shader.clone();
                                                        cloned.name = clone_name;
                                                        scene.shaders.push(cloned);
                                                        break;
                                                    }
                                                    n += 1;
                                                }
                                            } else {
                                                scene.shaders.push(shader.clone());
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
                                                            m.name = mapped.clone();
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
                                        // Model resources: namespace to prevent collisions
                                        for (res_name, res_info) in &src_model_resources {
                                            let new_name = format!("{}{}", ns, res_name);
                                            if !scene.model_resources.contains_key(&new_name) {
                                                let mut cloned_res = res_info.clone();
                                                for binding in &mut cloned_res.shader_bindings {
                                                    for mesh_shader in &mut binding.mesh_bindings {
                                                        if let Some(new_name) = shader_name_map.get(mesh_shader.as_str()) {
                                                            *mesh_shader = new_name.clone();
                                                        }
                                                    }
                                                }
                                                scene.model_resources.insert(new_name, cloned_res);
                                            }
                                        }
                                        // CLOD meshes: namespace to prevent collisions
                                        for (mesh_name, mesh_data) in &src_clod_meshes {
                                            let new_name = format!("{}{}", ns, mesh_name);
                                            if !scene.clod_meshes.contains_key(&new_name) {
                                                scene.clod_meshes.insert(new_name, mesh_data.clone());
                                            }
                                        }
                                        // Textures: only copy those used by copied shaders
                                        for (tex_name, tex_data) in &src_textures {
                                            if filter_shaders && !used_texture_names.contains(tex_name) {
                                                continue;
                                            }
                                            if !scene.texture_images.contains_key(tex_name) {
                                                scene.texture_images.insert(tex_name.clone(), tex_data.clone());
                                                scene.texture_content_version += 1;
                                            }
                                        }
                                        // Raw meshes: namespace to prevent collisions
                                        for raw_mesh in &src_raw_meshes {
                                            let new_name = format!("{}{}", ns, raw_mesh.name);
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
                                        // Copy skeletons
                                        let skel_key = if !source_model_resource_name.is_empty() {
                                            format!("{}{}", ns, source_model_resource_name)
                                        } else if !source_resource_name.is_empty() {
                                            format!("{}{}", ns, source_resource_name)
                                        } else { String::new() };
                                        for skeleton in &src_skeletons {
                                            if !skel_key.is_empty() && !scene.skeletons.iter().any(|s| s.name == skel_key) {
                                                let mut cloned = skeleton.clone();
                                                cloned.name = skel_key.clone();
                                                scene.skeletons.push(cloned);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Add the cloned object to the target scene
                        let ns = format!("{}_", obj_name);
                        let mapped_resource = if !source_resource_name.is_empty() {
                            format!("{}{}", ns, source_resource_name)
                        } else { source_resource_name.clone() };
                        let mapped_model_resource = if !source_model_resource_name.is_empty() {
                            format!("{}{}", ns, source_model_resource_name)
                        } else { source_model_resource_name.clone() };

                        // Don't propagate "DefaultShader" as the node-level shader —
                        // it overrides the model resource's per-mesh shader bindings
                        // (which have the correct materials with proper colors).
                        let effective_shader_name = if source_shader_name == "DefaultShader" || source_shader_name.is_empty() {
                            String::new()
                        } else {
                            shader_name_map.get(&source_shader_name)
                                .cloned()
                                .unwrap_or(source_shader_name)
                        };

                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    use crate::director::chunks::w3d::types::*;
                                    if obj_type == "model" {
                                        scene.nodes.push(W3dNode {
                                            name: obj_name.clone(), node_type: W3dNodeType::Model,
                                            parent_name: "World".to_string(),
                                            resource_name: mapped_resource,
                                            model_resource_name: mapped_model_resource,
                                            shader_name: effective_shader_name,
                                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                            screen_width: 640, screen_height: 480,
                                            transform: source_transform,
                                        });
                                        // Namespace every descendant's name to avoid collisions
                                        // with prior clones from the same source. Names are keyed
                                        // case-insensitively since Director is case-insensitive.
                                        let mut node_name_map: std::collections::HashMap<String, String> =
                                            std::collections::HashMap::new();
                                        for child in &src_child_nodes {
                                            let new_name = format!("{}{}", ns, child.name);
                                            node_name_map.insert(child.name.to_ascii_lowercase(), new_name);
                                        }

                                        // Clone child nodes from source scene, re-parenting
                                        // the direct children of source_model to obj_name and
                                        // rewiring deeper parent links to the namespaced names.
                                        for child in &src_child_nodes {
                                            let mut cloned = child.clone();
                                            // Rename the node itself
                                            if let Some(new_name) = node_name_map.get(&cloned.name.to_ascii_lowercase()) {
                                                cloned.name = new_name.clone();
                                            }
                                            // Re-parent: direct child of source_model → obj_name;
                                            // otherwise remap to the namespaced descendant name.
                                            if cloned.parent_name.eq_ignore_ascii_case(&source_model_name) {
                                                cloned.parent_name = obj_name.clone();
                                            } else if let Some(new_parent) = node_name_map.get(&cloned.parent_name.to_ascii_lowercase()) {
                                                cloned.parent_name = new_parent.clone();
                                            }
                                            // Namespace child resource names to match cloned mesh data
                                            if !cloned.resource_name.is_empty() {
                                                cloned.resource_name = format!("{}{}", ns, cloned.resource_name);
                                            }
                                            if !cloned.model_resource_name.is_empty() {
                                                cloned.model_resource_name = format!("{}{}", ns, cloned.model_resource_name);
                                            }
                                            // Remap shader name if it was renamed during clone
                                            if let Some(new_shader) = shader_name_map.get(&cloned.shader_name) {
                                                cloned.shader_name = new_shader.clone();
                                            }
                                            // Names are now unique per clone — push unconditionally
                                            scene.nodes.push(cloned);
                                        }
                                    } else if obj_type == "motion" {
                                        scene.motions.push(W3dMotion {
                                            name: obj_name.clone(),
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
                            object_type: obj_type.to_string(),
                            name: obj_name,
                        })));
                    }

                    // newTexture/newShader/newModel/etc. — create and return a ref
                    if handler_name.starts_with("new") || handler_name.starts_with("delete") {
                        let obj_type = match handler_name {
                            "newTexture" | "deleteTexture" => "texture",
                            "newShader" | "deleteShader" => "shader",
                            "newModel" | "deleteModel" => "model",
                            "newModelResource" | "deleteModelResource" | "newMesh" => "modelResource",
                            "newLight" | "deleteLight" => "light",
                            "newCamera" | "deleteCamera" => "camera",
                            "newGroup" | "deleteGroup" => "group",
                            "newMotion" | "deleteMotion" => "motion",
                            _ => "unknown",
                        };
                        let obj_name = if !args.is_empty() {
                            player.get_datum(&args[0]).string_value().unwrap_or_default()
                        } else {
                            String::new()
                        };

                        if handler_name.starts_with("delete") {
                            if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                                if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                    if let Some(scene) = w3d.scene_mut() {
                                        match obj_type {
                                            "model" | "group" | "camera" | "light" => {
                                                scene.nodes.retain(|n| n.name != obj_name);
                                            }
                                            "shader" => {
                                                // DefaultShader cannot be deleted (Director behavior)
                                                if obj_name != "DefaultShader" {
                                                    scene.shaders.retain(|s| s.name != obj_name);
                                                }
                                            }
                                            "motion" => {
                                                scene.motions.retain(|m| m.name != obj_name);
                                            }
                                            "texture" => {
                                                scene.texture_images.remove(&obj_name);
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
                        let mesh_num_faces = if handler_name == "newMesh" && args.len() >= 2 {
                            player.get_datum(&args[1]).int_value().unwrap_or(0) as u32
                        } else { 0 };

                        // Pre-read model resource name for newModel(name, modelResource)
                        let new_model_resource_name = if handler_name == "newModel" && args.len() >= 2 {
                            match player.get_datum(&args[1]) {
                                Datum::Shockwave3dObjectRef(r) if r.object_type == "modelResource" => r.name.clone(),
                                _ => String::new(),
                            }
                        } else { String::new() };

                        // Pre-read type arg for newModelResource(name, #type, #facing), newLight(name, #type)
                        let new_res_type = if (handler_name.eq_ignore_ascii_case("newModelResource")
                            || handler_name.eq_ignore_ascii_case("newMesh")
                            || handler_name.eq_ignore_ascii_case("newLight")) && args.len() >= 2
                        {
                            player.get_datum(&args[1]).string_value().unwrap_or_default()
                        } else { String::new() };
                        let new_res_facing = if handler_name == "newModelResource" && args.len() >= 3 {
                            player.get_datum(&args[2]).string_value().unwrap_or_default()
                        } else { String::new() };

                        // Add to parsed scene
                        if let Some(member) = player.movie.cast_manager.find_mut_member_by_ref(&member_ref) {
                            if let Some(w3d) = member.member_type.as_shockwave3d_mut() {
                                if let Some(scene) = w3d.scene_mut() {
                                    use crate::director::chunks::w3d::types::*;
                                    let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
                                    match obj_type {
                                        "model" => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::Model,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(),
                                                model_resource_name: new_model_resource_name.clone(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        "group" => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::Group,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        "camera" => {
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::View,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
                                            });
                                        }
                                        "light" => {
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
                                                name: obj_name.clone(),
                                                light_type,
                                                color: [191.0/255.0, 191.0/255.0, 191.0/255.0], // Director default: color(191,191,191)
                                                attenuation: [1.0, 0.0, 0.0],
                                                spot_angle: 90.0, // Director default
                                                enabled: true,
                                            });
                                            scene.nodes.push(W3dNode {
                                                name: obj_name.clone(), node_type: W3dNodeType::Light,
                                                parent_name: "World".to_string(),
                                                resource_name: String::new(), model_resource_name: String::new(),
                                                shader_name: String::new(),
                                                near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                                                screen_width: 640, screen_height: 480,
                                                transform: identity,
                                            });
                                        }
                                        "shader" => {
                                            scene.shaders.push(W3dShader {
                                                name: obj_name.clone(),
                                                ..Default::default()
                                            });
                                        }
                                        "modelResource" => {
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
                                                            name: obj_name.clone(),
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
                                                            name: obj_name.clone(),
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
                                                    name: obj_name.clone(),
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
                                            let shader_name = format!("{}_Shader", obj_name);
                                            let material_name = format!("{}_Material", obj_name);
                                            scene.materials.push(W3dMaterial {
                                                name: material_name.clone(),
                                                // Director's default for new primitives: ambient = diffuse = white
                                                ambient: [1.0, 1.0, 1.0, 1.0],
                                                ..Default::default()
                                            });
                                            scene.shaders.push(W3dShader {
                                                name: shader_name.clone(),
                                                material_name: material_name.clone(),
                                                ..Default::default()
                                            });
                                            let num_meshes = meshes.len().max(1);
                                            scene.model_resources.insert(obj_name.clone(), ModelResourceInfo {
                                                name: obj_name.clone(),
                                                mesh_infos: vec![mesh_info],
                                                shader_bindings: vec![ModelShaderBinding {
                                                    name: shader_name,
                                                    mesh_bindings: vec![String::new(); num_meshes],
                                                }],
                                                primitive_type: prim_type,
                                                primitive_width: 1.0,
                                                primitive_length: 1.0,
                                                primitive_height: 1.0,
                                                primitive_radius: 1.0,
                                                ..Default::default()
                                            });

                                            // Store generated mesh geometry so the renderer can upload it
                                            if !meshes.is_empty() {
                                                scene.clod_meshes.insert(obj_name.clone(), meshes);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }

                        // For newTexture(name, #fromImageObject/#fromCastMember, source)
                        if handler_name == "newTexture" && args.len() >= 3 {
                            let tex_type = player.get_datum(&args[1]).string_value().unwrap_or_default();
                            if tex_type == "fromCastMember" {
                                let source_member_ref = match player.get_datum(&args[2]) {
                                    Datum::CastMember(r) => Some(r.clone()),
                                    _ => None,
                                };
                                if let Some(src_ref) = source_member_ref {
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
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): {}x{} from member {}:{} '{}'",
                                                        obj_name, w, h, src_ref.cast_lib, src_ref.cast_member, m.name
                                                    ));
                                                    Some((w, h, rgba))
                                                }
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
                                                    scene.texture_images.insert(obj_name.clone(), tex_data);
                                                    scene.texture_content_version += 1;
                                                    log(&format!(
                                                        "[W3D] newTexture(\"{}\", #fromCastMember): stored {}x{} RGBA",
                                                        obj_name, w, h
                                                    ));
                                                }
                                            }
                                        }
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
                                                    scene.texture_images.insert(obj_name.clone(), tex_data);
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
                            object_type: obj_type.to_string(),
                            name: obj_name,
                        })));
                    }

                    // image — return the rendered 3D world as a bitmap ref
                    if handler_name == "image" {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // loadFile, extrude3d, getPref, setPref
                    if handler_name == "loadFile" || handler_name == "extrude3d"
                        || handler_name == "getPref" || handler_name == "setPref" {
                        return Ok(player.alloc_datum(Datum::Void));
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
                        };
                        empty_scene.nodes.push(W3dNode {
                            name: "World".to_string(),
                            node_type: W3dNodeType::Group,
                            parent_name: String::new(),
                            resource_name: String::new(),
                            model_resource_name: String::new(),
                            shader_name: String::new(),
                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                            screen_width: player.movie.rect.right as i32,
                            screen_height: player.movie.rect.bottom as i32,
                            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
                        });
                        empty_scene.nodes.push(W3dNode {
                            name: "DefaultView".to_string(),
                            node_type: W3dNodeType::View,
                            parent_name: "World".to_string(),
                            resource_name: String::new(),
                            model_resource_name: String::new(),
                            shader_name: String::new(),
                            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
                            screen_width: player.movie.rect.right as i32,
                            screen_height: player.movie.rect.bottom as i32,
                            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,500.0,1.0],
                        });
                        empty_scene.shaders.push(W3dShader {
                            name: "DefaultShader".to_string(),
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
                            Datum::String(s) => s,
                            Datum::Int(idx) => {
                                Self::get_3d_object_name_by_index(scene, handler_name, idx as usize)
                                    .unwrap_or_default()
                            }
                            _ => arg.string_value().unwrap_or_default(),
                        }
                    };

                    if obj_name.is_empty() {
                        return Ok(player.alloc_datum(Datum::Void));
                    }

                    // Check if the named object actually exists in the scene.
                    // Per Director docs: "If no [object] exists for the specified parameter, returns void."
                    // Name comparisons are case-insensitive (Director behavior).
                    use crate::director::chunks::w3d::types::W3dNodeType;
                    let obj_name_lower = obj_name.to_lowercase();
                    let resolved_name: Option<String> = match handler_name {
                        "modelResource" => scene.model_resources.keys()
                            .find(|k| k.to_lowercase() == obj_name_lower).cloned(),
                        "model" => scene.nodes.iter()
                            .find(|n| n.node_type == W3dNodeType::Model && n.name.to_lowercase() == obj_name_lower)
                            .map(|n| n.name.clone()),
                        "shader" => scene.shaders.iter()
                            .find(|s| s.name.to_lowercase() == obj_name_lower)
                            .map(|s| s.name.clone()),
                        "texture" => scene.texture_images.keys()
                            .find(|k| k.to_lowercase() == obj_name_lower).cloned(),
                        "light" => scene.lights.iter()
                            .find(|l| l.name.to_lowercase() == obj_name_lower)
                            .map(|l| l.name.clone()),
                        "camera" => scene.nodes.iter()
                            .find(|n| n.node_type == W3dNodeType::View && n.name.to_lowercase() == obj_name_lower)
                            .map(|n| n.name.clone()),
                        "group" => scene.nodes.iter()
                            .find(|n| n.node_type == W3dNodeType::Group && n.name.to_lowercase() == obj_name_lower)
                            .map(|n| n.name.clone()),
                        "motion" => scene.motions.iter()
                            .find(|m| m.name.to_lowercase() == obj_name_lower)
                            .map(|m| m.name.clone()),
                        _ => Some(obj_name.clone()), // Unknown collection types pass through
                    };
                    let resolved_name = match resolved_name {
                        Some(name) => name,
                        None => return Ok(player.alloc_datum(Datum::Void)),
                    };

                    use crate::director::lingo::datum::Shockwave3dObjectRef;
                    Ok(player.alloc_datum(Datum::Shockwave3dObjectRef(Shockwave3dObjectRef {
                        cast_lib: member_ref.cast_lib,
                        cast_member: member_ref.cast_member,
                        object_type: handler_name.to_string(),
                        name: resolved_name,
                    })))
                })
            }
            "modelsUnderRay" => {
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
                    let max_models = if args.len() > 2 { player.get_datum(&args[2]).int_value().unwrap_or(100) } else { 100 };
                    let detailed = if args.len() > 3 {
                        player.get_datum(&args[3]).string_value().unwrap_or_default() == "detailed"
                    } else { false };

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
                            for (name, &vis_mode) in &w3d.runtime_state.node_visibility {
                                if vis_mode == 0 { excluded.insert(name.clone()); } // #none
                            }
                            for name in &w3d.runtime_state.detached_nodes {
                                excluded.insert(name.clone());
                            }
                            if let Some(ref scene) = w3d.parsed_scene {
                                for node in &scene.nodes {
                                    if excluded.contains(&node.name) { continue; }
                                    let mut parent = &node.parent_name;
                                    for _ in 0..10 {
                                        if parent.is_empty() {
                                            excluded.insert(node.name.clone());
                                            break;
                                        }
                                        if *parent == "World" { break; }
                                        if w3d.runtime_state.detached_nodes.contains(parent.as_str()) {
                                            excluded.insert(node.name.clone());
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
                        let excluded_ref = if excluded_nodes.is_empty() { None } else { Some(&excluded_nodes) };
                        let hits = raycast_scene_multi(
                            &ray, &scene, 100000.0, max_models as usize,
                            node_transforms.as_ref(),
                            excluded_ref,
                        );
                        for hit in &hits {
                            if detailed {
                                let model_key = player.alloc_datum(Datum::Symbol("model".to_string()));
                                let model_val = player.alloc_datum(Datum::Shockwave3dObjectRef(
                                    crate::director::lingo::datum::Shockwave3dObjectRef {
                                        cast_lib: member_ref.cast_lib, cast_member: member_ref.cast_member,
                                        object_type: "model".to_string(),
                                        name: hit.model_name.clone(),
                                    }
                                ));
                                let dist_key = player.alloc_datum(Datum::Symbol("distance".to_string()));
                                let dist_val = player.alloc_datum(Datum::Float(hit.distance as f64));
                                let pos_key = player.alloc_datum(Datum::Symbol("isectPosition".to_string()));
                                let pos_val = player.alloc_datum(Datum::Vector([
                                    hit.position[0] as f64, hit.position[1] as f64, hit.position[2] as f64,
                                ]));
                                let norm_key = player.alloc_datum(Datum::Symbol("isectNormal".to_string()));
                                let norm_val = player.alloc_datum(Datum::Vector([
                                    hit.normal[0] as f64, hit.normal[1] as f64, hit.normal[2] as f64,
                                ]));
                                let mesh_key = player.alloc_datum(Datum::Symbol("meshID".to_string()));
                                let mesh_val = player.alloc_datum(Datum::Int(hit.mesh_id as i32));
                                let face_key = player.alloc_datum(Datum::Symbol("faceID".to_string()));
                                let face_val = player.alloc_datum(Datum::Int(hit.face_index as i32 + 1));
                                let vert_key = player.alloc_datum(Datum::Symbol("vertices".to_string()));
                                let mut vert_items = VecDeque::new();
                                for vtx in &hit.vertices {
                                    vert_items.push_back(player.alloc_datum(Datum::Vector([
                                        vtx[0] as f64, vtx[1] as f64, vtx[2] as f64,
                                    ])));
                                }
                                let vert_val = player.alloc_datum(Datum::List(
                                    crate::director::lingo::datum::DatumType::List, vert_items, false,
                                ));
                                let uv_key = player.alloc_datum(Datum::Symbol("uvCoord".to_string()));
                                let u_key = player.alloc_datum(Datum::Symbol("u".to_string()));
                                let u_val = player.alloc_datum(Datum::Float(hit.uv_coord[0] as f64));
                                let v_key = player.alloc_datum(Datum::Symbol("v".to_string()));
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
                                        object_type: "model".to_string(),
                                        name: hit.model_name.clone(),
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
            "modelsUnderLoc" | "modelUnderLoc" => {
                reserve_player_mut(|player| {
                    if handler_name == "modelUnderLoc" {
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

    pub fn get_3d_collection_count(scene: &crate::director::chunks::w3d::types::W3dScene, collection: &str) -> i32 {
        use crate::director::chunks::w3d::types::W3dNodeType;
        match collection {
            "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).count() as i32,
            "modelResource" => scene.model_resources.len() as i32,
            "shader" => scene.shaders.len() as i32,
            "texture" => scene.texture_images.len() as i32,
            "light" => scene.lights.len() as i32,
            "camera" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::View).count() as i32,
            "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).count() as i32,
            "motion" => scene.motions.len() as i32,
            _ => 0,
        }
    }

    pub fn get_3d_object_name_by_index(scene: &crate::director::chunks::w3d::types::W3dScene, collection: &str, index: usize) -> Option<String> {
        use crate::director::chunks::w3d::types::W3dNodeType;
        if index == 0 { return None; }
        let idx = index - 1; // 1-based to 0-based
        match collection {
            "model" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model).nth(idx).map(|n| n.name.clone()),
            "modelResource" => scene.model_resources.keys().nth(idx).cloned(),
            "shader" => scene.shaders.get(idx).map(|s| s.name.clone()),
            "texture" => scene.texture_images.keys().nth(idx).cloned(),
            "light" => scene.lights.get(idx).map(|l| l.name.clone()),
            "camera" => {
                // Director puts DefaultView as camera[1], then other cameras in scene order
                let mut cams: Vec<&str> = Vec::new();
                // DefaultView first
                if let Some(dv) = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View && n.name.eq_ignore_ascii_case("defaultview")) {
                    cams.push(&dv.name);
                }
                // Then other cameras in scene order
                for n in &scene.nodes {
                    if n.node_type == W3dNodeType::View && !n.name.eq_ignore_ascii_case("defaultview") {
                        cams.push(&n.name);
                    }
                }
                cams.get(idx).map(|s| s.to_string())
            }
            "group" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Group).nth(idx).map(|n| n.name.clone()),
            "motion" => scene.motions.get(idx).map(|m| m.name.clone()),
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
