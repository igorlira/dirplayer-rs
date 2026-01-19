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

use itertools::Itertools;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use std::collections::HashMap;

use crate::player::{
    bitmap::bitmap::{get_system_default_palette, resolve_color_ref, Bitmap, PaletteRef},
    bitmap::drawing::CopyPixelsParams,
    cast_lib::CastMemberRef,
    cast_member::CastMemberType,
    font::{measure_text, BitmapFont},
    score::{get_concrete_sprite_rect, ScoreRef},
    sprite::ColorRef,
    DirPlayer,
};

pub use context::WebGL2Context;
pub use geometry::QuadGeometry;
pub use shaders::{InkMode, ShaderManager};
pub use texture_cache::{TextureCache, TextureCacheKey, RenderedTextCache, RenderedTextCacheKey};

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
    /// Preview container element
    preview_container_element: Option<web_sys::HtmlElement>,
    /// Rendered text texture cache
    rendered_text_cache: RenderedTextCache,
}

impl WebGL2Renderer {
    /// Create a new WebGL2 renderer
    pub fn new(
        canvas: HtmlCanvasElement,
        preview_canvas: HtmlCanvasElement,
    ) -> Result<Self, JsValue> {
        let gl = canvas
            .get_context("webgl2")?
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
            preview_container_element: None,
            rendered_text_cache: RenderedTextCache::new(),
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

    /// Draw the current frame
    pub fn draw_frame(&mut self, player: &mut DirPlayer) {
        self.frame_count += 1;

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
        let sorted_channels = player
            .movie
            .score
            .get_sorted_channels(player.movie.current_frame)
            .iter()
            .map(|x| x.number as i16)
            .collect_vec();

        // Bind quad geometry once
        self.quad.bind(self.context.gl());

        // Render each sprite
        for channel_num in sorted_channels {
            self.render_sprite(player, channel_num);
        }

        // Unbind
        self.quad.unbind(self.context.gl());

        // Draw debug highlight for selected channel
        if let Some(selected_channel) = self.debug_selected_channel_num {
            self.draw_debug_highlight(player, selected_channel);
            // Reset shader manager state since draw_debug_highlight uses its own shader
            // and calls gl.use_program(None), which desynchronizes the manager's cached state
            self.shader_manager.clear_active();
        }
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

    /// Render a single sprite
    fn render_sprite(&mut self, player: &mut DirPlayer, channel_num: i16) {
        // Get sprite and member info
        let (member_ref, sprite_rect, ink, blend, flip_h, flip_v, rotation, bg_color, fg_color, has_fore_color, has_back_color, is_puppet, raw_loc, sprite_width, sprite_height) = {
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

        // Debug logging for sprite positions removed to reduce console spam

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
                line_spacing: u16,
                top_spacing: i16,
                width: u32,
                height: u32,
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
                CastMemberType::Shape(_shape_member) => {
                    // Skip rendering shapes with tiny dimensions (blank placeholders or zero-size)
                    // Skip if EITHER dimension is <= 1
                    if sprite_width <= 1 || sprite_height <= 1 {
                        return;
                    }

                    // Resolve foreground color to RGB
                    let palettes = player.movie.cast_manager.palettes();
                    let (r, g, b) = resolve_color_ref(
                        &palettes,
                        &fg_color,
                        &PaletteRef::BuiltIn(get_system_default_palette()),
                        8, // default bit depth
                    );
                    TextureSource::SolidColor { r, g, b }
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

                    let cache_key = RenderedTextCacheKey::new(
                        member_ref.clone(),
                        text,
                        ink,
                        blend,
                        fg_color.clone(),
                        bg_color.clone(),
                        width,
                        height,
                    );

                    TextureSource::RenderedText {
                        cache_key,
                        text: text.clone(),
                        font_name: font_member.font_info.name.clone(),
                        font_size: font_member.font_info.size,
                        font_style: Some(font_member.font_info.style),
                        line_spacing: font_member.fixed_line_space,
                        top_spacing: font_member.top_spacing,
                        width,
                        height,
                    }
                }
                CastMemberType::Text(text_member) => {
                    // Text member: render the text using specified font
                    let text = &text_member.text;
                    if text.is_empty() {
                        return; // No text to render
                    }

                    // Use sprite rect dimensions for the texture (not measured text)
                    // This prevents stretching when sprite rect differs from text size
                    let width = (sprite_rect.width()).max(1) as u32;
                    let height = (sprite_rect.height()).max(1) as u32;

                    let cache_key = RenderedTextCacheKey::new(
                        member_ref.clone(),
                        text,
                        ink,
                        blend,
                        fg_color.clone(),
                        bg_color.clone(),
                        width,
                        height,
                    );

                    TextureSource::RenderedText {
                        cache_key,
                        text: text.clone(),
                        font_name: text_member.font.clone(),
                        font_size: text_member.font_size,
                        font_style: None,
                        line_spacing: text_member.fixed_line_space,
                        top_spacing: text_member.top_spacing,
                        width,
                        height,
                    }
                }
                CastMemberType::Field(field_member) => {
                    // Field member: editable text field
                    let text = &field_member.text;

                    // Use sprite rect dimensions for the texture (not measured text)
                    // This prevents stretching when sprite rect differs from text size
                    let width = (sprite_rect.width()).max(1) as u32;
                    let height = (sprite_rect.height()).max(1) as u32;

                    // Check if this field has keyboard focus (for cursor rendering)
                    let has_focus = player.keyboard_focus_sprite == channel_num;

                    // Include focus state in cache key so cursor state changes invalidate cache
                    let cache_key = RenderedTextCacheKey::new_with_focus(
                        member_ref.clone(),
                        text,
                        ink,
                        blend,
                        fg_color.clone(),
                        bg_color.clone(),
                        has_focus,
                        width,
                        height,
                    );

                    TextureSource::RenderedText {
                        cache_key,
                        text: text.clone(),
                        font_name: field_member.font.clone(),
                        font_size: field_member.font_size,
                        font_style: None,
                        line_spacing: field_member.fixed_line_space,
                        top_spacing: field_member.top_spacing,
                        width,
                        height,
                    }
                }
                // TODO: Handle other member types (film loops)
                _ => {
                    // Unhandled member types are silently skipped
                    return;
                }
            }
        };

        // Get bitmap info for palette reference and bit depth (only used for bitmap sprites)
        let (bitmap_palette_ref, bitmap_bit_depth) = match &texture_source {
            TextureSource::Bitmap { image_ref } => {
                match player.bitmap_manager.get_bitmap(*image_ref) {
                    Some(bitmap) => (bitmap.palette_ref.clone(), bitmap.original_bit_depth),
                    None => (PaletteRef::BuiltIn(get_system_default_palette()), 8),
                }
            }
            TextureSource::SolidColor { .. } | TextureSource::RenderedText { .. } => {
                // Solid colors and rendered text use system default palette
                (PaletteRef::BuiltIn(get_system_default_palette()), 8)
            }
        };

        // Resolve colors to RGB for shader uniforms and colorize
        let palettes = player.movie.cast_manager.palettes();
        let bg_color_rgb = resolve_color_ref(
            &palettes,
            &bg_color,
            &bitmap_palette_ref,
            bitmap_bit_depth,
        );
        let fg_color_rgb = resolve_color_ref(
            &palettes,
            &fg_color,
            &bitmap_palette_ref,
            bitmap_bit_depth,
        );

        // Build colorize parameters if colorize is active
        // IMPORTANT: Canvas2D only applies colorize for 32-bit bitmaps in the general pixel loop.
        // For indexed bitmaps with ink 0, 8, 36, Canvas2D uses early paths that skip colorize.
        // So we only pass colorize_params for 32-bit bitmaps.
        let colorize_params = if (has_fore_color || has_back_color)
            && bitmap_bit_depth == 32
            && (ink == 0 || ink == 8 || ink == 9)
        {
            // Debug: Log when colorize is being applied
            // Debug logging removed to reduce console spam
            Some((
                has_fore_color,
                has_back_color,
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
        // For ink 8 indexed bitmaps, we also pass sprite's bgColor for matte computation
        // (this matches Canvas2D's copy_pixels_with_params which uses sprite bgColor)
        // Colorize is also baked into the texture when has_fore_color or has_back_color is set
        let tex = match texture_source {
            TextureSource::Bitmap { image_ref } => {
                // For ink 8, pass sprite's bgColor for matte computation
                let sprite_bg_for_matte = if ink == 8 { Some(bg_color_rgb) } else { None };
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
                line_spacing,
                top_spacing,
                width,
                height,
            } => {
                // Check cache first
                if let Some(cached) = self.rendered_text_cache.get(cache_key) {
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
                        line_spacing,
                        top_spacing,
                        width,
                        height,
                        ink,
                        blend,
                        &fg_color,
                        &bg_color,
                    ) {
                        Some(tex) => tex,
                        None => return,
                    }
                }
            }
        };

        // Debug: Log ink 8 sprites with their bitmap info
        // Debug logging for ink 8 removed to reduce console spam

        // Select shader based on ink mode
        // The bgColor-based transparency is now baked into the texture's alpha channel
        // via the flood-fill matte mask computation, so we use the ink mode directly
        let ink_mode = InkMode::from_ink_number(ink);
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
        let u_blend = program.u_blend.clone();
        let u_bg_color = program.u_bg_color.clone();
        let u_color_tolerance = program.u_color_tolerance.clone();

        // Debug: warn if critical uniforms are missing
        if u_sprite_rect.is_none() && self.frame_count == 1 {
            web_sys::console::warn_1(
                &format!("WebGL2: u_sprite_rect uniform not found for ink {:?}", effective_ink).into()
            );
        }

        // Set blend mode based on effective ink (to match the shader being used)
        match effective_ink {
            InkMode::AddPin => {
                self.context.set_blend_additive();
            }
            InkMode::Darken => {
                // Multiply blend: result = dst * src
                // Shader outputs lerp(1.0, src.rgb, blend) to apply blend %
                self.context.set_blend_multiply();
            }
            InkMode::Lighten => self.context.set_blend_lighten(),
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

        // Set flip
        if let Some(ref loc) = u_flip {
            gl.uniform2f(
                Some(loc),
                if flip_h { 1.0 } else { 0.0 },
                if flip_v { 1.0 } else { 0.0 },
            );
        }

        // Set rotation (convert degrees to radians)
        if let Some(ref loc) = u_rotation {
            gl.uniform1f(Some(loc), (rotation as f32).to_radians());
        }

        // Set blend (0-100 -> 0.0-1.0)
        if let Some(ref loc) = u_blend {
            gl.uniform1f(Some(loc), blend as f32 / 100.0);
        }

        // Set background color for ink modes that need it:
        // - BackgroundTransparent: for color-key transparency
        // - NotGhost: for color-key transparency
        // - Darken: for src * bg_color multiplication
        // (using the already-resolved bg_color_rgb from earlier)
        if effective_ink == InkMode::BackgroundTransparent
            || effective_ink == InkMode::NotGhost
            || effective_ink == InkMode::Darken
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
                gl.uniform1f(Some(loc), 0.01); // Small tolerance for floating-point comparison
            }
        }

        // Draw the quad
        self.quad.draw(gl);

        // Unbind texture
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);

        // Reset blend equation if we used Lighten (which changes the blend equation)
        if effective_ink == InkMode::Lighten {
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
        // IMPORTANT: Canvas2D only applies colorize in the general pixel loop (drawing.rs line 1275+),
        // NOT in the ink-specific early paths (ink 0 indexed lines 1072-1092, ink 8 indexed lines 1229-1260).
        // So for indexed bitmaps with ink 0, 8, 36, Canvas2D does NOT apply colorize - only for 32-bit.
        let allow_colorize = match (bitmap.original_bit_depth, ink as u32) {
            (32, 0) => true,           // 32-bit ink 0: grayscale remap
            (32, 8) | (32, 9) => true, // 32-bit ink 8/9: foreColor only
            // REMOVED: indexed ink 0, 8, 9 - Canvas2D handles these in early paths without colorize
            _ => false,
        };

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
        // Matte mask usage: ONLY inks 0 and 8 use matte in score sprite rendering
        // See rendering.rs lines 582-583:
        //   let should_use_matte = (is_indexed && (ink == 0 || ink == 8)) || (is_16bit && ink == 0);
        //
        // Important: inks 7, 33, 36, 41 do NOT use matte mask for score sprites!
        // They have their own transparency mechanisms in the blend function:
        // - Ink 7 (Not Ghost): color-key comparison in blend_pixel
        // - Ink 33 (Add Pin): no transparency, just additive blend
        // - Ink 36 (BgTransparent): color-key comparison in blend_pixel
        // - Ink 41 (Darken): no transparency, just multiply blend
        let should_use_matte_ink0 = bitmap.trim_white_space && ink == 0;
        // Ink 8 uses matte ONLY when trim_white_space is true (see drawing.rs lines 880-884)
        let should_use_matte_ink8 = bitmap.trim_white_space && ink == 8;
        // Ink 36 uses color-key transparency for indexed bitmaps (not flood-fill)
        let should_use_colorkey_ink36 = ink == 36 && is_indexed;
        // Total: when to use matte (either pre-computed or on-the-fly)
        let should_use_matte = should_use_matte_ink0 || should_use_matte_ink8;

        // IMPORTANT: Canvas2D only computes matte for ink 8 when is_matte_bitmap is true
        // (is_matte_bitmap = trim_white_space || is_text_rendering)
        // See drawing.rs lines 782-784 and 880-884.
        // If trim_white_space is false, ink 8 renders all pixels as opaque (no transparency).
        // This is critical for shadow bitmaps that should be fully opaque.
        //
        // For indexed bitmaps with ink 8 AND trim_white_space, compute matte using RGB comparison
        // (matching Canvas2D's copy_pixels_with_params behavior)
        let is_matte_bitmap = bitmap.trim_white_space;
        let ink_8_needs_matte = ink == 8 && is_indexed && is_matte_bitmap;

        let needs_computed_matte = (bitmap.matte.is_none() || ink_8_needs_matte)
            && should_use_matte
            && width > 0
            && height > 0
            && (
                // Indexed bitmaps: matte for ink 0 (when trim_white_space) or ink 8 (when trim_white_space)
                (is_indexed && is_matte_bitmap && (ink == 0 || ink == 8))
                // 16-bit bitmaps: matte for ink 0 only (when trim_white_space)
                || (is_16bit && should_use_matte_ink0)
                // 32-bit bitmaps WITHOUT use_alpha: matte for ink 0 or ink 8 (when trim_white_space)
                || (is_32bit && !bitmap.use_alpha && is_matte_bitmap && (ink == 0 || ink == 8))
            );

        // Compute matte mask
        let computed_matte: Option<Vec<bool>> = if needs_computed_matte {
            if is_indexed {
                // For indexed bitmaps:
                // - Most inks: use palette index 0 comparison (like bitmap.matte / create_matte())
                //   This matches Director's standard behavior where index 0 is background.
                // - ink 8 (Matte): use RGB comparison with edge color (like Canvas2D's copy_pixels_with_params)
                //   Canvas2D computes its own matte for ink 8 using RGB comparison.
                //
                // Using palette index comparison is more correct for indexed bitmaps because
                // it only marks palette index 0 as background, not any pixel that happens
                // to have the same RGB color. This is important for grayscale palettes where
                // index 0 = white but other indices may also be close to white.
                if ink == 8 {
                    // Ink 8: use RGB comparison with sprite's bgColor (matches Canvas2D behavior)
                    // Canvas2D uses sprite's bgColor as the background color for flood-fill matte,
                    // NOT the edge pixel color. This is critical for bitmaps with borders.
                    // If sprite_bg_color is not provided, fall back to edge pixel color.
                    let bg_color_for_matte = sprite_bg_color.unwrap_or_else(|| {
                        bitmap.get_pixel_color(palettes, 0, 0)
                    });

                    Some(Self::compute_edge_matte_mask_rgb(bitmap, palettes, bg_color_for_matte, width, height))
                } else {
                    // All other inks: use palette index comparison (background = index 0)
                    Some(Self::compute_edge_matte_mask_indexed(bitmap, width, height))
                }
            } else if is_32bit {
                // For 32-bit bitmaps without use_alpha, use RGB comparison with edge color
                // (matching drawing.rs lines 891-897)
                let edge_color = bitmap.get_pixel_color(palettes, 0, 0);
                Some(Self::compute_edge_matte_mask_rgb(bitmap, palettes, edge_color, width, height))
            } else {
                // For 16-bit bitmaps, use RGB comparison with white as background
                let bg_color_rgb = (255u8, 255u8, 255u8);
                Some(Self::compute_edge_matte_mask_rgb(bitmap, palettes, bg_color_rgb, width, height))
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
                let a = if use_embedded_alpha {
                    // 32-bit with use_alpha: use embedded alpha directly, ignore any matte
                    let index = (y * width + x) * 4;
                    if index + 3 < bitmap.data.len() {
                        bitmap.data[index + 3]
                    } else {
                        255
                    }
                } else if should_use_colorkey_ink36 {
                    // Ink 36 with indexed bitmap: simple color-key transparency
                    // Index 0 = background = transparent, all other indices = opaque
                    // This matches Canvas2D behavior (lines 1094-1105 in drawing.rs)
                    let color_ref = bitmap.get_pixel_color_ref(x as u16, y as u16);
                    match color_ref {
                        ColorRef::PaletteIndex(0) => 0,   // Background = transparent
                        _ => 255,                         // Everything else = opaque
                    }
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
                } else if bitmap.original_bit_depth == 32 {
                    // For 32-bit bitmaps without matte (use_alpha=false), alpha is in the data
                    let index = (y * width + x) * 4;
                    if index + 3 < bitmap.data.len() {
                        bitmap.data[index + 3]
                    } else {
                        255
                    }
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

                        // ---------- INDEXED (≤8-bit) ----------
                        _ => {
                            // Get palette index
                            let color_ref = bitmap.get_pixel_color_ref(x as u16, y as u16);

                            if let ColorRef::PaletteIndex(i) = color_ref {
                                let max = (1u16 << bitmap.original_bit_depth) - 1;
                                let t = i as f32 / max as f32;

                                if has_fore && has_back && use_back_color {
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

        // Create cache key including ink, colorize, and sprite_bg_color for ink 8
        // For ink 8 indexed bitmaps, the sprite's bgColor affects matte computation
        let cache_key_bg_color = if ink == 8 && bitmap.original_bit_depth <= 8 {
            sprite_bg_color
        } else {
            None // Only include bgColor in cache key for ink 8 indexed bitmaps
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

        // Debug: Log matte info for troubleshooting (only on first frame to avoid spam)
        // Only log on the very first frame for any new texture
        let _is_first_creation = self.frame_count == 1 && !self.texture_cache.has(&cache_key);
        // Commented out debug logging to reduce console spam
        // if is_first_creation {
        //     let has_matte = bitmap.matte.is_some();
        //     let matte_stats = if let Some(ref matte) = bitmap.matte {
        //         let total = (bitmap.width as usize) * (bitmap.height as usize);
        //         let opaque_count = (0..bitmap.height).flat_map(|y| (0..bitmap.width).map(move |x| (x, y)))
        //             .filter(|(x, y)| matte.get_bit(*x, *y))
        //             .count();
        //         format!("opaque={}/{}", opaque_count, total)
        //     } else {
        //         "no matte".to_string()
        //     };
        //     web_sys::console::log_1(
        //         &format!(
        //             "WebGL2: Texture {:?} ink={} depth={} palette={:?} has_matte={} {}",
        //             member_ref, ink, bitmap.bit_depth, bitmap.palette_ref, has_matte, matte_stats
        //         ).into()
        //     );
        // }

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

        // Debug logging for RGBA upload removed to reduce console spam

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
        line_spacing: u16,
        top_spacing: i16,
        width: u32,
        height: u32,
        ink: i32,
        blend: i32,
        fg_color: &ColorRef,
        bg_color: &ColorRef,
    ) -> Option<web_sys::WebGlTexture> {
        // Get or load the font
        let font = {
            let font_opt = player.font_manager.get_font_with_cast(
                font_name,
                Some(&player.movie.cast_manager),
                Some(font_size),
                font_style,
            );

            font_opt.or_else(|| player.font_manager.get_system_font())
        };

        let font = match font {
            Some(f) => f,
            None => {
                // No font available - silently return None
                return None;
            }
        };

        // Get the font bitmap
        let font_bitmap = match player.bitmap_manager.get_bitmap(font.bitmap_ref) {
            Some(b) => b,
            None => {
                // Font bitmap not found - silently return None
                return None;
            }
        };

        // Create a 32-bit RGBA bitmap for rendering text
        let mut text_bitmap = Bitmap::new(
            width as u16,
            height as u16,
            32,
            32,
            0,
            PaletteRef::BuiltIn(get_system_default_palette()),
        );

        // Fill with transparent background (alpha = 0)
        // The bitmap is already zero-initialized, so we just need to verify it's transparent
        for i in 0..text_bitmap.data.len() / 4 {
            text_bitmap.data[i * 4 + 3] = 0; // Set alpha to 0 for transparent background
        }

        let palettes = player.movie.cast_manager.palettes();

        // Set up copy parameters for text rendering
        let params = CopyPixelsParams {
            blend,
            ink: ink as u32,
            color: fg_color.clone(),
            bg_color: bg_color.clone(),
            mask_image: None,
            is_text_rendering: true,
            rotation: 0.0,
            sprite: None,
            original_dst_rect: None,
        };

        // Render text to the bitmap
        text_bitmap.draw_text(
            text,
            &font,
            font_bitmap,
            0,  // loc_h - render at origin
            0,  // loc_v - render at origin
            params,
            &palettes,
            line_spacing,
            top_spacing,
        );

        // Render cursor/caret if the field has focus
        if cache_key.has_focus {
            // Measure text to find cursor position (at end of text)
            let (text_width, _) = measure_text(
                text,
                &font,
                None,
                line_spacing,
                top_spacing,
            );

            let cursor_x = text_width as i32;
            let cursor_y = top_spacing as i32;
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

        // Debug logging for text rendering removed to reduce console spam

        // Upload the bitmap as a texture
        let texture = self.context.create_texture().ok()?;
        self.context
            .upload_texture_rgba(&texture, width, height, &text_bitmap.data)
            .ok()?;

        // Cache the texture
        self.rendered_text_cache.insert(
            cache_key.clone(),
            texture.clone(),
            width,
            height,
        );

        Some(texture)
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

        let member_ref = self.preview_member_ref.as_ref().unwrap();
        let member = player.movie.cast_manager.find_member_by_ref(member_ref);
        if member.is_none() {
            return;
        }
        let member = member.unwrap();

        match &member.member_type {
            CastMemberType::Bitmap(sprite_member) => {
                let sprite_bitmap = player.bitmap_manager.get_bitmap(sprite_member.image_ref);
                if sprite_bitmap.is_none() {
                    return;
                }
                let sprite_bitmap = sprite_bitmap.unwrap();
                let width = sprite_bitmap.width as u32;
                let height = sprite_bitmap.height as u32;

                // Create a 32-bit bitmap for display
                let mut bitmap = Bitmap::new(
                    width as u16,
                    height as u16,
                    32,
                    32,
                    0,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );

                let palettes = &player.movie.cast_manager.palettes();

                // Fill with background color
                bitmap.fill_relative_rect(
                    0,
                    0,
                    0,
                    0,
                    resolve_color_ref(
                        &palettes,
                        &player.bg_color,
                        &PaletteRef::BuiltIn(get_system_default_palette()),
                        sprite_bitmap.original_bit_depth,
                    ),
                    palettes,
                    1.0,
                );

                // Copy the sprite bitmap
                bitmap.copy_pixels(
                    &palettes,
                    sprite_bitmap,
                    crate::player::geometry::IntRect::from(
                        0,
                        0,
                        sprite_bitmap.width as i32,
                        sprite_bitmap.height as i32,
                    ),
                    crate::player::geometry::IntRect::from(
                        0,
                        0,
                        sprite_bitmap.width as i32,
                        sprite_bitmap.height as i32,
                    ),
                    &HashMap::new(),
                    None,
                );

                // Mark registration point with magenta
                bitmap.set_pixel(
                    sprite_member.reg_point.0 as i32,
                    sprite_member.reg_point.1 as i32,
                    (255, 0, 255),
                    palettes,
                );

                // Update preview size if needed
                if self.preview_size.0 != width || self.preview_size.1 != height {
                    self.set_preview_size(width, height);
                }

                // Render to canvas using ImageData
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
            CastMemberType::FilmLoop(loop_member) => {
                let width = loop_member.info.width as u32;
                let height = loop_member.info.height as u32;

                // Create a bitmap for the film loop
                let mut bitmap = Bitmap::new(
                    width as u16,
                    height as u16,
                    32,
                    32,
                    0,
                    PaletteRef::BuiltIn(get_system_default_palette()),
                );

                // Render the film loop score to bitmap
                crate::rendering::render_score_to_bitmap(
                    player,
                    &ScoreRef::FilmLoop(member_ref.clone()),
                    &mut bitmap,
                    None,
                    crate::player::geometry::IntRect::from_size(0, 0, width as i32, height as i32),
                );

                // Update preview size if needed
                if self.preview_size.0 != width || self.preview_size.1 != height {
                    self.set_preview_size(width, height);
                }

                // Render to canvas using ImageData
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
            _ => {}
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
}
