//! WebGL2 GPU-accelerated renderer
//!
//! This module provides hardware-accelerated rendering using WebGL2.
//! It renders sprites as textured quads with shaders implementing
//! Director's ink modes for pixel-perfect compatibility.
//!
//! This is work in progress - the renderer is not yet fully functional.

mod context;
mod geometry;
mod shaders;
mod texture_cache;

use log::debug;
use itertools::Itertools;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use std::collections::HashMap;

use crate::player::{
    bitmap::bitmap::{get_system_default_palette, resolve_color_ref, Bitmap, PaletteRef},
    bitmap::drawing::CopyPixelsParams,
    cast_lib::CastMemberRef,
    cast_member::CastMemberType,
    font::{bitmap_font_copy_char, measure_text, measure_text_wrapped, get_glyph_preference, GlyphPreference, BitmapFont},
    geometry::IntRect,
    handlers::datum_handlers::cast_member::font::{FontMemberHandlers, StyledSpan, HtmlStyle, TextAlignment},
    score::{get_concrete_sprite_rect, get_sprite_at, ScoreRef},
    sprite::{ColorRef, CursorRef, is_skew_flip},
    DirPlayer,
};
use crate::rendering::{render_score_to_bitmap_with_offset, FilmLoopParentProps};

pub use context::WebGL2Context;
pub use geometry::QuadGeometry;
pub use shaders::{InkMode, ShaderManager};
pub use texture_cache::{TextureCache, TextureCacheKey, RenderedTextCache, RenderedTextCacheKey};
const DEBUG_WEBGL2_TEXT: bool = false;

/// WebGL2 hardware-accelerated renderer
///
/// This renderer uses WebGL2 to offload compositing to the GPU,
/// freeing up CPU cycles for script execution.
#[allow(dead_code)]
pub struct WebGL2Renderer {
    /// WebGL2 context wrapper
    context: WebGL2Context,
    /// Main canvas element
    canvas: HtmlCanvasElement,
    /// Preview canvas element (uses Canvas2D for simplicity)
    preview_canvas: HtmlCanvasElement,
    /// Preview 2D context for member preview rendering
    preview_ctx2d: web_sys::CanvasRenderingContext2d,
    /// Canvas size
    size: (u32, u32),
    /// Preview size
    preview_size: (u32, u32),
    /// Shader manager for ink mode shaders
    shader_manager: ShaderManager,
    /// Texture cache for bitmap textures
    texture_cache: TextureCache,
    /// Quad geometry for sprite rendering
    quad: QuadGeometry,
    /// Orthographic projection matrix (column-major for WebGL)
    projection_matrix: [f32; 16],
    /// Frame counter for debug logging
    frame_count: u64,
    /// Debug: currently selected channel number for inspector
    pub debug_selected_channel_num: Option<i16>,
    /// Solid color texture cache (keyed by RGB tuple)
    solid_color_textures: HashMap<(u8, u8, u8), web_sys::WebGlTexture>,
    /// Current preview member reference
    preview_member_ref: Option<CastMemberRef>,
    /// Preview font size override
    pub preview_font_size: Option<u16>,
    /// Preview container element
    preview_container_element: Option<web_sys::HtmlElement>,
    /// Rendered text texture cache
    rendered_text_cache: RenderedTextCache,
    /// Last known palette version - used to clear texture cache when palettes change
    last_palette_version: u32,
    /// Trails framebuffer object for accumulating trails sprite images
    trails_fbo: Option<web_sys::WebGlFramebuffer>,
    /// Trails texture (color attachment for trails FBO)
    trails_texture: Option<web_sys::WebGlTexture>,
    /// Size of the trails FBO texture
    trails_size: (u32, u32),
}

impl WebGL2Renderer {
    /// Create a new WebGL2 renderer
    pub fn new(
        canvas: HtmlCanvasElement,
        preview_canvas: HtmlCanvasElement,
    ) -> Result<Self, JsValue> {
        // Force pixel-perfect rendering: prevent browser from bilinear-scaling
        // the canvas when CSS size or devicePixelRatio differs from canvas dimensions.
        // Also disable ClearType/subpixel rendering and compositor smoothing.
        {
            let style = canvas.style();
            let _ = style.set_property("image-rendering", "pixelated");
            let _ = style.set_property("image-rendering", "-moz-crisp-edges");
            let _ = style.set_property("image-rendering", "crisp-edges");
            let _ = style.set_property("-webkit-font-smoothing", "none");
            let _ = style.set_property("-moz-osx-font-smoothing", "grayscale");
            let _ = style.set_property("font-smooth", "never");
            let _ = style.set_property("text-rendering", "optimizeSpeed");
            let _ = style.set_property("backface-visibility", "hidden");
        }

        // Create WebGL2 context with pixel-perfect settings:
        // - antialias: false - disable MSAA to prevent sub-pixel blurring
        // - alpha: false - stage is always opaque, no HTML page bleed-through
        let context_options = js_sys::Object::new();
        js_sys::Reflect::set(&context_options, &"antialias".into(), &false.into())?;
        js_sys::Reflect::set(&context_options, &"alpha".into(), &false.into())?;

        let gl = canvas
            .get_context_with_context_options("webgl2", &context_options)?
            .ok_or_else(|| JsValue::from_str("WebGL2 not supported"))?
            .dyn_into::<WebGl2RenderingContext>()?;

        let context = WebGL2Context::new(gl)?;
        let shader_manager = ShaderManager::new(&context)?;
        let texture_cache = TextureCache::new();
        let quad = QuadGeometry::new(&context)?;

        let size = (canvas.width(), canvas.height());
        let preview_size = (preview_canvas.width(), preview_canvas.height());

        // Create Canvas2D context for preview rendering
        let preview_ctx2d = preview_canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("Failed to get 2D context for preview"))?
            .dyn_into::<web_sys::CanvasRenderingContext2d>()?;

        // Create initial orthographic projection matrix
        let projection_matrix = Self::create_ortho_matrix(size.0 as f32, size.1 as f32);

        Ok(Self {
            context,
            canvas,
            preview_canvas,
            preview_ctx2d,
            size,
            preview_size,
            shader_manager,
            texture_cache,
            quad,
            projection_matrix,
            frame_count: 0,
            debug_selected_channel_num: None,
            solid_color_textures: HashMap::new(),
            preview_member_ref: None,
            preview_font_size: None,
            preview_container_element: None,
            rendered_text_cache: RenderedTextCache::new(),
            last_palette_version: 0,
            trails_fbo: None,
            trails_texture: None,
            trails_size: (0, 0),
        })
    }

    /// Create an orthographic projection matrix (column-major for WebGL)
    /// Maps (0,0)-(width,height) to (-1,-1)-(1,1) with Y flipped for screen coords
    fn create_ortho_matrix(width: f32, height: f32) -> [f32; 16] {
        // Ortho projection: x: 0..width -> -1..1, y: 0..height -> 1..-1 (flip Y)
        let sx = 2.0 / width;
        let sy = -2.0 / height;  // Negative to flip Y
        let tx = -1.0;
        let ty = 1.0;

        // Column-major 4x4 matrix
        [
            sx,  0.0, 0.0, 0.0,  // column 0
            0.0, sy,  0.0, 0.0,  // column 1
            0.0, 0.0, 1.0, 0.0,  // column 2
            tx,  ty,  0.0, 1.0,  // column 3
        ]
    }

    /// Check if WebGL2 is available
    pub fn is_supported() -> bool {
        super::is_webgl2_supported()
    }

    /// Ensure the trails framebuffer exists and is the correct size.
    /// The trails FBO accumulates images of trails sprites across frames.
    fn ensure_trails_fbo(&mut self, width: u32, height: u32) {
        if self.trails_fbo.is_some() && self.trails_size == (width, height) {
            return;
        }

        let gl = self.context.gl();

        // Delete old resources
        if let Some(fbo) = self.trails_fbo.take() {
            gl.delete_framebuffer(Some(&fbo));
        }
        if let Some(tex) = self.trails_texture.take() {
            gl.delete_texture(Some(&tex));
        }

        // Create texture
        let tex = match gl.create_texture() {
            Some(t) => t,
            None => return,
        };
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&tex));
        let _ = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            WebGl2RenderingContext::TEXTURE_2D,
            0,
            WebGl2RenderingContext::RGBA as i32,
            width as i32,
            height as i32,
            0,
            WebGl2RenderingContext::RGBA,
            WebGl2RenderingContext::UNSIGNED_BYTE,
            None,
        );
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::NEAREST as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::NEAREST as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);

        // Create framebuffer and attach texture
        let fbo = match gl.create_framebuffer() {
            Some(f) => f,
            None => {
                gl.delete_texture(Some(&tex));
                return;
            }
        };
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(&fbo));
        gl.framebuffer_texture_2d(
            WebGl2RenderingContext::FRAMEBUFFER,
            WebGl2RenderingContext::COLOR_ATTACHMENT0,
            WebGl2RenderingContext::TEXTURE_2D,
            Some(&tex),
            0,
        );

        // Clear to transparent
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT);

        // Unbind
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);

        self.trails_fbo = Some(fbo);
        self.trails_texture = Some(tex);
        self.trails_size = (width, height);
    }

    /// Delete the trails FBO and texture.
    fn destroy_trails_fbo(&mut self) {
        let gl = self.context.gl();
        if let Some(fbo) = self.trails_fbo.take() {
            gl.delete_framebuffer(Some(&fbo));
        }
        if let Some(tex) = self.trails_texture.take() {
            gl.delete_texture(Some(&tex));
        }
        self.trails_size = (0, 0);
    }

    /// Draw the accumulated trails texture as a full-screen quad.
    fn draw_trails_texture(&mut self) {
        let trails_tex = match &self.trails_texture {
            Some(t) => t,
            None => return,
        };

        let gl = self.context.gl();
        let (width, height) = self.size;

        // Use Copy ink for simple blitting
        let effective_ink = self.shader_manager.use_program(&self.context, InkMode::Copy);
        let program = match self.shader_manager.get_program(effective_ink) {
            Some(p) => p,
            None => return,
        };

        self.context.set_blend_alpha();

        // Bind trails texture
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(trails_tex));
        if let Some(ref loc) = program.u_texture {
            gl.uniform1i(Some(loc), 0);
        }

        // Full-screen quad
        if let Some(ref loc) = program.u_sprite_rect {
            gl.uniform4f(Some(loc), 0.0, 0.0, width as f32, height as f32);
        }
        if let Some(ref loc) = program.u_tex_rect {
            gl.uniform4f(Some(loc), 0.0, 0.0, 1.0, 1.0);
        }
        if let Some(ref loc) = program.u_flip {
            gl.uniform2f(Some(loc), 0.0, 0.0);
        }
        if let Some(ref loc) = program.u_rotation {
            gl.uniform1f(Some(loc), 0.0);
        }
        if let Some(ref loc) = program.u_skew_flip {
            gl.uniform1f(Some(loc), 0.0);
        }
        if let Some(ref loc) = program.u_rotation_center {
            gl.uniform2f(Some(loc), 0.0, 0.0);
        }
        if let Some(ref loc) = program.u_blend {
            gl.uniform1f(Some(loc), 1.0);
        }

        self.quad.draw(gl);

        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
    }

    /// Draw the current frame
    pub fn draw_frame(&mut self, player: &mut DirPlayer) {
        self.frame_count += 1;

        // Check if palettes changed and clear texture cache if so
        // This handles external cast loading where palette members may load after initial render
        let current_palette_version = player.movie.cast_manager.palette_version();
        if current_palette_version != self.last_palette_version {
            self.texture_cache.clear();
            self.last_palette_version = current_palette_version;
        }

        // Clear with stage background color
        let bg_color = self.get_stage_bg_color(player);
        {
            let gl = self.context.gl();
            gl.clear_color(bg_color.0, bg_color.1, bg_color.2, 1.0);
            gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT);
        }

        // Advance texture cache frame counters
        self.texture_cache.next_frame();
        self.rendered_text_cache.next_frame();

        // Get sorted channel numbers for current frame
        let sorted_channels: Vec<(i16, i32)> = player
            .movie
            .score
            .get_sorted_channels(player.movie.current_frame)
            .iter()
            .map(|x| (x.number as i16, x.sprite.loc_z))
            .collect_vec();

        // Check if any sprite has trails
        let trails_channels: Vec<i16> = sorted_channels.iter()
            .filter(|(ch, _)| {
                player.movie.score.get_sprite(*ch).map_or(false, |s| s.trails)
            })
            .map(|(ch, _)| *ch)
            .collect();
        let has_trails = !trails_channels.is_empty();

        // Bind quad geometry once
        self.quad.bind(self.context.gl());

        // Draw accumulated trails texture onto the cleared stage
        if has_trails && self.trails_texture.is_some() {
            self.draw_trails_texture();
        }

        // Render each sprite
        for (channel_num, _) in &sorted_channels {
            self.render_sprite(player, *channel_num);
        }

        // Accumulate trails sprites into the trails texture via GPU-side copy.
        // Instead of re-rendering sprites to the FBO, we copy the already-rendered
        // pixel rects from the default framebuffer using copyTexSubImage2D.
        if has_trails {
            let (w, h) = self.size;
            self.ensure_trails_fbo(w, h);

            if self.trails_texture.is_some() {
                let gl = self.context.gl();

                // Read from default framebuffer (screen)
                gl.bind_framebuffer(WebGl2RenderingContext::READ_FRAMEBUFFER, None);

                // Bind trails texture to copy into
                let tex = self.trails_texture.as_ref().unwrap();
                gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));

                // For each trails sprite, copy its bounding rect from screen to trails texture
                for ch in &trails_channels {
                    if let Some(sprite) = player.movie.score.get_sprite(*ch) {
                        let rect = get_concrete_sprite_rect(player, sprite);
                        let x = rect.left.max(0).min(w as i32);
                        let y = rect.top.max(0).min(h as i32);
                        let r = rect.right.max(0).min(w as i32);
                        let b = rect.bottom.max(0).min(h as i32);
                        if r > x && b > y {
                            // WebGL Y is flipped: screen Y=0 is bottom, texture Y=0 is top
                            let gl_y = h as i32 - b;
                            let _ = gl.copy_tex_sub_image_2d(
                                WebGl2RenderingContext::TEXTURE_2D,
                                0,
                                x,         // xoffset in texture
                                gl_y,      // yoffset in texture (flipped)
                                x,         // x in framebuffer
                                gl_y,      // y in framebuffer (flipped)
                                r - x,     // width
                                b - y,     // height
                            );
                        }
                    }
                }

                gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
            }
        } else if self.trails_fbo.is_some() {
            // No trails sprites - clean up FBO
            self.destroy_trails_fbo();
        }

        // Draw custom cursor sprite
        self.draw_cursor(player);

        // Draw debug text overlay (datum count, script count)
        self.draw_debug_text_overlay(player);

        // Unbind
        self.quad.unbind(self.context.gl());

        // Draw debug highlight for selected channel
        if let Some(selected_channel) = self.debug_selected_channel_num {
            self.draw_debug_highlight(player, selected_channel);
            // Reset shader manager state since draw_debug_highlight uses its own shader
            // and calls gl.use_program(None), which desynchronizes the manager's cached state
            self.shader_manager.clear_active();
        }

        // Draw pick highlight for hovered sprite
        if player.picking_mode {
            let hovered = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
            if let Some(channel) = hovered {
                self.draw_pick_highlight(player, channel as i16);
                self.shader_manager.clear_active();
            }
        }
    }

    /// Draw the custom cursor sprite at the mouse position.
    /// Mirrors the logic from `draw_cursor` in rendering.rs.
    fn draw_cursor(&mut self, player: &mut DirPlayer) {
        // Determine which cursor to use: hovered sprite's cursor or global cursor
        let hovered_sprite = get_sprite_at(player, player.mouse_loc.0, player.mouse_loc.1, false);
        let cursor_ref = if let Some(hovered_sprite) = hovered_sprite {
            let sprite = player.movie.score.get_sprite(hovered_sprite as i16);
            sprite.and_then(|s| s.cursor_ref.clone())
        } else {
            None
        };
        let cursor_ref = cursor_ref.as_ref().unwrap_or(&player.cursor);
        let cursor_list = match cursor_ref {
            CursorRef::Member(ids) => Some(ids),
            _ => None,
        };

        let cursor_bitmap_member = cursor_list
            .and_then(|ids| ids.first().map(|x| *x))
            .and_then(|id| player.movie.cast_manager.find_member_by_slot_number(id as u32))
            .and_then(|m| m.member_type.as_bitmap().cloned());

        let cursor_mask_member = cursor_list
            .and_then(|ids| ids.get(1).map(|x| *x))
            .and_then(|id| player.movie.cast_manager.find_member_by_slot_number(id as u32))
            .and_then(|m| m.member_type.as_bitmap().cloned());

        let cursor_bitmap_member = match cursor_bitmap_member {
            Some(m) => m,
            None => {
                // No custom cursor - restore native cursor
                let _ = self.canvas.style().set_property("cursor", "default");
                return;
            }
        };

        let cursor_bitmap = match player.bitmap_manager.get_bitmap(cursor_bitmap_member.image_ref) {
            Some(b) => b,
            None => {
                let _ = self.canvas.style().set_property("cursor", "default");
                return;
            }
        };

        // Build RGBA data with mask applied to alpha
        let palettes = player.movie.cast_manager.palettes();
        let w = cursor_bitmap.width as u32;
        let h = cursor_bitmap.height as u32;
        let mut rgba = vec![0u8; (w * h * 4) as usize];

        // Convert cursor bitmap to RGBA
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = cursor_bitmap.get_pixel_color(&palettes, x as u16, y as u16);
                let idx = ((y * w + x) * 4) as usize;
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = 255;
            }
        }

        // Apply mask if available
        if let Some(mask_member) = cursor_mask_member {
            if let Some(mask_bitmap) = player.bitmap_manager.get_bitmap(mask_member.image_ref) {
                for y in 0..h.min(mask_bitmap.height as u32) {
                    for x in 0..w.min(mask_bitmap.width as u32) {
                        let (mr, mg, mb) = mask_bitmap.get_pixel_color(&palettes, x as u16, y as u16);
                        let idx = ((y * w + x) * 4) as usize;
                        // White mask pixels = transparent, black = opaque
                        if mr > 127 && mg > 127 && mb > 127 {
                            rgba[idx + 3] = 0;
                        }
                    }
                }
            }
        }

        // Upload as texture
        let texture = match self.context.create_texture() {
            Ok(t) => t,
            Err(_) => return,
        };
        if self.context.upload_texture_rgba(&texture, w, h, &rgba).is_err() {
            return;
        }

        // Draw at mouse position, offset by reg_point
        let dest_x = player.mouse_loc.0 - cursor_bitmap_member.reg_point.0 as i32;
        let dest_y = player.mouse_loc.1 - cursor_bitmap_member.reg_point.1 as i32;

        // Use matte ink for cursor (alpha-based transparency)
        let effective_ink = self.shader_manager.use_program(&self.context, InkMode::Matte);
        let program = match self.shader_manager.get_program(effective_ink) {
            Some(p) => p,
            None => return,
        };

        let gl = self.context.gl();
        self.context.set_blend_alpha();

        // Bind texture
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&texture));
        if let Some(ref loc) = program.u_texture {
            gl.uniform1i(Some(loc), 0);
        }

        // Set sprite rect
        if let Some(ref loc) = program.u_sprite_rect {
            gl.uniform4f(Some(loc), dest_x as f32, dest_y as f32, w as f32, h as f32);
        }

        // Set texture rect (full texture)
        if let Some(ref loc) = program.u_tex_rect {
            gl.uniform4f(Some(loc), 0.0, 0.0, 1.0, 1.0);
        }

        // No flip, rotation, or skew for cursor
        if let Some(ref loc) = program.u_flip {
            gl.uniform2f(Some(loc), 0.0, 0.0);
        }
        if let Some(ref loc) = program.u_skew_flip {
            gl.uniform1f(Some(loc), 0.0);
        }
        if let Some(ref loc) = program.u_rotation {
            gl.uniform1f(Some(loc), 0.0);
        }
        if let Some(ref loc) = program.u_rotation_center {
            gl.uniform2f(Some(loc), dest_x as f32, dest_y as f32);
        }

        // Full opacity
        if let Some(ref loc) = program.u_blend {
            gl.uniform1f(Some(loc), 1.0);
        }

        // Draw
        self.quad.draw(gl);

        // Cleanup
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);

        // Hide native system cursor since we're drawing a custom one
        let _ = self.canvas.style().set_property("cursor", "none");
    }

    /// Get stage background color as normalized floats
    fn get_stage_bg_color(&self, player: &DirPlayer) -> (f32, f32, f32) {
        let palettes = player.movie.cast_manager.palettes();
        let (r, g, b) = resolve_color_ref(
            &palettes,
            &player.movie.stage_color_ref,
            &PaletteRef::BuiltIn(get_system_default_palette()),
            8, // bit depth
        );
        (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }

    /// Draw debug text overlay showing datum and script counts
    fn draw_debug_text_overlay(&mut self, player: &mut DirPlayer) {
        // Get system font
        let font = match player.font_manager.get_system_font() {
            Some(f) => f,
            None => return, // No font available
        };

        // Get font bitmap
        let font_bitmap = match player.bitmap_manager.get_bitmap(font.bitmap_ref) {
            Some(b) => b,
            None => return, // No font bitmap
        };

        // Create debug text
        let txt = format!(
            "Datum count: {}\nScript count: {}",
            player.allocator.datum_count(),
            player.allocator.script_instance_count()
        );

        // Measure text to determine texture size
        let (text_width, text_height) = measure_text(&txt, &font, None, 0, 0, 0);
        let width = (text_width as u32).max(1);
        let height = (text_height as u32).max(1);

        // Create a 32-bit RGBA bitmap for rendering text
        let mut text_bitmap = Bitmap::new(
            width as u16,
            height as u16,
            32,
            32,
            0,
            PaletteRef::BuiltIn(get_system_default_palette()),
        );

        // Bitmap::new initializes 32-bit data to 255 (white/opaque)
        // We'll make white pixels transparent after rendering text

        let palettes = player.movie.cast_manager.palettes();

        // Set up copy parameters for text rendering with background transparent ink
        let params = CopyPixelsParams {
            blend: 100,
            ink: 36, // Background transparent
            color: ColorRef::Rgb(0, 0, 0), // Black text
            bg_color: ColorRef::Rgb(255, 255, 255), // White background (transparent)
            mask_image: None,
            is_text_rendering: true,
            rotation: 0.0,
            skew: 0.0,
            sprite: None,
            original_dst_rect: None,
        };

        // Render text to the bitmap
        text_bitmap.draw_text(
            &txt,
            &font,
            font_bitmap,
            0,
            0,
            params,
            &palettes,
            0,
            0,
        );

        // After drawing text, convert white pixels to transparent
        for i in 0..text_bitmap.data.len() / 4 {
            let r = text_bitmap.data[i * 4];
            let g = text_bitmap.data[i * 4 + 1];
            let b = text_bitmap.data[i * 4 + 2];
            if r == 255 && g == 255 && b == 255 {
                text_bitmap.data[i * 4 + 3] = 0;
            }
        }

        // Upload the bitmap as a texture
        let texture = match self.context.create_texture() {
            Ok(t) => t,
            Err(_) => return,
        };
        if self.context.upload_texture_rgba(&texture, width, height, &text_bitmap.data).is_err() {
            return;
        }

        // Draw the debug text texture at top-left corner
        let ink_mode = InkMode::Copy;
        let effective_ink = self.shader_manager.use_program(&self.context, ink_mode);

        let program = match self.shader_manager.get_program(effective_ink) {
            Some(p) => p,
            None => return,
        };

        self.context.set_blend_alpha();

        let gl = self.context.gl();

        // Set projection matrix
        if let Some(ref loc) = program.u_projection {
            gl.uniform_matrix4fv_with_f32_array(Some(loc), false, &self.projection_matrix);
        }

        // Bind texture
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&texture));
        if let Some(ref loc) = program.u_texture {
            gl.uniform1i(Some(loc), 0);
        }

        // Set sprite rect uniform (position at 0,0 with text dimensions)
        if let Some(ref loc) = program.u_sprite_rect {
            gl.uniform4f(Some(loc), 0.0, 0.0, width as f32, height as f32);
        }

        // Set texture rect (full texture)
        if let Some(ref loc) = program.u_tex_rect {
            gl.uniform4f(Some(loc), 0.0, 0.0, 1.0, 1.0);
        }

        // No flip
        if let Some(ref loc) = program.u_flip {
            gl.uniform2f(Some(loc), 0.0, 0.0);
        }

        // No rotation
        if let Some(ref loc) = program.u_rotation {
            gl.uniform1f(Some(loc), 0.0);
        }
        if let Some(ref loc) = program.u_rotation_center {
            gl.uniform2f(Some(loc), 0.0, 0.0);
        }
        // No skew flip
        if let Some(ref loc) = program.u_skew_flip {
            gl.uniform1f(Some(loc), 0.0);
        }

        // Full blend
        if let Some(ref loc) = program.u_blend {
            gl.uniform1f(Some(loc), 1.0);
        }

        // Draw the quad
        self.quad.draw(gl);

        // Cleanup
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
        gl.delete_texture(Some(&texture));
    }

    /// Draw debug highlight rectangle around selected sprite
    fn draw_debug_highlight(&self, player: &DirPlayer, channel_num: i16) {
        let sprite = match player.movie.score.get_sprite(channel_num) {
            Some(s) => s,
            None => return,
        };

        let rect = get_concrete_sprite_rect(player, sprite);
        let gl = self.context.gl();

        // Draw red border using WebGL lines
        // Convert to normalized device coordinates
        let width = self.size.0 as f32;
        let height = self.size.1 as f32;

        let left = (rect.left as f32 / width) * 2.0 - 1.0;
        let right = (rect.right as f32 / width) * 2.0 - 1.0;
        let top = 1.0 - (rect.top as f32 / height) * 2.0;
        let bottom = 1.0 - (rect.bottom as f32 / height) * 2.0;

        // Create line vertices for a rectangle
        let vertices: [f32; 16] = [
            left, top,      // top-left
            right, top,     // top-right
            right, top,     // top-right
            right, bottom,  // bottom-right
            right, bottom,  // bottom-right
            left, bottom,   // bottom-left
            left, bottom,   // bottom-left
            left, top,      // back to top-left
        ];

        // Create a VAO to isolate our state changes
        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(&vao));

        // Create a simple line shader program if we don't have one
        // For now, use a direct WebGL approach
        let buffer = gl.create_buffer().unwrap();
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&buffer));

        unsafe {
            let vert_array = js_sys::Float32Array::view(&vertices);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &vert_array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }

        // Use a simple shader for colored lines
        let vs_source = "
            attribute vec2 a_position;
            void main() {
                gl_Position = vec4(a_position, 0.0, 1.0);
            }
        ";
        let fs_source = "
            precision mediump float;
            uniform vec4 u_color;
            void main() {
                gl_FragColor = u_color;
            }
        ";

        let vs = gl.create_shader(WebGl2RenderingContext::VERTEX_SHADER).unwrap();
        gl.shader_source(&vs, vs_source);
        gl.compile_shader(&vs);

        let fs = gl.create_shader(WebGl2RenderingContext::FRAGMENT_SHADER).unwrap();
        gl.shader_source(&fs, fs_source);
        gl.compile_shader(&fs);

        let program = gl.create_program().unwrap();
        gl.attach_shader(&program, &vs);
        gl.attach_shader(&program, &fs);
        gl.link_program(&program);
        gl.use_program(Some(&program));

        let pos_loc = gl.get_attrib_location(&program, "a_position") as u32;
        gl.enable_vertex_attrib_array(pos_loc);
        gl.vertex_attrib_pointer_with_i32(pos_loc, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);

        let color_loc = gl.get_uniform_location(&program, "u_color");
        gl.uniform4f(color_loc.as_ref(), 1.0, 0.0, 0.0, 1.0); // Red color

        gl.draw_arrays(WebGl2RenderingContext::LINES, 0, 8);

        // Draw a green pixel at the sprite's loc point
        let loc_x = (sprite.loc_h as f32 / width) * 2.0 - 1.0;
        let loc_y = 1.0 - (sprite.loc_v as f32 / height) * 2.0;

        let point_vertices: [f32; 2] = [loc_x, loc_y];
        unsafe {
            let vert_array = js_sys::Float32Array::view(&point_vertices);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &vert_array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }

        gl.uniform4f(color_loc.as_ref(), 0.0, 1.0, 0.0, 1.0); // Green color
        gl.draw_arrays(WebGl2RenderingContext::POINTS, 0, 1);

        // Cleanup - unbind and delete everything to restore state
        gl.bind_vertex_array(None);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, None);
        gl.use_program(None);

        gl.delete_vertex_array(Some(&vao));
        gl.delete_buffer(Some(&buffer));
        gl.delete_program(Some(&program));
        gl.delete_shader(Some(&vs));
        gl.delete_shader(Some(&fs));
    }

    /// Draw a green highlight rectangle around the hovered sprite during picking mode.
    fn draw_pick_highlight(&self, player: &DirPlayer, channel_num: i16) {
        let sprite = match player.movie.score.get_sprite(channel_num) {
            Some(s) => s,
            None => return,
        };

        let rect = get_concrete_sprite_rect(player, sprite);
        let gl = self.context.gl();

        let width = self.size.0 as f32;
        let height = self.size.1 as f32;

        let left = (rect.left as f32 / width) * 2.0 - 1.0;
        let right = (rect.right as f32 / width) * 2.0 - 1.0;
        let top = 1.0 - (rect.top as f32 / height) * 2.0;
        let bottom = 1.0 - (rect.bottom as f32 / height) * 2.0;

        let vertices: [f32; 16] = [
            left, top,
            right, top,
            right, top,
            right, bottom,
            right, bottom,
            left, bottom,
            left, bottom,
            left, top,
        ];

        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(&vao));

        let buffer = gl.create_buffer().unwrap();
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&buffer));

        unsafe {
            let vert_array = js_sys::Float32Array::view(&vertices);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &vert_array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }

        let vs_source = "
            attribute vec2 a_position;
            void main() {
                gl_Position = vec4(a_position, 0.0, 1.0);
            }
        ";
        let fs_source = "
            precision mediump float;
            uniform vec4 u_color;
            void main() {
                gl_FragColor = u_color;
            }
        ";

        let vs = gl.create_shader(WebGl2RenderingContext::VERTEX_SHADER).unwrap();
        gl.shader_source(&vs, vs_source);
        gl.compile_shader(&vs);

        let fs = gl.create_shader(WebGl2RenderingContext::FRAGMENT_SHADER).unwrap();
        gl.shader_source(&fs, fs_source);
        gl.compile_shader(&fs);

        let program = gl.create_program().unwrap();
        gl.attach_shader(&program, &vs);
        gl.attach_shader(&program, &fs);
        gl.link_program(&program);
        gl.use_program(Some(&program));

        let pos_loc = gl.get_attrib_location(&program, "a_position") as u32;
        gl.enable_vertex_attrib_array(pos_loc);
        gl.vertex_attrib_pointer_with_i32(pos_loc, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);

        let color_loc = gl.get_uniform_location(&program, "u_color");
        gl.uniform4f(color_loc.as_ref(), 0.0, 1.0, 0.0, 1.0); // Green color

        gl.draw_arrays(WebGl2RenderingContext::LINES, 0, 8);

        gl.bind_vertex_array(None);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, None);
        gl.use_program(None);

        gl.delete_vertex_array(Some(&vao));
        gl.delete_buffer(Some(&buffer));
        gl.delete_program(Some(&program));
        gl.delete_shader(Some(&vs));
        gl.delete_shader(Some(&fs));
    }

    /// Render a single sprite
    fn render_sprite(&mut self, player: &mut DirPlayer, channel_num: i16) {
        // Get sprite and member info
        let (member_ref, mut sprite_rect, ink, blend, flip_h, flip_v, rotation, skew, bg_color, fg_color, has_fore_color, has_back_color, is_puppet, raw_loc, sprite_width, sprite_height) = {
            let score = &player.movie.score;
            let sprite = match score.get_sprite(channel_num) {
                Some(s) => s,
                None => return,
            };

            let member_ref = match &sprite.member {
                Some(m) => m.clone(),
                None => return,
            };

            let rect = get_concrete_sprite_rect(player, sprite);
            (
                member_ref,
                rect,
                sprite.ink,
                sprite.blend,
                sprite.flip_h,
                sprite.flip_v,
                sprite.rotation,
                sprite.skew,
                sprite.bg_color.clone(),
                sprite.color.clone(),
                sprite.has_fore_color,
                sprite.has_back_color,
                sprite.puppet,
                (sprite.loc_h, sprite.loc_v),
                sprite.width,
                sprite.height,
            )
        };

        // Determine what kind of texture we need based on member type
        enum TextureSource {
            Bitmap { image_ref: u32 },
            SolidColor { r: u8, g: u8, b: u8 },
            RenderedText {
                cache_key: RenderedTextCacheKey,
                text: String,
                font_name: String,
                font_size: u16,
                font_style: Option<u8>,
                font_id: Option<u16>,
                line_spacing: u16,
                top_spacing: i16,
                bottom_spacing: i16,
                width: u32,
                height: u32,
                text_fg_color: ColorRef,
                text_bg_color: ColorRef,
                styled_spans: Option<Vec<StyledSpan>>,
                alignment: String,
                word_wrap: bool,
                border: u16,
                box_drop_shadow: u16,
            },
            FilmLoop {
                initial_rect: IntRect,
                width: u32,
                height: u32,
            },
            ButtonBitmap {
                width: u32,
                height: u32,
                button_type: crate::player::cast_member::ButtonType,
                hilite: bool,
                text: String,
                font_name: String,
                font_size: u16,
                font_id: Option<u16>,
                alignment: String,
                ink: i32,
            },
            ShapeBitmap {
                width: u32,
                height: u32,
                shape_info: crate::director::enums::ShapeInfo,
                fg_color: (u8, u8, u8),
                bg_color: (u8, u8, u8),
            },
            VectorShapeBitmap {
                width: u32,
                height: u32,
                vector_member: crate::player::cast_member::VectorShapeMember,
            },
        }

        let texture_source = {
            let member = match player.movie.cast_manager.find_member_by_ref(&member_ref) {
                Some(m) => m,
                None => return,
            };

            match &member.member_type {
                CastMemberType::Bitmap(bitmap_member) => {
                    TextureSource::Bitmap { image_ref: bitmap_member.image_ref }
                }
                CastMemberType::Shape(shape_member) => {
                    // Skip rendering shapes with tiny dimensions (blank placeholders or zero-size)
                    if sprite_width <= 1 || sprite_height <= 1 {
                        return;
                    }

                    // Skip rendering shapes that use member 1:1 (placeholder)
                    if member_ref.cast_lib == 1 && member_ref.cast_member == 1 {
                        return;
                    }

                    let palettes = player.movie.cast_manager.palettes();
                    let fg_rgb = resolve_color_ref(
                        &palettes,
                        &fg_color,
                        &PaletteRef::BuiltIn(get_system_default_palette()),
                        8,
                    );
                    let bg_rgb = resolve_color_ref(
                        &palettes,
                        &bg_color,
                        &PaletteRef::BuiltIn(get_system_default_palette()),
                        8,
                    );

                    TextureSource::ShapeBitmap {
                        width: sprite_width as u32,
                        height: sprite_height as u32,
                        shape_info: shape_member.shape_info.clone(),
                        fg_color: fg_rgb,
                        bg_color: bg_rgb,
                    }
                }
                CastMemberType::VectorShape(vector_member) => {
                    if sprite_width <= 1 || sprite_height <= 1 {
                        return;
                    }
                    if member_ref.cast_lib == 1 && member_ref.cast_member == 1 {
                        return;
                    }
                    TextureSource::VectorShapeBitmap {
                        width: sprite_width as u32,
                        height: sprite_height as u32,
                        vector_member: vector_member.clone(),
                    }
                }
                CastMemberType::Font(font_member) => {
                    // Font member: render preview_text using the font
                    let text = &font_member.preview_text;
                    if text.is_empty() {
                        return; // No text to render
                    }

                    // Use sprite rect dimensions for the texture (not measured text)
                    // This prevents stretching when sprite rect differs from text size
                    let width = (sprite_rect.width()).max(1) as u32;
                    let height = (sprite_rect.height()).max(1) as u32;

                    // Get styled spans reference for cache key
                    let styled_spans_ref = if font_member.preview_html_spans.is_empty() {
                        None
                    } else {
                        Some(font_member.preview_html_spans.as_slice())
                    };

                    let cache_key = RenderedTextCacheKey::new_with_styled_spans(
                        member_ref.clone(),
                        text,
                        styled_spans_ref,
                        ink,
                        blend,
                        fg_color.clone(),
                        bg_color.clone(),
                        false,
                        width,
                        height,
                        "left",  // Font members default to left alignment
                        false,   // Font members default to no word wrap
                        &font_member.font_info.name,
                        font_member.font_info.size,
                        Some(font_member.font_info.style),
                        font_member.fixed_line_space,
                        font_member.top_spacing,
                    );

                    TextureSource::RenderedText {
                        cache_key,
                        text: text.clone(),
                        font_name: font_member.font_info.name.clone(),
                        font_size: font_member.font_info.size,
                        font_style: Some(font_member.font_info.style),
                        font_id: Some(font_member.font_info.font_id),
                        line_spacing: font_member.fixed_line_space,
                        top_spacing: font_member.top_spacing,
                        bottom_spacing: 0,
                        width,
                        height,
                        text_fg_color: fg_color.clone(),
                        text_bg_color: bg_color.clone(),
                        styled_spans: if font_member.preview_html_spans.is_empty() {
                            None
                        } else {
                            Some(font_member.preview_html_spans.clone())
                        },
                        alignment: "left".to_string(),
                        word_wrap: false,
                        border: 0,
                        box_drop_shadow: 0,
                    }
                }
                CastMemberType::Text(text_member) => {
                    // Text member: render the text using specified font
                    let text = &text_member.text;
                    if text.is_empty() {
                        return; // No text to render
                    }

                    // Derive wrapping behavior from text member box type + explicit wordWrap flag.
                    // Director commonly uses #adjust with wrapped multi-line text.
                    let box_type_key = text_member
                        .box_type
                        .trim()
                        .trim_start_matches('#')
                        .to_ascii_lowercase();
                    let box_type_implies_wrap = matches!(box_type_key.as_str(), "adjust");
                    let effective_word_wrap = text_member.word_wrap || box_type_implies_wrap;
                    let long_wrapped_text = effective_word_wrap && text.len() > 80;

                    // Keep mixed source sizing behavior stable to avoid regressions in small labels/buttons.
                    let width = if long_wrapped_text {
                        sprite_rect.width().max(1) as u32
                    } else if text_member.width > 0 {
                        (text_member.width as i32).max(sprite_rect.width()).max(1) as u32
                    } else {
                        sprite_rect.width().max(1) as u32
                    };
                    let height = sprite_rect.height().max(1) as u32;

                    // Extract font properties from first styled span if available
                    let (font_name, font_size, font_style) = if !text_member.html_styled_spans.is_empty() {
                        let first_style = &text_member.html_styled_spans[0].style;
                        // Always prefer text_member.font (may have been changed at runtime via Lingo)
                        // Only fall back to styled span font_face when member font is empty
                        let name = if !text_member.font.is_empty() {
                            text_member.font.clone()
                        } else {
                            first_style.font_face.clone().unwrap_or_else(|| "Arial".to_string())
                        };
                        // Prefer runtime font_size (set by Lingo) over original XMED span size.
                        // The XMED span retains the authoring-time value; the runtime value takes priority.
                        let size = if text_member.font_size > 0 {
                            text_member.font_size
                        } else {
                            first_style.font_size.map(|s| s as u16).unwrap_or(12)
                        };
                        // Convert bold/italic/underline to font_style: bit 0 = bold, bit 1 = italic, bit 2 = underline
                        let style = (if first_style.bold { 1u8 } else { 0 })
                            | (if first_style.italic { 2u8 } else { 0 })
                            | (if first_style.underline { 4u8 } else { 0 });
                        (name, size, Some(style))
                    } else {
                        let mut style = 0u8;
                        if text_member.font_style.iter().any(|s| s == "bold") {
                            style |= 1;
                        }
                        if text_member.font_style.iter().any(|s| s == "italic") {
                            style |= 2;
                        }
                        if text_member.font_style.iter().any(|s| s == "underline") {
                            style |= 4;
                        }
                        (
                            text_member.font.clone(),
                            text_member.font_size,
                            if style == 0 { None } else { Some(style) },
                        )
                    };
                    // Color priority for text:
                    // 1. Sprite has explicit foreColor (tween/Lingo has_fore_color) -> override everything
                    // 2. Non-default, non-black sprite.color (score frame data) -> matches non-WebGL2
                    //    PFR bitmap rendering where params.color = sprite.color always wins
                    // 3. Styled span color from XMED/HTML -> use per-span color
                    // 4. CastMember.color (XMED foreColor preserved at member level) -> fallback
                    // 5. Sprite.color (default palette) -> final fallback
                    let text_fg_color = if has_fore_color {
                        fg_color.clone()
                    } else if fg_color != ColorRef::PaletteIndex(255) && fg_color != ColorRef::Rgb(0, 0, 0) {
                        // Non-default, non-black sprite.color always wins over XMED span colors
                        // Matches non-WebGL2 PFR bitmap rendering: params.color = sprite.color
                        fg_color.clone()
                    } else if text_member.html_styled_spans.len() == 1 {
                        let style_color = text_member.html_styled_spans[0].style.color;
                        if let Some(c) = style_color {
                            ColorRef::Rgb(
                                ((c >> 16) & 0xFF) as u8,
                                ((c >> 8) & 0xFF) as u8,
                                (c & 0xFF) as u8,
                            )
                        } else if member.color != ColorRef::PaletteIndex(255) {
                            // Fallback to member's stored foreColor (from XMED)
                            member.color.clone()
                        } else {
                            fg_color.clone()
                        }
                    } else if member.color != ColorRef::PaletteIndex(255) {
                        // Multi-span text with no explicit sprite color: use member foreColor
                        member.color.clone()
                    } else {
                        fg_color.clone()
                    };
                    // Resolve PaletteIndex to RGB so span color assignment gets real colors
                    let text_fg_color = match &text_fg_color {
                        ColorRef::PaletteIndex(_) => {
                            let palettes = player.movie.cast_manager.palettes();
                            let (r, g, b) = resolve_color_ref(
                                &palettes,
                                &text_fg_color,
                                &PaletteRef::BuiltIn(get_system_default_palette()),
                                8,
                            );
                            ColorRef::Rgb(r, g, b)
                        }
                        _ => text_fg_color,
                    };
                    // Priority: sprite bgColor (if explicitly set) > member bgColor > sprite bgColor (default)
                    let effective_bg = if has_back_color {
                        bg_color.clone()
                    } else {
                        match &member.bg_color {
                            ColorRef::Rgb(_, _, _) => member.bg_color.clone(),
                            _ => bg_color.clone(),
                        }
                    };
                    let text_bg_color = match &effective_bg {
                        ColorRef::PaletteIndex(_) => {
                            let palettes = player.movie.cast_manager.palettes();
                            let (r, g, b) = resolve_color_ref(
                                &palettes,
                                &effective_bg,
                                &PaletteRef::BuiltIn(get_system_default_palette()),
                                8,
                            );
                            ColorRef::Rgb(r, g, b)
                        }
                        _ => effective_bg,
                    };

                    // Log color decision for text members
                    {
                        let span_colors: Vec<String> = text_member.html_styled_spans.iter().enumerate().map(|(i, s)| {
                            let c = s.style.color.map(|c| format!("#{:06X}", c & 0xFFFFFF)).unwrap_or_else(|| "none".to_string());
                            format!("span[{}]={}", i, c)
                        }).collect();
                        debug!(
                            "TextMember color decision: member='{}' has_fore_color={} sprite.color={:?} -> text_fg={:?} spans=[{}]",
                            member.name, has_fore_color, fg_color, text_fg_color, span_colors.join(", ")
                        );
                    }

                    let initial_span_size = text_member
                        .html_styled_spans
                        .iter()
                        .filter_map(|s| s.style.font_size)
                        .filter(|s| *s > 0)
                        .max()
                        .unwrap_or(0);
                    let runtime_size_scale = if text_member.font_size > 0
                        && initial_span_size > 0
                        && (text_member.font_size as i32) != initial_span_size
                    {
                        Some(text_member.font_size as f32 / initial_span_size as f32)
                    } else {
                        None
                    };

                    // Fill in defaults from text_member and apply runtime overrides when needed.
                    // Preserve span colors from XMED styles. Member foreColor is only fallback.

                    let styled_spans_with_defaults: Option<Vec<StyledSpan>> = if text_member.html_styled_spans.is_empty() {
                        None
                    } else {
                        Some(text_member.html_styled_spans.iter().map(|span| {
                            let mut style = span.style.clone();

                            // ALWAYS use text_member's font if set (movie may have changed it)
                            if !text_member.font.is_empty() {
                                style.font_face = Some(text_member.font.clone());
                            } else if style.font_face.as_ref().map_or(true, |f| f.is_empty()) {
                                style.font_face = Some("Arial".to_string());
                            }

                            // Preserve per-span sizes from styled text, but allow runtime
                            // text_member.font_size to scale the whole style run set.
                            if let Some(scale) = runtime_size_scale {
                                if let Some(span_size) = style.font_size.filter(|s| *s > 0) {
                                    style.font_size = Some(((span_size as f32 * scale).round() as i32).max(1));
                                } else {
                                    style.font_size = Some(text_member.font_size as i32);
                                }
                            } else if style.font_size.map_or(true, |s| s <= 0) {
                                style.font_size = Some(12);
                            }

                            // Override span colors when:
                            // - sprite has explicit foreColor from tween/Lingo (has_fore_color)
                            // - sprite has non-default, non-black color from score data
                            //   (matches non-WebGL2 PFR bitmap rendering where sprite.color always wins)
                            // - span has no color set (fill in missing values)
                            if has_fore_color || (fg_color != ColorRef::PaletteIndex(255) && fg_color != ColorRef::Rgb(0, 0, 0)) || style.color.is_none() {
                                style.color = match &text_fg_color {
                                    ColorRef::Rgb(r, g, b) => {
                                        Some(((*r as u32) << 16) | ((*g as u32) << 8) | (*b as u32))
                                    }
                                    ColorRef::PaletteIndex(idx) => {
                                        match *idx {
                                            0 => Some(0xFFFFFF),
                                            255 => Some(0x000000),
                                            _ => Some(0x000000),
                                        }
                                    }
                                };
                            }

                            // ALWAYS apply text_member's fontStyle (movie may have changed it)
                            if !text_member.font_style.is_empty() {
                                style.bold = text_member.font_style.iter().any(|s| s == "bold");
                                style.italic = text_member.font_style.iter().any(|s| s == "italic");
                                style.underline = text_member.font_style.iter().any(|s| s == "underline");
                            }

                            StyledSpan {
                                text: span.text.clone(),
                                style,
                            }
                        }).collect())
                    };

                    // Build cache key from the final styled spans that will actually be rendered.
                    let styled_spans_ref = styled_spans_with_defaults.as_ref().map(|s| s.as_slice());
                    let cache_key = RenderedTextCacheKey::new_with_styled_spans(
                        member_ref.clone(),
                        text,
                        styled_spans_ref,
                        ink,
                        blend,
                        text_fg_color.clone(),
                        text_bg_color.clone(),
                        false,
                        width,
                        height,
                        &text_member.alignment,
                        effective_word_wrap,
                        &font_name,
                        font_size,
                        font_style,
                        text_member.fixed_line_space,
                        text_member.top_spacing,
                    );

                    TextureSource::RenderedText {
                        cache_key,
                        text: text.clone(),
                        font_name,
                        font_size,
                        font_style,
                        font_id: None,
                        line_spacing: text_member.fixed_line_space,
                        top_spacing: text_member.top_spacing,
                        bottom_spacing: text_member.bottom_spacing,
                        width,
                        height,
                        text_fg_color,
                        text_bg_color,
                        styled_spans: styled_spans_with_defaults,
                        alignment: text_member.alignment.clone(),
                        word_wrap: effective_word_wrap,
                        border: 0,
                        box_drop_shadow: 0,
                    }
                }
                CastMemberType::Field(field_member) => {
                    // Field member: editable text field
                    let text = &field_member.text;

                    // Use sprite_rect dimensions — get_concrete_sprite_rect already
                    // computes the correct size from text_height/rect/borders
                    let width = sprite_rect.width().max(1) as u32;
                    let height = sprite_rect.height().max(1) as u32;

                    // Check if this field has keyboard focus (for cursor rendering)
                    let has_focus = player.keyboard_focus_sprite == channel_num;

                    let mut style = 0u8;
                    let style_lc = field_member.font_style.to_lowercase();
                    if style_lc.contains("bold") {
                        style |= 1;
                    }
                    if style_lc.contains("italic") {
                        style |= 2;
                    }
                    if style_lc.contains("underline") {
                        style |= 4;
                    }

                    // Color priority for fields:
                    // - If sprite has explicit foreColor (tween/Lingo), that overrides member colors
                    // - Otherwise, prefer the member's own STXT/FieldInfo colors
                    let effective_fg = if has_fore_color {
                        fg_color.clone()
                    } else {
                        field_member.fore_color.clone()
                            .unwrap_or_else(|| fg_color.clone())
                    };
                    let effective_bg = if has_back_color {
                        bg_color.clone()
                    } else {
                        field_member.back_color.clone()
                            .unwrap_or_else(|| bg_color.clone())
                    };

                    debug!(
                        "[FIELD] sprite#{} text='{}' ink={} font='{}' fontSize={} fg={:?} bg={:?} member.fg={:?} member.bg={:?} has_fore={} has_back={} -> eff_fg={:?} eff_bg={:?} box_type='{}' editable={} border={} size={}x{}",
                        channel_num, &text[..text.len().min(30)], ink, field_member.font, field_member.font_size,
                        fg_color, bg_color, field_member.fore_color, field_member.back_color,
                        has_fore_color, has_back_color, effective_fg, effective_bg,
                        field_member.box_type, field_member.editable, field_member.border, width, height,
                    );

                    // Include focus state in cache key so cursor state changes invalidate cache
                    let cache_key = RenderedTextCacheKey::new_with_focus(
                        member_ref.clone(),
                        text,
                        ink,
                        blend,
                        effective_fg.clone(),
                        effective_bg.clone(),
                        has_focus,
                        width,
                        height,
                        &field_member.alignment,
                        field_member.word_wrap,
                        &field_member.font,
                        field_member.font_size,
                        if style == 0 { None } else { Some(style) },
                        field_member.fixed_line_space,
                        field_member.top_spacing,
                    );

                    TextureSource::RenderedText {
                        cache_key,
                        text: text.clone(),
                        font_name: field_member.font.clone(),
                        font_size: field_member.font_size,
                        font_style: if style == 0 { None } else { Some(style) },
                        font_id: field_member.font_id,
                        line_spacing: field_member.fixed_line_space,
                        top_spacing: field_member.top_spacing,
                        bottom_spacing: 0,
                        width,
                        height,
                        text_fg_color: effective_fg,
                        text_bg_color: effective_bg,
                        styled_spans: None,
                        alignment: field_member.alignment.clone(),
                        word_wrap: field_member.word_wrap,
                        border: field_member.border,
                        box_drop_shadow: field_member.box_drop_shadow,
                    }
                }
                CastMemberType::Button(button_member) => {
                    // Button member: render to offscreen bitmap with button chrome
                    let width = sprite_rect.width().max(1) as u32;
                    let height = sprite_rect.height().max(1) as u32;
                    TextureSource::ButtonBitmap {
                        width,
                        height,
                        button_type: button_member.button_type.clone(),
                        hilite: button_member.hilite,
                        text: button_member.field.text.clone(),
                        font_name: button_member.field.font.clone(),
                        font_size: button_member.field.font_size,
                        font_id: button_member.field.font_id,
                        alignment: button_member.field.alignment.clone(),
                        ink,
                    }
                }
                CastMemberType::FilmLoop(film_loop) => {
                    // Film loop: render the film loop's score to an offscreen bitmap.
                    // Prefer the info rect (authoritative viewport from Director file)
                    // over the load-time computed initial_rect.
                    let info_rect = IntRect::from(
                        film_loop.info.reg_point.0 as i32,
                        film_loop.info.reg_point.1 as i32,
                        film_loop.info.width as i32,
                        film_loop.info.height as i32,
                    );
                    let initial_rect = if info_rect.width() > 0 && info_rect.height() > 0 {
                        info_rect
                    } else {
                        film_loop.initial_rect.clone()
                    };

                    // Store just the metadata - we'll render after this block ends
                    let width = initial_rect.width().max(1);
                    let height = initial_rect.height().max(1);

                    TextureSource::FilmLoop {
                        initial_rect,
                        width: width as u32,
                        height: height as u32,
                    }
                }
                _ => {
                    // Unhandled member types are silently skipped
                    return;
                }
            }
        };

        // For filmloops, use info rect (already computed above) as the authoritative viewport.
        // When the sprite doesn't have explicit dimensions, use center registration
        // to position the filmloop on the stage.
        let texture_source = match texture_source {
            TextureSource::FilmLoop { initial_rect, width, height } => {
                let w = width as i32;
                let h = height as i32;

                // Only override sprite_rect when the sprite doesn't have explicit
                // dimensions from the score. When the score specifies width/height
                // (e.g. a filmloop with Scale checked, stretched to a specific size),
                // those dimensions must be respected — the GPU will scale the texture
                // from its natural size to the sprite's display rect.
                if sprite_width <= 0 || sprite_height <= 0 {
                    let reg_x = w / 2;
                    let reg_y = h / 2;
                    sprite_rect = IntRect::from(
                        raw_loc.0 as i32 - reg_x,
                        raw_loc.1 as i32 - reg_y,
                        raw_loc.0 as i32 - reg_x + w,
                        raw_loc.1 as i32 - reg_y + h,
                    );
                }

                TextureSource::FilmLoop {
                    initial_rect,
                    width,
                    height,
                }
            }
            other => other,
        };

        // Get bitmap info for palette reference, bit depth, and use_alpha (only used for bitmap sprites)
        let (bitmap_palette_ref, bitmap_bit_depth, bitmap_use_alpha) = match &texture_source {
            TextureSource::Bitmap { image_ref } => {
                match player.bitmap_manager.get_bitmap(*image_ref) {
                    Some(bitmap) => {
                        (bitmap.palette_ref.clone(), bitmap.original_bit_depth, bitmap.use_alpha)
                    }
                    None => (PaletteRef::BuiltIn(get_system_default_palette()), 8, false),
                }
            }
            TextureSource::SolidColor { .. } => {
                // Solid colors use system default palette, no alpha
                (PaletteRef::BuiltIn(get_system_default_palette()), 8, false)
            }
            TextureSource::RenderedText { .. } => {
                // Rendered text uses 32-bit RGBA with alpha for transparent background
                (PaletteRef::BuiltIn(get_system_default_palette()), 32, true)
            }
            TextureSource::FilmLoop { .. } => {
                // Film loops are rendered as 32-bit RGBA with alpha
                (PaletteRef::BuiltIn(get_system_default_palette()), 32, true)
            }
            TextureSource::ButtonBitmap { .. } => {
                // Buttons are rendered as 32-bit RGBA with alpha
                (PaletteRef::BuiltIn(get_system_default_palette()), 32, true)
            }
            TextureSource::ShapeBitmap { .. } | TextureSource::VectorShapeBitmap { .. } => {
                // Shapes are rendered as 32-bit RGBA with alpha
                (PaletteRef::BuiltIn(get_system_default_palette()), 32, true)
            }
        };

        // Resolve colors to RGB for shader uniforms and colorize
        let palettes = player.movie.cast_manager.palettes();
        // Sprite foreColor/backColor palette indices are resolved against the bitmap's palette,
        // so they work together correctly (e.g., index 248/255 in a custom 256-color palette).
        // Director behavior: for 32-bit bitmaps without use_alpha and ink != 0,
        // palette indices are ignored for bgColor and white (255,255,255) is used instead.
        // This matches Canvas2D behavior in drawing.rs lines 780-795.
        let bg_color_rgb = if bitmap_bit_depth == 32 && !bitmap_use_alpha && ink != 0 {
            match &bg_color {
                ColorRef::Rgb(r, g, b) => (*r, *g, *b),
                ColorRef::PaletteIndex(_) => (255, 255, 255),
            }
        } else {
            resolve_color_ref(
                &palettes,
                &bg_color,
                &bitmap_palette_ref,
                bitmap_bit_depth,
            )
        };
        let fg_color_rgb = resolve_color_ref(
            &palettes,
            &fg_color,
            &bitmap_palette_ref,
            bitmap_bit_depth,
        );

        // Build colorize parameters if colorize is active
        // - For 1-bit bitmaps with ink 0 or 36: ALWAYS apply foreColor/bgColor
        //   (Director behavior - 1-bit bitmaps always use sprite colors)
        // - For 2-8 bit indexed bitmaps: only colorize if has_fore_color or has_back_color is set via Lingo
        // - For 32-bit bitmaps with ink 0, 8, 9: general colorize (requires has_fore_color or has_back_color)
        // - For indexed bitmaps with ink 36: foreColor tinting for monochrome-style bitmaps
        // Note: Ink 40 does NOT use foreColor tinting - it just uses color-key transparency
        let is_indexed = bitmap_bit_depth >= 1 && bitmap_bit_depth <= 8;
        let is_ink36_indexed = is_indexed && ink == 36;

        let colorize_params = if bitmap_bit_depth == 1 && (ink == 0 || ink == 36) {
            // 1-bit bitmaps with ink 0: ALWAYS apply foreColor/bgColor
            // This is Director behavior - 1-bit bitmaps always use sprite colors
            Some((
                true, // has_fore is always true for 1-bit bitmaps
                true, // has_back is always true for 1-bit bitmaps
                fg_color_rgb.0,
                fg_color_rgb.1,
                fg_color_rgb.2,
                bg_color_rgb.0,
                bg_color_rgb.1,
                bg_color_rgb.2,
            ))
        } else if is_ink36_indexed {
            // Ink 36 indexed: ALWAYS apply foreColor to foreground pixels (index 255 or black)
            // This matches drawing.rs behavior where fg_color_resolved is always used
            // Note: Ink 40 does NOT apply foreColor tinting - it just skips bgColor pixels
            Some((
                true, // has_fore is always true for ink 36 indexed
                true, // has_back is always true for ink 36 indexed
                fg_color_rgb.0,
                fg_color_rgb.1,
                fg_color_rgb.2,
                bg_color_rgb.0,
                bg_color_rgb.1,
                bg_color_rgb.2,
            ))
        } else if (has_fore_color || has_back_color) && (
            // 32-bit colorize (ink 0, 8, 9)
            (bitmap_bit_depth == 32 && (ink == 0 || ink == 8 || ink == 9)) ||
            (bitmap_bit_depth >= 2 && bitmap_bit_depth <= 8 && (ink == 0))
        ) {
            Some((
                fg_color_rgb != (0, 0, 0) && has_fore_color,
                bg_color_rgb != (255, 255, 255) && has_back_color,
                fg_color_rgb.0,
                fg_color_rgb.1,
                fg_color_rgb.2,
                bg_color_rgb.0,
                bg_color_rgb.1,
                bg_color_rgb.2,
            ))
        } else {
            None
        };

        // Get or create texture based on source type
        // For bitmaps, pass the ink so the texture has the correct matte mask baked in
        // For ink 8, we pass sprite's bgColor for matte computation
        // (this matches Canvas2D's copy_pixels_with_params which uses sprite bgColor)
        // Note: Ink 33 uses shader color-key, not texture matte
        // Colorize is also baked into the texture when has_fore_color or has_back_color is set

        let is_rendered_text = matches!(texture_source, TextureSource::RenderedText { .. });
        let is_button_alpha_matte = matches!(texture_source, TextureSource::ButtonBitmap { ink: i, .. } if i == 36 || i == 8 || i == 7);

        let tex = match texture_source {
            TextureSource::Bitmap { image_ref } => {
                // For inks 7, 8, 9, 40, and 41, pass sprite's bgColor for matte/transparency computation
                // - Ink 7: uses bgColor for matte (not ghost - skips bgColor pixels)
                // - Ink 8: always uses bgColor for matte
                // - Ink 9: uses bgColor for 32-bit bitmaps
                // - Ink 40: uses bgColor for transparency (lighten - skips bgColor pixels)
                // - Ink 41: uses bgColor for 32-bit bitmaps, palette index 0 for indexed
                let sprite_bg_for_matte = if ink == 7 || ink == 8 || ink == 9 || ink == 40 || ink == 41 { Some(bg_color_rgb) } else { None };

                match self.get_or_create_texture(player, &member_ref, image_ref, ink, colorize_params, sprite_bg_for_matte) {
                    Some((tex, _w, _h)) => tex,
                    None => return,
                }
            }
            TextureSource::SolidColor { r, g, b } => {
                self.get_or_create_solid_color_texture(r, g, b)
            }
            TextureSource::RenderedText {
                ref cache_key,
                ref text,
                ref font_name,
                font_size,
                font_style,
                font_id,
                line_spacing,
                top_spacing,
                bottom_spacing,
                width,
                height,
                ref text_fg_color,
                ref text_bg_color,
                ref styled_spans,
                ref alignment,
                word_wrap,
                border,
                box_drop_shadow,
            } => {
                // Check cache first
                if let Some(cached) = self.rendered_text_cache.get(cache_key) {
                    // Update sprite_rect to match cached texture dimensions
                    let actual_h = cached.height as i32;
                    if actual_h != sprite_rect.height() {
                        sprite_rect = IntRect::from(
                            sprite_rect.left, sprite_rect.top,
                            sprite_rect.right, sprite_rect.top + actual_h,
                        );
                    }
                    cached.texture.clone()
                } else {
                    // Render text to a bitmap and upload as texture
                    match self.render_text_to_texture(
                        player,
                        cache_key,
                        text,
                        font_name,
                        font_size,
                        font_style,
                        font_id,
                        line_spacing,
                        top_spacing,
                        bottom_spacing,
                        width,
                        height,
                        ink,
                        blend,
                        text_fg_color,
                        text_bg_color,
                        styled_spans.as_ref(),
                        alignment,
                        word_wrap,
                        border,
                        box_drop_shadow,
                    ) {
                        Some((tex, _actual_w, actual_h)) => {
                            // Update sprite_rect to match actual rendered dimensions
                            if actual_h as i32 != sprite_rect.height() {
                                sprite_rect = IntRect::from(
                                    sprite_rect.left, sprite_rect.top,
                                    sprite_rect.right, sprite_rect.top + actual_h as i32,
                                );
                            }
                            tex
                        }
                        None => return,
                    }
                }
            }
            TextureSource::FilmLoop { initial_rect, width, height } => {
                // Render the film loop's score to an offscreen bitmap
                let mut filmloop_bitmap = Bitmap::new(
                    width as u16,
                    height as u16,
                    32,
                    32,
                    8, // alpha_depth = 8 for transparency support
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                // Enable alpha channel for filmloop transparency
                filmloop_bitmap.use_alpha = true;
                // Clear to fully transparent (RGBA 0,0,0,0) so only rendered sprites are visible
                filmloop_bitmap.data.fill(0);

                // Render the film loop's score to the offscreen bitmap
                render_score_to_bitmap_with_offset(
                    player,
                    &ScoreRef::FilmLoop(member_ref.clone()),
                    &mut filmloop_bitmap,
                    None, // debug_sprite_num
                    IntRect::from_size(0, 0, width as i32, height as i32),
                    (initial_rect.left, initial_rect.top),
                    Some(FilmLoopParentProps {
                        ink: ink as u32,
                        color: fg_color.clone(),
                        bg_color: bg_color.clone(),
                    }),
                );

                // Upload the filmloop bitmap as a texture
                let texture = match self.context.create_texture() {
                    Ok(t) => t,
                    Err(_) => return,
                };
                if self.context.upload_texture_rgba(&texture, width, height, &filmloop_bitmap.data).is_err() {
                    return;
                }
                texture
            }
            TextureSource::ButtonBitmap {
                width, height, button_type, hilite, text,
                font_name, font_size, font_id, alignment, ink: button_ink,
            } => {
                use crate::player::cast_member::ButtonType;

                let w = width as i32;
                let h = height as i32;

                // Create a 32-bit RGBA bitmap for the button
                let mut btn_bitmap = Bitmap::new(
                    width as u16, height as u16, 32, 32, 8,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                btn_bitmap.use_alpha = true;
                btn_bitmap.data.fill(0); // Start fully transparent

                let palettes = player.movie.cast_manager.palettes();

                // Only push buttons invert everything; radio/checkbox keep black text
                let is_push = matches!(button_type, ButtonType::PushButton);
                let (frame_color, fill_color, text_color): ((u8,u8,u8),(u8,u8,u8),(u8,u8,u8)) = if hilite && is_push {
                    // Hilited push button: white frame, white text, black fill
                    ((255,255,255), (0,0,0), (255,255,255))
                } else {
                    // Normal / radio/checkbox: black frame, black text, white fill
                    ((0,0,0), (255,255,255), (0,0,0))
                };

                // For matte-like inks (bgTransparent 36, Matte 8, Not Ghost 7),
                // skip the fill and rely on the alpha channel instead of color-keying.
                // The shader will use alpha-based compositing for these inks.
                let use_alpha_matte = button_ink == 36 || button_ink == 8 || button_ink == 7;

                match button_type {
                    ButtonType::PushButton => {
                        // Fill interior with fill color — skip for matte-like inks
                        // to avoid AA text fringe from color-keying near-white pixels
                        if !use_alpha_matte {
                            btn_bitmap.fill_rect(1, 1, w - 1, h - 1, fill_color, &palettes, 1.0);
                        }
                        // Draw border (top, bottom, left, right)
                        btn_bitmap.fill_rect(2, 0, w - 2, 1, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(2, h - 1, w - 2, h, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(0, 2, 1, h - 2, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(w - 1, 2, w, h - 2, frame_color, &palettes, 1.0);
                        // Corner pixels for rounded corners (1px radius)
                        btn_bitmap.fill_rect(1, 0, 2, 1, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(w - 2, 0, w - 1, 1, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(0, 1, 1, 2, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(w - 1, 1, w, 2, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(1, h - 1, 2, h, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(w - 2, h - 1, w - 1, h, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(0, h - 2, 1, h - 1, frame_color, &palettes, 1.0);
                        btn_bitmap.fill_rect(w - 1, h - 2, w, h - 1, frame_color, &palettes, 1.0);
                        // Set corner pixels fully transparent for rounded look
                        for &(cx, cy) in &[(0i32, 0i32), (w-1, 0), (0, h-1), (w-1, h-1)] {
                            if cx >= 0 && cy >= 0 && cx < w && cy < h {
                                let idx = ((cy * w + cx) * 4) as usize;
                                if idx + 3 < btn_bitmap.data.len() {
                                    btn_bitmap.data[idx + 3] = 0;
                                }
                            }
                        }
                    }
                    ButtonType::CheckBox => {
                        let box_y = 0;
                        // Box outline
                        btn_bitmap.fill_rect(0, box_y, 10, box_y + 1, (0,0,0), &palettes, 1.0);
                        btn_bitmap.fill_rect(0, box_y + 9, 10, box_y + 10, (0,0,0), &palettes, 1.0);
                        btn_bitmap.fill_rect(0, box_y, 1, box_y + 10, (0,0,0), &palettes, 1.0);
                        btn_bitmap.fill_rect(9, box_y, 10, box_y + 10, (0,0,0), &palettes, 1.0);
                        // White fill inside
                        btn_bitmap.fill_rect(1, box_y + 1, 9, box_y + 9, (255,255,255), &palettes, 1.0);
                        // Make the text area opaque
                        for y in 0..h {
                            for x in 12..w {
                                let idx = ((y * w + x) * 4) as usize;
                                if idx + 3 < btn_bitmap.data.len() && btn_bitmap.data[idx + 3] == 0 {
                                    btn_bitmap.data[idx + 3] = 1; // minimal alpha so it's not cut
                                }
                            }
                        }
                        if hilite {
                            for i in 1..9 {
                                btn_bitmap.fill_rect(i, box_y + i, i + 1, box_y + i + 1, (0,0,0), &palettes, 1.0);
                                btn_bitmap.fill_rect(9 - i, box_y + i, 10 - i, box_y + i + 1, (0,0,0), &palettes, 1.0);
                            }
                        }
                    }
                    ButtonType::RadioButton => {
                        let base_x = 0;
                        let base_y = 0;
                        // Outer circle outline (midpoint circle, radius 5, center 5,5)
                        let circle_points: &[(i32, i32)] = &[
                            (4,0),(5,0),(6,0),
                            (3,1),(7,1),
                            (2,2),(8,2),
                            (1,3),(9,3),
                            (0,4),(10,4),
                            (0,5),(10,5),
                            (0,6),(10,6),
                            (1,7),(9,7),
                            (2,8),(8,8),
                            (3,9),(7,9),
                            (4,10),(5,10),(6,10),
                        ];
                        for &(px, py) in circle_points {
                            btn_bitmap.fill_rect(base_x + px, base_y + py, base_x + px + 1, base_y + py + 1, (0,0,0), &palettes, 1.0);
                        }
                        if hilite {
                            // Filled inner circle (radius 2, center 5,5)
                            btn_bitmap.fill_rect(base_x + 4, base_y + 3, base_x + 7, base_y + 4, (0,0,0), &palettes, 1.0);
                            btn_bitmap.fill_rect(base_x + 3, base_y + 4, base_x + 8, base_y + 5, (0,0,0), &palettes, 1.0);
                            btn_bitmap.fill_rect(base_x + 3, base_y + 5, base_x + 8, base_y + 6, (0,0,0), &palettes, 1.0);
                            btn_bitmap.fill_rect(base_x + 3, base_y + 6, base_x + 8, base_y + 7, (0,0,0), &palettes, 1.0);
                            btn_bitmap.fill_rect(base_x + 4, base_y + 7, base_x + 7, base_y + 8, (0,0,0), &palettes, 1.0);
                        }
                    }
                }

                // Draw text label
                let chrome_offset_x = match button_type {
                    ButtonType::CheckBox => 13, // 10px box + 3px gap
                    ButtonType::RadioButton => 14, // 11px circle + 3px gap
                    _ => 0,
                };

                // Load font and draw text
                let font_opt = player.font_manager.get_font_with_cast_and_bitmap(
                    &font_name,
                    &player.movie.cast_manager,
                    &mut player.bitmap_manager,
                    Some(font_size),
                    None,
                );
                let font_loaded = font_opt.or_else(|| {
                    if let Some(id) = font_id {
                        if let Some(fref) = player.font_manager.font_by_id.get(&id).copied() {
                            player.font_manager.fonts.get(&fref).cloned()
                        } else { None }
                    } else { None }
                });

                let text_area_w = w - chrome_offset_x;
                let is_pfr_font = font_loaded.as_ref().map_or(false, |f| f.char_widths.is_some());

                if let (true, Some(font)) = (is_pfr_font, &font_loaded) {
                    if let Some(font_bmp) = player.bitmap_manager.get_bitmap(font.bitmap_ref) {
                        let wrapped_lines = Bitmap::wrap_text_lines(&text, font, text_area_w);
                        let line_h = font.char_height as i32;
                        let total_text_h = (wrapped_lines.len() as i32) * line_h;
                        // Push buttons center text vertically; radio/checkbox start at top
                        let text_y = if is_push {
                            ((h - total_text_h) / 2).max(0)
                        } else {
                            0
                        };

                        let params = CopyPixelsParams {
                            blend: 100,
                            ink: 36, // bg transparent
                            color: ColorRef::Rgb(text_color.0, text_color.1, text_color.2),
                            bg_color: ColorRef::Rgb(255, 255, 255),
                            mask_image: None,
                            is_text_rendering: true,
                            rotation: 0.0,
                            skew: 0.0,
                            sprite: None,
                            original_dst_rect: None,
                        };
                        btn_bitmap.draw_text_wrapped(
                            &text, font, font_bmp,
                            chrome_offset_x, text_y,
                            text_area_w, &alignment,
                            params, &palettes, 0, 0,
                        );
                    }
                } else {
                    // Use native Canvas2D rendering for system fonts (Arial etc.)
                    let native_font_name = if font_name.is_empty() { "Arial".to_string() } else { font_name.clone() };
                    let native_font_size = if font_size > 0 { font_size as i32 } else { 12 };
                    let tc = ((text_color.0 as u32) << 16)
                        | ((text_color.1 as u32) << 8)
                        | (text_color.2 as u32);

                    let span = StyledSpan {
                        text: text.clone(),
                        style: HtmlStyle {
                            font_face: Some(native_font_name),
                            font_size: Some(native_font_size),
                            color: Some(tc),
                            ..HtmlStyle::default()
                        },
                    };

                    let text_alignment = match alignment.to_lowercase().as_str() {
                        "center" | "#center" => TextAlignment::Center,
                        "right" | "#right" => TextAlignment::Right,
                        _ => TextAlignment::Left,
                    };

                    // Push buttons center text vertically; radio/checkbox start at top
                    let text_y = if is_push {
                        ((h - native_font_size) / 2).max(0)
                    } else {
                        0
                    };

                    if let Err(e) = FontMemberHandlers::render_native_text_to_bitmap(
                        &mut btn_bitmap,
                        &[span],
                        chrome_offset_x,
                        text_y,
                        text_area_w,
                        h,
                        text_alignment,
                        text_area_w,
                        true,
                        None,
                        0,
                        0,
                        0,
                    ) {
                        web_sys::console::warn_1(&format!("Native text render error for Button (WebGL2): {:?}", e).into());
                    }
                }

                // Upload button bitmap as texture
                let texture = match self.context.create_texture() {
                    Ok(t) => t,
                    Err(_) => return,
                };
                if self.context.upload_texture_rgba(&texture, width, height, &btn_bitmap.data).is_err() {
                    return;
                }
                texture
            }
            TextureSource::ShapeBitmap {
                width, height, shape_info, fg_color, bg_color,
            } => {
                use crate::director::enums::ShapeType;

                let w = width as i32;
                let h = height as i32;

                let mut shape_bitmap = Bitmap::new(
                    width as u16, height as u16, 32, 32, 8,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                shape_bitmap.use_alpha = true;
                shape_bitmap.data.fill(0);

                let palettes = player.movie.cast_manager.palettes();
                let filled = shape_info.fill_type != 0;
                let thickness = if filled {
                    (shape_info.line_thickness as i32).max(1)
                } else {
                    (shape_info.line_thickness as i32) - 1
                };

                match shape_info.shape_type {
                    ShapeType::Rect => {
                        if filled {
                            shape_bitmap.fill_rect(0, 0, w, h, fg_color, &palettes, 1.0);
                        }
                        if thickness > 0 {
                            for t in 0..thickness {
                                shape_bitmap.stroke_rect(t, t, w - t, h - t, fg_color, &palettes, 1.0);
                            }
                        }
                    }
                    ShapeType::OvalRect => {
                        let radius = 12;
                        if filled {
                            shape_bitmap.fill_round_rect(0, 0, w, h, radius, fg_color, &palettes, 1.0);
                        }
                        if thickness > 0 {
                            shape_bitmap.stroke_round_rect(0, 0, w, h, radius, fg_color, &palettes, 1.0, thickness);
                        }
                    }
                    ShapeType::Oval => {
                        if filled {
                            shape_bitmap.fill_ellipse(0, 0, w, h, fg_color, &palettes, 1.0);
                        }
                        if thickness > 0 {
                            shape_bitmap.stroke_ellipse(0, 0, w, h, fg_color, &palettes, 1.0, thickness);
                        }
                    }
                    ShapeType::Line => {
                        let t = (shape_info.line_thickness as i32).max(1);
                        if shape_info.line_direction == 6 {
                            shape_bitmap.draw_line_thick(0, h - 1, w - 1, 0, fg_color, &palettes, 1.0, t);
                        } else {
                            shape_bitmap.draw_line_thick(0, 0, w - 1, h - 1, fg_color, &palettes, 1.0, t);
                        }
                    }
                    ShapeType::Unknown => {
                        shape_bitmap.fill_rect(0, 0, w, h, fg_color, &palettes, 1.0);
                    }
                }

                let texture = match self.context.create_texture() {
                    Ok(t) => t,
                    Err(_) => return,
                };
                if self.context.upload_texture_rgba(&texture, width, height, &shape_bitmap.data).is_err() {
                    return;
                }
                texture
            }
            TextureSource::VectorShapeBitmap {
                width, height, vector_member,
            } => {
                let w = width as i32;
                let h = height as i32;

                let mut shape_bitmap = Bitmap::new(
                    width as u16, height as u16, 32, 32, 8,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );
                shape_bitmap.use_alpha = true;
                shape_bitmap.data.fill(0);

                let palettes = player.movie.cast_manager.palettes();
                let dst_rect = IntRect::from(0, 0, w, h);
                shape_bitmap.draw_vector_shape(&vector_member, dst_rect, &palettes, 1.0);

                let texture = match self.context.create_texture() {
                    Ok(t) => t,
                    Err(_) => return,
                };
                if self.context.upload_texture_rgba(&texture, width, height, &shape_bitmap.data).is_err() {
                    return;
                }
                texture
            }
        };

        // Select shader based on ink mode
        // For rendered text with ink 36: the text bitmap already has alpha=0 for background
        // and alpha>0 for text pixels (bg fill is suppressed for ink 36). Use Copy (alpha
        // blending) so the shader doesn't color-key text pixels that match bgColor.
        let ink_mode = if is_rendered_text && ink == 36 {
            InkMode::Copy
        } else if is_button_alpha_matte {
            // Button bitmap with matte-like ink: fill was omitted, alpha channel
            // encodes transparency. Use Copy (alpha blending) instead of color-keying.
            InkMode::Copy
        } else {
            InkMode::from_ink_number(ink)
        };
        // use_program returns the effective ink mode (after fallback if needed)
        let effective_ink = self.shader_manager.use_program(&self.context, ink_mode);

        // Get the active program's uniform locations (use effective_ink to ensure consistency)
        let program = match self.shader_manager.get_program(effective_ink) {
            Some(p) => p,
            None => return,
        };
        let u_projection = program.u_projection.clone();
        let u_texture = program.u_texture.clone();
        let u_sprite_rect = program.u_sprite_rect.clone();
        let u_tex_rect = program.u_tex_rect.clone();
        let u_flip = program.u_flip.clone();
        let u_rotation = program.u_rotation.clone();
        let u_rotation_center = program.u_rotation_center.clone();
        let u_blend = program.u_blend.clone();
        let u_bg_color = program.u_bg_color.clone();
        let u_color_tolerance = program.u_color_tolerance.clone();

        // Set blend mode based on effective ink (to match the shader being used)
        match effective_ink {
            InkMode::AddPin => {
                self.context.set_blend_additive();
            }
            InkMode::SubPin => {
                // SubPin uses reverse subtract: result = dst - src
                self.context.set_blend_subtractive();
            }
            InkMode::Darken => {
                // Darken uses standard alpha blending
                // The shader multiplies src by bgColor, then we alpha-blend the result
                self.context.set_blend_alpha();
            }
            InkMode::Lighten => {
                // Lighten in Director: skip bgColor pixels, blend others normally
                // This is NOT max(src, dst) - it's just normal alpha blending
                // with transparency for bgColor pixels (baked into texture alpha)
                self.context.set_blend_alpha();
            }
            _ => self.context.set_blend_alpha(),
        }

        // Now we can get gl reference and set uniforms
        let gl = self.context.gl();

        // Set projection matrix
        if let Some(ref loc) = u_projection {
            gl.uniform_matrix4fv_with_f32_array(Some(loc), false, &self.projection_matrix);
        }

        // Bind texture
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&tex));
        if let Some(ref loc) = u_texture {
            gl.uniform1i(Some(loc), 0);
        }

        // Set sprite rect uniform (x, y, width, height)
        if let Some(ref loc) = u_sprite_rect {
            gl.uniform4f(
                Some(loc),
                sprite_rect.left as f32,
                sprite_rect.top as f32,
                sprite_rect.width() as f32,
                sprite_rect.height() as f32,
            );
        }

        // Set texture rect (full texture for now)
        if let Some(ref loc) = u_tex_rect {
            gl.uniform4f(Some(loc), 0.0, 0.0, 1.0, 1.0);
        }

        // Handle skew
        // rotation=180 alone: (x,y) -> (-x,-y) = upside down
        // rotation=180 + skew=180: (x,-y) -> (-x,y) = left-right mirror
        let has_skew_flip = is_skew_flip(skew);

        // Set flip (texture coordinates only, for sprite.flip_h and sprite.flip_v)
        if let Some(ref loc) = u_flip {
            gl.uniform2f(
                Some(loc),
                if flip_h { 1.0 } else { 0.0 },
                if flip_v { 1.0 } else { 0.0 },
            );
        }

        // Set skew flip (vertex space y-negation before rotation)
        if let Some(ref loc) = program.u_skew_flip {
            gl.uniform1f(Some(loc), if has_skew_flip { 1.0 } else { 0.0 });
        }

        // Set rotation (convert degrees to radians)
        // Note: drawing.rs negates for inverse rotation (dst->src mapping),
        // but WebGL does forward rotation (vertex transformation), so no negation needed
        if let Some(ref loc) = u_rotation {
            gl.uniform1f(Some(loc), (rotation as f32).to_radians());
        }

        // Set rotation center (sprite's registration point: loc_h, loc_v)
        if let Some(ref loc) = u_rotation_center {
            gl.uniform2f(Some(loc), raw_loc.0 as f32, raw_loc.1 as f32);
        }

        // Set blend (0-100 -> 0.0-1.0)
        if let Some(ref loc) = u_blend {
            gl.uniform1f(Some(loc), blend as f32 / 100.0);
        }

        // Set background color for ink modes that need it:
        // - BackgroundTransparent: for color-key transparency
        // - NotGhost: for color-key transparency
        // - Darken: for src * bg_color multiplication
        // - AddPin: for color-key transparency (ALL bgColor pixels transparent)
        // - SubPin: for color-key transparency (ALL bgColor pixels transparent)
        // Note: Matte (ink 8) uses flood-fill matte in texture alpha, not color-key
        // (using the already-resolved bg_color_rgb from earlier)
        if effective_ink == InkMode::BackgroundTransparent
            || effective_ink == InkMode::NotGhost
            || effective_ink == InkMode::Darken
            || effective_ink == InkMode::AddPin
            || effective_ink == InkMode::SubPin
            || effective_ink == InkMode::Lighten
        {
            if let Some(ref loc) = u_bg_color {
                gl.uniform4f(
                    Some(loc),
                    bg_color_rgb.0 as f32 / 255.0,
                    bg_color_rgb.1 as f32 / 255.0,
                    bg_color_rgb.2 as f32 / 255.0,
                    1.0,
                );
            }
            if let Some(ref loc) = u_color_tolerance {
                // For 1-bit bitmaps: transparency is baked into texture alpha via is_1bit_transparent
                // For indexed ink 40 (2-8 bit): transparency is baked via RGB comparison with sprite's bgColor
                // In both cases, disable shader color-key by setting tolerance to 0.
                // For 16-bit and 32-bit: use small tolerance for floating-point RGB comparison.
                let is_indexed_ink40 = bitmap_bit_depth >= 2 && bitmap_bit_depth <= 8 && ink == 40;
                let tolerance = if bitmap_bit_depth == 1 || is_indexed_ink40 { 0.0 } else { 0.01 };
                gl.uniform1f(Some(loc), tolerance);
            }
        }

        // Draw the quad
        self.quad.draw(gl);

        // Unbind texture
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);

        // Reset blend equation if we used SubPin (which changes the blend equation to REVERSE_SUBTRACT)
        // Note: Lighten now uses normal alpha blend, not MAX
        if effective_ink == InkMode::SubPin {
            self.context.reset_blend_equation();
        }
    }

    /// Convert bitmap data to RGBA format for GPU texture upload
    ///
    /// For bitmaps without explicit matte, computes a flood-fill matte mask from edges
    /// using the bitmap's intrinsic background color (palette index 0 for indexed bitmaps,
    /// white for 32-bit). This matches Director's behavior where the matte is computed
    /// once per bitmap based on its own palette, not the sprite's bgColor.
    ///
    /// For indexed bitmaps, we compare palette INDICES (not RGB values) to match
    /// how Canvas2D's create_matte works. This ensures that only pixels with exactly
    /// palette index 0 are considered background, not pixels that happen to have the
    /// same RGB color at a different palette index.
    ///
    /// Colorize parameters: (has_fore, has_back, fg_r, fg_g, fg_b, bg_r, bg_g, bg_b)
    /// sprite_bg_color: The sprite's bgColor, used for ink 8 matte computation on indexed bitmaps
    fn bitmap_to_rgba(
        bitmap: &Bitmap,
        palettes: &crate::player::bitmap::palette_map::PaletteMap,
        ink: i32,
        colorize: Option<(bool, bool, u8, u8, u8, u8, u8, u8)>,
        sprite_bg_color: Option<(u8, u8, u8)>,
    ) -> Vec<u8> {
        let width = bitmap.width as usize;
        let height = bitmap.height as usize;
        let mut rgba_data = Vec::with_capacity(width * height * 4);

        // Extract colorize info if present
        let (has_fore, has_back, fg_rgb, bg_rgb) = match colorize {
            Some((has_f, has_b, fg_r, fg_g, fg_b, bg_r, bg_g, bg_b)) => {
                (has_f, has_b, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b))
            }
            None => (false, false, (0, 0, 0), (255, 255, 255)),
        };

        // Check if colorize should be applied
        // Matches drawing.rs allows_colorize() function (lines 750-762)
        // EXCEPTION: ink 36 indexed has special foreColor tinting (drawing.rs lines 1188-1196)
        //   For ink 36 indexed, foreColor is ALWAYS applied to foreground pixels (index 255 or black),
        //   regardless of has_fore_color flag - this is Director behavior.
        // Note: Ink 40 does NOT use colorization - it only uses color-key transparency
        let allow_colorize = match (bitmap.original_bit_depth, ink as u32) {
            (32, 0) => true,                    // 32-bit ink 0: grayscale remap
            (32, 8) | (32, 9) => true,          // 32-bit ink 8/9: foreColor only
            (d, 0) if d <= 8 => true,           // indexed ink 0: palette index interpolation
            (d, 8) | (d, 9) if d <= 8 => true,  // indexed ink 8/9: foreColor only
            (d, 36) if d <= 8 => true,          // indexed ink 36: foreColor tinting for index 255/black (ALWAYS)
            _ => false,
        };

        // Special flag for ink 36 indexed foreColor tinting
        // For ink 36 indexed, ALWAYS tint foreground pixels with foreColor (has_fore is always true)
        let ink36_indexed_tint = bitmap.original_bit_depth <= 8 && ink == 36;

        // Check if backColor should be used (for interpolation)
        let use_back_color = match (bitmap.original_bit_depth, ink as u32) {
            (32, 0) => true,
            (d, 0) if d <= 8 => true,
            _ => false,
        };

        // Compute flood-fill matte from edges for bitmaps without explicit matte.
        //
        // Director behavior for matte:
        // - Ink 0 (Copy): use matte ONLY when trim_white_space is true
        // - Ink 8 (Matte): ALWAYS use matte (this is the "matte" ink, its whole purpose is transparency)
        //
        // For ink 8, the pre-computed bitmap.matte should be used (created by rendering.rs).
        // If bitmap.matte doesn't exist, we compute it here.
        //
        // Bit depth support:
        // - Indexed bitmaps (depth <= 8): use matte for ink 0 (when trim_white_space) and ink 8 (always)
        // - 16-bit bitmaps: use matte for ink 0 only (when trim_white_space)
        // - 32-bit bitmaps with use_alpha=false: use flood-fill matte for ink 0/8
        // - 32-bit bitmaps with use_alpha=true: use embedded alpha channel (no matte)
        //
        // IMPORTANT: Use original_bit_depth for these checks because bit_depth can change
        // during execution (e.g., 4-bit stored as 8-bit), but original_bit_depth reliably
        // indicates when palette colors should be applied.
        let is_indexed = bitmap.original_bit_depth > 0 && bitmap.original_bit_depth <= 8;
        let is_16bit = bitmap.original_bit_depth == 16;
        let is_32bit = bitmap.original_bit_depth == 32;

        let is_grayscale = matches!(&bitmap.palette_ref, crate::player::bitmap::bitmap::PaletteRef::BuiltIn(
            crate::player::bitmap::bitmap::BuiltInPalette::GrayScale
        ));

        // Determine when to use matte based on ink mode:
        //
        // Matte mask usage for indexed bitmaps in score sprite rendering:
        // - Ink 0: matte when trim_white_space is true
        // - Ink 7 (Not Ghost): use matte for indexed bitmaps (flood-fill, not color-key)
        // - Ink 8 (Matte): ALWAYS use matte (flood-fill transparency)
        // - Ink 9 (Mask): use matte for 32-bit bitmaps (embedded alpha or grayscale-as-alpha)
        // - Ink 41 (Darken): use matte for indexed and 32-bit bitmaps
        //
        // Important: some inks use color-key transparency in the shader instead of matte:
        // - Ink 33 (Add Pin): color-key comparison in shader
        // - Ink 36 (BgTransparent): color-key comparison in shader
        let should_use_matte_ink0 = bitmap.trim_white_space && ink == 0;
        // Ink 7 (Not Ghost) uses matte for indexed and 16-bit bitmaps (flood-fill from edges)
        // This matches Canvas2D's should_matte_sprite which includes ink 7
        let should_use_matte_ink7 = ink == 7 && (is_indexed || is_16bit);
        // Ink 8 (Matte) ALWAYS uses matte for indexed, 16-bit, and 32-bit bitmaps in score rendering
        // The flood-fill matte makes edge-connected background pixels transparent
        // while keeping interior pixels (even if same color) opaque
        // For 32-bit, only applies when use_alpha is false (otherwise embedded alpha is used)
        let should_use_matte_ink8 = ink == 8 && (is_indexed || is_16bit || (is_32bit && !bitmap.use_alpha));
        // Ink 9 (Mask) uses matte for 32-bit bitmaps (embedded alpha like ink 8)
        let should_use_matte_ink9 = ink == 9 && (is_32bit && !bitmap.use_alpha);
        // Ink 41 (Darken) uses matte for indexed, 16-bit, and 32-bit bitmaps
        // Background pixels should be transparent so they don't darken the destination
        // For indexed: use palette index 0
        // For 16-bit/32-bit: use RGB comparison with bgColor (typically white)
        let should_use_matte_ink41 = ink == 41 && (is_indexed || is_16bit || (is_32bit && !bitmap.use_alpha));
        // Ink 33 (Add Pin) uses COLOR-KEY transparency (ALL bgColor pixels transparent)
        // NOT flood-fill matte. See drawing.rs lines 160-163: if src == bg_color { dst }
        // Color-key comparison is handled in the shader, not texture matte.
        // For 16-bit and 32-bit, the shader compares pixel RGB with bgColor uniform.
        let should_use_colorkey_ink33 = ink == 33 && (is_indexed || is_16bit || (is_32bit && !bitmap.use_alpha));
        // Ink 35 (Sub Pin) uses COLOR-KEY transparency (ALL bgColor pixels transparent)
        // Same behavior as ink 33 but with subtractive blending
        let should_use_colorkey_ink35 = ink == 35 && (is_indexed || is_16bit || (is_32bit && !bitmap.use_alpha));
        // Ink 36 uses color-key transparency for indexed (2-8 bit), 16-bit, and 32-bit bitmaps (not flood-fill)
        // EXCEPTION: 1-bit bitmaps already have alpha baked into texture via is_1bit_transparent,
        // so they don't need shader color-key (which would incorrectly discard colorized pixels)
        // See drawing.rs lines 1210-1231 for 16-bit ink 36 handling
        // See drawing.rs lines 1421-1437 for 32-bit ink 36 handling
        let is_indexed_not_1bit = bitmap.original_bit_depth >= 2 && bitmap.original_bit_depth <= 8;
        let should_use_colorkey_ink36 = ink == 36 && (is_indexed_not_1bit || is_16bit || (is_32bit && !bitmap.use_alpha));
        // Ink 40 (Lighten) uses color-key transparency: skip bgColor pixels
        // For indexed bitmaps: use palette index comparison in texture (baked alpha)
        // For 16-bit and 32-bit: use RGB color-key in shader
        let should_use_colorkey_ink40 = ink == 40 && (is_16bit || (is_32bit && !bitmap.use_alpha));
        // For indexed ink 40, we bake transparency based on palette index
        let is_ink40_indexed_transparent = ink == 40 && is_indexed_not_1bit;
        // Total: when to use matte (either pre-computed or on-the-fly)
        // Note: ink 33 uses color-key in shader, NOT matte
        let should_use_matte = should_use_matte_ink0 || should_use_matte_ink7 || should_use_matte_ink8 || should_use_matte_ink9 || should_use_matte_ink41;

        // For ink 7, 8, 9, and 41, ALWAYS compute matte (flood-fill from edges)
        // This matches score rendering behavior where these inks use matte
        // The matte makes edge-connected background transparent while keeping interior pixels opaque
        //
        // For ink 0, only compute matte when trim_white_space is true
        // Note: Ink 33 does NOT use flood-fill matte - it uses shader color-key
        let is_matte_bitmap = bitmap.trim_white_space;
        let ink_7_needs_matte = ink == 7 && (is_indexed || is_16bit);
        let ink_8_needs_matte = ink == 8 && (is_indexed || is_16bit || (is_32bit && !bitmap.use_alpha));
        let ink_9_needs_matte = ink == 9 && (is_32bit && !bitmap.use_alpha);
        let ink_41_needs_matte = ink == 41 && (is_indexed || is_16bit || (is_32bit && !bitmap.use_alpha));

        let needs_computed_matte = (bitmap.matte.is_none() || ink_7_needs_matte || ink_8_needs_matte || ink_9_needs_matte || ink_41_needs_matte)
            && should_use_matte
            && width > 0
            && height > 0
            && (
                // Indexed bitmaps ink 0: matte only when trim_white_space
                (is_indexed && is_matte_bitmap && ink == 0)
                // Indexed bitmaps ink 7: ALWAYS use matte (flood-fill, not color-key)
                || (is_indexed && ink == 7)
                // Indexed bitmaps ink 8: ALWAYS use matte (flood-fill transparency)
                || (is_indexed && ink == 8)
                // Indexed bitmaps ink 41: ALWAYS use matte (background shouldn't darken)
                || (is_indexed && ink == 41)
                // 16-bit bitmaps ink 0: matte only when trim_white_space
                || (is_16bit && should_use_matte_ink0)
                // 16-bit bitmaps ink 7: ALWAYS use matte (flood-fill, not color-key)
                || (is_16bit && ink == 7)
                // 16-bit bitmaps ink 8: ALWAYS use matte (flood-fill transparency)
                || (is_16bit && ink == 8)
                // 16-bit bitmaps ink 41: ALWAYS use matte (background shouldn't darken)
                || (is_16bit && ink == 41)
                // 32-bit bitmaps WITHOUT use_alpha ink 0: matte only when trim_white_space
                || (is_32bit && !bitmap.use_alpha && is_matte_bitmap && ink == 0)
                // 32-bit bitmaps WITHOUT use_alpha ink 8: ALWAYS use matte (flood-fill transparency)
                || (is_32bit && !bitmap.use_alpha && ink == 8)
                // 32-bit bitmaps WITHOUT use_alpha ink 9: ALWAYS use matte (embedded alpha)
                || (is_32bit && !bitmap.use_alpha && ink == 9)
                // 32-bit bitmaps WITHOUT use_alpha ink 41: ALWAYS use matte with bgColor
                || (is_32bit && !bitmap.use_alpha && ink == 41)
            );

        // Compute matte mask
        let computed_matte: Option<Vec<bool>> = if needs_computed_matte {
            if is_indexed {
                // For indexed bitmaps:
                // - Ink 7 (Not Ghost): use RGB comparison with sprite's bgColor
                //   This ink makes edge-connected bgColor pixels transparent (skips bgColor pixels)
                // - Ink 8 (Matte): use RGB comparison with sprite's bgColor
                //   This ink makes edge-connected bgColor pixels transparent via flood-fill
                // - Ink 41 (Darken): use palette index 0 comparison (standard matte)
                //   The bgColor is used for color multiplication in the shader, not for matte
                // - Other inks: use palette index 0 comparison (like bitmap.matte / create_matte())
                //   This matches Director's standard behavior where index 0 is background.
                // Note: Ink 33 uses shader color-key, NOT flood-fill matte
                if ink == 7 || ink == 8 {
                    // Ink 7 and 8: use RGB comparison with sprite's bgColor (matches Canvas2D behavior)
                    // Canvas2D uses sprite's bgColor as the background color for flood-fill matte,
                    // NOT the edge pixel color. This is critical for bitmaps with borders.
                    // If sprite_bg_color is not provided, fall back to edge pixel color.
                    let bg_color_for_matte = sprite_bg_color.unwrap_or_else(|| {
                        bitmap.get_pixel_color(palettes, 0, 0)
                    });

                    Some(Self::compute_edge_matte_mask_rgb(bitmap, palettes, bg_color_for_matte, width, height))
                } else {
                    // Ink 41 and other inks: use palette index comparison (background = index 0)
                    Some(Self::compute_edge_matte_mask_indexed(bitmap, width, height))
                }
            } else if is_32bit {
                // For 32-bit bitmaps without use_alpha:
                // - Ink 8: use sprite's bgColor for matte (matches Canvas2D behavior)
                // - Ink 9: use sprite's bgColor for matte (mask ink uses bgColor for transparency)
                // - Ink 41: use sprite's bgColor for matte (typically white for transparency)
                // - Other inks: use edge color (matching drawing.rs lines 891-897)
                let bg_color_for_matte = if ink == 8 || ink == 9 || ink == 41 {
                    sprite_bg_color.unwrap_or_else(|| bitmap.get_pixel_color(palettes, 0, 0))
                } else {
                    bitmap.get_pixel_color(palettes, 0, 0)
                };
                Some(Self::compute_edge_matte_mask_rgb(bitmap, palettes, bg_color_for_matte, width, height))
            } else if is_16bit {
                // For 16-bit bitmaps:
                // - Ink 7, 8, 41: use sprite's bgColor for matte (matches Canvas2D behavior)
                // - Ink 0: use white as background (default for trim_white_space)
                let bg_color_for_matte = if ink == 7 || ink == 8 || ink == 41 {
                    sprite_bg_color.unwrap_or((255u8, 255u8, 255u8))
                } else {
                    (255u8, 255u8, 255u8)
                };
                Some(Self::compute_edge_matte_mask_rgb(bitmap, palettes, bg_color_for_matte, width, height))
            } else {
                None
            }
        } else {
            None
        };

        let mut opaque_count = 0usize;
        let mut transparent_count = 0usize;

        // Check if this is a 32-bit bitmap with embedded alpha (use_alpha=true)
        // These should use embedded alpha, ignoring any matte that may have been computed
        // Use original_bit_depth since bit_depth can change during execution
        let use_embedded_alpha = bitmap.original_bit_depth == 32 && bitmap.use_alpha;

        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = bitmap.get_pixel_color(palettes, x as u16, y as u16);

                // Get alpha:
                // 1. For 32-bit bitmaps with use_alpha=true, ALWAYS use embedded alpha
                // 2. For ink 36 (BgTransparent) with indexed bitmaps: color-key (index 0 = transparent)
                // 3. For ink 8 (Matte) and other flood-fill inks: use matte
                // 4. For ink 0 with trim_white_space: use matte
                // 5. For other 32-bit bitmaps (use_alpha=false, no matte): use embedded alpha
                // 6. For other bitmaps: fully opaque
                //
                // Special handling for 1-bit bitmaps (all inks except ink 0):
                // Index 0 (bit=0) = background 
                // Index 1/255 (bit=1) = foreground
                let is_1bit_transparent = if bitmap.original_bit_depth == 1 && ink != 0 {
                    let color_ref = bitmap.get_pixel_color_ref(x as u16, y as u16);
                    if let ColorRef::PaletteIndex(i) = color_ref {
                        i == 0 // Index 0 = background = transparent
                    } else {
                        false
                    }
                } else {
                    false
                };

                // Special handling for ink 40 indexed bitmaps (2-8 bit):
                // Compare RGB against sprite's bgColor (like drawing.rs lines 203-209)
                // This matches Canvas2D: if src == bg_color, skip (transparent)
                let is_ink40_indexed_bg = if is_ink40_indexed_transparent {
                    if let Some(bg_color) = sprite_bg_color {
                        // Compare this pixel's RGB against sprite's bgColor
                        (r, g, b) == bg_color
                    } else {
                        false
                    }
                } else {
                    false
                };

                let a = if is_1bit_transparent || is_ink40_indexed_bg {
                    // 1-bit or ink 40 indexed background pixel
                    0
                } else if ink == 0 && use_embedded_alpha {
                    // For 32-bit bitmaps with ink 0 (Copy) and use_alpha=true: use embedded alpha
                    let index = (y * width + x) * 4;
                    if index + 3 < bitmap.data.len() {
                        bitmap.data[index + 3]
                    } else {
                        255
                    }
                } else if ink == 0 {
                    // For ink 0 (Copy) without use_alpha: always fully opaque
                    // This applies to indexed, 16-bit, and 32-bit with use_alpha=false
                    255
                } else if use_embedded_alpha {
                    // 32-bit with use_alpha (non-ink-0): use embedded alpha directly, ignore any matte
                    let index = (y * width + x) * 4;
                    if index + 3 < bitmap.data.len() {
                        bitmap.data[index + 3]
                    } else {
                        255
                    }
                } else if should_use_colorkey_ink33 || should_use_colorkey_ink35 || should_use_colorkey_ink36 {
                    // Ink 33/35/36 color-key transparency is handled by shader
                    // The shader compares pixel RGB with bgColor uniform
                    // All pixels are uploaded as opaque, shader discards matching pixels
                    // Works for indexed (2-8 bit), 16-bit, and 32-bit (without use_alpha) bitmaps
                    // Note: 1-bit bitmaps are excluded - they use is_1bit_transparent for alpha instead
                    255
                } else if should_use_matte {
                    // Use matte for inks 0 and 8 when trim_white_space is true
                    if let Some(ref computed) = computed_matte {
                        // Use computed flood-fill matte
                        if computed[y * width + x] { 255 } else { 0 }
                    } else if let Some(ref matte) = bitmap.matte {
                        // Use pre-computed bitmap.matte
                        if matte.get_bit(x as u16, y as u16) { 255 } else { 0 }
                    } else {
                        255 // No matte available, fully opaque
                    }
                } else if bitmap.original_bit_depth == 32 && !bitmap.use_alpha {
                    // For 32-bit bitmaps with use_alpha=false and ink 0 (Copy):
                    // Pixels are fully opaque (matching drawing.rs lines 1366-1367)
                    // The embedded alpha is ignored when use_alpha is false
                    255
                } else {
                    // For other bitmaps without matte conditions, all pixels are opaque
                    255
                };

                // Apply colorize if enabled (only when has_fore_color or has_back_color is explicitly set)
                // Matching drawing.rs lines 1304-1326 for indexed and lines 1283-1301 for 32-bit
                let (final_r, final_g, final_b) = if allow_colorize && (has_fore || has_back) {
                    match bitmap.original_bit_depth {
                        // ---------- 32-BIT ----------
                        32 => {
                            // Treat source as grayscale intensity
                            let gray = ((r as u16 + g as u16 + b as u16) / 3) as u8;

                            if has_fore && has_back && use_back_color {
                                // Interpolate between fg and bg based on gray
                                let t = gray as f32 / 255.0;
                                (
                                    ((1.0 - t) * fg_rgb.0 as f32 + t * bg_rgb.0 as f32) as u8,
                                    ((1.0 - t) * fg_rgb.1 as f32 + t * bg_rgb.1 as f32) as u8,
                                    ((1.0 - t) * fg_rgb.2 as f32 + t * bg_rgb.2 as f32) as u8,
                                )
                            } else if has_fore && gray <= 1 {
                                // Replace near-black with fg color
                                fg_rgb
                            } else {
                                (r, g, b)
                            }
                        }

                        // ---------- 1-BIT BITMAPS ----------
                        1 => {
                            // For 1-bit bitmaps with ink 0 or 36:
                            // - foreground (index 255, bit=1)
                            // - background (index 0, bit=0) 
                            // Note: 1-bit bitmaps store indices as 0 and 255 (not 0 and 1)
                            let color_ref = bitmap.get_pixel_color_ref(x as u16, y as u16);
                            if let ColorRef::PaletteIndex(i) = color_ref {
                                if i != 0 {
                                    // Foreground bit (index 255)
                                    if has_fore {
                                        fg_rgb
                                    } else {
                                        (r, g, b)
                                    }
                                } else {
                                    // Background bit (index 0)
                                    if has_back && use_back_color {
                                        bg_rgb
                                    } else {
                                        (r, g, b)
                                    }
                                }
                            } else {
                                (r, g, b)
                            }
                        }

                        // ---------- INDEXED (2-8 bit) ----------
                        _ => {
                            // Get palette index
                            let color_ref = bitmap.get_pixel_color_ref(x as u16, y as u16);

                            if let ColorRef::PaletteIndex(i) = color_ref {
                                // Ink 36 special case: foreColor tinting for monochrome-style bitmaps
                                // See drawing.rs lines 1172-1176: index 255 or black pixels get foreColor
                                // Note: Ink 40 does NOT apply foreColor tinting - it just uses color-key transparency
                                if ink36_indexed_tint {
                                    if i == 255 || (r == 0 && g == 0 && b == 0) {
                                        fg_rgb
                                    } else {
                                        (r, g, b)
                                    }
                                } else {
                                    // General indexed colorize
                                    let max = (1u16 << bitmap.original_bit_depth) - 1;
                                    let t = i as f32 / max as f32;

                                    if (has_fore || has_back) && use_back_color {
                                        // Interpolate between fg and bg based on palette index
                                        (
                                            ((1.0 - t) * fg_rgb.0 as f32 + t * bg_rgb.0 as f32) as u8,
                                            ((1.0 - t) * fg_rgb.1 as f32 + t * bg_rgb.1 as f32) as u8,
                                            ((1.0 - t) * fg_rgb.2 as f32 + t * bg_rgb.2 as f32) as u8,
                                        )
                                    } else if has_fore && i == 0 {
                                        // Replace palette index 0 with fg color
                                        fg_rgb
                                    } else {
                                        (r, g, b)
                                    }
                                }
                            } else {
                                (r, g, b)
                            }
                        }
                    }
                } else {
                    (r, g, b)
                };

                if a > 0 {
                    opaque_count += 1;
                } else {
                    transparent_count += 1;
                }

                rgba_data.push(final_r);
                rgba_data.push(final_g);
                rgba_data.push(final_b);
                rgba_data.push(a);
            }
        }

        // Unused variables to suppress warnings
        let _ = is_grayscale;
        let _ = opaque_count;
        let _ = transparent_count;

        rgba_data
    }

    /// Compute a matte mask using flood-fill from all edges for INDEXED bitmaps
    ///
    /// This compares PALETTE INDICES (not RGB values), matching how Canvas2D's
    /// create_matte works. Background is palette index 0.
    fn compute_edge_matte_mask_indexed(
        bitmap: &Bitmap,
        width: usize,
        height: usize,
    ) -> Vec<bool> {
        // Start with all pixels visible (true)
        let mut matte = vec![true; width * height];

        // Track which pixels we've visited during flood fill
        let mut visited = vec![false; width * height];

        // Stack for flood fill (use iterative approach to avoid stack overflow)
        let mut stack: Vec<(usize, usize)> = Vec::new();

        // Background color is palette index 0
        let bg_color_ref = ColorRef::PaletteIndex(0);

        // Helper to check if a pixel matches the background color (by palette index)
        let matches_bg = |x: usize, y: usize| -> bool {
            let pixel_ref = bitmap.get_pixel_color_ref(x as u16, y as u16);
            pixel_ref == bg_color_ref
        };

        // Seed the flood fill from all edge pixels that match bg color
        // Top and bottom edges
        for x in 0..width {
            if matches_bg(x, 0) {
                stack.push((x, 0));
            }
            if height > 1 && matches_bg(x, height - 1) {
                stack.push((x, height - 1));
            }
        }
        // Left and right edges (skip corners, already added)
        for y in 1..height.saturating_sub(1) {
            if matches_bg(0, y) {
                stack.push((0, y));
            }
            if width > 1 && matches_bg(width - 1, y) {
                stack.push((width - 1, y));
            }
        }

        // Flood fill - mark connected bg-color pixels as transparent
        while let Some((x, y)) = stack.pop() {
            let idx = y * width + x;

            // Skip if already visited
            if visited[idx] {
                continue;
            }
            visited[idx] = true;

            // Check if this pixel matches bg color (by palette index)
            if !matches_bg(x, y) {
                continue;
            }

            // Mark as transparent
            matte[idx] = false;

            // Add neighbors to stack (4-connected)
            if x > 0 {
                stack.push((x - 1, y));
            }
            if x + 1 < width {
                stack.push((x + 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if y + 1 < height {
                stack.push((x, y + 1));
            }
        }

        matte
    }

    /// Compute a matte mask using flood-fill from all edges for non-indexed bitmaps
    ///
    /// This compares RGB values, used for 16-bit bitmaps with ink 0 (Copy).
    fn compute_edge_matte_mask_rgb(
        bitmap: &Bitmap,
        palettes: &crate::player::bitmap::palette_map::PaletteMap,
        bg_color_rgb: (u8, u8, u8),
        width: usize,
        height: usize,
    ) -> Vec<bool> {
        // Start with all pixels visible (true)
        let mut matte = vec![true; width * height];

        // Track which pixels we've visited during flood fill
        let mut visited = vec![false; width * height];

        // Stack for flood fill (use iterative approach to avoid stack overflow)
        let mut stack: Vec<(usize, usize)> = Vec::new();

        // Helper to check if a pixel matches the background color (by RGB)
        let matches_bg = |x: usize, y: usize| -> bool {
            let (r, g, b) = bitmap.get_pixel_color(palettes, x as u16, y as u16);
            r == bg_color_rgb.0 && g == bg_color_rgb.1 && b == bg_color_rgb.2
        };

        // Seed the flood fill from all edge pixels that match bg color
        // Top and bottom edges
        for x in 0..width {
            if matches_bg(x, 0) {
                stack.push((x, 0));
            }
            if height > 1 && matches_bg(x, height - 1) {
                stack.push((x, height - 1));
            }
        }
        // Left and right edges (skip corners, already added)
        for y in 1..height.saturating_sub(1) {
            if matches_bg(0, y) {
                stack.push((0, y));
            }
            if width > 1 && matches_bg(width - 1, y) {
                stack.push((width - 1, y));
            }
        }

        // Flood fill - mark connected bg-color pixels as transparent
        while let Some((x, y)) = stack.pop() {
            let idx = y * width + x;

            // Skip if already visited
            if visited[idx] {
                continue;
            }
            visited[idx] = true;

            // Check if this pixel matches bg color (by RGB)
            if !matches_bg(x, y) {
                continue;
            }

            // Mark as transparent
            matte[idx] = false;

            // Add neighbors to stack (4-connected)
            if x > 0 {
                stack.push((x - 1, y));
            }
            if x + 1 < width {
                stack.push((x + 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if y + 1 < height {
                stack.push((x, y + 1));
            }
        }

        matte
    }

    /// Check if ink mode requires matte computation
    /// Matches Canvas2D's should_matte_sprite function
    fn should_matte_sprite(ink: i32) -> bool {
        ink == 36 || ink == 33 || ink == 41 || ink == 8 || ink == 7
    }

    /// Get or create a texture for a bitmap member
    ///
    /// The ink is included in the cache key because 32-bit bitmaps with ink 8 (Matte)
    /// need matte computation while other inks use the embedded alpha.
    ///
    /// Colorize parameters are also included in the cache key because Director's colorize
    /// feature remaps palette indices to interpolate between fore and back colors.
    fn get_or_create_texture(
        &mut self,
        player: &mut DirPlayer,
        member_ref: &CastMemberRef,
        image_ref: u32,
        ink: i32,
        colorize: Option<(bool, bool, u8, u8, u8, u8, u8, u8)>,
        sprite_bg_color: Option<(u8, u8, u8)>,
    ) -> Option<(web_sys::WebGlTexture, u32, u32)> {
        // For inks that need matte, ensure create_matte is called first
        // This matches Canvas2D behavior in rendering.rs
        // EXCEPTION: For 32-bit bitmaps with use_alpha=true, we use the embedded alpha channel
        // instead of computing a matte (matching drawing.rs lines 1268-1269)
        if Self::should_matte_sprite(ink) {
            let palettes = player.movie.cast_manager.palettes();
            if let Some(bitmap) = player.bitmap_manager.get_bitmap_mut(image_ref) {
                // Don't create matte for 32-bit bitmaps with embedded alpha
                // Use original_bit_depth since bit_depth can change during execution
                let use_embedded_alpha = bitmap.original_bit_depth == 32 && bitmap.use_alpha;
                if bitmap.matte.is_none() && !use_embedded_alpha {
                    bitmap.create_matte(&palettes);
                }
            }
        }

        // Get bitmap data to check version
        let bitmap = player.bitmap_manager.get_bitmap(image_ref)?;
        if bitmap.data.is_empty() {
            return None;
        }

        let bitmap_version = bitmap.version;
        let width = bitmap.width as u32;
        let height = bitmap.height as u32;

        // Create cache key including ink, colorize, and sprite_bg_color for inks that use bgColor matte
        // These inks use bgColor for matte/transparency computation:
        // - Ink 7: indexed bitmaps (not ghost - skips bgColor pixels via flood-fill matte)
        // - Ink 8: indexed bitmaps and 32-bit bitmaps without use_alpha (flood-fill matte)
        // - Ink 9: 32-bit bitmaps without use_alpha (mask with bgColor-based matte)
        // - Ink 40: indexed bitmaps (lighten - skips bgColor pixels via RGB comparison)
        // - Ink 41: 32-bit bitmaps without use_alpha (darken with bgColor-based matte)
        // Note: Ink 33 uses shader color-key, not texture matte, so no bgColor in cache key
        // Note: Ink 41 for indexed bitmaps uses palette index 0, not bgColor
        let is_ink_with_bgcolor_matte =
            ((ink == 7 || ink == 8 || ink == 40) && bitmap.original_bit_depth >= 2 && bitmap.original_bit_depth <= 8)
            || (ink == 8 && bitmap.original_bit_depth == 32 && !bitmap.use_alpha)
            || ((ink == 9 || ink == 41) && bitmap.original_bit_depth == 32 && !bitmap.use_alpha);
        let cache_key_bg_color = if is_ink_with_bgcolor_matte {
            sprite_bg_color
        } else {
            None
        };
        let cache_key = TextureCacheKey {
            member_ref: member_ref.clone(),
            ink,
            colorize,
            sprite_bg_color: cache_key_bg_color,
        };

        // Check cache - return cached texture if version matches
        if let Some(cached) = self.texture_cache.get(&cache_key) {
            if cached.version == bitmap_version {
                return Some((cached.texture.clone(), cached.width, cached.height));
            }
            // Version changed - texture needs to be re-uploaded (don't log to avoid spam)
        }

        // Convert bitmap to RGBA format with ink for matte computation
        let palettes = player.movie.cast_manager.palettes();

        // Only log on the very first frame for any new texture
        let _is_first_creation = self.frame_count == 1 && !self.texture_cache.has(&cache_key);

        let rgba_data = Self::bitmap_to_rgba(bitmap, &palettes, ink, colorize, sprite_bg_color);

        // Validate data size
        let expected_size = (width * height * 4) as usize;
        if rgba_data.len() != expected_size {
            web_sys::console::warn_1(
                &format!(
                    "WebGL2: RGBA data size mismatch for member {:?}: expected {}, got {}",
                    member_ref, expected_size, rgba_data.len()
                ).into()
            );
            return None;
        }

        // Create texture
        let texture = self.context.create_texture().ok()?;

        // Upload RGBA data to texture
        self.context
            .upload_texture_rgba(
                &texture,
                width,
                height,
                &rgba_data,
            )
            .ok()?;

        // Cache the texture with current bitmap version
        self.texture_cache.insert(
            cache_key,
            texture.clone(),
            width,
            height,
            bitmap_version,
        );

        Some((texture, width, height))
    }

    /// Get or create a 1x1 solid color texture for shape rendering
    fn get_or_create_solid_color_texture(&mut self, r: u8, g: u8, b: u8) -> web_sys::WebGlTexture {
        let key = (r, g, b);

        // Check if we already have this color cached
        if let Some(texture) = self.solid_color_textures.get(&key) {
            return texture.clone();
        }

        // Create a new 1x1 RGBA texture with the solid color
        let rgba_data: [u8; 4] = [r, g, b, 255];

        let texture = self.context.create_texture().expect("Failed to create solid color texture");

        self.context
            .upload_texture_rgba(&texture, 1, 1, &rgba_data)
            .expect("Failed to upload solid color texture");

        // Cache the texture
        self.solid_color_textures.insert(key, texture.clone());

        texture
    }

    /// Check if a font is actually available in the browser's Canvas2D.
    /// Compares measureText widths against known fallbacks to detect substitution.
    fn is_font_available_in_canvas2d(font_name: &str, font_size: u16) -> bool {
        let check = || -> Option<bool> {
            let document = web_sys::window()?.document()?;
            let canvas: web_sys::HtmlCanvasElement = document
                .create_element("canvas").ok()?
                .dyn_into().ok()?;
            canvas.set_width(1);
            canvas.set_height(1);
            let ctx: web_sys::CanvasRenderingContext2d = canvas
                .get_context("2d").ok()??
                .dyn_into().ok()?;

            let test_str = "ABCDwxyz0189";

            ctx.set_font(&format!("{}px {}", font_size, font_name));
            let requested_width = ctx.measure_text(test_str).ok()?.width();

            ctx.set_font(&format!("{}px sans-serif", font_size));
            let sans_width = ctx.measure_text(test_str).ok()?.width();

            ctx.set_font(&format!("{}px serif", font_size));
            let serif_width = ctx.measure_text(test_str).ok()?.width();

            Some(requested_width != sans_width && requested_width != serif_width)
        };
        check().unwrap_or(false)
    }

    /// Render text to a CPU bitmap, then upload as a WebGL texture
    ///
    /// This allows us to reuse the existing text rendering code from Canvas2D
    /// while still benefiting from GPU compositing for the final render.
    #[allow(clippy::too_many_arguments)]
    fn render_text_to_texture(
        &mut self,
        player: &mut DirPlayer,
        cache_key: &RenderedTextCacheKey,
        text: &str,
        font_name: &str,
        font_size: u16,
        font_style: Option<u8>,
        font_id: Option<u16>,
        line_spacing: u16,
        top_spacing: i16,
        bottom_spacing: i16,
        width: u32,
        height: u32,
        ink: i32,
        blend: i32,
        fg_color: &ColorRef,
        bg_color: &ColorRef,
        styled_spans: Option<&Vec<StyledSpan>>,
        alignment: &str,
        word_wrap: bool,
        border: u16,
        box_drop_shadow: u16,
    ) -> Option<(web_sys::WebGlTexture, u32, u32)> {
        let styled_span_count = styled_spans.map_or(0, |s| s.len());
        // Use the text member's own font_size for PFR rasterization target.
        // Taking the max of styled span sizes causes pixel fonts to be rasterized
        // at the wrong size (e.g. 15 instead of 12), producing wrong glyph shapes.
        let requested_font_size = font_size;
        if DEBUG_WEBGL2_TEXT {
            web_sys::console::log_1(&format!(
                "[webgl2.text] text_len={} spans={} font='{}' in_size={} req={} box={}x{} wrap={} align='{}'",
                text.len(),
                styled_span_count,
                font_name,
                font_size,
                requested_font_size,
                width,
                height,
                word_wrap,
                alignment
            ).into());
        }

        // Get or load the font
        let font = {
            let mut font_opt = player.font_manager.get_font_with_cast_and_bitmap(
                font_name,
                &player.movie.cast_manager,
                &mut player.bitmap_manager,
                Some(requested_font_size),
                font_style,
            );

            let mut lookup_method = "name";

            // Try font_id-based lookup if name-based lookup failed
            if font_opt.is_none() {
                if let Some(id) = font_id {
                    if let Some(font_ref) = player.font_manager.font_by_id.get(&id).copied() {
                        if let Some(font) = player.font_manager.fonts.get(&font_ref) {
                            font_opt = Some(font.clone());
                            lookup_method = "font_id";
                        }
                    }
                }
            }

            // Try case-insensitive match in font cache before falling back to system font
            if font_opt.is_none() {
                let font_name_lower = font_name.to_lowercase();
                for (key, font) in player.font_manager.font_cache.iter() {
                    if key.to_lowercase() == font_name_lower
                        || key.to_lowercase().starts_with(&format!("{}_", font_name_lower))
                    {
                        font_opt = Some(font.clone());
                        lookup_method = "cache_case_insensitive";
                        break;
                    }
                }
            }

            // PFR canonical fallback: if a PFR font with matching canonical name exists
            // in the cache, prefer it over falling back to system font / Canvas2D native.
            // Director movies embed PFR fonts to be used for all matching text — the PFR
            // font name may differ slightly from the text member's font reference.
            if font_opt.is_none() && !font_name.is_empty() {
                use crate::player::font::FontManager;
                let canon = FontManager::canonical_font_name(font_name);
                if !canon.is_empty() {
                    for (_key, font) in player.font_manager.font_cache.iter() {
                        if font.char_widths.is_some() && FontManager::canonical_font_name(&font.font_name) == canon {
                            font_opt = Some(font.clone());
                            lookup_method = "pfr_canonical_fallback";
                            break;
                        }
                    }
                }
            }

            if font_opt.is_none() {
                lookup_method = "system_fallback";

                if DEBUG_WEBGL2_TEXT {
                    web_sys::console::warn_1(
                        &format!(
                            "WebGL2 text: font '{}' (id={:?}) not found, using system font. Available fonts: {:?}",
                            font_name, font_id,
                            player.font_manager.font_cache.keys().collect::<Vec<_>>()
                        ).into()
                    );
                }
            }

            let result = font_opt.or_else(|| player.font_manager.get_system_font());
            
            if DEBUG_WEBGL2_TEXT {
                if let Some(ref f) = result {
                    web_sys::console::log_1(
                        &format!("WebGL2 text: using font '{}' (lookup={}, pfr={}, size={}x{})",
                            f.font_name, lookup_method, f.char_widths.is_some(),
                            f.char_width, f.char_height).into()
                    );
                }
            }
            result
        };

        let font = match font {
            Some(f) => f,
            None => {
                // No font available - log warning and return None
                web_sys::console::warn_1(
                    &format!("WebGL2 render_text_to_texture: No font found for '{}' size {}", font_name, font_size).into()
                );
                return None;
            }
        };

        let is_pfr_font = font.char_widths.is_some();
        if DEBUG_WEBGL2_TEXT {
            web_sys::console::log_1(&format!(
                "[webgl2.text] selected font='{}' pfr={} font_size={} char={}x{} bitmap_ref={}",
                font.font_name,
                is_pfr_font,
                font.font_size,
                font.char_width,
                font.char_height,
                font.bitmap_ref
            ).into());
        }
        let style_bits = font_style.unwrap_or(font.font_style);
        let bold = (style_bits & 1) != 0;
        let italic = (style_bits & 2) != 0;
        let underline = (style_bits & 4) != 0;
        let alignment_key = alignment.trim().trim_start_matches('#').to_ascii_lowercase();

        // Get the font bitmap
        let font_bitmap = match player.bitmap_manager.get_bitmap(font.bitmap_ref) {
            Some(b) => b,
            None => {
                // Font bitmap not found - log warning and return None
                web_sys::console::warn_1(
                    &format!("WebGL2 render_text_to_texture: Font bitmap {} not found", font.bitmap_ref).into()
                );
                return None;
            }
        };

        let mut render_width = width as u16;
        let mut render_height = height as u16;
        let mut render_line_spacing = line_spacing;

        // Measure actual text height and shrink render_height if the measured
        // content is smaller. Never grow beyond the member's rect — the rect
        // defines the visual boundary and the background fill must not exceed it.
        if word_wrap && render_width > 0 {
            let (_, measured_h) = measure_text_wrapped(
                text, &font, render_width, true,
                line_spacing, top_spacing, bottom_spacing,
            );
            let measured_h = measured_h
                + (2 * border) + (4 * box_drop_shadow);
            if measured_h > 0 && measured_h < render_height {
                render_height = measured_h;
            }
        } else if !word_wrap {
            let (_, measured_h) = measure_text(
                text, &font, None, line_spacing, top_spacing, bottom_spacing,
            );
            let measured_h = measured_h
                + (2 * border) + (4 * box_drop_shadow);
            if measured_h > 0 && measured_h < render_height {
                render_height = measured_h;
            }
        }

        // For bitmap font rendering (PFR or System font), keep rendering constrained to the sprite text box.
        // Expanding to measured width breaks wrapping because max_width tracks render width.
        // System font is a bitmap font that needs the same treatment as PFR fonts
        if (is_pfr_font || font.font_name == "System") && styled_spans.is_none() {
            render_line_spacing = 0;
        }

        // Compute alignment offset for bitmap font rendering (native text handles alignment internally).
        let mut bitmap_start_x = 0i32;
        if styled_spans.is_none() && !word_wrap {
            let (line_width, _) = if is_pfr_font {
                measure_text(text, &font, Some(font.char_height), 0, top_spacing, bottom_spacing)
            } else {
                measure_text(text, &font, None, render_line_spacing, top_spacing, bottom_spacing)
            };
            let box_width = width as i32;
            let line_width = line_width as i32;
            if matches!(alignment.to_lowercase().as_str(), "center" | "#center") {
                bitmap_start_x = ((box_width - line_width) / 2).max(0);
            }
        }

        // Create a 32-bit RGBA bitmap for rendering text
        let mut text_bitmap = Bitmap::new(
            render_width,
            render_height,
            32,
            32,
            0,
            PaletteRef::BuiltIn(get_system_default_palette()),
        );

        // Initialize as fully transparent (alpha=0).
        // PFR and Canvas2D rendering will write text pixels with alpha>0.
        // Background detection then simply checks alpha==0.
        text_bitmap.data.fill(0);

        let palettes = player.movie.cast_manager.palettes();

        // Check glyph preference to allow runtime switching
        let glyph_pref = get_glyph_preference();

        // PFR fonts with char_widths have a valid bitmap atlas from the PFR rasterizer.
        // Never use Canvas2D native for these — the PFR font name (e.g. "v") is not
        // registered in the browser, so Canvas2D fillText would fall back to a system font.
        let use_native_for_pfr = match glyph_pref {
            GlyphPreference::Native => true,  // Force native even for PFR
            GlyphPreference::Bitmap | GlyphPreference::Outline => false,  // Force bitmap atlas
            GlyphPreference::Auto => {
                if is_pfr_font {
                    if font.char_widths.is_some() {
                        // PFR rasterized font — use bitmap rendering
                        false
                    } else {
                        Self::is_font_available_in_canvas2d(&font.font_name, font_size)
                    }
                } else {
                    false
                }
            }
        };

        // Build synthetic spans for native rendering (non-PFR fonts OR PFR with system equivalent).
        // This ensures alignment, font size, and font style are applied for field/text input.
        // EXCEPTION: System font is a bitmap font and must use bitmap rendering, not native.
        // Check the REQUESTED font name, not the loaded font. When "Arial" is requested but
        // falls back to System bitmap, we should still use Canvas2D native with "Arial".
        let is_system_font_requested = font_name == "System" || font_name.is_empty();
        let force_bitmap = glyph_pref == GlyphPreference::Bitmap || glyph_pref == GlyphPreference::Outline;
        let force_native = glyph_pref == GlyphPreference::Native;
        let mut synthetic_spans: Option<Vec<StyledSpan>> = None;
        let spans_for_native: Option<&Vec<StyledSpan>> = if force_bitmap {
            // Force bitmap rendering for everything
            None
        } else if (force_native || !is_pfr_font || use_native_for_pfr) && !is_system_font_requested {
            if let Some(spans) = styled_spans {
                Some(spans)
            } else {
                let (r, g, b) = resolve_color_ref(
                    &palettes,
                    fg_color,
                    &PaletteRef::BuiltIn(get_system_default_palette()),
                    8,
                );
                let mut style = HtmlStyle::default();
                // Use the REQUESTED font name for Canvas2D rendering, not the fallback.
                // When "Arial" is requested but the PFR lookup falls back to System,
                // we want Canvas2D to render with "Arial" (a real browser font).
                style.font_face = Some(font_name.to_string());
                style.font_size = Some(font_size as i32);
                style.color = Some(((r as u32) << 16) | ((g as u32) << 8) | (b as u32));
                style.bold = bold;
                style.italic = italic;
                style.underline = underline;
                synthetic_spans = Some(vec![StyledSpan {
                    text: text.to_string(),
                    style,
                }]);
                synthetic_spans.as_ref()
            }
        } else {
            None
        };

        // Set up copy parameters for text rendering
        // Use ink 36 (background transparent) so white pixels become transparent
        let params = CopyPixelsParams {
            blend,
            ink: 36, // Background transparent - white background becomes transparent
            color: fg_color.clone(),
            bg_color: ColorRef::Rgb(255, 255, 255), // White background for transparency
            mask_image: None,
            is_text_rendering: true,
            rotation: 0.0,
            skew: 0.0,
            sprite: None,
            original_dst_rect: None,
        };

        let pfr_multi_span_styled = is_pfr_font && styled_spans.map_or(false, |s| s.len() > 1);

        // Diagnostic: log the rendering path and colors
        {
            let span_info: String = if let Some(spans) = spans_for_native.as_ref().or(styled_spans.as_ref()) {
                spans.iter().enumerate().map(|(i, s)| {
                    let c = s.style.color.map(|c| format!("#{:06X}", c & 0xFFFFFF)).unwrap_or("none".into());
                    format!("span[{}]={}", i, c)
                }).collect::<Vec<_>>().join(", ")
            } else { "no spans".to_string() };

            if DEBUG_WEBGL2_TEXT {
                web_sys::console::log_1(&format!(
                    "[render_text] font='{}' pfr={} native={} fg={:?} text='{}' spans=[{}]",
                    font_name, is_pfr_font, spans_for_native.is_some(), fg_color,
                    &text[..text.len().min(30)], span_info,
                ).into());
            }
        }

        // Render text to the bitmap - use styled spans if available
        // BUT only use native rendering if the font is NOT a PFR bitmap font
        // PFR fonts can't be used by Canvas2D, so we must use bitmap rendering
        if let Some(spans) = spans_for_native {
            // Parse alignment string to TextAlignment enum
            let text_alignment = match alignment_key.as_str() {
                "center" => TextAlignment::Center,
                "right" => TextAlignment::Right,
                "justify" => TextAlignment::Justify,
                _ => TextAlignment::Left,
            };

            // Use native browser text rendering for smooth, anti-aliased text
            // Don't pass fg_color since colors are now in the styled spans
            if let Err(e) = FontMemberHandlers::render_native_text_to_bitmap(
                &mut text_bitmap,
                spans,
                0,  // loc_h - render at origin
                top_spacing as i32,  // loc_v
                width as i32,
                height as i32,
                text_alignment,
                width as i32,
                word_wrap,
                None, // Color is in the spans
                render_line_spacing,
                top_spacing,
                bottom_spacing,
            ) {
                web_sys::console::warn_1(
                    &format!("WebGL2 render_text_to_texture: Native text render error: {:?}", e).into()
                );
            }
        } else {
            if DEBUG_WEBGL2_TEXT {
                web_sys::console::log_1(&format!(
                    "[render_text] Using BITMAP rendering path for font='{}' text_len={}",
                    font.font_name, text.len()
                ).into());
            }

            use crate::player::font::{
                bitmap_font_copy_char, bitmap_font_copy_char_scaled, bitmap_font_copy_char_tight,
            };

            let line_height = if font.font_size > 0 { font.font_size as i32 } else { font.char_height as i32 };
            let max_width = width as i32;
            let mut y = top_spacing as i32;

            // Resolve foreground color to RGB once for PFR direct copy
            let fg_color_rgb = resolve_color_ref(
                &palettes,
                fg_color,
                &PaletteRef::BuiltIn(get_system_default_palette()),
                8,
            );

                let mut render_line = |line: &str, y_pos: i32, bitmap: &mut Bitmap| {
                    let line_width: i32 = line
                        .chars()
                        .map(|c| font.get_char_advance(c as u8) as i32)
                        .sum();
                let start_x = if matches!(alignment.to_lowercase().as_str(), "center" | "#center") {
                    ((max_width - line_width) / 2).max(0)
                } else {
                    bitmap_start_x
                };

                    let cell_has_ink = |code: u8| -> bool {
                        if code < font.first_char_num {
                            return false;
                        }
                        let idx = (code - font.first_char_num) as usize;
                        let cx = (idx % font.grid_columns as usize) as i32;
                        let cy = (idx / font.grid_columns as usize) as i32;
                        let src_x =
                            (cx * font.grid_cell_width as i32 + font.char_offset_x as i32) as i32;
                        let src_y =
                            (cy * font.grid_cell_height as i32 + font.char_offset_y as i32) as i32;
                        let bmp_w = font_bitmap.width as i32;
                        let bmp_h = font_bitmap.height as i32;
                        for gy in 0..(font.char_height as i32) {
                            let sy = src_y + gy;
                            if sy < 0 || sy >= bmp_h {
                                continue;
                            }
                            for gx in 0..(font.char_width as i32) {
                                let sx = src_x + gx;
                                if sx < 0 || sx >= bmp_w {
                                    continue;
                                }
                                let p = ((sy * bmp_w + sx) * 4) as usize;
                                if p + 3 >= font_bitmap.data.len() {
                                    continue;
                                }
                                let rr = font_bitmap.data[p];
                                let gg = font_bitmap.data[p + 1];
                                let bb = font_bitmap.data[p + 2];
                                let aa = font_bitmap.data[p + 3];
                                if aa > 0 && !(rr >= 250 && gg >= 250 && bb >= 250) {
                                    return true;
                                }
                            }
                        }
                        false
                    };

                    let mut caps_only_missing = 0;
                    if is_pfr_font {
                        for lc in b'a'..=b'z' {
                            let uc = lc - 32;
                            if !cell_has_ink(lc) && cell_has_ink(uc) {
                                caps_only_missing += 1;
                            }
                        }
                    }
                    let force_uppercase_fallback = is_pfr_font && caps_only_missing >= 8;
                    if DEBUG_WEBGL2_TEXT && force_uppercase_fallback {
                        web_sys::console::log_1(&format!(
                            "[webgl2.text.pfr.char] caps-only fallback enabled missing_lowercase={}",
                            caps_only_missing
                        ).into());
                    }

                    let mut x = start_x;
                    let mut char_i: usize = 0;
                    for ch in line.chars() {
                        let adv = font.get_char_advance(ch as u8) as i32;
                        if ch == ' ' {
                            x += adv;
                            char_i += 1;
                            continue;
                        }

                        let mut glyph_code = ch as u8;
                        if force_uppercase_fallback && ch.is_ascii_lowercase() {
                            let lower = ch as u8;
                            let upper = lower.saturating_sub(32);
                            if cell_has_ink(upper) {
                                glyph_code = upper;
                                if DEBUG_WEBGL2_TEXT {
                                    web_sys::console::log_1(&format!(
                                        "[webgl2.text.pfr.char] fallback '{}' ({}) -> '{}' ({})",
                                        ch,
                                        lower,
                                        upper as char,
                                        upper
                                    ).into());
                                }
                            }
                        } else if is_pfr_font && ch.is_ascii_lowercase() {
                            let lower = ch as u8;
                            let upper = lower.saturating_sub(32);
                            if !cell_has_ink(lower) && cell_has_ink(upper) {
                                glyph_code = upper;
                                if DEBUG_WEBGL2_TEXT {
                                    web_sys::console::log_1(&format!(
                                        "[webgl2.text.pfr.char] fallback '{}' ({}) -> '{}' ({})",
                                        ch,
                                        lower,
                                        upper as char,
                                        upper
                                    ).into());
                                }
                            }
                        }

                        let use_tight_pfr = is_pfr_font && (font.char_width as i32) > (adv * 2).max(16);
                        if DEBUG_WEBGL2_TEXT && is_pfr_font {
                            if glyph_code >= font.first_char_num {
                                let char_index = (glyph_code - font.first_char_num) as usize;
                                let char_x = (char_index % font.grid_columns as usize) as u16;
                                let char_y = (char_index / font.grid_columns as usize) as u16;
                                let src_x =
                                    (char_x * font.grid_cell_width + font.char_offset_x) as i32;
                                let src_y =
                                    (char_y * font.grid_cell_height + font.char_offset_y) as i32;
                                web_sys::console::log_1(&format!(
                                    "[webgl2.text.pfr.char] i={} ch='{}' code={} adv={} dst=({}, {}) src=({}, {}) cell={}x{} glyph={}x{} first={} grid_cols={}",
                                    char_i,
                                    ch,
                                    glyph_code as u32,
                                    adv,
                                    x,
                                    y_pos,
                                    src_x,
                                    src_y,
                                    font.grid_cell_width,
                                    font.grid_cell_height,
                                    font.char_width,
                                    font.char_height,
                                    font.first_char_num,
                                    font.grid_columns
                                ).into());
                                if use_tight_pfr {
                                    web_sys::console::log_1(&format!(
                                        "[webgl2.text.pfr.char] i={} code={} using tight copy (adv={}, glyph_w={})",
                                        char_i,
                                        glyph_code as u32,
                                        adv,
                                        font.char_width
                                    ).into());
                                }
                                // Debug source glyph occupancy/bounds in the atlas cell.
                                // This helps detect glyphs that are effectively empty or very thin.
                                let mut ink_px: i32 = 0;
                                let mut min_ix = font.char_width as i32;
                                let mut max_ix = -1;
                                let mut min_iy = font.char_height as i32;
                                let mut max_iy = -1;

                                let bmp_w = font_bitmap.width as i32;
                                let bmp_h = font_bitmap.height as i32;
                                for gy in 0..(font.char_height as i32) {
                                    let sy = src_y + gy;
                                    if sy < 0 || sy >= bmp_h {
                                        continue;
                                    }
                                    for gx in 0..(font.char_width as i32) {
                                        let sx = src_x + gx;
                                        if sx < 0 || sx >= bmp_w {
                                            continue;
                                        }
                                        let p = ((sy * bmp_w + sx) * 4) as usize;
                                        if p + 3 >= font_bitmap.data.len() {
                                            continue;
                                        }
                                        let rr = font_bitmap.data[p];
                                        let gg = font_bitmap.data[p + 1];
                                        let bb = font_bitmap.data[p + 2];
                                        let aa = font_bitmap.data[p + 3];
                                        let is_ink = aa > 0 && !(rr >= 250 && gg >= 250 && bb >= 250);
                                        if is_ink {
                                            ink_px += 1;
                                            min_ix = min_ix.min(gx);
                                            max_ix = max_ix.max(gx);
                                            min_iy = min_iy.min(gy);
                                            max_iy = max_iy.max(gy);
                                        }
                                    }
                                }
                                let bbox_w = if max_ix >= min_ix { max_ix - min_ix + 1 } else { 0 };
                                let bbox_h = if max_iy >= min_iy { max_iy - min_iy + 1 } else { 0 };
                                web_sys::console::log_1(&format!(
                                    "[webgl2.text.pfr.char.metrics] i={} code={} ink_px={} bbox={}x{}",
                                    char_i, glyph_code as u32, ink_px, bbox_w, bbox_h
                                ).into());
                            } else {
                                web_sys::console::log_1(&format!(
                                    "[webgl2.text.pfr.char] i={} ch='{}' code={} below first_char_num={} -> skipped",
                                    char_i,
                                    ch,
                                    glyph_code as u32,
                                    font.first_char_num
                                ).into());
                            }
                        }
                        if use_tight_pfr {
                            bitmap_font_copy_char_tight(
                                &font,
                                font_bitmap,
                                glyph_code,
                                bitmap,
                                x,
                                y_pos,
                                &palettes,
                                &params,
                            );
                        } else {
                            bitmap_font_copy_char(
                                &font,
                                font_bitmap,
                                glyph_code,
                                bitmap,
                                x,
                                y_pos,
                                &palettes,
                                &params,
                            );
                        }
                        if bold {
                            if use_tight_pfr {
                                bitmap_font_copy_char_tight(
                                    &font,
                                    font_bitmap,
                                    glyph_code,
                                    bitmap,
                                    x + 1,
                                    y_pos,
                                    &palettes,
                                    &params,
                                );
                            } else {
                                bitmap_font_copy_char(
                                    &font,
                                    font_bitmap,
                                    glyph_code,
                                    bitmap,
                                    x + 1,
                                    y_pos,
                                    &palettes,
                                    &params,
                                );
                            }
                        }
                        if italic {
                            let shear = (y_pos / 4) as i32;
                            if use_tight_pfr {
                                bitmap_font_copy_char_tight(
                                    &font,
                                    font_bitmap,
                                    glyph_code,
                                    bitmap,
                                    x + shear,
                                    y_pos,
                                    &palettes,
                                    &params,
                                );
                            } else {
                                bitmap_font_copy_char(
                                    &font,
                                    font_bitmap,
                                    glyph_code,
                                    bitmap,
                                    x + shear,
                                    y_pos,
                                    &palettes,
                                    &params,
                                );
                            }
                        }
                        x += adv;
                        char_i += 1;
                    }

                if underline {
                    let (r, g, b) = resolve_color_ref(
                        &palettes,
                        &params.color,
                        &PaletteRef::BuiltIn(get_system_default_palette()),
                        8,
                    );
                    let underline_y = y_pos + line_height - 1;
                    for ux in start_x..(start_x + line_width).max(start_x) {
                        bitmap.set_pixel(ux, underline_y, (r, g, b), &palettes);
                    }
                }
            };

            if pfr_multi_span_styled {
                if DEBUG_WEBGL2_TEXT {
                    web_sys::console::log_1(&format!(
                        "[webgl2.text] PFR styled path spans={} line_spacing={} top_spacing={}",
                        styled_span_count,
                        render_line_spacing,
                        top_spacing
                    ).into());
                }
                #[derive(Clone)]
                struct PfrRunStyle {
                    size_px: i32,
                    color: ColorRef,
                    bold: bool,
                    italic: bool,
                    underline: bool,
                }

                #[derive(Clone)]
                struct PfrToken {
                    text: String,
                    is_whitespace: bool,
                    style: PfrRunStyle,
                }

                #[derive(Clone)]
                struct PfrLineRun {
                    text: String,
                    width: i32,
                    style: PfrRunStyle,
                }

                #[derive(Default)]
                struct PfrLine {
                    runs: Vec<PfrLineRun>,
                    width: i32,
                    max_size: i32,
                }

                enum Piece {
                    Token(PfrToken),
                    Newline,
                }

                let fallback_color = fg_color.clone();
                let base_size = font_size.max(1) as i32;
                let native_char_height = font.char_height.max(1) as i32;

                let color_from_style = |style: &HtmlStyle| -> ColorRef {
                    if let Some(c) = style.color {
                        ColorRef::Rgb(
                            ((c >> 16) & 0xFF) as u8,
                            ((c >> 8) & 0xFF) as u8,
                            (c & 0xFF) as u8,
                        )
                    } else {
                        fallback_color.clone()
                    }
                };

                let style_from_span = |style: &HtmlStyle| -> PfrRunStyle {
                    PfrRunStyle {
                        size_px: style.font_size.unwrap_or(base_size).max(1),
                        color: color_from_style(style),
                        bold: style.bold,
                        italic: style.italic,
                        underline: style.underline,
                    }
                };

                let token_width = |token_text: &str, style: &PfrRunStyle| -> i32 {
                    token_text
                        .chars()
                        .map(|c| {
                            ((font.get_char_advance(c as u8) as i32) * style.size_px / native_char_height)
                                .max(1)
                        })
                        .sum()
                };

                let mut pieces: Vec<Piece> = Vec::new();
                if let Some(spans) = styled_spans {
                    for span in spans {
                        if span.text.is_empty() {
                            continue;
                        }
                        let run_style = style_from_span(&span.style);
                        let mut token = String::new();
                        let mut token_is_ws: Option<bool> = None;

                        for ch in span.text.chars() {
                            if ch == '\r' || ch == '\n' {
                                if !token.is_empty() {
                                    pieces.push(Piece::Token(PfrToken {
                                        text: std::mem::take(&mut token),
                                        is_whitespace: token_is_ws.unwrap_or(false),
                                        style: run_style.clone(),
                                    }));
                                    token_is_ws = None;
                                }
                                pieces.push(Piece::Newline);
                                continue;
                            }

                            let is_ws = ch.is_whitespace();
                            if token_is_ws != Some(is_ws) && !token.is_empty() {
                                pieces.push(Piece::Token(PfrToken {
                                    text: std::mem::take(&mut token),
                                    is_whitespace: token_is_ws.unwrap_or(false),
                                    style: run_style.clone(),
                                }));
                            }
                            token_is_ws = Some(is_ws);
                            token.push(ch);
                        }

                        if !token.is_empty() {
                            pieces.push(Piece::Token(PfrToken {
                                text: token,
                                is_whitespace: token_is_ws.unwrap_or(false),
                                style: run_style.clone(),
                            }));
                        }
                    }
                }

                let mut lines: Vec<PfrLine> = Vec::new();
                let mut current_line = PfrLine::default();

                let mut push_line = |line: &mut PfrLine, lines_out: &mut Vec<PfrLine>| {
                    lines_out.push(std::mem::take(line));
                };

                for piece in pieces {
                    match piece {
                        Piece::Newline => push_line(&mut current_line, &mut lines),
                        Piece::Token(token) => {
                            let width = token_width(&token.text, &token.style);
                            if word_wrap
                                && max_width > 0
                                && !token.is_whitespace
                                && !current_line.runs.is_empty()
                                && (current_line.width + width > max_width)
                            {
                                push_line(&mut current_line, &mut lines);
                            }

                            if token.is_whitespace && current_line.runs.is_empty() {
                                continue;
                            }

                            current_line.width += width;
                            current_line.max_size = current_line.max_size.max(token.style.size_px);
                            current_line.runs.push(PfrLineRun {
                                text: token.text,
                                width,
                                style: token.style,
                            });
                        }
                    }
                }

                if !current_line.runs.is_empty() || lines.is_empty() {
                    lines.push(current_line);
                }

                if DEBUG_WEBGL2_TEXT {
                    let line_count = lines.len();
                    let max_line_width = lines.iter().map(|l| l.width).max().unwrap_or(0);
                    web_sys::console::log_1(&format!(
                        "[webgl2.text.pfr.styled] lines={} max_line_width={} box={}x{} y_start={} line_h={} spacing={} wrap={}",
                        line_count,
                        max_line_width,
                        render_width,
                        render_height,
                        y,
                        line_height,
                        render_line_spacing,
                        word_wrap
                    ).into());
                }

                for line in lines {
                    let start_x = match alignment_key.as_str() {
                        "center" => ((max_width - line.width) / 2).max(0),
                        "right" => (max_width - line.width).max(0),
                        _ => bitmap_start_x,
                    };

                    let mut x = start_x;
                    for run in line.runs {
                        let mut run_params = CopyPixelsParams {
                            blend: params.blend,
                            ink: params.ink,
                            color: run.style.color.clone(),
                            bg_color: params.bg_color.clone(),
                            mask_image: None,
                            is_text_rendering: params.is_text_rendering,
                            rotation: params.rotation,
                            skew: params.skew,
                            sprite: None,
                            original_dst_rect: params.original_dst_rect.clone(),
                        };

                        let char_h = run.style.size_px.max(1);
                        let char_w =
                            ((font.char_width as i32) * char_h / native_char_height).max(1);
                        let underline_y = y + char_h - 1;
                        let run_start_x = x;

                        for ch in run.text.chars() {
                            let advance =
                                ((font.get_char_advance(ch as u8) as i32) * char_h / native_char_height)
                                    .max(1);

                            if ch == ' ' {
                                x += advance;
                                continue;
                            }

                            bitmap_font_copy_char_scaled(
                                &font,
                                font_bitmap,
                                ch as u8,
                                &mut text_bitmap,
                                x,
                                y,
                                char_w,
                                char_h,
                                &palettes,
                                &run_params,
                            );

                            if run.style.bold {
                                bitmap_font_copy_char_scaled(
                                    &font,
                                    font_bitmap,
                                    ch as u8,
                                    &mut text_bitmap,
                                    x + 1,
                                    y,
                                    char_w,
                                    char_h,
                                    &palettes,
                                    &run_params,
                                );
                            }

                            if run.style.italic {
                                let shear = (y / 4).max(0);
                                bitmap_font_copy_char_scaled(
                                    &font,
                                    font_bitmap,
                                    ch as u8,
                                    &mut text_bitmap,
                                    x + shear,
                                    y,
                                    char_w,
                                    char_h,
                                    &palettes,
                                    &run_params,
                                );
                            }

                            x += advance;
                        }

                        if run.style.underline && underline_y >= 0 && underline_y < text_bitmap.height as i32 {
                            let (r, g, b) = resolve_color_ref(
                                &palettes,
                                &run_params.color,
                                &PaletteRef::BuiltIn(get_system_default_palette()),
                                8,
                            );
                            for ux in run_start_x..x {
                                text_bitmap.set_pixel(ux, underline_y, (r, g, b), &palettes);
                            }
                        }
                    }

                    let effective_lh = if render_line_spacing > 0 { render_line_spacing as i32 } else { line_height };
                    let line_step = effective_lh + bottom_spacing as i32 + top_spacing as i32;
                    if DEBUG_WEBGL2_TEXT {
                        web_sys::console::log_1(&format!(
                            "[webgl2.text.pfr.styled] line width={} start_x={} y={} step={} next_y={}",
                            line.width,
                            start_x,
                            y,
                            line_step,
                            y + line_step
                        ).into());
                    }
                    y += line_step;
                }
            } else {
                let raw_lines: Vec<&str> = text.split(|c| c == '\r' || c == '\n').collect();
                let mut lines_to_draw: Vec<String> = Vec::new();

                if word_wrap && max_width > 0 {
                    for raw in raw_lines {
                        if raw.is_empty() {
                            lines_to_draw.push(String::new());
                            continue;
                        }

                        let mut current = String::new();
                        for word in raw.split_whitespace() {
                            let candidate = if current.is_empty() {
                                word.to_string()
                            } else {
                                format!("{} {}", current, word)
                            };

                            let candidate_width: i32 = candidate
                                .chars()
                                .map(|c| font.get_char_advance(c as u8) as i32)
                                .sum();

                            if candidate_width <= max_width || current.is_empty() {
                                current = candidate;
                            } else {
                                lines_to_draw.push(current);
                                current = word.to_string();
                            }
                        }

                        if !current.is_empty() {
                            lines_to_draw.push(current);
                        }
                    }
                } else {
                    lines_to_draw = raw_lines.iter().map(|s| s.to_string()).collect();
                }

                if DEBUG_WEBGL2_TEXT {
                    let max_line_width = lines_to_draw
                        .iter()
                        .map(|line| {
                            line.chars()
                                .map(|c| font.get_char_advance(c as u8) as i32)
                                .sum::<i32>()
                        })
                        .max()
                        .unwrap_or(0);
                    web_sys::console::log_1(&format!(
                        "[webgl2.text.bitmap] lines={} max_line_width={} box={}x{} y_start={} line_h={} spacing={} wrap={} align='{}'",
                        lines_to_draw.len(),
                        max_line_width,
                        render_width,
                        render_height,
                        y,
                        line_height,
                        render_line_spacing,
                        word_wrap,
                        alignment_key
                    ).into());
                }

                for line in lines_to_draw {
                    if DEBUG_WEBGL2_TEXT {
                        let line_width: i32 = line.chars().map(|c| font.get_char_advance(c as u8) as i32).sum();
                        web_sys::console::log_1(&format!(
                            "[webgl2.text.bitmap] draw line_w={} y={} text='{}'",
                            line_width,
                            y,
                            line.chars().take(60).collect::<String>()
                        ).into());
                    }
                    render_line(&line, y, &mut text_bitmap);
                    let effective_lh = if render_line_spacing > 0 { render_line_spacing as i32 } else { line_height };
                    let line_step = effective_lh + bottom_spacing as i32 + top_spacing as i32;
                    if DEBUG_WEBGL2_TEXT && is_pfr_font {
                        web_sys::console::log_1(&format!(
                            "[webgl2.text.pfr.simple] step={} next_y={} h={}",
                            line_step,
                            y + line_step,
                            render_height
                        ).into());
                    }
                    y += line_step;
                }
            }
        }

        // Resolve the bg_color to RGB. The caller already picks the correct source:
        // member bgColor (from XMED Section 0x0000) with sprite bgColor as fallback.
        let bg_rgb = resolve_color_ref(
            &palettes,
            bg_color,
            &PaletteRef::BuiltIn(get_system_default_palette()),
            8,
        );
        // For transparency inks, NEVER fill the background.
        // The text bitmap already has alpha=0 for background, alpha>0 for text.
        // These inks rely on the alpha channel for transparency:
        // - Ink 7 (Not Ghost): discards alpha=0 pixels via matte
        // - Ink 8 (Matte): discards alpha<0.01 pixels in shader
        // - Ink 9 (Mask): uses alpha as mask
        // - Ink 36 (BgTransparent): composited with alpha blending
        // Filling background with bg_color at alpha=255 would make the entire
        // text area opaque, producing solid colored rectangles instead of text.
        let is_transparency_ink = ink == 7 || ink == 8 || ink == 9 || ink == 36;
        // For non-transparency inks (e.g. ink 0 Copy), always fill the background
        // with bgColor at alpha=255. This matches Director behavior where ink 0
        // for field/text members renders as an opaque rectangle. Only transparency
        // inks (36, 7, 8, 9) leave the background transparent.
        let has_bg_fill = !is_transparency_ink;

        // After drawing text, handle background pixels.
        // The bitmap was pre-filled with alpha=0 (transparent).
        // Both Canvas2D and PFR rendering write text pixels with alpha>0.
        // Background = alpha==0 (never written to by either renderer).
        //
        // For anti-aliased text, Canvas2D produces edge pixels with alpha 1-254
        // composited against transparent black. When has_bg_fill is active, these
        // semi-transparent pixels need to be blended against the background color
        // and made fully opaque, otherwise the stage color bleeds through.
        for i in 0..text_bitmap.data.len() / 4 {
            let a = text_bitmap.data[i * 4 + 3];

            if a == 0 {
                // Background pixel - either fill with bg color or leave transparent
                if has_bg_fill {
                    text_bitmap.data[i * 4] = bg_rgb.0;
                    text_bitmap.data[i * 4 + 1] = bg_rgb.1;
                    text_bitmap.data[i * 4 + 2] = bg_rgb.2;
                    text_bitmap.data[i * 4 + 3] = 255; // Fully opaque
                }
                // else: already alpha=0 (transparent), nothing to do
            } else if has_bg_fill && a < 255 {
                // Anti-aliased edge pixel: blend text color with background color.
                // Canvas2D composited against transparent black, so we need to
                // re-composite against the actual background color.
                let alpha = a as u32;
                let inv_alpha = 255 - alpha;
                let r = text_bitmap.data[i * 4] as u32;
                let g = text_bitmap.data[i * 4 + 1] as u32;
                let b = text_bitmap.data[i * 4 + 2] as u32;
                text_bitmap.data[i * 4]     = ((r * alpha + bg_rgb.0 as u32 * inv_alpha) / 255) as u8;
                text_bitmap.data[i * 4 + 1] = ((g * alpha + bg_rgb.1 as u32 * inv_alpha) / 255) as u8;
                text_bitmap.data[i * 4 + 2] = ((b * alpha + bg_rgb.2 as u32 * inv_alpha) / 255) as u8;
                text_bitmap.data[i * 4 + 3] = 255; // Fully opaque
            }
        }

        // Render cursor/caret if the field has focus
        if cache_key.has_focus {
            let text_width: i32 = text.chars()
                .map(|c| font.get_char_advance(c as u8) as i32)
                .sum();
            let cursor_x = bitmap_start_x + text_width;
            let cursor_y = 0;
            let cursor_width = 1;
            let cursor_height = font.char_height as i32;

            // Draw black cursor line
            text_bitmap.fill_rect(
                cursor_x,
                cursor_y,
                cursor_x + cursor_width,
                cursor_y + cursor_height,
                (0, 0, 0),
                &palettes,
                1.0,
            );
        }

        // Draw border and drop shadow for fields
        if border > 0 || box_drop_shadow > 0 {
            let border_color = (0, 0, 0); // Black border
            let shadow_color = (0, 0, 0); // Black shadow
            let w = render_width as i32;
            let h = render_height as i32;
            let shadow_offset = box_drop_shadow as i32;

            // Calculate the content area (excluding shadow space)
            let content_width = if shadow_offset > 0 { w - shadow_offset } else { w };
            let content_height = if shadow_offset > 0 { h - shadow_offset } else { h };

            // Draw drop shadow OUTSIDE the content area (bottom-right)
            if shadow_offset > 0 {
                // Right edge shadow - from top+offset to bottom, outside content area
                text_bitmap.fill_rect(
                    content_width,
                    shadow_offset,
                    w,
                    h,
                    shadow_color,
                    &palettes,
                    1.0, // Fully opaque shadow
                );
                // Bottom edge shadow - from left+offset to right-shadow, outside content area
                text_bitmap.fill_rect(
                    shadow_offset,
                    content_height,
                    content_width,
                    h,
                    shadow_color,
                    &palettes,
                    1.0, // Fully opaque shadow
                );
                // Corner shadow - where both shadows meet
                text_bitmap.fill_rect(
                    content_width,
                    content_height,
                    w,
                    h,
                    shadow_color,
                    &palettes,
                    1.0, // Fully opaque
                );

                // Clear corner pixels to transparent
                // Upper-right corner of shadow area (no shadow should appear here)
                for y in 0..shadow_offset {
                    for x in content_width..w {
                        let idx = ((y * render_width as i32 + x) * 4) as usize;
                        if idx + 3 < text_bitmap.data.len() {
                            text_bitmap.data[idx + 3] = 0; // Set alpha to 0 (transparent)
                        }
                    }
                }
                // Lower-left corner of content area (no shadow should appear here)
                for y in content_height..h {
                    for x in 0..shadow_offset {
                        let idx = ((y * render_width as i32 + x) * 4) as usize;
                        if idx + 3 < text_bitmap.data.len() {
                            text_bitmap.data[idx + 3] = 0; // Set alpha to 0 (transparent)
                        }
                    }
                }
            }

            // Draw border INSIDE the content area (not extending into shadow)
            if border > 0 {
                let b = border as i32;
                // Top border
                text_bitmap.fill_rect(0, 0, content_width, b, border_color, &palettes, 1.0);
                // Bottom border
                text_bitmap.fill_rect(0, content_height - b, content_width, content_height, border_color, &palettes, 1.0);
                // Left border
                text_bitmap.fill_rect(0, b, b, content_height - b, border_color, &palettes, 1.0);
                // Right border
                text_bitmap.fill_rect(content_width - b, b, content_width, content_height - b, border_color, &palettes, 1.0);
            }
        }

        // Upload the bitmap as a texture
        let texture = self.context.create_texture().ok()?;
        self.context
            .upload_texture_rgba(&texture, render_width as u32, render_height as u32, &text_bitmap.data)
            .ok()?;
        // Cache the texture
        self.rendered_text_cache.insert(
            cache_key.clone(),
            texture.clone(),
            render_width as u32,
            render_height as u32,
        );

        Some((texture, render_width as u32, render_height as u32))
    }

    /// Set preview size
    pub fn set_preview_size(&mut self, width: u32, height: u32) {
        self.preview_size = (width, height);
        self.preview_canvas.set_width(width);
        self.preview_canvas.set_height(height);
    }

    /// Set preview container element
    pub fn set_preview_container_element(
        &mut self,
        container_element: Option<web_sys::HtmlElement>,
    ) {
        if self.preview_canvas.parent_node().is_some() {
            self.preview_canvas.remove();
        }
        if let Some(container_element) = container_element {
            container_element
                .append_child(&self.preview_canvas)
                .unwrap();
            self.preview_container_element = Some(container_element);
        } else {
            self.preview_container_element = None;
        }
    }

    /// Set preview member reference
    pub fn set_preview_member_ref(&mut self, member_ref: Option<CastMemberRef>) {
        self.preview_member_ref = member_ref;
    }

    /// Draw the preview frame using Canvas2D
    pub fn draw_preview_frame(&mut self, player: &mut DirPlayer) {
        use wasm_bindgen::Clamped;

        if self.preview_member_ref.is_none()
            || self.preview_container_element.is_none()
        {
            return;
        }

        let member_ref = self.preview_member_ref.as_ref().unwrap().clone();
        let bitmap = crate::rendering::render_preview_bitmap(player, &member_ref, self.preview_font_size);
        if let Some(bitmap) = bitmap {
            let width = bitmap.width as u32;
            let height = bitmap.height as u32;

            if self.preview_size.0 != width || self.preview_size.1 != height {
                self.set_preview_size(width, height);
            }

            let slice_data = Clamped(bitmap.data.as_slice());
            let image_data = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                slice_data,
                width,
                height,
            );

            if let Ok(image_data) = image_data {
                let _ = self.preview_ctx2d.put_image_data(&image_data, 0.0, 0.0);
            }
        }
    }

    /// Set the canvas size
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.canvas.set_width(width);
        self.canvas.set_height(height);
        self.context.gl().viewport(0, 0, width as i32, height as i32);
        // Update projection matrix for new size
        self.projection_matrix = Self::create_ortho_matrix(width as f32, height as f32);
    }

    /// Get the canvas
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }

    /// Get the size
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Get the backend name
    pub fn backend_name(&self) -> &'static str {
        "WebGL2"
    }
}

impl super::Renderer for WebGL2Renderer {
    fn draw_frame(&mut self, player: &mut DirPlayer) {
        WebGL2Renderer::draw_frame(self, player)
    }

    fn draw_preview_frame(&mut self, player: &mut DirPlayer) {
        WebGL2Renderer::draw_preview_frame(self, player)
    }

    fn set_size(&mut self, width: u32, height: u32) {
        WebGL2Renderer::set_size(self, width, height)
    }

    fn size(&self) -> (u32, u32) {
        WebGL2Renderer::size(self)
    }

    fn backend_name(&self) -> &'static str {
        WebGL2Renderer::backend_name(self)
    }

    fn canvas(&self) -> &HtmlCanvasElement {
        WebGL2Renderer::canvas(self)
    }

    fn set_preview_member_ref(&mut self, member_ref: Option<CastMemberRef>) {
        WebGL2Renderer::set_preview_member_ref(self, member_ref)
    }

    fn set_preview_container_element(&mut self, container_element: Option<web_sys::HtmlElement>) {
        WebGL2Renderer::set_preview_container_element(self, container_element)
    }

    fn set_preview_font_size(&mut self, size: Option<u16>) {
        self.preview_font_size = size;
    }

    fn preview_font_size(&self) -> Option<u16> {
        self.preview_font_size
    }
}
