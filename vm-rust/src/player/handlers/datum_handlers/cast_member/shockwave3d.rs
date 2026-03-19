use std::collections::VecDeque;

use crate::{
    director::lingo::datum::Datum,
    player::{cast_lib::CastMemberRef, DirPlayer, ScriptError},
};

pub struct Shockwave3dMemberHandlers {}

impl Shockwave3dMemberHandlers {
    pub fn get_prop(
        player: &mut DirPlayer,
        cast_member_ref: &CastMemberRef,
        prop: &String,
    ) -> Result<Datum, ScriptError> {
        // Clone info and scene data upfront to avoid borrow conflicts with player.alloc_datum
        let (info, scene_data) = {
            let member = player
                .movie
                .cast_manager
                .find_member_by_ref(cast_member_ref)
                .ok_or_else(|| ScriptError::new("Cast member not found".to_string()))?;
            let w3d = member.member_type.as_shockwave3d()
                .ok_or_else(|| ScriptError::new("Not a Shockwave3D member".to_string()))?;
            (w3d.info.clone(), w3d.parsed_scene.clone())
        };

        use crate::director::chunks::w3d::types::W3dNodeType;

        match prop.as_str() {
            // ─── Member-level properties ───
            "directToStage" => Ok(Datum::Int(if info.direct_to_stage { 1 } else { 0 })),
            "preLoad" | "preload" => Ok(Datum::Int(if info.preload { 1 } else { 0 })),
            "duration" => Ok(Datum::Int(info.duration as i32)),

            "regPoint" => {
                let x = player.alloc_datum(Datum::Int(info.reg_point.0));
                let y = player.alloc_datum(Datum::Int(info.reg_point.1));
                Ok(Datum::Point([x, y]))
            }
            "rect" => {
                let r = info.default_rect;
                Ok(Datum::Rect([
                    player.alloc_datum(Datum::Int(r.0)),
                    player.alloc_datum(Datum::Int(r.1)),
                    player.alloc_datum(Datum::Int(r.2)),
                    player.alloc_datum(Datum::Int(r.3)),
                ]))
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
                        "camera" => scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::View).map(|n| n.name.clone()).collect(),
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

            _ => {
                web_sys::console::log_1(&format!("[W3D] Unknown Shockwave3D property: {}", prop).into());
                Err(ScriptError::new(format!(
                    "Cannot get Shockwave3D property '{}'", prop
                )))
            }
        }
    }

    pub fn set_prop(
        _player: &mut DirPlayer,
        _cast_member_ref: &CastMemberRef,
        prop: &String,
        _value: &Datum,
    ) -> Result<(), ScriptError> {
        match prop.as_str() {
            "directToStage" | "preLoad" | "preload" | "loop" | "animationEnabled" => {
                // Accept but don't apply yet
                Ok(())
            }
            _ => {
                web_sys::console::log_1(&format!("[W3D] Unknown Shockwave3D set property: {}", prop).into());
                Err(ScriptError::new(format!(
                    "Cannot set Shockwave3D property '{}'", prop
                )))
            }
        }
    }
}

/// Render a Shockwave3D scene to RGBA pixels using a temporary offscreen WebGL2 context.
fn render_3d_to_rgba(
    scene_data: &Option<crate::director::chunks::w3d::types::W3dScene>,
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
            web_sys::console::log_1(&format!("[W3D] render_3d_to_rgba failed: {:?}", e).into());
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
