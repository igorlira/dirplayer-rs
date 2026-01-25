//! WebGL2 context wrapper with error handling

use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext, WebGlBuffer, WebGlFramebuffer, WebGlProgram, WebGlShader,
    WebGlTexture, WebGlUniformLocation, WebGlVertexArrayObject,
};

/// Wrapper around WebGL2RenderingContext with convenience methods
pub struct WebGL2Context {
    gl: WebGl2RenderingContext,
}

impl WebGL2Context {
    /// Create a new WebGL2 context wrapper
    pub fn new(gl: WebGl2RenderingContext) -> Result<Self, JsValue> {
        // Enable required extensions and set default state
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(
            WebGl2RenderingContext::SRC_ALPHA,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
        );

        // Disable depth testing (2D rendering)
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);

        Ok(Self { gl })
    }

    /// Get the raw WebGL2 context
    pub fn gl(&self) -> &WebGl2RenderingContext {
        &self.gl
    }

    /// Compile a shader from source
    pub fn compile_shader(
        &self,
        shader_type: u32,
        source: &str,
    ) -> Result<WebGlShader, JsValue> {
        let shader = self
            .gl
            .create_shader(shader_type)
            .ok_or_else(|| JsValue::from_str("Failed to create shader"))?;

        self.gl.shader_source(&shader, source);
        self.gl.compile_shader(&shader);

        if self
            .gl
            .get_shader_parameter(&shader, WebGl2RenderingContext::COMPILE_STATUS)
            .as_bool()
            .unwrap_or(false)
        {
            Ok(shader)
        } else {
            let log = self
                .gl
                .get_shader_info_log(&shader)
                .unwrap_or_else(|| "Unknown error".to_string());
            self.gl.delete_shader(Some(&shader));
            Err(JsValue::from_str(&format!("Shader compilation failed: {}", log)))
        }
    }

    /// Link a program from vertex and fragment shaders
    pub fn link_program(
        &self,
        vertex_shader: &WebGlShader,
        fragment_shader: &WebGlShader,
    ) -> Result<WebGlProgram, JsValue> {
        let program = self
            .gl
            .create_program()
            .ok_or_else(|| JsValue::from_str("Failed to create program"))?;

        self.gl.attach_shader(&program, vertex_shader);
        self.gl.attach_shader(&program, fragment_shader);
        self.gl.link_program(&program);

        if self
            .gl
            .get_program_parameter(&program, WebGl2RenderingContext::LINK_STATUS)
            .as_bool()
            .unwrap_or(false)
        {
            Ok(program)
        } else {
            let log = self
                .gl
                .get_program_info_log(&program)
                .unwrap_or_else(|| "Unknown error".to_string());
            self.gl.delete_program(Some(&program));
            Err(JsValue::from_str(&format!("Program linking failed: {}", log)))
        }
    }

    /// Create a buffer
    pub fn create_buffer(&self) -> Result<WebGlBuffer, JsValue> {
        self.gl
            .create_buffer()
            .ok_or_else(|| JsValue::from_str("Failed to create buffer"))
    }

    /// Create a vertex array object
    pub fn create_vertex_array(&self) -> Result<WebGlVertexArrayObject, JsValue> {
        self.gl
            .create_vertex_array()
            .ok_or_else(|| JsValue::from_str("Failed to create VAO"))
    }

    /// Create a texture
    pub fn create_texture(&self) -> Result<WebGlTexture, JsValue> {
        self.gl
            .create_texture()
            .ok_or_else(|| JsValue::from_str("Failed to create texture"))
    }

    /// Create a framebuffer
    pub fn create_framebuffer(&self) -> Result<WebGlFramebuffer, JsValue> {
        self.gl
            .create_framebuffer()
            .ok_or_else(|| JsValue::from_str("Failed to create framebuffer"))
    }

    /// Get uniform location
    pub fn get_uniform_location(
        &self,
        program: &WebGlProgram,
        name: &str,
    ) -> Option<WebGlUniformLocation> {
        self.gl.get_uniform_location(program, name)
    }

    /// Upload RGBA texture data
    pub fn upload_texture_rgba(
        &self,
        texture: &WebGlTexture,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<(), JsValue> {
        self.gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(texture));

        // Set texture parameters for pixel-perfect rendering
        self.gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_MIN_FILTER,
            WebGl2RenderingContext::NEAREST as i32,
        );
        self.gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_MAG_FILTER,
            WebGl2RenderingContext::NEAREST as i32,
        );
        self.gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_WRAP_S,
            WebGl2RenderingContext::CLAMP_TO_EDGE as i32,
        );
        self.gl.tex_parameteri(
            WebGl2RenderingContext::TEXTURE_2D,
            WebGl2RenderingContext::TEXTURE_WRAP_T,
            WebGl2RenderingContext::CLAMP_TO_EDGE as i32,
        );

        // Upload texture data
        self.gl
            .tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                WebGl2RenderingContext::TEXTURE_2D,
                0,
                WebGl2RenderingContext::RGBA as i32,
                width as i32,
                height as i32,
                0,
                WebGl2RenderingContext::RGBA,
                WebGl2RenderingContext::UNSIGNED_BYTE,
                Some(data),
            )?;

        self.gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
        Ok(())
    }

    /// Set blend mode for additive blending (ink 33)
    pub fn set_blend_additive(&self) {
        // Ensure blend equation is FUNC_ADD (additive needs: dst + src)
        self.gl.blend_equation(WebGl2RenderingContext::FUNC_ADD);
        // Use separate blend for RGB and Alpha:
        // - RGB: ONE, ONE (additive: result = src + dst)
        // - Alpha: ZERO, ONE (preserve destination alpha)
        // This prevents AddPin from modifying the alpha channel,
        // which can cause issues when AddPin is drawn on top of
        // semi-transparent sprites (like blend < 100).
        self.gl.blend_func_separate(
            WebGl2RenderingContext::ONE,           // srcRGB
            WebGl2RenderingContext::ONE,           // dstRGB
            WebGl2RenderingContext::ZERO,          // srcAlpha
            WebGl2RenderingContext::ONE,           // dstAlpha
        );
    }

    /// Set blend mode for standard alpha blending (ink 0)
    pub fn set_blend_alpha(&self) {
        // Always reset blend equation to FUNC_ADD in case a previous sprite
        // used a different equation (Lighten uses MAX, SubPin uses REVERSE_SUBTRACT)
        self.gl.blend_equation(WebGl2RenderingContext::FUNC_ADD);
        // Use separate blend for RGB and Alpha:
        // - RGB: SRC_ALPHA, ONE_MINUS_SRC_ALPHA (standard alpha blend)
        // - Alpha: ZERO, ONE (preserve destination alpha = 1.0)
        // In Director, the stage is always opaque. The blend value controls
        // color mixing but the framebuffer alpha should stay at 1.0.
        self.gl.blend_func_separate(
            WebGl2RenderingContext::SRC_ALPHA,           // srcRGB
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA, // dstRGB
            WebGl2RenderingContext::ZERO,                // srcAlpha (ignore source alpha)
            WebGl2RenderingContext::ONE,                 // dstAlpha (preserve dest alpha)
        );
    }

    /// Set blend mode for multiply (ink 41 - darken)
    pub fn set_blend_multiply(&self) {
        // Ensure blend equation is FUNC_ADD for multiply blend
        self.gl.blend_equation(WebGl2RenderingContext::FUNC_ADD);
        // Preserve destination alpha (same fix as other blend modes)
        self.gl.blend_func_separate(
            WebGl2RenderingContext::DST_COLOR,  // srcRGB
            WebGl2RenderingContext::ZERO,       // dstRGB
            WebGl2RenderingContext::ZERO,       // srcAlpha
            WebGl2RenderingContext::ONE,        // dstAlpha
        );
    }

    /// Set blend mode for lighten (ink 40)
    /// Uses MAX blend equation to only show lighter pixels
    pub fn set_blend_lighten(&self) {
        self.gl.blend_equation(WebGl2RenderingContext::MAX);
        // Preserve destination alpha (same fix as AddPin)
        self.gl.blend_func_separate(
            WebGl2RenderingContext::ONE,           // srcRGB
            WebGl2RenderingContext::ONE,           // dstRGB
            WebGl2RenderingContext::ZERO,          // srcAlpha
            WebGl2RenderingContext::ONE,           // dstAlpha
        );
    }

    /// Set blend mode for subtractive blending (ink 35 - Sub Pin)
    /// Uses FUNC_REVERSE_SUBTRACT equation: result = dst - src
    pub fn set_blend_subtractive(&self) {
        self.gl.blend_equation(WebGl2RenderingContext::FUNC_REVERSE_SUBTRACT);
        // Preserve destination alpha (same fix as AddPin)
        self.gl.blend_func_separate(
            WebGl2RenderingContext::ONE,           // srcRGB
            WebGl2RenderingContext::ONE,           // dstRGB
            WebGl2RenderingContext::ZERO,          // srcAlpha
            WebGl2RenderingContext::ONE,           // dstAlpha
        );
    }

    /// Reset blend equation to default (used after lighten/subtractive)
    pub fn reset_blend_equation(&self) {
        self.gl.blend_equation(WebGl2RenderingContext::FUNC_ADD);
    }
}
