//! Shader management for WebGL2 renderer
//!
//! This module provides GLSL shaders for rendering sprites with
//! Director's ink modes implemented as fragment shaders.

use std::collections::HashMap;
use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext, WebGlProgram, WebGlUniformLocation};

use super::context::WebGL2Context;

/// Director ink modes that require different shaders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InkMode {
    /// Ink 0: Standard copy with alpha blend
    Copy,
    /// Ink 7: Not Ghost - skip bg_color pixels
    NotGhost,
    /// Ink 8: Matte - alpha from matte mask texture
    Matte,
    /// Ink 9: Mask - use mask texture
    Mask,
    /// Ink 33: Add Pin - additive blending
    AddPin,
    /// Ink 35: Sub Pin - subtractive blending
    SubPin,
    /// Ink 36: Background Transparent - color key
    BackgroundTransparent,
    /// Ink 40: Lighten
    Lighten,
    /// Ink 41: Darken - multiply blend
    Darken,
}

impl InkMode {
    /// Convert Director ink number to InkMode
    pub fn from_ink_number(ink: i32) -> Self {
        match ink {
            7 => InkMode::NotGhost,
            8 => InkMode::Matte,
            9 => InkMode::Mask,
            33 => InkMode::AddPin,
            35 => InkMode::SubPin,
            36 => InkMode::BackgroundTransparent,
            40 => InkMode::Lighten,
            41 => InkMode::Darken,
            _ => InkMode::Copy, // Default to copy for unknown inks
        }
    }
}

/// Compiled shader program with uniform locations
pub struct ShaderProgram {
    pub program: WebGlProgram,
    pub u_projection: Option<WebGlUniformLocation>,
    pub u_texture: Option<WebGlUniformLocation>,
    pub u_matte: Option<WebGlUniformLocation>,
    pub u_use_matte: Option<WebGlUniformLocation>,
    pub u_blend: Option<WebGlUniformLocation>,
    pub u_fg_color: Option<WebGlUniformLocation>,
    pub u_bg_color: Option<WebGlUniformLocation>,
    pub u_color_tolerance: Option<WebGlUniformLocation>,
    // Per-sprite transformation uniforms
    pub u_sprite_rect: Option<WebGlUniformLocation>,
    pub u_tex_rect: Option<WebGlUniformLocation>,
    pub u_flip: Option<WebGlUniformLocation>,
    pub u_rotation: Option<WebGlUniformLocation>,
    pub u_rotation_center: Option<WebGlUniformLocation>,
    pub u_skew_flip: Option<WebGlUniformLocation>,
}

/// Manages shader programs for different ink modes
pub struct ShaderManager {
    programs: HashMap<InkMode, ShaderProgram>,
    active_ink: Option<InkMode>,
}

impl ShaderManager {
    /// Create shader manager and compile all shaders
    pub fn new(context: &WebGL2Context) -> Result<Self, JsValue> {
        let mut programs = HashMap::new();

        // Compile shader for each ink mode
        programs.insert(InkMode::Copy, Self::compile_ink_copy(context)?);
        programs.insert(InkMode::BackgroundTransparent, Self::compile_ink_bg_transparent(context)?);
        programs.insert(InkMode::AddPin, Self::compile_ink_add_pin(context)?);
        programs.insert(InkMode::SubPin, Self::compile_ink_sub_pin(context)?);
        programs.insert(InkMode::Darken, Self::compile_ink_darken(context)?);
        programs.insert(InkMode::NotGhost, Self::compile_ink_not_ghost(context)?);
        programs.insert(InkMode::Matte, Self::compile_ink_matte(context)?);
        programs.insert(InkMode::Lighten, Self::compile_ink_lighten(context)?);

        Ok(Self {
            programs,
            active_ink: None,
        })
    }

    /// Get shader program for ink mode, falling back to Copy if not found
    pub fn get_program(&self, ink: InkMode) -> Option<&ShaderProgram> {
        self.programs.get(&ink).or_else(|| self.programs.get(&InkMode::Copy))
    }

    /// Get the effective ink mode (the one that will actually be used after fallback)
    fn effective_ink(&self, ink: InkMode) -> InkMode {
        if self.programs.contains_key(&ink) {
            ink
        } else {
            InkMode::Copy
        }
    }

    /// Use shader program for ink mode, returns the effective ink mode used
    pub fn use_program(&mut self, context: &WebGL2Context, ink: InkMode) -> InkMode {
        let effective = self.effective_ink(ink);
        if self.active_ink != Some(effective) {
            if let Some(program) = self.programs.get(&effective) {
                context.gl().use_program(Some(&program.program));
                self.active_ink = Some(effective);
            }
        }
        effective
    }

    /// Clear the active shader state (call after external code changes GL program)
    pub fn clear_active(&mut self) {
        self.active_ink = None;
    }

    /// Common vertex shader for all ink modes
    fn vertex_shader_source() -> &'static str {
        r#"#version 300 es
precision highp float;

layout(location = 0) in vec2 a_position;
layout(location = 1) in vec2 a_texcoord;

uniform mat4 u_projection;

// Per-sprite uniforms (will use instancing later)
uniform vec4 u_sprite_rect;  // x, y, width, height
uniform vec4 u_tex_rect;     // src tex coords
uniform vec2 u_flip;         // flip x, flip y
uniform float u_rotation;
uniform vec2 u_rotation_center;  // sprite's loc (registration point)
uniform float u_skew_flip;   // 1.0 = negate y before rotation (for skew=180 effect)

out vec2 v_texcoord;

void main() {
    // Position is NOT flipped - quad stays in same screen location
    vec2 pos = a_position;

    // Scale and translate to sprite rect
    vec2 world_pos = u_sprite_rect.xy + pos * u_sprite_rect.zw;

    // Apply rotation around registration point (sprite's loc)
    // Director rotates around the registration point, not the sprite center
    if (abs(u_rotation) > 0.001 || u_skew_flip > 0.5) {
        vec2 center = u_rotation_center;
        world_pos -= center;

        // Apply skew flip: negate y before rotation
        // This matches the software path in drawing.rs
        // rotation=180 alone: (x,y) -> (-x,-y) = upside down
        // rotation=180 + skew=180: (x,-y) -> (-x,y) = left-right mirror
        if (u_skew_flip > 0.5) {
            world_pos.y = -world_pos.y;
        }

        float c = cos(u_rotation);
        float s = sin(u_rotation);
        world_pos = vec2(world_pos.x * c - world_pos.y * s,
                         world_pos.x * s + world_pos.y * c);
        world_pos += center;
    }

    gl_Position = u_projection * vec4(world_pos, 0.0, 1.0);

    // Flip is applied to texture coordinates only (samples from opposite side)
    vec2 tc = a_texcoord;
    if (u_flip.x > 0.5) tc.x = 1.0 - tc.x;
    if (u_flip.y > 0.5) tc.y = 1.0 - tc.y;
    v_texcoord = u_tex_rect.xy + tc * u_tex_rect.zw;
}
"#
    }

    /// Compile Ink 0 (Copy) shader
    fn compile_ink_copy(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;  // 0.0 to 1.0

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Discard fully transparent pixels (matte info baked into alpha)
    if (src.a < 0.01) discard;

    // Apply blend (Director blend 0-100 maps to 0.0-1.0)
    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 36 (Background Transparent) shader
    fn compile_ink_bg_transparent(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_bg_color;
uniform float u_color_tolerance;

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Color-key transparency: discard if matches bg_color
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 33 (Add Pin) shader
    /// Director Add Pin: Color-key transparency + additive blending
    /// Pixels matching bgColor are transparent, others are additively blended
    fn compile_ink_add_pin(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_bg_color;

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Color-key transparency: discard pixels matching bgColor
    // Use small threshold for floating point comparison
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    if (diff.r < 0.004 && diff.g < 0.004 && diff.b < 0.004) discard;

    // Also discard fully transparent pixels (from embedded alpha)
    if (src.a < 0.01) discard;

    // Director Add Pin: ONLY uses blend %, ignores bitmap alpha
    // final = dst + src.rgb * blend (with GL_ONE, GL_ONE blend func)
    fragColor = vec4(src.rgb * u_blend, 1.0);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 35 (Sub Pin) shader
    /// Director Sub Pin: Subtract source from destination, pin to 0 (black)
    /// bgColor pixels are transparent
    fn compile_ink_sub_pin(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_bg_color;

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Color-key transparency: discard pixels matching bgColor
    // Use small threshold for floating point comparison
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    if (diff.r < 0.004 && diff.g < 0.004 && diff.b < 0.004) discard;

    // Also discard fully transparent pixels (from embedded alpha)
    if (src.a < 0.01) discard;

    // Director Sub Pin: ONLY uses blend %, ignores bitmap alpha
    // final = dst - src.rgb * blend (with GL_ONE_MINUS_SRC_COLOR, GL_ONE blend func)
    // We output the value to subtract; the blend equation GL_FUNC_REVERSE_SUBTRACT does: dst - src
    fragColor = vec4(src.rgb * u_blend, 1.0);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 41 (Darken) shader
    /// Director Darken: result = src * bgColor, then alpha-blended with destination
    /// This multiplies the source color by the background color, creating a tinting/darkening effect.
    /// Uses standard alpha blending (not GL_DST_COLOR multiply blend).
    fn compile_ink_darken(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_bg_color;

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Director ink 41 (Darken): multiply source by bgColor
    // result_color = src.rgb * bgColor.rgb
    // Then alpha-blend with destination using standard blending
    vec3 darkened = src.rgb * u_bg_color.rgb;

    // Apply blend factor and source alpha
    float alpha = src.a * u_blend;
    fragColor = vec4(darkened, alpha);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 7 (Not Ghost) shader
    /// Not Ghost: Opposite of background-transparent - NON-bgColor pixels are transparent.
    /// From ScummVM: `*dst = (src == colorWhite) ? backColor : *dst`
    /// - If src matches bgColor (white): output backColor
    /// - If src does NOT match bgColor: leave dst unchanged (discard/transparent)
    /// This makes foreground pixels (like a black door) transparent,
    /// while background pixels either blend or are masked by matte.
    fn compile_ink_not_ghost(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_bg_color;
uniform float u_color_tolerance;

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Not Ghost: discard pixels that do NOT match bgColor
    // (opposite of background-transparent which discards bgColor pixels)
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist >= u_color_tolerance) discard;  // Non-bgColor pixels are transparent

    // Pixels matching bgColor: output backColor (which is u_bg_color)
    float alpha = src.a * u_blend;
    fragColor = vec4(u_bg_color.rgb, alpha);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 8 (Matte) shader
    /// Matte: Uses alpha channel from texture (flood-fill matte baked in during texture upload)
    /// Edge-connected background pixels have alpha=0, interior pixels stay opaque
    /// This matches Canvas2D behavior where only edge-connected bgColor pixels are transparent
    fn compile_ink_matte(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Matte: transparency comes from alpha channel (flood-fill matte baked in)
    // Discard fully transparent pixels (edge-connected background)
    if (src.a < 0.01) discard;

    // Standard alpha blending
    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 40 (Lighten) shader
    /// Lighten: Only draws pixels that are lighter than the destination
    /// Note: This requires reading the framebuffer which isn't directly possible,
    /// so we use a MAX blend equation instead
    fn compile_ink_lighten(context: &WebGL2Context) -> Result<ShaderProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_bg_color;
uniform float u_color_tolerance;

out vec4 fragColor;

void main() {
    vec4 src = texture(u_texture, v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Color-key transparency: discard if matches bg_color
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    // Lighten: output color, actual MAX blending done via blend equation
    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::compile_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile and link a shader program
    fn compile_program(
        context: &WebGL2Context,
        vert_source: &str,
        frag_source: &str,
    ) -> Result<ShaderProgram, JsValue> {
        let gl = context.gl();

        let vert_shader = context.compile_shader(
            WebGl2RenderingContext::VERTEX_SHADER,
            vert_source,
        )?;
        let frag_shader = context.compile_shader(
            WebGl2RenderingContext::FRAGMENT_SHADER,
            frag_source,
        )?;

        let program = context.link_program(&vert_shader, &frag_shader)?;

        // Clean up shaders after linking
        gl.delete_shader(Some(&vert_shader));
        gl.delete_shader(Some(&frag_shader));

        // Get uniform locations
        let u_projection = gl.get_uniform_location(&program, "u_projection");
        let u_texture = gl.get_uniform_location(&program, "u_texture");
        let u_matte = gl.get_uniform_location(&program, "u_matte");
        let u_use_matte = gl.get_uniform_location(&program, "u_use_matte");
        let u_blend = gl.get_uniform_location(&program, "u_blend");
        let u_fg_color = gl.get_uniform_location(&program, "u_fg_color");
        let u_bg_color = gl.get_uniform_location(&program, "u_bg_color");
        let u_color_tolerance = gl.get_uniform_location(&program, "u_color_tolerance");
        // Per-sprite transformation uniforms
        let u_sprite_rect = gl.get_uniform_location(&program, "u_sprite_rect");
        let u_tex_rect = gl.get_uniform_location(&program, "u_tex_rect");
        let u_flip = gl.get_uniform_location(&program, "u_flip");
        let u_rotation = gl.get_uniform_location(&program, "u_rotation");
        let u_rotation_center = gl.get_uniform_location(&program, "u_rotation_center");
        let u_skew_flip = gl.get_uniform_location(&program, "u_skew_flip");

        Ok(ShaderProgram {
            program,
            u_projection,
            u_texture,
            u_matte,
            u_use_matte,
            u_blend,
            u_fg_color,
            u_bg_color,
            u_color_tolerance,
            u_sprite_rect,
            u_tex_rect,
            u_flip,
            u_rotation,
            u_rotation_center,
            u_skew_flip,
        })
    }
}
