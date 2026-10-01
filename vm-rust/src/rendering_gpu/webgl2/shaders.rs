//! Shader management for WebGL2 renderer
//!
//! This module provides GLSL shaders for rendering sprites with
//! Director's ink modes implemented as fragment shaders.

use std::collections::HashMap;
use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext, WebGlProgram, WebGlShader, WebGlUniformLocation};

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
    /// Ink 37: Light - MAX(src, dst) per channel
    Light,
    /// Ink 39: Dark - MIN(src, dst) per channel
    Dark,
    /// Ink 40: Lighten
    Lighten,
    /// Ink 41: Darken - multiply blend
    Darken,
    /// Ink 2: Not Copy / Reverse - invert colors
    Reverse,
    /// Ink 3: Ghost - background transparent, foreground inverted
    Ghost,
}

impl InkMode {
    /// Does this ink's shader compare the sampled texel against `u_bg_color`?
    ///
    /// These inks decide a pixel's fate from its exact colour — usually
    /// `discard` on a near-exact match with the sprite's bgColor. That test is
    /// only meaningful on a texel the source actually contains, so a draw using
    /// one of them must POINT-sample: a filtered sample invents in-between
    /// colours along every colour-key boundary, and those survive the key.
    ///
    /// The Dirty Duck pub's `pub_light_02` is the case that found this — a
    /// 292x292 black glow on a white (keyed) field, drawn into a 408x186 sprite.
    /// The 1.57x vertical shrink opted it into mipmapped minification, which
    /// averaged the white surround with the dark disc, and the resulting greys
    /// missed the key and went through AddPin's additive blend as a bright ring
    /// around the light.
    pub fn keys_on_bg_color(self) -> bool {
        matches!(
            self,
            InkMode::BackgroundTransparent
                | InkMode::Ghost
                | InkMode::NotGhost
                | InkMode::Darken
                | InkMode::AddPin
                | InkMode::SubPin
                | InkMode::Lighten
                | InkMode::Reverse
        )
    }

    /// Convert Director ink number to InkMode
    pub fn from_ink_number(ink: i32) -> Self {
        match ink {
            36 => InkMode::BackgroundTransparent,
            3 => InkMode::Ghost,
            7 => InkMode::NotGhost,
            8 => InkMode::Matte,
            9 => InkMode::Mask,
            33 => InkMode::AddPin,
            35 => InkMode::SubPin,
            37 => InkMode::Light,
            39 => InkMode::Dark,
            40 => InkMode::Lighten,
            41 => InkMode::Darken,
            2 => InkMode::Reverse,
            // Ink 6 (Not Reverse): like Reverse, the background color is
            // transparent. dirplayer has no distinct "not reverse" compositing
            // (the CPU path falls through to a normal blend), so render it as a
            // background-transparent copy — the visible fix Director users
            // expect is the transparent background, not a colour inversion.
            6 => InkMode::BackgroundTransparent,
            // Ink 38 (Subtract): dst - src with the background color transparent.
            // Visually identical to Sub Pin (35) on the GPU — both clamp to 0 via
            // GL_FUNC_REVERSE_SUBTRACT — so reuse it (SubPin already color-keys
            // the bgColor). Without this, ink 38 fell back to opaque Copy.
            38 => InkMode::SubPin,
            // Ink 34 (Add): dst + src with the background transparent. Same GPU
            // result as Add Pin (33) — both clamp to 255 via additive blend and
            // color-key the bgColor — so reuse it. (Was opaque Copy.)
            34 => InkMode::AddPin,
            // Ink 4 (Not Copy): an inverted copy with the background transparent.
            // The Reverse shader already does invert-RGB + bgColor color-key,
            // which matches Not Copy's "inverted source, background out". (Was
            // opaque Copy — showing the white bounding box.)
            4 => InkMode::Reverse,
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
    /// 1.0 = sample a shrunk sprite at floor(d * src / dst) (see sampleSprite).
    pub u_floor_rule: Option<WebGlUniformLocation>,
    /// Continuous skew shear, in radians. The vertex shader applies a
    /// horizontal shear `x += y * tan(skew)` around the registration point
    /// before rotation. The special `skew_flip` (±180°) is handled
    /// separately because tan(180°) = 0 wouldn't preserve Director's
    /// vertical-flip behaviour at that exact value.
    pub u_skew: Option<WebGlUniformLocation>,
    /// Device pixels per movie pixel for an authored bitmap (see sampleSprite).
    pub u_stage_scale: Option<WebGlUniformLocation>,
}

const SAMPLE_SPRITE_GLSL: &str = r#"// Director picks the source texel of a scaled sprite as floor(d * src / dst),
// the top-left corner of the destination pixel, where GPU nearest sampling
// picks the texel under the pixel CENTRE. Measured on a ruler bitmap
// (239x46 drawn at 166x32): the floor rule reproduces the projector's
// pixels 100%, the centre rule 88%, and the difference is the digit rows
// the centre rule skips. At 1:1 both rules pick the same texel.
//
// src and dst sizes come from the same uniforms the vertex shader placed
// the quad with, so they are exact. A first version derived the step from
// texcoord derivatives; those carry about 1e-5 relative error, which at
// row 196 of a 1024x768 backdrop already rounded to the texel above.
// A rotated or skewed sprite keeps the centre rule so its resampling
// stays symmetric.
uniform vec4 u_sprite_rect;
uniform vec4 u_tex_rect;
uniform float u_rotation;
uniform float u_skew_flip;
uniform float u_skew;
uniform float u_floor_rule;
// Colour key of the ink (shaders that key define SAMPLE_KEYS_BG): a texel
// within u_color_tolerance of u_bg_color is transparent.
uniform vec4 u_bg_color;
uniform float u_color_tolerance;
// Device pixels per movie pixel for an authored bitmap on a stage drawn
// larger than the movie rect (swStretchStyle). At (1,1) sampleSprite is the
// measured path below, unchanged. Above 1 a device pixel takes the bilinear
// mix of the four movie-pixel samples around it: the same picture a 1:1
// stage gives once the browser scales it up, while text and shapes, which
// are rasterised at device size and sample with (1,1), stay sharp. The mix
// is premultiplied by each sample's alpha (and its colour key), so a keyed
// or transparent neighbour lends no colour to an edge.
uniform vec2 u_stage_scale;

// The sample a 1:1 stage draws at movie pixel d of the sprite.
vec4 moviePixel(vec2 d, vec2 dstMovie, vec2 srcSize, vec2 ts) {
    d = clamp(d, vec2(0.0), dstMovie - vec2(1.0));
    if (u_floor_rule < 0.5 || dstMovie.x > srcSize.x || dstMovie.y > srcSize.y) {
        vec2 tc = u_tex_rect.xy + ((d + 0.5) / dstMovie) * u_tex_rect.zw;
        return texture(u_texture, tc);
    }
    vec2 src = floor(d * srcSize / dstMovie + 1e-3);
    ivec2 texel = ivec2(floor(u_tex_rect.xy * ts + 1e-3)) + ivec2(src);
    texel = clamp(texel, ivec2(0), ivec2(ts) - ivec2(1));
    return texelFetch(u_texture, texel, 0);
}

vec4 premultiplied(vec4 c) {
#ifdef SAMPLE_KEYS_BG
    vec3 diff = abs(c.rgb - u_bg_color.rgb);
    if (max(max(diff.r, diff.g), diff.b) < u_color_tolerance) return vec4(0.0);
#endif
    return vec4(c.rgb * c.a, c.a);
}

vec4 sampleSpriteScaled(vec2 tc) {
    vec2 ts = vec2(textureSize(u_texture, 0));
    vec2 srcSize = abs(u_tex_rect.zw) * ts;
    vec2 dstMovie = max(floor(abs(u_sprite_rect.zw) / u_stage_scale + 0.5), vec2(1.0));
    vec2 rel = (tc - u_tex_rect.xy) / u_tex_rect.zw;
    vec2 p = rel * dstMovie - 0.5;
    vec2 i0 = floor(p);
    vec2 f = p - i0;
    vec4 s00 = premultiplied(moviePixel(i0, dstMovie, srcSize, ts));
    vec4 s10 = premultiplied(moviePixel(i0 + vec2(1.0, 0.0), dstMovie, srcSize, ts));
    vec4 s01 = premultiplied(moviePixel(i0 + vec2(0.0, 1.0), dstMovie, srcSize, ts));
    vec4 s11 = premultiplied(moviePixel(i0 + vec2(1.0, 1.0), dstMovie, srcSize, ts));
    vec4 m = mix(mix(s00, s10, f.x), mix(s01, s11, f.x), f.y);
    if (m.a < 0.001) return vec4(0.0);
    return vec4(m.rgb / m.a, m.a);
}

vec4 sampleSprite(vec2 tc) {
    if (abs(u_rotation) > 0.001 || u_skew_flip > 0.5 || abs(u_skew) > 0.001) {
        return texture(u_texture, tc);
    }
    if (u_stage_scale.x > 1.001 || u_stage_scale.y > 1.001) {
        return sampleSpriteScaled(tc);
    }
    vec2 ts = vec2(textureSize(u_texture, 0));
    vec2 srcSize = abs(u_tex_rect.zw) * ts;
    vec2 dstSize = max(abs(u_sprite_rect.zw), vec2(1.0));
    // The floor rule is measured for an authored bitmap scaling DOWN
    // (u_floor_rule). Text, field and shape textures keep the pixel-centre
    // rule, and so does scaling up: unmeasured, and the floor rule there
    // shifts a sprite by up to half a source pixel.
    if (u_floor_rule < 0.5 || dstSize.x > srcSize.x || dstSize.y > srcSize.y) {
        return texture(u_texture, tc);
    }
    // Position within the sprite, 0..1; a pixel centre sits at (d + 0.5) / dst,
    // so floor has half a pixel of margin when recovering d.
    vec2 rel = (tc - u_tex_rect.xy) / u_tex_rect.zw;
    vec2 d = floor(rel * dstSize);
    vec2 src = floor(d * srcSize / dstSize + 1e-3);
    ivec2 texel = ivec2(floor(u_tex_rect.xy * ts + 1e-3)) + ivec2(src);
    texel = clamp(texel, ivec2(0), ivec2(ts) - ivec2(1));
    return texelFetch(u_texture, texel, 0);
}"#;

/// Manages shader programs for different ink modes
pub struct ShaderManager {
    programs: HashMap<InkMode, ShaderProgram>,
    active_ink: Option<InkMode>,
}

/// A program whose compile and link were issued but not yet waited for.
struct PendingProgram {
    program: WebGlProgram,
    vert: WebGlShader,
    frag: WebGlShader,
}

impl ShaderManager {
    /// Create shader manager and compile all shaders
    pub fn new(context: &WebGL2Context) -> Result<Self, JsValue> {
        // Start every program before asking about any of them. A status or
        // uniform query waits for that program's compile and link to finish,
        // so querying each one as it was made built the twelve programs one
        // after another: a first visit on Windows (ANGLE, no shader cache yet)
        // spent 420-500 ms blocked here. Issued together, the driver works
        // on them in parallel and the queries below mostly find them done.
        // KHR_parallel_shader_compile, where offered, lets it use threads.
        let _ = context.gl().get_extension("KHR_parallel_shader_compile");
        let pending = vec![
            (InkMode::Copy, Self::compile_ink_copy(context)?),
            (InkMode::BackgroundTransparent, Self::compile_ink_bg_transparent(context)?),
            (InkMode::AddPin, Self::compile_ink_add_pin(context)?),
            (InkMode::SubPin, Self::compile_ink_sub_pin(context)?),
            (InkMode::Darken, Self::compile_ink_darken(context)?),
            (InkMode::NotGhost, Self::compile_ink_not_ghost(context)?),
            (InkMode::Matte, Self::compile_ink_matte(context)?),
            (InkMode::Lighten, Self::compile_ink_lighten(context)?),
            (InkMode::Reverse, Self::compile_ink_reverse(context)?),
            (InkMode::Light, Self::compile_ink_light(context)?),
            (InkMode::Dark, Self::compile_ink_dark(context)?),
            (InkMode::Ghost, Self::compile_ink_ghost(context)?),
        ];
        let mut programs = HashMap::new();
        for (ink, program) in pending {
            programs.insert(ink, Self::finish_program(context, program)?);
        }

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
uniform float u_skew;        // continuous horizontal-shear angle, in RADIANS

out vec2 v_texcoord;

void main() {
    // Position is NOT flipped - quad stays in same screen location
    vec2 pos = a_position;

    // Scale and translate to sprite rect
    vec2 world_pos = u_sprite_rect.xy + pos * u_sprite_rect.zw;

    // Apply rotation around registration point (sprite's loc)
    // Director rotates around the registration point, not the sprite center
    bool needs_skew = abs(u_skew) > 0.001;
    if (abs(u_rotation) > 0.001 || u_skew_flip > 0.5 || needs_skew) {
        vec2 center = u_rotation_center;
        world_pos -= center;

        // Apply skew flip: negate y before rotation
        // This matches the software path in drawing.rs
        // rotation=180 alone: (x,y) -> (-x,-y) = upside down
        // rotation=180 + skew=180: (x,-y) -> (-x,y) = left-right mirror
        if (u_skew_flip > 0.5) {
            world_pos.y = -world_pos.y;
        }

        // Continuous horizontal shear (Director's `the skew`). Apply BEFORE
        // rotation so the rectangle is parallelogram'd in sprite-local space
        // and then the parallelogram is rotated around the registration
        // point — matches Director's transform order.
        // Sign: positive Director skew shifts the TOP of the sprite to the
        // RIGHT (and bottom to the left) in screen coords with Y-down.
        // Verified against CS RemoteControlCamera CRT (skew=26, rot=-26):
        // the +x += y*tan path leaned the parallelogram opposite to the
        // original Director output, so we negate to match.
        if (needs_skew) {
            world_pos.x -= world_pos.y * tan(u_skew);
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
    fn compile_ink_copy(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;  // 0.0 to 1.0

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Discard fully transparent pixels (matte info baked into alpha)
    if (src.a < 0.01) discard;

    // Apply blend (Director blend 0-100 maps to 0.0-1.0)
    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 36 (Background Transparent) shader
    fn compile_ink_bg_transparent(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
#define SAMPLE_KEYS_BG 1
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Discard fully transparent pixels (bgColor already baked as alpha=0 in texture)
    if (src.a < 0.01) discard;

    // For non-indexed bitmaps (16/32-bit), also do runtime color-key as fallback
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 33 (Add Pin) shader
    /// Director Add Pin: Color-key transparency + additive blending
    /// Pixels matching bgColor are transparent, others are additively blended
    fn compile_ink_add_pin(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
#define SAMPLE_KEYS_BG 1
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Color-key transparency: discard pixels matching bgColor. The
    // tolerance is 0 when the bitmap carries its own alpha, so a white
    // pixel inside an alpha-masked image keeps adding (see mod.rs).
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    // Also discard fully transparent pixels (from embedded alpha)
    if (src.a < 0.01) discard;

    // Add Pin: src.rgb * blend additively contributes, modulated by the
    // bitmap's per-pixel alpha. Output `src.a` (not 1.0) so the
    // glBlendFunc(SRC_ALPHA, ONE) blend func correctly weighs each pixel
    // by its authored opacity — bright bulb pixels (alpha=255) contribute
    // fully, gradient edges (alpha=3..128) contribute proportionally,
    // alpha=0 leftover RGB doesn't contribute at all (the lighthouse buoy
    // halo bug). For sprites with alpha=255 everywhere this degenerates
    // to straight additive, preserving the moodlight halo behaviour.
    fragColor = vec4(src.rgb * u_blend, src.a);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 35 (Sub Pin) shader
    /// Director Sub Pin: Subtract source from destination, pin to 0 (black)
    /// bgColor pixels are transparent
    fn compile_ink_sub_pin(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

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

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 41 (Darken) shader
    /// Director Darken: result = src * bgColor, then alpha-blended with destination
    /// This multiplies the source color by the background color, creating a tinting/darkening effect.
    /// Uses standard alpha blending (not GL_DST_COLOR multiply blend).
    fn compile_ink_darken(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_fg_color;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Director ink 41 (Darken): a foreColor/bgColor remap applied PER CHANNEL,
    // result.c = mix(fg.c, bg.c, src.c). For the common case (foreColor black,
    // bgColor white) this is the identity, so COLOR bitmaps keep their colors
    // (e.g. the Habbo catalogue's palette-swapped wall/floor decorations).
    // For a grayscale source (r==g==b) it is identical to a luminance duotone,
    // so Habbo's camera photo (color=dark, bgColor=light) still maps to its
    // sepia look. Using luminance here instead collapsed COLOR bitmaps to gray.
    vec3 duotone = mix(u_fg_color.rgb, u_bg_color.rgb, src.rgb);

    float alpha = src.a * u_blend;
    fragColor = vec4(duotone, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 7 (Not Ghost) shader
    /// Not Ghost: Opposite of background-transparent - NON-bgColor pixels are transparent.
    /// `*dst = (src == colorWhite) ? backColor : *dst`
    /// - If src matches bgColor (white): output backColor
    /// - If src does NOT match bgColor: leave dst unchanged (discard/transparent)
    /// This makes foreground pixels (like a black door) transparent,
    /// while background pixels either blend or are masked by matte.
    fn compile_ink_not_ghost(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

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

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 8 (Matte) shader
    /// Matte: Uses alpha channel from texture (flood-fill matte baked in during texture upload)
    /// Edge-connected background pixels have alpha=0, interior pixels stay opaque
    /// This matches Canvas2D behavior where only edge-connected bgColor pixels are transparent
    fn compile_ink_matte(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Matte: transparency comes from alpha channel (flood-fill matte baked in)
    // Discard fully transparent pixels (edge-connected background)
    if (src.a < 0.01) discard;

    // Standard alpha blending
    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 40 (Lighten) shader
    /// Lighten: Only draws pixels that are lighter than the destination
    /// Note: This requires reading the framebuffer which isn't directly possible,
    /// so we use a MAX blend equation instead
    fn compile_ink_lighten(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
#define SAMPLE_KEYS_BG 1
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;
uniform vec4 u_fg_color;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Color-key transparency: discard if matches bg_color
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    // Lighten: the sprite's foreColor is added to the image (Using
    // Director, "Using sprite inks"); the default black foreColor leaves
    // it unchanged.
    float alpha = src.a * u_blend;
    fragColor = vec4(min(src.rgb + u_fg_color.rgb, 1.0), alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 2 (Reverse) shader — inverts RGB, preserves alpha, and
    /// color-keys the background color (Director's Reverse ink treats the
    /// bgColor as transparent; the CPU path mattes it via should_matte_sprite).
    /// Without the color-key the sprite drew an opaque inverted rectangle.
    fn compile_ink_reverse(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
#define SAMPLE_KEYS_BG 1
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    if (src.a < 0.01) discard;

    // Color-key transparency: discard pixels matching the sprite's bgColor.
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    // Invert RGB, keep alpha
    float alpha = src.a * u_blend;
    fragColor = vec4(1.0 - src.r, 1.0 - src.g, 1.0 - src.b, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

        /// Compile Ink 37 (Light) shader
    /// Light: MAX(src, dst) per channel. bgColor pixels are transparent.
    /// Uses GL_MAX blend equation for per-channel max.
    fn compile_ink_light(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
#define SAMPLE_KEYS_BG 1
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Color-key transparency: discard if matches bg_color
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    // Light: output color, actual MAX blending done via blend equation
    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 39 (Dark) shader
    /// Dark: MIN(src, dst) per channel. bgColor pixels are transparent.
    /// Uses GL_MIN blend equation for per-channel min.
    fn compile_ink_dark(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
#define SAMPLE_KEYS_BG 1
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Color-key transparency: discard if matches bg_color
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    // Dark: output color, actual MIN blending done via blend equation
    float alpha = src.a * u_blend;
    fragColor = vec4(src.rgb, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Compile Ink 3 (Ghost) shader
    /// Ghost ink makes background-color pixels transparent and inverts foreground pixels.
    /// Original Director behavior: dst = dst & ~src on palette indices.
    /// WebGL approximation: color-key bg + invert RGB for foreground.
    fn compile_ink_ghost(context: &WebGL2Context) -> Result<PendingProgram, JsValue> {
        let frag_source = r#"#version 300 es
#define SAMPLE_KEYS_BG 1
precision highp float;

in vec2 v_texcoord;

uniform sampler2D u_texture;
uniform float u_blend;

out vec4 fragColor;

//@SAMPLE_SPRITE@

void main() {
    vec4 src = sampleSprite(v_texcoord);

    // Discard fully transparent pixels (from matte mask)
    if (src.a < 0.01) discard;

    // Color-key transparency: discard if matches bg_color
    vec3 diff = abs(src.rgb - u_bg_color.rgb);
    float dist = max(max(diff.r, diff.g), diff.b);
    if (dist < u_color_tolerance) discard;

    // Ghost effect: invert the foreground colors
    float alpha = src.a * u_blend;
    fragColor = vec4(1.0 - src.rgb, alpha);
}
"#;

        Self::start_program(context, Self::vertex_shader_source(), frag_source)
    }

    /// Start compiling and linking a shader program without waiting for it.
    fn start_program(
        context: &WebGL2Context,
        vert_source: &str,
        frag_source: &str,
    ) -> Result<PendingProgram, JsValue> {
        let gl = context.gl();
        let frag_source = frag_source.replace("//@SAMPLE_SPRITE@", SAMPLE_SPRITE_GLSL);
        let make = |kind: u32, source: &str| -> Result<WebGlShader, JsValue> {
            let shader = gl
                .create_shader(kind)
                .ok_or_else(|| JsValue::from_str("Failed to create shader"))?;
            gl.shader_source(&shader, source);
            gl.compile_shader(&shader);
            Ok(shader)
        };
        let vert = make(WebGl2RenderingContext::VERTEX_SHADER, vert_source)?;
        let frag = make(WebGl2RenderingContext::FRAGMENT_SHADER, &frag_source)?;
        let program = gl
            .create_program()
            .ok_or_else(|| JsValue::from_str("Failed to create program"))?;
        gl.attach_shader(&program, &vert);
        gl.attach_shader(&program, &frag);
        gl.link_program(&program);
        Ok(PendingProgram { program, vert, frag })
    }

    /// Wait for a started program, report a failure with the shader logs,
    /// and read its uniform locations.
    fn finish_program(
        context: &WebGL2Context,
        pending: PendingProgram,
    ) -> Result<ShaderProgram, JsValue> {
        let gl = context.gl();
        let PendingProgram { program, vert, frag } = pending;
        let linked = gl
            .get_program_parameter(&program, WebGl2RenderingContext::LINK_STATUS)
            .as_bool()
            .unwrap_or(false);
        if !linked {
            let mut log = String::new();
            for (name, shader) in [("vertex", &vert), ("fragment", &frag)] {
                let compiled = gl
                    .get_shader_parameter(shader, WebGl2RenderingContext::COMPILE_STATUS)
                    .as_bool()
                    .unwrap_or(false);
                if !compiled {
                    log.push_str(&format!(
                        "Shader compilation failed ({}): {}\n",
                        name,
                        gl.get_shader_info_log(shader).unwrap_or_default()
                    ));
                }
            }
            log.push_str(&format!(
                "Program linking failed: {}",
                gl.get_program_info_log(&program).unwrap_or_default()
            ));
            gl.delete_shader(Some(&vert));
            gl.delete_shader(Some(&frag));
            gl.delete_program(Some(&program));
            return Err(JsValue::from_str(&log));
        }

        // Clean up shaders after linking
        gl.delete_shader(Some(&vert));
        gl.delete_shader(Some(&frag));

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
        let u_floor_rule = gl.get_uniform_location(&program, "u_floor_rule");
        let u_skew = gl.get_uniform_location(&program, "u_skew");
        let u_stage_scale = gl.get_uniform_location(&program, "u_stage_scale");

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
            u_floor_rule,
            u_skew,
            u_stage_scale,
        })
    }
}
