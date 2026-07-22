//! Shockwave 3D scene renderer using WebGL2
//!
//! Renders W3dScene data to an offscreen FBO, producing a texture
//! that can be composited as a regular sprite in the 2D pipeline.

use std::collections::HashMap;
use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext, WebGlFramebuffer, WebGlProgram, WebGlTexture,
    WebGlUniformLocation,
};

use log::debug;

use super::context::WebGL2Context;
use super::mesh3d::Mesh3dBuffers;
use crate::{
    director::chunks::w3d::types::*,
    console_warn,
};

const SCENE3D_LOG: bool = false;

fn log(msg: &str) {
    if SCENE3D_LOG {
        debug!("[SCENE-3D] {}", msg);
    }
}

/// GPU state for a single Shockwave3D member
struct MemberGpuData {
    /// Mesh buffers keyed by resource name (matches ModelNode.model_resource_name)
    mesh_groups: HashMap<String, Vec<Mesh3dBuffers>>,
    /// All meshes in upload order (fallback when no scene graph match)
    all_meshes: Vec<Mesh3dBuffers>,
    /// Texture images decoded and uploaded to GPU
    textures: HashMap<String, WebGlTexture>,
    /// Texture dimensions (width, height) keyed by lowercase name
    texture_sizes: HashMap<String, (u32, u32)>,
    /// Cube map textures (keyed by base name)
    cube_maps: HashMap<String, WebGlTexture>,
    /// Cached inverse bind matrices per skeleton name
    inverse_bind_cache: HashMap<String, Vec<[f32; 16]>>,
    /// Snapshot of scene content counts when GPU data was built
    scene_version: (usize, usize, usize, usize), // (nodes, clod_meshes, texture_images, shaders)
    /// Scene's mesh_content_version at last upload
    mesh_content_version: u64,
    /// Per-texture data length at upload time (for incremental re-upload detection)
    texture_versions: HashMap<String, u64>,
    /// Scene's texture_content_version at last check
    texture_content_version: u64,
    /// Texture names (lowercase) that contain alpha < 250 (need alpha blending)
    alpha_textures: std::collections::HashSet<String>,
}

/// 3D shader program with uniform locations
struct Shader3d {
    program: WebGlProgram,
    u_model: Option<WebGlUniformLocation>,
    u_view: Option<WebGlUniformLocation>,
    u_projection: Option<WebGlUniformLocation>,
    u_diffuse_color: Option<WebGlUniformLocation>,
    u_has_vertex_color: Option<WebGlUniformLocation>,
    u_ambient_color: Option<WebGlUniformLocation>,
    u_specular_color: Option<WebGlUniformLocation>,
    u_emissive_color: Option<WebGlUniformLocation>,
    u_shininess: Option<WebGlUniformLocation>,
    u_opacity: Option<WebGlUniformLocation>,
    u_alpha_threshold: Option<WebGlUniformLocation>,
    u_diffuse_tex: Option<WebGlUniformLocation>,
    u_has_texture: Option<WebGlUniformLocation>,
    u_texture_unlit: Option<WebGlUniformLocation>,
    u_lightmap_tex: Option<WebGlUniformLocation>,
    u_has_lightmap: Option<WebGlUniformLocation>,
    u_lightmap_intensity: Option<WebGlUniformLocation>,
    u_has_texcoord2: Option<WebGlUniformLocation>,
    u_texcoord2_direct: Option<WebGlUniformLocation>,
    // Layer 2 (third texture layer)
    u_layer2_tex: Option<WebGlUniformLocation>,
    u_layer2_blend: Option<WebGlUniformLocation>,
    u_layer2_intensity: Option<WebGlUniformLocation>,
    // Specular map
    u_specular_tex: Option<WebGlUniformLocation>,
    u_has_specular_map: Option<WebGlUniformLocation>,
    // Environment/cube map (sampler added when cubemaps are loaded)
    u_has_env_map: Option<WebGlUniformLocation>,
    u_reflectivity: Option<WebGlUniformLocation>,
    // Texture coordinate transform (post-projection UV-space tweak)
    u_tex_transform: Option<WebGlUniformLocation>,
    // UV projection mode for the diffuse layer (matches W3dTextureLayer.tex_mode):
    // 0 = mesh UVs (default), 5 = #wrapPlanar (project object-space XY).
    u_uv_proj_mode: Option<WebGlUniformLocation>,
    // wrapTransformList[i] for the diffuse layer — applied to model-space
    // position before generating UVs (used by #wrapPlanar etc.).
    u_wrap_transform: Option<WebGlUniformLocation>,
    // Skeletal skinning
    u_skinning_enabled: Option<WebGlUniformLocation>,
    u_bone_matrices: Option<WebGlUniformLocation>,
    // NPR/toon
    u_shader_mode: Option<WebGlUniformLocation>,
    u_toon_steps: Option<WebGlUniformLocation>,
    // Lighting
    u_num_lights: Option<WebGlUniformLocation>,
    u_light_pos: Option<WebGlUniformLocation>,
    u_light_color: Option<WebGlUniformLocation>,
    u_light_type: Option<WebGlUniformLocation>,
    u_light_atten: Option<WebGlUniformLocation>,
    u_light_dir: Option<WebGlUniformLocation>,
    u_light_spot_angle: Option<WebGlUniformLocation>,
    u_camera_pos: Option<WebGlUniformLocation>,
    u_global_ambient: Option<WebGlUniformLocation>,
    u_fog_enabled: Option<WebGlUniformLocation>,
    u_fog_near: Option<WebGlUniformLocation>,
    u_fog_far: Option<WebGlUniformLocation>,
    u_fog_color: Option<WebGlUniformLocation>,
    u_fog_mode: Option<WebGlUniformLocation>,
}

/// Result of resolving texture layers for a shader
struct TextureLayerBinding<'a> {
    tex: &'a WebGlTexture,
    blend: i32,       // 1=multiply, 2=add, 3=replace, 4=decal
    intensity: f32,
    wrap: (u8, u8),   // (repeat_s, repeat_t): 0=clamp, 1=repeat
}

struct TextureBindResult<'a> {
    diffuse: Option<&'a WebGlTexture>,
    diffuse_tex_transform: [f32; 16], // texture coordinate transform for diffuse layer
    diffuse_wrap_transform: [f32; 16], // wrapTransformList[i] for #wrapPlanar et al.
    diffuse_wrap: (u8, u8), // (repeat_s, repeat_t) for diffuse: 0=clamp, 1=repeat
    diffuse_tex_mode: u8,   // W3dTextureLayer.tex_mode (0=mesh UVs, 5=#wrapPlanar)
    extra_layers: Vec<TextureLayerBinding<'a>>, // up to 2 extra layers (layer1 + layer2)
    specular: Option<&'a WebGlTexture>,
}

/// Particle billboard shader
struct ParticleShader {
    program: WebGlProgram,
    u_view_projection: Option<WebGlUniformLocation>,
    u_camera_right: Option<WebGlUniformLocation>,
    u_camera_up: Option<WebGlUniformLocation>,
    u_color_start: Option<WebGlUniformLocation>,
    u_color_end: Option<WebGlUniformLocation>,
    u_size_start: Option<WebGlUniformLocation>,
    u_size_end: Option<WebGlUniformLocation>,
    u_blend_start: Option<WebGlUniformLocation>,
    u_blend_end: Option<WebGlUniformLocation>,
    u_lifetime: Option<WebGlUniformLocation>,
    u_tex: Option<WebGlUniformLocation>,
    u_has_tex: Option<WebGlUniformLocation>,
}

/// Simple fullscreen quad shader for post-processing passes
struct PostProcessShader {
    program: WebGlProgram,
    u_input_tex: Option<WebGlUniformLocation>,
    u_resolution: Option<WebGlUniformLocation>,
    u_direction: Option<WebGlUniformLocation>,
    u_threshold: Option<WebGlUniformLocation>,
    u_intensity: Option<WebGlUniformLocation>,
    u_mode: Option<WebGlUniformLocation>,
    u_color_matrix: Option<WebGlUniformLocation>,
}

/// Outline/edge shader for ShaderInker NPR effect
struct OutlineShader {
    program: WebGlProgram,
    u_model: Option<WebGlUniformLocation>,
    u_view: Option<WebGlUniformLocation>,
    u_projection: Option<WebGlUniformLocation>,
    u_outline_width: Option<WebGlUniformLocation>,
    u_outline_color: Option<WebGlUniformLocation>,
}

/// Manages 3D rendering for all Shockwave3D members
pub struct Scene3dRenderer {
    shader: Option<Shader3d>,
    particle_shader: Option<ParticleShader>,
    pp_shader: Option<PostProcessShader>,
    outline_shader: Option<OutlineShader>,
    member_data: HashMap<(i32, i32), MemberGpuData>,
    pub fbo: Option<WebGlFramebuffer>,
    pub fbo_texture: Option<WebGlTexture>,
    overlay_quad_vbo: Option<web_sys::WebGlBuffer>,
    overlay_quad_uv: Option<web_sys::WebGlBuffer>,
    fbo_depth: Option<web_sys::WebGlRenderbuffer>,
    fbo_width: u32,
    fbo_height: u32,
    // Bloom post-processing FBOs (half resolution)
    bloom_fbo_a: Option<WebGlFramebuffer>,
    bloom_tex_a: Option<WebGlTexture>,
    bloom_fbo_b: Option<WebGlFramebuffer>,
    bloom_tex_b: Option<WebGlTexture>,
    bloom_width: u32,
    bloom_height: u32,
    fullscreen_vao: Option<web_sys::WebGlVertexArrayObject>,
    logged_members: std::collections::HashSet<(i32, i32)>,
    animation_time: f32,
    motion_transforms: HashMap<String, [f32; 16]>,
    /// Single-track keyframe (object) motions that REPLACE a node's local
    /// transform — Director keyframePlayer semantics: the keyframe stores the
    /// node's full local transform, so it overrides the base rather than
    /// multiplying onto it (motion_transforms). Used by the multi-player path.
    motion_replace_transforms: HashMap<String, [f32; 16]>,
    pub active_camera: Option<String>,
    /// Set when a non-looping motion reaches its end — caller should advance the queue
    pub motion_ended: bool,
    /// Track last motion name to detect changes (sync animation_time from runtime state)
    last_motion_name: Option<String>,
    /// Local blend state (progressed each frame)
    blend_elapsed: f32,
    blend_weight: f32,
    blend_duration: f32,
    /// Director's default red/white checkerboard texture (2×2, generated on first use)
    default_checker_texture: Option<WebGlTexture>,
    /// Render-to-texture FBO (created on demand)
    rtt_fbo: Option<WebGlFramebuffer>,
    rtt_texture: Option<WebGlTexture>,
    rtt_depth: Option<web_sys::WebGlRenderbuffer>,
    rtt_width: u32,
    rtt_height: u32,
}

/// Director's default checkerboard texture: 2×2 pink-red / white pattern.
const CHECKER_PIXELS: [u8; 16] = [
    255, 255, 255, 255,   204, 102, 102, 255,  // row 0: white, pink-red
    204, 102, 102, 255,   255, 255, 255, 255,  // row 1: pink-red, white
];

impl Scene3dRenderer {
    /// Create the default checker texture if it doesn't exist yet.
    fn ensure_checker_texture(&mut self, gl: &WebGl2RenderingContext) {
        if self.default_checker_texture.is_some() { return; }
        if let Some(tex) = gl.create_texture() {
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&tex));
            let _ = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                WebGl2RenderingContext::TEXTURE_2D, 0,
                WebGl2RenderingContext::RGBA as i32, 2, 2, 0,
                WebGl2RenderingContext::RGBA, WebGl2RenderingContext::UNSIGNED_BYTE, Some(&CHECKER_PIXELS),
            );
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::REPEAT as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::REPEAT as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::NEAREST as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::NEAREST as i32);
            self.default_checker_texture = Some(tex);
        }
    }

    pub fn new() -> Self {
        Self {
            shader: None,
            particle_shader: None,
            pp_shader: None,
            outline_shader: None,
            member_data: HashMap::new(),
            fbo: None,
            fbo_texture: None,
            fbo_depth: None,
            bloom_fbo_a: None,
            bloom_tex_a: None,
            bloom_fbo_b: None,
            bloom_tex_b: None,
            bloom_width: 0,
            bloom_height: 0,
            fullscreen_vao: None,
            fbo_width: 0,
            fbo_height: 0,
            logged_members: std::collections::HashSet::new(),
            animation_time: 0.0,
            motion_transforms: HashMap::new(),
            motion_replace_transforms: HashMap::new(),
            active_camera: None,
            motion_ended: false,
            last_motion_name: None,
            blend_elapsed: 0.0,
            blend_weight: 1.0,
            blend_duration: 0.0,
            default_checker_texture: None,
            rtt_fbo: None,
            rtt_texture: None,
            rtt_depth: None,
            rtt_width: 0,
            rtt_height: 0,
            overlay_quad_vbo: None,
            overlay_quad_uv: None,
        }
    }

    /// Reset all cached state - forces full rebuild on next render
    pub fn reset_all(&mut self) {
        self.shader = None;
        self.fbo = None;
        self.fbo_texture = None;
        self.fbo_depth = None;
        self.member_data.clear();
        self.logged_members.clear();
    }

    /// Compile 3D shaders (lazy init on first use)
    fn ensure_shader(&mut self, context: &WebGL2Context) -> Result<(), JsValue> {
        if self.shader.is_some() {
            return Ok(());
        }

        let vs_source = r#"#version 300 es
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_texcoord;
layout(location = 3) in vec2 a_texcoord2;
layout(location = 4) in vec4 a_bone_indices;
layout(location = 5) in vec4 a_bone_weights;
layout(location = 6) in vec4 a_vertex_color;

uniform mat4 u_model;
uniform mat4 u_view;
uniform mat4 u_projection;

// Skeletal skinning
uniform int u_skinning_enabled;
uniform mat4 u_bone_matrices[48];

// Texture coordinate transform (post-projection UV-space tweak)
uniform mat4 u_tex_transform;
uniform int u_texcoord2_direct;
// Diffuse layer UV projection mode: 0=mesh UVs, 5=#wrapPlanar (object-space XY)
uniform int u_uv_proj_mode;
// Per Director spec: wrapTransformList[i] is applied to the mapping space
// (model-space position) before texture coordinates are generated.
uniform mat4 u_wrap_transform;

out vec3 v_position;
out vec3 v_normal;
out vec2 v_texcoord;
out vec2 v_texcoord2;
out float v_view_dist;
out vec4 v_vertex_color;

void main() {
    v_vertex_color = a_vertex_color;
    vec4 local_pos = vec4(a_position, 1.0);
    vec3 local_normal = a_normal;

    if (u_skinning_enabled > 0) {
        mat4 skin_mat = a_bone_weights.x * u_bone_matrices[int(a_bone_indices.x)]
                      + a_bone_weights.y * u_bone_matrices[int(a_bone_indices.y)]
                      + a_bone_weights.z * u_bone_matrices[int(a_bone_indices.z)]
                      + a_bone_weights.w * u_bone_matrices[int(a_bone_indices.w)];
        local_pos = skin_mat * local_pos;
        local_normal = mat3(skin_mat) * local_normal;
    }

    vec4 world_pos = u_model * local_pos;
    vec4 view_pos = u_view * world_pos;
    v_position = world_pos.xyz;
    v_normal = mat3(u_model) * local_normal;
    // W3D CLOD UVs are in [-0.5, 0.5] range — remap to [0, 1]
    // IFX applies V-flip via texture matrix: new_v = 1 - v
    // UV coordinate handling:
    // 3D meshes (u_skinning_enabled >= 0): CLOD UVs in [-0.5, 0.5] → remap to [0, 1]
    // Overlays (u_skinning_enabled == -1): UVs already [0,1], just flip V for OpenGL
    vec2 base_uv;
    if (u_uv_proj_mode == 5) {
        // #wrapPlanar (per Director spec): the model-space position is first
        // transformed by wrapTransformList[i], then UV is the XY of the result
        // (Z is the projection axis, "extruded" — i.e. dropped). u_tex_transform
        // is then applied below as a post-projection UV-space tweak.
        vec4 mapping_pos = u_wrap_transform * vec4(a_position, 1.0);
        base_uv = mapping_pos.xy;
    } else if (u_skinning_enabled == -1) {
        base_uv = a_texcoord;  // overlay: pass through as-is
    } else {
        base_uv = vec2(a_texcoord.x + 0.5, 0.5 - a_texcoord.y);  // CLOD remap
    }
    v_texcoord = (u_tex_transform * vec4(base_uv, 0.0, 1.0)).xy;
    if (u_skinning_enabled == -1) {
        v_texcoord2 = a_texcoord2;  // overlay: pass through as-is
    } else if (u_texcoord2_direct > 0) {
        v_texcoord2 = vec2(a_texcoord2.x, 1.0 - a_texcoord2.y);  // meshDeform: flip V for Director→OpenGL
    } else {
        v_texcoord2 = vec2(a_texcoord2.x + 0.5, 0.5 - a_texcoord2.y);
    }
    v_view_dist = -view_pos.z;
    gl_Position = u_projection * view_pos;
}
"#;

        let fs_source = r#"#version 300 es
precision mediump float;
// Fragment shaders default `int` to mediump, but vertex shaders default it to
// highp. A uniform shared across both stages must match precision or GLSL ES
// linking fails ("Uniform `u_texcoord2_direct` is not linkable between attached
// shaders" — Firefox enforces this; Chrome/ANGLE silently tolerates it). Force
// highp int here so every shared int uniform matches the VS default.
precision highp int;

in vec3 v_position;
in vec3 v_normal;
in vec2 v_texcoord;
in vec2 v_texcoord2;
in float v_view_dist;
in vec4 v_vertex_color;

// Must match the vertex shader's precision (highp, the VS default) — a precision
// mismatch on a uniform shared across stages is a GLSL ES link error.
uniform highp mat4 u_view;  // for eye-space reflection (sphere map)
uniform vec4 u_diffuse_color;
uniform int u_has_vertex_color;
uniform vec4 u_ambient_color;
uniform vec4 u_specular_color;
uniform vec4 u_emissive_color;
uniform float u_shininess;
uniform float u_opacity;
// Alpha-test threshold for cutout (opaque-but-alpha-textured) models drawn in the
// opaque pass: discard texels below this so they write neither colour nor depth.
// 0 disables the test (default / blended-transparent paths).
uniform float u_alpha_threshold;
uniform sampler2D u_diffuse_tex;
uniform int u_has_texture;
uniform int u_texture_unlit;   // 1 = #replace first layer: show texture as-is (unlit)
uniform sampler2D u_lightmap_tex;
uniform int u_has_lightmap;       // blend mode: 0=none, 1=multiply, 2=add, 3=replace, 4=decal
uniform float u_lightmap_intensity;
uniform int u_has_texcoord2;
uniform int u_texcoord2_direct;
// Layer 2 (third texture layer)
uniform sampler2D u_layer2_tex;
uniform int u_layer2_blend;       // same encoding as u_has_lightmap
uniform float u_layer2_intensity;
// Specular map
uniform sampler2D u_specular_tex;
uniform int u_has_specular_map;
// Environment/cube map reflection (future: samplerCube)
uniform int u_has_env_map;
uniform float u_reflectivity;

// NPR/toon shading
uniform int u_shader_mode;     // 0=phong, 1=toon/painter
uniform float u_toon_steps;    // number of quantization steps (e.g. 3.0)

uniform int u_num_lights;
uniform vec3 u_light_pos[8];
uniform vec3 u_light_color[8];
uniform int u_light_type[8];
uniform vec3 u_light_atten[8];   // (constant, linear, quadratic) per light
uniform vec3 u_light_dir[8];     // direction for directional/spot lights
uniform float u_light_spot_angle[8]; // spot cone angle (radians, 0 = not spot)
uniform vec3 u_camera_pos;
uniform vec3 u_global_ambient;

// Fog
uniform int u_fog_enabled;
uniform float u_fog_near;
uniform float u_fog_far;
uniform vec3 u_fog_color;
uniform int u_fog_mode; // 0=linear, 1=exp, 2=exp2

out vec4 frag_color;

// Apply a texture layer blend: mode 1=multiply, 2=add, 3=replace, 4=decal
vec3 blend_layer(vec3 base, vec4 layer_sample, int mode, float intensity) {
    if (mode == 1) {
        // Multiply (shadow map): darken
        return base * mix(vec3(1.0), layer_sample.rgb, intensity);
    } else if (mode == 2) {
        // Add (lightmap): brighten
        return base + layer_sample.rgb * intensity;
    } else if (mode == 3) {
        // Replace: layer replaces base entirely
        return mix(base, layer_sample.rgb, intensity);
    } else if (mode == 4) {
        // Decal: alpha-blended overlay
        return mix(base, layer_sample.rgb, layer_sample.a * intensity);
    }
    return base;
}

// Classic OpenGL GL_SPHERE_MAP UV for a reflection / environment map, computed
// in EYE space (like Director's #reflection mode). worldN = world-space surface
// normal, worldPos = world-space fragment position. Working in eye space makes
// every camera-facing surface reflect consistently — a world-space version made
// the sampled sky region depend on the surface's world orientation, so only
// windows facing one direction appeared to reflect the backdrop.
vec2 sphere_map_uv(vec3 worldN, vec3 worldPos) {
    vec3 n_eye = normalize(mat3(u_view) * worldN);
    vec3 pos_eye = (u_view * vec4(worldPos, 1.0)).xyz;
    vec3 incident = normalize(pos_eye);          // eye(origin) → fragment
    vec3 r = reflect(incident, n_eye);
    float m = 2.0 * sqrt(r.x * r.x + r.y * r.y + (r.z + 1.0) * (r.z + 1.0));
    m = max(m, 1e-4);
    // Flip the V (vertical) coord: the sky texture is stored top-down, so the raw
    // sphere-map t would render the reflection upside down.
    return vec2(r.x / m + 0.5, 0.5 - r.y / m);
}

// Blend a colour toward the fog colour by distance. fog_mode: 0=linear, 1=exp,
// 2=exp2. Applied to BOTH the textured and non-textured paths so all geometry
// fogs consistently (the estate movie enables fog; previously the textured path
// returned before fogging, so the brick house never faded into the fog).
vec3 apply_fog(vec3 color) {
    if (u_fog_enabled <= 0) {
        return color;
    }
    // Use the euclidean world distance from the camera, NOT v_view_dist (-view_pos.z).
    // The eye-space Z sign depends on IFX's view-matrix handedness; if it came out
    // negative, fog_factor clamped to 1 and fog never showed. length() is always
    // positive and matches Director's camera-relative fog distance.
    float dist = length(u_camera_pos - v_position);
    float fog_factor;
    if (u_fog_mode == 0) {
        // #linear: interpolate between near and far (GL_LINEAR).
        fog_factor = (u_fog_far - dist) / (u_fog_far - u_fog_near);
    } else if (u_fog_mode == 1) {
        // #exponential (Director default): GL_EXP, density = ln(100)/far, near ignored.
        // Matches IFX CIFXRenderDevice::CalcFogDensity (EXPONENTIAL_FOG_CONSTANT/fFar).
        float density = 4.6051701859880914 / u_fog_far;
        fog_factor = exp(-density * dist);
    } else {
        // #exponential2: GL_EXP2, density = sqrt(ln(100))/far, near ignored.
        float density = 2.1459660262893472 / u_fog_far;
        fog_factor = exp(-(density * dist) * (density * dist));
    }
    return mix(u_fog_color, color, clamp(fog_factor, 0.0, 1.0));
}

void main() {
    // A fully-transparent material (frog01's `clearS`, blend 0 → opacity 0) is
    // invisible, but in the opaque/cutout pass (depth_mask ON) it would still write
    // DEPTH — an invisible depth wall that culls everything behind it. The game-over
    // `back` wall's clearS face sits ~2.5u in front of the camera and covers the
    // upper view, so it was depth-hiding the banner / side walls / far logs behind it
    // (they showed in play because the camera wasn't behind that face). Discard it so
    // it writes neither colour nor depth. (Genuine translucency like water2 blend=50 →
    // opacity 0.5 is untouched.)
    if (u_opacity < 0.004) discard;
    vec3 N = normalize(v_normal);
    vec3 V = normalize(u_camera_pos - v_position);

    vec4 tex_sample = texture(u_diffuse_tex, v_texcoord);

    // Alpha-test cutout: opaque models whose texture carries alpha (e.g. frog01's
    // Flash bark/leaf textures) are drawn in the opaque pass; discard transparent
    // texels so they don't write depth and aren't sorted as translucent.
    if (u_alpha_threshold > 0.0 && tex_sample.a < u_alpha_threshold) discard;

    // When textured: GL_MODULATE mode = texture * vertex_lighting
    // IFX default: UseDiffuse=OFF → material diffuse forced to white (1,1,1)
    // This means lighting fully illuminates the texture without material color attenuation
    if (u_has_texture > 0) {
        // IFX fixed-function lighting equation with UseDiffuse OFF
        vec3 lighting = u_emissive_color.rgb + u_global_ambient * u_ambient_color.rgb;

        for (int i = 0; i < 8; i++) {
            if (i >= u_num_lights) break;
            if (u_light_type[i] == 0) {
                // Ambient light: adds lightColor * materialAmbient
                lighting += u_light_color[i] * u_ambient_color.rgb;
            } else {
                vec3 L;
                float atten = 1.0;
                if (u_light_type[i] == 1) {
                    L = normalize(u_light_pos[i]);
                } else {
                    vec3 light_dir = u_light_pos[i] - v_position;
                    float dist = length(light_dir);
                    L = light_dir / dist;
                    atten = 1.0 / (u_light_atten[i].x + u_light_atten[i].y * dist + u_light_atten[i].z * dist * dist);
                    // Spot light cone attenuation. Director's spotAngle is the
                    // HALF-cone angle (dict: "corresponds to half the angle; for a
                    // 90° angle pass 45.0"), so the cone edge is cos(spotAngle) — do
                    // NOT halve it again.
                    if (u_light_spot_angle[i] > 0.0) {
                        float spot_cos = dot(normalize(-light_dir), u_light_dir[i]);
                        float cone_cos = cos(u_light_spot_angle[i]);
                        if (spot_cos < cone_cos) atten = 0.0;
                        else atten *= smoothstep(cone_cos, cone_cos + 0.1, spot_cos);
                    }
                }
                // Two-sided lighting: use abs(N·L) so back faces also receive light
                float diff = abs(dot(N, L));
                // Toon shading: quantize NdotL into discrete steps
                if (u_shader_mode == 1 && u_toon_steps > 0.0) {
                    diff = floor(diff * u_toon_steps + 0.5) / u_toon_steps;
                }
                lighting += atten * diff * u_light_color[i] * u_diffuse_color.rgb;
            }
        }

        // IFX fixed-function clamps per-vertex lighting to [0,1] before GL_MODULATE
        lighting = clamp(lighting, vec3(0.0), vec3(1.0));
        // GL_MODULATE: fragment = texture * lighting * vertex color. Vertex colors
        // are identity-white for normal meshes; extruded 3D text bakes its tunnel
        // shading here (gray side walls vs white front) so the glyphs read 3D.
        vec3 vcol_t = (u_has_vertex_color > 0) ? v_vertex_color.rgb : vec3(1.0);
        // #replace first layer (u_texture_unlit): the texture is shown as-is, not
        // blended with the surface shading (Director: "prevents the texture from
        // being blended with the color set by the shader's diffuse property"). Used
        // by skybox/backdrop planes so the nebula shows at full brightness instead of
        // being dimmed by the ambient-only lighting.
        vec3 final_color = (u_texture_unlit > 0) ? tex_sample.rgb : tex_sample.rgb * lighting * vcol_t;

        // Apply second texture layer (shadow/lightmap) if present
        if (u_has_lightmap > 0) {
            // Use 2nd UV set if available, otherwise same as primary
            vec2 lm_uv = (u_has_texcoord2 > 0) ? v_texcoord2 : v_texcoord;
            vec4 lm_sample = texture(u_lightmap_tex, lm_uv);
            float intensity = u_lightmap_intensity;
            if (u_has_lightmap == 1) {
                // Multiply blend: lightmap represents light intensity.
                // Bright lightmap = lit (keep base), dark lightmap = shadow (darken).
                final_color *= mix(vec3(1.0), lm_sample.rgb, intensity);
            } else if (u_has_lightmap == 2) {
                // Additive blend (lightmap): brighten with light data
                final_color += lm_sample.rgb * intensity;
            }
        }

        // Apply third texture layer if present
        if (u_layer2_blend == 5) {
            // Reflection / environment map (#reflection): sphere-mapped sky blended
            // over the textured surface at the #constant factor (u_layer2_intensity).
            vec3 refl = texture(u_layer2_tex, sphere_map_uv(N, v_position)).rgb;
            final_color = mix(final_color, refl, u_layer2_intensity);
        } else if (u_layer2_blend > 0) {
            vec2 l2_uv = (u_has_texcoord2 > 0) ? v_texcoord2 : v_texcoord;
            vec4 l2_sample = texture(u_layer2_tex, l2_uv);
            float l2_intensity = u_layer2_intensity;
            if (u_layer2_blend == 1) {
                // Multiply blend (shadow map)
                final_color *= mix(vec3(1.0), l2_sample.rgb, l2_intensity);
            } else if (u_layer2_blend == 2) {
                // Additive blend (lightmap)
                final_color += l2_sample.rgb * l2_intensity;
            }
        }

        frag_color = vec4(apply_fog(final_color), u_opacity * tex_sample.a);
        return;
    }

    // Non-textured path: use material diffuse color (or vertex color if available)
    vec3 base_color = (u_has_vertex_color > 0) ? v_vertex_color.rgb : u_diffuse_color.rgb;
    bool lightmap_only = (u_has_texture == 0 && u_has_lightmap > 0);
    vec3 result = u_emissive_color.rgb;

    if (lightmap_only) {
        // Director lightmap-only shaders use the material diffuse color as the base
        // that the baked lightmap multiplies over. Applying dynamic lighting again
        // here washes out the floor and doesn't match Director's output.
        result += base_color;
    } else {
        result += u_global_ambient * u_ambient_color.rgb;

        for (int i = 0; i < 8; i++) {
            if (i >= u_num_lights) break;
            if (u_light_type[i] == 0) {
                result += u_light_color[i] * u_ambient_color.rgb;
            } else {
                vec3 L;
                float atten = 1.0;
                if (u_light_type[i] == 1) {
                    L = normalize(u_light_pos[i]);
                } else {
                    vec3 light_dir = u_light_pos[i] - v_position;
                    float dist = length(light_dir);
                    L = light_dir / dist;
                    atten = 1.0 / (u_light_atten[i].x + u_light_atten[i].y * dist + u_light_atten[i].z * dist * dist);
                    // Spot cone (spotAngle = half-cone angle, per Director dict).
                    if (u_light_spot_angle[i] > 0.0) {
                        float spot_cos = dot(normalize(-light_dir), u_light_dir[i]);
                        float cone_cos = cos(u_light_spot_angle[i]);
                        if (spot_cos < cone_cos) atten = 0.0;
                        else atten *= smoothstep(cone_cos, cone_cos + 0.1, spot_cos);
                    }
                }

                // Two-sided lighting: use abs(N·L) so back faces also receive light
                float diff = abs(dot(N, L));
                if (u_shader_mode == 1 && u_toon_steps > 0.0) {
                    diff = floor(diff * u_toon_steps + 0.5) / u_toon_steps;
                }
                result += atten * u_light_color[i] * base_color * diff;

                if (u_shininess > 0.0 && diff > 0.0) {
                    vec3 H = normalize(L + V);
                    float spec = pow(max(dot(N, H), 0.0), u_shininess);
                    result += u_light_color[i] * u_specular_color.rgb * spec * atten;
                }
            }
        }
    }

    // Apply lightmap even for non-textured models (e.g., floor with no base texture
    // but lightmap in textureList[2] from lightmapmanager)
    if (u_has_lightmap > 0) {
        vec2 lm_uv = (u_has_texcoord2 > 0) ? v_texcoord2 : v_texcoord;
        vec4 lm_sample = texture(u_lightmap_tex, lm_uv);
        float intensity = u_lightmap_intensity;
        if (u_has_lightmap == 1) {
            result *= mix(vec3(1.0), lm_sample.rgb, intensity);
        } else if (u_has_lightmap == 2) {
            result += lm_sample.rgb * intensity;
        }
    }

    // Reflection / environment map on an untextured surface — e.g. tinted glass:
    // material diffuse colour with a sphere-mapped sky reflection mixed in at the
    // #constant blend factor (reflectionMap helper, u_layer2_blend == 5).
    if (u_layer2_blend == 5) {
        vec3 refl = texture(u_layer2_tex, sphere_map_uv(N, v_position)).rgb;
        result = mix(result, refl, u_layer2_intensity);
    }

    // Apply fog (shared with the textured path via apply_fog).
    result = apply_fog(result);

    float alpha = u_opacity * tex_sample.a * u_diffuse_color.a;
    frag_color = vec4(result, alpha);
}
"#;

        let vs = context.compile_shader(WebGl2RenderingContext::VERTEX_SHADER, vs_source)?;
        let fs = context.compile_shader(WebGl2RenderingContext::FRAGMENT_SHADER, fs_source)?;
        let program = context.link_program(&vs, &fs)?;

        let gl = context.gl();
        let u = |name: &str| gl.get_uniform_location(&program, name);

        self.shader = Some(Shader3d {
            u_model: u("u_model"),
            u_view: u("u_view"),
            u_projection: u("u_projection"),
            u_diffuse_color: u("u_diffuse_color"),
            u_has_vertex_color: u("u_has_vertex_color"),
            u_ambient_color: u("u_ambient_color"),
            u_specular_color: u("u_specular_color"),
            u_emissive_color: u("u_emissive_color"),
            u_shininess: u("u_shininess"),
            u_opacity: u("u_opacity"),
            u_alpha_threshold: u("u_alpha_threshold"),
            u_diffuse_tex: u("u_diffuse_tex"),
            u_has_texture: u("u_has_texture"),
            u_texture_unlit: u("u_texture_unlit"),
            u_lightmap_tex: u("u_lightmap_tex"),
            u_has_lightmap: u("u_has_lightmap"),
            u_lightmap_intensity: u("u_lightmap_intensity"),
            u_has_texcoord2: u("u_has_texcoord2"),
            u_texcoord2_direct: u("u_texcoord2_direct"),
            u_layer2_tex: u("u_layer2_tex"),
            u_layer2_blend: u("u_layer2_blend"),
            u_layer2_intensity: u("u_layer2_intensity"),
            u_specular_tex: u("u_specular_tex"),
            u_has_specular_map: u("u_has_specular_map"),
            u_has_env_map: u("u_has_env_map"),
            u_reflectivity: u("u_reflectivity"),
            u_tex_transform: u("u_tex_transform"),
            u_uv_proj_mode: u("u_uv_proj_mode"),
            u_wrap_transform: u("u_wrap_transform"),
            u_skinning_enabled: u("u_skinning_enabled"),
            u_bone_matrices: u("u_bone_matrices[0]"),
            u_shader_mode: u("u_shader_mode"),
            u_toon_steps: u("u_toon_steps"),
            u_num_lights: u("u_num_lights"),
            u_light_pos: u("u_light_pos[0]"),
            u_light_color: u("u_light_color[0]"),
            u_light_type: u("u_light_type[0]"),
            u_light_atten: u("u_light_atten[0]"),
            u_light_dir: u("u_light_dir[0]"),
            u_light_spot_angle: u("u_light_spot_angle[0]"),
            u_camera_pos: u("u_camera_pos"),
            u_global_ambient: u("u_global_ambient"),
            u_fog_enabled: u("u_fog_enabled"),
            u_fog_near: u("u_fog_near"),
            u_fog_far: u("u_fog_far"),
            u_fog_color: u("u_fog_color"),
            u_fog_mode: u("u_fog_mode"),
            program,
        });

        Ok(())
    }

    /// Compile particle billboard shader (lazy init)
    fn ensure_particle_shader(&mut self, context: &WebGL2Context) -> Result<(), JsValue> {
        if self.particle_shader.is_some() {
            return Ok(());
        }

        let vs = r#"#version 300 es
layout(location = 0) in vec3 a_center;
layout(location = 1) in float a_age;
layout(location = 2) in vec2 a_corner; // (-1,-1) to (1,1)

uniform mat4 u_view_projection;
uniform vec3 u_camera_right;
uniform vec3 u_camera_up;
uniform float u_size_start;
uniform float u_size_end;
uniform float u_lifetime;

out float v_age_ratio;
out vec2 v_uv;

void main() {
    v_age_ratio = clamp(a_age / u_lifetime, 0.0, 1.0);
    v_uv = a_corner * 0.5 + 0.5;

    // sizeRange is the world-unit sprite size (IFX builds a `size`-wide quad per
    // particle); a_corner spans -1..1 so 0.5 == `size` wide. We use 0.25 (half
    // that) so the stream is a thin jet matching Shockwave's water output, rather
    // than a thick column — the visible blob of the particle texture is narrower
    // than the full quad.
    float size_factor = mix(u_size_start, u_size_end, v_age_ratio) * 0.25;

    // Cull particles that are behind the eye OR so close that the billboard balloons
    // across the screen. A chase camera following a car repeatedly passes through the
    // exhaust/wheel-spray it just emitted; as a particle's center nears the eye
    // (clip.w -> small) its size/w blows the quad up to many times the screen, and a
    // few such quads white out the entire 3D scene. Measure the billboard's on-screen
    // half-extent in NDC and discard the quad when it would exceed the screen.
    vec4 center_clip = u_view_projection * vec4(a_center, 1.0);
    vec4 edge_clip   = u_view_projection * vec4(a_center + u_camera_right * size_factor, 1.0);
    bool bad = center_clip.w <= 0.0001 || edge_clip.w <= 0.0001;
    float ndc_half = bad ? 1e9
        : length(edge_clip.xy / edge_clip.w - center_clip.xy / center_clip.w);
    if (bad || ndc_half > 1.0) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0); // outside NDC clip volume → discarded
    } else {
        vec3 world_pos = a_center
            + u_camera_right * a_corner.x * size_factor
            + u_camera_up * a_corner.y * size_factor;
        gl_Position = u_view_projection * vec4(world_pos, 1.0);
    }
}
"#;

        let fs = r#"#version 300 es
precision mediump float;

in float v_age_ratio;
in vec2 v_uv;

uniform vec3 u_color_start;
uniform vec3 u_color_end;
uniform float u_blend_start;
uniform float u_blend_end;
uniform sampler2D u_tex;
uniform int u_has_tex;

out vec4 frag_color;

void main() {
    // colorRange / blendRange interpolate start->end over the particle's life.
    vec3 color = mix(u_color_start, u_color_end, v_age_ratio);
    // blendRange is the per-particle ALPHA (the script comments call it the
    // "transparency", scaled by how far the tap is turned). The ParticleTexture is
    // a solid white square with useAlpha=false, so the texture adds nothing — the
    // whole look is colorRange (tint) + blendRange (alpha). The faucet drives blend
    // to 2..3 (cold) / 6..7 (hot) at full flow; clamped that's opaque, but Shockwave
    // renders the stream translucent (you see the drain through it), so we cap full
    // flow at ~half. Low flow scales below that toward a faint trickle.
    float opacity = clamp(mix(u_blend_start, u_blend_end, v_age_ratio), 0.0, 1.0) * 0.5;

    float alpha;
    if (u_has_tex > 0) {
        vec4 tex = texture(u_tex, v_uv);
        alpha = tex.a;
        color *= tex.rgb;
    } else {
        // Soft circular fallback when no particle texture is assigned.
        float dist = length(v_uv - 0.5) * 2.0;
        if (dist > 1.0) discard;
        alpha = 1.0 - dist * dist;
    }

    frag_color = vec4(color, alpha * opacity);
}
"#;

        let vs_compiled = context.compile_shader(WebGl2RenderingContext::VERTEX_SHADER, vs)?;
        let fs_compiled = context.compile_shader(WebGl2RenderingContext::FRAGMENT_SHADER, fs)?;
        let program = context.link_program(&vs_compiled, &fs_compiled)?;

        let gl = context.gl();
        let u = |name: &str| gl.get_uniform_location(&program, name);

        self.particle_shader = Some(ParticleShader {
            u_view_projection: u("u_view_projection"),
            u_camera_right: u("u_camera_right"),
            u_camera_up: u("u_camera_up"),
            u_color_start: u("u_color_start"),
            u_color_end: u("u_color_end"),
            u_size_start: u("u_size_start"),
            u_size_end: u("u_size_end"),
            u_blend_start: u("u_blend_start"),
            u_blend_end: u("u_blend_end"),
            u_lifetime: u("u_lifetime"),
            u_tex: u("u_tex"),
            u_has_tex: u("u_has_tex"),
            program,
        });

        Ok(())
    }

    /// Render all active particle systems
    fn render_particles(
        &mut self,
        context: &WebGL2Context,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
        view_matrix: &[f32; 16],
        projection_matrix: &[f32; 16],
    ) -> Result<(), JsValue> {
        let rs = match runtime_state {
            Some(rs) if !rs.particles.is_empty() => rs,
            _ => return Ok(()),
        };
        self.ensure_particle_shader(context)?;
        let gl = context.gl();
        let gpu_data = self.member_data.get(member_key);
        let shader = self.particle_shader.as_ref().unwrap();

        gl.use_program(Some(&shader.program));

        // Compute view-projection matrix
        let vp = mat4_multiply_col_major(projection_matrix, view_matrix);
        gl.uniform_matrix4fv_with_f32_array(shader.u_view_projection.as_ref(), false, &vp);

        // Extract camera right/up from view matrix (inverse of view = camera world)
        // View matrix columns 0,1 in row-major = camera right, up in world space
        gl.uniform3f(shader.u_camera_right.as_ref(), view_matrix[0], view_matrix[4], view_matrix[8]);
        gl.uniform3f(shader.u_camera_up.as_ref(), view_matrix[1], view_matrix[5], view_matrix[9]);

        // Standard alpha blending — Director's default particle blend. Additive
        // (SRC_ALPHA, ONE) over-saturates dense overlapping particles to white
        // (e.g. the faucet's translucent pink/red water turned into an opaque white
        // jet); alpha blending keeps them translucent and colored like Shockwave.
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.depth_mask(false); // Don't write to depth buffer

        for (_name, ps) in &rs.particles {
            if ps.positions.is_empty() { continue; }

            // colorRange / sizeRange / blendRange — interpolated over each particle's
            // life in the shader (see vs/fs above).
            gl.uniform1f(shader.u_size_start.as_ref(), ps.size_start);
            gl.uniform1f(shader.u_size_end.as_ref(), ps.size_end);
            gl.uniform1f(shader.u_lifetime.as_ref(), ps.lifetime.max(0.001));
            gl.uniform3f(shader.u_color_start.as_ref(), ps.color_start[0], ps.color_start[1], ps.color_start[2]);
            gl.uniform3f(shader.u_color_end.as_ref(), ps.color_end[0], ps.color_end[1], ps.color_end[2]);
            gl.uniform1f(shader.u_blend_start.as_ref(), ps.blend_start);
            gl.uniform1f(shader.u_blend_end.as_ref(), ps.blend_end);

            // Bind the particle texture (set via resource.texture) if present.
            let mut has_tex = false;
            if !ps.texture_name.is_empty() {
                if let Some(tex) = gpu_data.and_then(|d| d.textures.get(&ps.texture_name)) {
                    gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
                    gl.uniform1i(shader.u_tex.as_ref(), 0);
                    has_tex = true;
                }
            }
            gl.uniform1i(shader.u_has_tex.as_ref(), if has_tex { 1 } else { 0 });

            // Build billboard quad vertex data: 4 verts per particle (center + age + corner)
            let alive_count = ps.alive.iter().filter(|&&a| a).count();
            if alive_count == 0 { continue; }

            let mut vertices: Vec<f32> = Vec::with_capacity(alive_count * 4 * 6); // 4 verts * 6 floats
            let mut indices: Vec<u32> = Vec::with_capacity(alive_count * 6);
            let mut vert_idx = 0u32;

            let corners: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

            for i in 0..ps.max_particles.min(ps.positions.len()) {
                if !ps.alive[i] { continue; }
                let pos = ps.positions[i];
                let age = ps.ages[i];

                for corner in &corners {
                    vertices.extend_from_slice(&pos);    // center (3 floats)
                    vertices.push(age);                   // age (1 float)
                    vertices.extend_from_slice(corner);   // corner (2 floats)
                }

                indices.push(vert_idx);
                indices.push(vert_idx + 1);
                indices.push(vert_idx + 2);
                indices.push(vert_idx);
                indices.push(vert_idx + 2);
                indices.push(vert_idx + 3);
                vert_idx += 4;
            }

            if indices.is_empty() { continue; }

            // Upload to temporary buffers
            let vao = context.create_vertex_array()?;
            gl.bind_vertex_array(Some(&vao));

            let vbo = context.create_buffer()?;
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
            unsafe {
                let array = js_sys::Float32Array::view(&vertices);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER, &array,
                    WebGl2RenderingContext::DYNAMIC_DRAW,
                );
            }

            let stride = 6 * 4; // 6 floats * 4 bytes
            // a_center (location 0) - vec3
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_with_i32(0, 3, WebGl2RenderingContext::FLOAT, false, stride, 0);
            // a_age (location 1) - float
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_with_i32(1, 1, WebGl2RenderingContext::FLOAT, false, stride, 12);
            // a_corner (location 2) - vec2
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_with_i32(2, 2, WebGl2RenderingContext::FLOAT, false, stride, 16);

            let ibo = context.create_buffer()?;
            gl.bind_buffer(WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER, Some(&ibo));
            unsafe {
                let array = js_sys::Uint32Array::view(&indices);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER, &array,
                    WebGl2RenderingContext::DYNAMIC_DRAW,
                );
            }

            gl.draw_elements_with_i32(
                WebGl2RenderingContext::TRIANGLES,
                indices.len() as i32,
                WebGl2RenderingContext::UNSIGNED_INT,
                0,
            );

            gl.bind_vertex_array(None);
            gl.delete_buffer(Some(&vbo));
            gl.delete_buffer(Some(&ibo));
            gl.delete_vertex_array(Some(&vao));
        }

        // Restore state. Crucially DISABLE blend: the rest of the camera pass and the
        // FBO→stage composite run with blend OFF (it's disabled at pass start). Leaving
        // it enabled here made the 2D composite alpha-blend the whole 3D FBO by its
        // alpha — fine for the faucet (opaque alpha=1 FBO) but the car/track models
        // write alpha<1, so the entire scene faded out and read as "3D doesn't render".
        gl.depth_mask(true);
        gl.enable(WebGl2RenderingContext::CULL_FACE);
        gl.disable(WebGl2RenderingContext::BLEND);

        Ok(())
    }

    /// Ensure FBO exists at the right size
    fn ensure_fbo(&mut self, context: &WebGL2Context, width: u32, height: u32) -> Result<(), JsValue> {
        if self.fbo.is_some() && self.fbo_width == width && self.fbo_height == height {
            return Ok(());
        }

        let gl = context.gl();

        // Create color texture
        let texture = context.create_texture()?;
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&texture));
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            WebGl2RenderingContext::TEXTURE_2D,
            0,
            WebGl2RenderingContext::RGBA as i32,
            width as i32,
            height as i32,
            0,
            WebGl2RenderingContext::RGBA,
            WebGl2RenderingContext::UNSIGNED_BYTE,
            None,
        )?;
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::LINEAR as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::LINEAR as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);

        // Create depth renderbuffer
        let depth_rb = gl.create_renderbuffer()
            .ok_or_else(|| JsValue::from_str("Failed to create renderbuffer"))?;
        gl.bind_renderbuffer(WebGl2RenderingContext::RENDERBUFFER, Some(&depth_rb));
        gl.renderbuffer_storage(
            WebGl2RenderingContext::RENDERBUFFER,
            WebGl2RenderingContext::DEPTH_COMPONENT16,
            width as i32,
            height as i32,
        );

        // Create FBO
        let fbo = context.create_framebuffer()?;
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(&fbo));
        gl.framebuffer_texture_2d(
            WebGl2RenderingContext::FRAMEBUFFER,
            WebGl2RenderingContext::COLOR_ATTACHMENT0,
            WebGl2RenderingContext::TEXTURE_2D,
            Some(&texture),
            0,
        );
        gl.framebuffer_renderbuffer(
            WebGl2RenderingContext::FRAMEBUFFER,
            WebGl2RenderingContext::DEPTH_ATTACHMENT,
            WebGl2RenderingContext::RENDERBUFFER,
            Some(&depth_rb),
        );

        // Unbind
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
        gl.bind_renderbuffer(WebGl2RenderingContext::RENDERBUFFER, None);

        self.fbo = Some(fbo);
        self.fbo_texture = Some(texture);
        self.fbo_depth = Some(depth_rb);
        self.fbo_width = width;
        self.fbo_height = height;

        Ok(())
    }

    /// Upload mesh and texture data to GPU for a member
    fn ensure_member_data(
        &mut self,
        context: &WebGL2Context,
        key: (i32, i32),
        scene: &W3dScene,
    ) -> Result<(), JsValue> {
        let current_version = (scene.nodes.len(), scene.clod_meshes.len() + scene.raw_meshes.len(), scene.texture_images.len(), scene.shaders.len());
        if let Some(existing) = self.member_data.get(&key) {
            if existing.scene_version == current_version
                && existing.mesh_content_version == scene.mesh_content_version
            {
                if existing.texture_content_version != scene.texture_content_version {
                    self.update_textures_incremental(context, key, scene);
                }
                return Ok(());
            }
            // Scene changed — remove stale data and rebuild
            log(&format!(
                "[W3D-GPU] Rebuilding GPU data for {:?}: version {:?} → {:?} (nodes={}, clod={}, raw={}, tex={}, shaders={})",
                key, existing.scene_version, current_version,
                scene.nodes.len(), scene.clod_meshes.len(), scene.raw_meshes.len(),
                scene.texture_images.len(), scene.shaders.len(),
            ));
            self.logged_members.remove(&key);
        }
        self.member_data.remove(&key);

        let mut mesh_groups: HashMap<String, Vec<Mesh3dBuffers>> = HashMap::new();
        let mut all_meshes = Vec::new();

        // Collect resource names used by LIGHT nodes (to skip their geometry)
        let light_resources: std::collections::HashSet<&str> = scene.nodes.iter()
            .filter(|n| n.node_type == W3dNodeType::Light)
            .flat_map(|n| {
                let mut names = vec![];
                if !n.model_resource_name.is_empty() { names.push(n.model_resource_name.as_str()); }
                if !n.resource_name.is_empty() && n.resource_name != "." { names.push(n.resource_name.as_str()); }
                names
            })
            .collect();

        // Upload CLOD meshes (skip light geometry)
        for (name, decoded_meshes) in &scene.clod_meshes {
            if light_resources.contains(name.as_str()) {
                continue; // Skip light cone/sphere meshes
            }
            let mut group = Vec::new();
            for mesh in decoded_meshes.iter() {
                if mesh.positions.is_empty() || mesh.faces.is_empty() {
                    continue;
                }
                // Use decoded texcoords, or generate UVs based on resource UV generator mode
                let uv_gen_mode = scene.model_resources.get(name.as_str())
                    .and_then(|r| r.uv_gen_mode);
                let tc_data;
                let tc = if !mesh.tex_coords.is_empty() && !mesh.tex_coords[0].is_empty() {
                    let tcs = &mesh.tex_coords[0];
                    // Check if all texcoords are identical (needs UV generation)
                    let all_same = tcs.len() > 1 && tcs.iter().all(|t| (t[0] - tcs[0][0]).abs() < 0.001 && (t[1] - tcs[0][1]).abs() < 0.001);
                    if all_same && !mesh.positions.is_empty() {
                        tc_data = generate_uvs_by_mode(&mesh.positions, uv_gen_mode);
                        Some(tc_data.as_slice())
                    } else {
                        Some(tcs.as_slice())
                    }
                } else if !mesh.positions.is_empty() {
                    tc_data = generate_uvs_by_mode(&mesh.positions, uv_gen_mode);
                    Some(tc_data.as_slice())
                } else {
                    None
                };
                // Get 2nd UV set if available (for lightmap/shadow textures)
                let tc2 = if mesh.tex_coords.len() >= 2 && !mesh.tex_coords[1].is_empty() {
                    Some(mesh.tex_coords[1].as_slice())
                } else {
                    None
                };

                // Pack bone data (variable-length per-vertex → fixed vec4)
                let (bone_idx_packed, bone_wgt_packed);
                let (bi_opt, bw_opt) = if !mesh.bone_indices.is_empty() && !mesh.bone_weights.is_empty()
                    && mesh.bone_indices.len() == mesh.positions.len()
                {
                    bone_idx_packed = pack_bone_vec4_f32(&mesh.bone_indices);
                    bone_wgt_packed = pack_bone_weights_vec4(&mesh.bone_weights);
                    // Diagnostic: log bone data stats for first mesh with bones
                    {
                        use std::sync::Mutex; use std::collections::HashSet;
                        static LOGGED_BD: Mutex<Option<HashSet<String>>> = Mutex::new(None);
                        if let Ok(mut g) = LOGGED_BD.lock() { let set = g.get_or_insert_with(HashSet::new);
                        if set.insert(name.clone()) {
                            let max_idx = bone_idx_packed.iter().flat_map(|v| v.iter()).cloned().fold(0.0f32, f32::max);
                            let wgt_sums: Vec<f32> = bone_wgt_packed.iter().map(|w| w.iter().sum::<f32>()).collect();
                            let min_sum = wgt_sums.iter().cloned().fold(f32::MAX, f32::min);
                            let max_sum = wgt_sums.iter().cloned().fold(0.0f32, f32::max);
                            let zero_wgt = wgt_sums.iter().filter(|s| **s < 0.001).count();
                            let raw_lens: Vec<usize> = mesh.bone_indices.iter().take(3).map(|v| v.len()).collect();
                            debug!(
                                "[W3D-BONEDATA] mesh=\"{}\" verts={} bone_idx_count={} bone_wgt_count={} max_bone_idx={:.0} wgt_range=[{:.3},{:.3}] zero_wgt_verts={} raw_per_vert_lens={:?} first3_idx={:?} first3_wgt={:?}",
                                name, mesh.positions.len(), mesh.bone_indices.len(), mesh.bone_weights.len(),
                                max_idx, min_sum, max_sum, zero_wgt, raw_lens,
                                &bone_idx_packed[..3.min(bone_idx_packed.len())],
                                &bone_wgt_packed[..3.min(bone_wgt_packed.len())],
                            );
                        }}
                    }
                    (Some(bone_idx_packed.as_slice()), Some(bone_wgt_packed.as_slice()))
                } else {
                    (None, None)
                };
                // Vertex colors (diffuse)
                let vc_opt = if !mesh.diffuse_colors.is_empty()
                    && mesh.diffuse_colors.len() == mesh.positions.len()
                {
                    Some(mesh.diffuse_colors.as_slice())
                } else {
                    None
                };
                let mut buffers = Mesh3dBuffers::new_full(
                    context,
                    &mesh.positions,
                    &mesh.normals,
                    tc,
                    tc2,
                    &mesh.faces,
                    bi_opt,
                    bw_opt,
                    vc_opt,
                )?;
                // A file-provided 2nd UV set is a lightmap/shadowmap atlas coord in
                // [0,1], NOT pre-centered like the base set, so it must bypass the CLOD
                // (u+0.5, 0.5-v) remap. Without this it shifts to ~[0.5,1.5] and the
                // forced CLAMP smears the lightmap's edge across the whole surface.
                if tc2.is_some() {
                    buffers.texcoord2_direct = true;
                }
                group.push(buffers);
            }
            mesh_groups.insert(name.clone(), group);
        }

        // Upload raw meshes to mesh_groups (keyed by name) so draw_model_node can find them
        for mesh in &scene.raw_meshes {
            if light_resources.contains(mesh.name.as_str()) {
                continue; // Skip light cone/sphere meshes
            }
            if mesh.positions.is_empty() || mesh.faces.is_empty() {
                continue;
            }
            let tc = if !mesh.tex_coords.is_empty() {
                Some(mesh.tex_coords.as_slice())
            } else {
                None
            };
            let vc_opt = if !mesh.vertex_colors.is_empty()
                && mesh.vertex_colors.len() == mesh.positions.len()
            {
                Some(mesh.vertex_colors.as_slice())
            } else {
                None
            };
            let buffers = Mesh3dBuffers::new_full(
                context,
                &mesh.positions,
                &mesh.normals,
                tc,
                None, // raw meshes don't have 2nd UV set
                &mesh.faces,
                None, // no bone indices
                None, // no bone weights
                vc_opt,
            )?;
            // Add to mesh_groups keyed by name so draw_model_node can look up by resource_name
            mesh_groups.entry(mesh.name.clone())
                .or_insert_with(Vec::new)
                .push(buffers);
        }

        // Upload textures (decode JPEG/PNG or raw RGBA)
        // Store with lowercase keys for case-insensitive lookup
        let mut textures = HashMap::new();
        let mut texture_sizes: HashMap<String, (u32, u32)> = HashMap::new();
        let mut alpha_textures = std::collections::HashSet::new();
        for (tex_name, image_data) in &scene.texture_images {
            let lower = tex_name.to_lowercase();
            // The SkyLine* textures in this game are authored vertically inverted in
            // the W3D (the JPEGs are stored upside-down, while houses/buildings/icons
            // are stored right-side-up). The skyline mesh UVs use the same convention
            // as everything else, and the texture declarations carry no orientation
            // flag, so flip these on upload to render the horizon the right way up.
            let flip_v = lower.contains("skyline");
            if let Some((tex, w, h, has_alpha)) = self.decode_and_upload_texture(context, image_data, flip_v) {
                texture_sizes.insert(lower.clone(), (w, h));
                if has_alpha {
                    alpha_textures.insert(lower.clone());
                }
                textures.insert(lower, tex);
            }
        }

        // Pre-compute inverse bind matrices for all skeletons (cached for skinning)
        let mut inverse_bind_cache = HashMap::new();
        for skeleton in &scene.skeletons {
            let inv_bind = crate::director::chunks::w3d::skeleton::build_inverse_bind_matrices(skeleton);
            inverse_bind_cache.insert(skeleton.name.clone(), inv_bind);
        }

        // Detect and create cubemap textures from 6-face naming convention
        let cube_maps = self.detect_and_create_cubemaps(context, scene);

        let mut texture_versions = HashMap::new();
        for (tex_name, image_data) in &scene.texture_images {
            texture_versions.insert(tex_name.to_lowercase(), image_data.len() as u64);
        }
        self.member_data.insert(key, MemberGpuData {
            mesh_groups, all_meshes, textures, texture_sizes, cube_maps, inverse_bind_cache,
            scene_version: current_version,
            mesh_content_version: scene.mesh_content_version,
            texture_versions,
            texture_content_version: scene.texture_content_version,
            alpha_textures,
        });
        Ok(())
    }

    /// Decode JPEG/PNG image data and upload as WebGL texture (delegates to free function)
    fn decode_and_upload_texture(&self, context: &WebGL2Context, data: &[u8], flip_v: bool) -> Option<(WebGlTexture, u32, u32, bool)> {
        decode_and_upload_texture_impl(context, data, flip_v)
    }

    /// Incrementally re-upload only changed/new textures to GPU
    fn update_textures_incremental(&mut self, context: &WebGL2Context, key: (i32, i32), scene: &W3dScene) {
        let gpu_data = match self.member_data.get_mut(&key) { Some(d) => d, None => return };

        for (tex_name, image_data) in &scene.texture_images {
            let lower = tex_name.to_lowercase();
            let data_len = image_data.len() as u64;
            let needs_upload = match gpu_data.texture_versions.get(&lower) {
                None => true,
                Some(&old_len) => old_len != data_len,
            };
            if needs_upload {
                let flip_v = lower.contains("skyline");
                if let Some((tex, w, h, has_alpha)) = decode_and_upload_texture_impl(context, image_data, flip_v) {
                    gpu_data.texture_sizes.insert(lower.clone(), (w, h));
                    if has_alpha {
                        gpu_data.alpha_textures.insert(lower.clone());
                    } else {
                        gpu_data.alpha_textures.remove(&lower);
                    }
                    gpu_data.textures.insert(lower.clone(), tex);
                    gpu_data.texture_versions.insert(lower, data_len);
                }
            }
        }

        // Remove GPU textures no longer in the scene
        let scene_keys: std::collections::HashSet<String> = scene.texture_images.keys()
            .map(|k| k.to_lowercase()).collect();
        gpu_data.textures.retain(|k, _| scene_keys.contains(k));
        gpu_data.texture_sizes.retain(|k, _| scene_keys.contains(k));
        gpu_data.alpha_textures.retain(|k| scene_keys.contains(k));
        gpu_data.texture_versions.retain(|k, _| scene_keys.contains(k));

        gpu_data.texture_content_version = scene.texture_content_version;
    }

    /// Render directly to the default framebuffer (for offscreen canvas readPixels)
    pub fn render_to_default_framebuffer(
        &mut self,
        context: &WebGL2Context,
        member_key: (i32, i32),
        scene: &W3dScene,
        width: u32,
        height: u32,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> Result<(), JsValue> {
        self.ensure_shader(context)?;
        self.ensure_member_data(context, member_key, scene)?;

        let gl = context.gl();
        self.ensure_checker_texture(&gl);
        let shader = self.shader.as_ref().unwrap();

        // Render to DEFAULT framebuffer (no FBO)
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
        gl.viewport(0, 0, width as i32, height as i32);

        gl.clear_color(0.2, 0.2, 0.2, 1.0);
        gl.enable(WebGl2RenderingContext::DEPTH_TEST);
        gl.depth_func(WebGl2RenderingContext::LEQUAL);
        gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT | WebGl2RenderingContext::DEPTH_BUFFER_BIT);

        gl.enable(WebGl2RenderingContext::CULL_FACE);
        gl.cull_face(WebGl2RenderingContext::BACK);

        gl.use_program(Some(&shader.program));

        let (view_matrix, camera_pos) = self.build_view_matrix(scene, runtime_state);
        let projection_matrix = self.build_projection_matrix(scene, width as f32 / height as f32, runtime_state);

        gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &view_matrix);
        gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &projection_matrix);
        gl.uniform3f(shader.u_camera_pos.as_ref(), camera_pos[0], camera_pos[1], camera_pos[2]);

        self.setup_lights(gl, shader, scene, &camera_pos, runtime_state);
        gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
        gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
        gl.uniform1i(shader.u_has_texcoord2.as_ref(), 0);
        gl.uniform1i(shader.u_texcoord2_direct.as_ref(), 0);

        // Draw all meshes with proper material/texture binding
        if let Some(gpu_data) = self.member_data.get(&member_key) {
            let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
            gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &identity);

            // Reset per-frame uniforms to known defaults (mirrors the main scene path)
            let identity_uv = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0f32];
            gl.uniform_matrix4fv_with_f32_array(shader.u_tex_transform.as_ref(), false, &identity_uv);
            gl.uniform_matrix4fv_with_f32_array(shader.u_wrap_transform.as_ref(), false, &identity_uv);
            gl.uniform1i(shader.u_uv_proj_mode.as_ref(), 0);
            gl.uniform1i(shader.u_skinning_enabled.as_ref(), 0);

            // Try to find and bind material + texture from scene shaders
            let mut tex_bound = false;
            if let Some(mat) = scene.materials.iter().find(|m| !m.name.contains("Default")) {
                self.set_material_uniforms(gl, shader, mat);
            } else {
                self.bind_default_material(gl, shader, scene);
            }

            // Use the full texture-layer resolver so #wrapPlanar / wrapTransform /
            // textureTransform reach the shader. Pick the first shader that resolves
            // to a diffuse texture (mirror-trick W3D files have one shader, so this
            // is fine; multi-shader scenes aren't supported by world.image anyway).
            let mut bound_tex_mode: u8 = 0;
            for w3d_shader in &scene.shaders {
                if tex_bound { break; }
                let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                if layers.diffuse.is_some() {
                    bound_tex_mode = layers.diffuse_tex_mode;
                    tex_bound = Self::bind_texture_layers(gl, shader, &layers);
                }
            }
            if !tex_bound {
                gl.uniform1i(shader.u_has_texture.as_ref(), 0);
            }

            // For #wrapPlanar with an identity wrapTransform (the typical
            // newTexture-then-assign mirror trick case), Director auto-fits the
            // texture to the model's XY bounding box. Compute that fit here and
            // overwrite u_wrap_transform.
            //
            // Restrict the bbox to meshes whose owning node uses the bound shader —
            // otherwise extras like the mirror frame / base inflate the bbox and the
            // texture appears too small inside the actual reflective surface.
            if tex_bound && bound_tex_mode == 5 {
                use crate::director::chunks::w3d::types::W3dNodeType;
                // Restrict to meshes that are actually drawn — i.e. referenced by a
                // Model node. Unused mesh resources (e.g. "defaultmodel" placeholder)
                // would otherwise inflate the bbox and shrink the projected texture.
                let mut drawn_resources: std::collections::HashSet<String> = std::collections::HashSet::new();
                for node in &scene.nodes {
                    if node.node_type != W3dNodeType::Model { continue; }
                    if !node.model_resource_name.is_empty() {
                        drawn_resources.insert(node.model_resource_name.to_lowercase());
                    } else if !node.resource_name.is_empty() {
                        drawn_resources.insert(node.resource_name.trim().to_lowercase());
                    }
                }
                let mut min_x = f32::MAX;
                let mut max_x = f32::MIN;
                let mut min_y = f32::MAX;
                let mut max_y = f32::MIN;
                for (resource_name, meshes) in &scene.clod_meshes {
                    if !drawn_resources.is_empty()
                        && !drawn_resources.contains(&resource_name.to_lowercase())
                    {
                        continue;
                    }
                    for mesh in meshes {
                        for p in &mesh.positions {
                            if p[0] < min_x { min_x = p[0]; }
                            if p[0] > max_x { max_x = p[0]; }
                            if p[1] < min_y { min_y = p[1]; }
                            if p[1] > max_y { max_y = p[1]; }
                        }
                    }
                }
                if min_x.is_finite() && max_x > min_x && max_y > min_y {
                    let rx = (max_x - min_x).max(1e-6);
                    let ry = (max_y - min_y).max(1e-6);
                    let bbox_xform: [f32; 16] = [
                        1.0 / rx,    0.0,         0.0, 0.0,
                        0.0,         -1.0 / ry,   0.0, 0.0,
                        0.0,         0.0,         1.0, 0.0,
                        -min_x / rx, max_y / ry,  0.0, 1.0,
                    ];
                    gl.uniform_matrix4fv_with_f32_array(shader.u_wrap_transform.as_ref(), false, &bbox_xform);
                }
            }

            for mesh_group in gpu_data.mesh_groups.values() {
                for mesh_buf in mesh_group {
                    mesh_buf.bind(gl);
                    mesh_buf.draw(gl);
                    mesh_buf.unbind(gl);
                }
            }
        }

        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::CULL_FACE);

        Ok(())
    }

    /// Render a Shockwave3D scene to the FBO and return the resulting texture
    pub fn render_scene(
        &mut self,
        context: &WebGL2Context,
        member_key: (i32, i32),
        scene: &W3dScene,
        width: u32,
        height: u32,
    ) -> Result<Option<&WebGlTexture>, JsValue> {
        self.render_scene_with_state(context, member_key, scene, width, height, None)
    }

    /// Render with optional runtime state for transform overrides and animation
    pub fn render_scene_with_state(
        &mut self,
        context: &WebGL2Context,
        member_key: (i32, i32),
        scene: &W3dScene,
        width: u32,
        height: u32,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> Result<Option<&WebGlTexture>, JsValue> {
        self.render_scene_with_state_ex(context, member_key, scene, width, height, runtime_state, true)
    }

    /// Render with optional clearing control (for multi-camera setups)
    pub fn render_scene_with_state_ex(
        &mut self,
        context: &WebGL2Context,
        member_key: (i32, i32),
        scene: &W3dScene,
        width: u32,
        height: u32,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
        clear_fbo: bool,
    ) -> Result<Option<&WebGlTexture>, JsValue> {
        self.ensure_shader(context)?;
        self.ensure_fbo(context, width, height)?;
        self.ensure_member_data(context, member_key, scene)?;
        self.ensure_checker_texture(&context.gl());
        // Backdrops are drawn with the overlay quad later in this pass; ensure it
        // exists now while we still have &mut self (before the shader borrow).
        self.ensure_overlay_quad(&context.gl());

        // Sync lightmap UVs: check scene CLOD mesh tex_coords[1] and upload to GPU if new
        if let Some(gpu_data) = self.member_data.get_mut(&member_key) {
            for (resource_name, mesh_group) in gpu_data.mesh_groups.iter_mut() {
                if let Some(clod_meshes) = scene.clod_meshes.get(resource_name) {
                    for (mesh_idx, mesh_buf) in mesh_group.iter_mut().enumerate() {
                        if mesh_buf.meshdeform_uv_synced { continue; }
                        if let Some(mesh) = clod_meshes.get(mesh_idx) {
                            if mesh.tex_coords.len() >= 2 && !mesh.tex_coords[1].is_empty() {
                                mesh_buf.update_texcoord2(context.gl(), &mesh.tex_coords[1]);
                                mesh_buf.meshdeform_uv_synced = true;
                                // Log UV2 sync for MAP and Main models
                                if resource_name.contains("MAP") || resource_name.starts_with("map")
                                    || resource_name.starts_with("Main")
                                {
                                    log(&format!(
                                        "[W3D-UV2-SYNC] resource=\"{}\" mesh={} uv2_count={}",
                                        resource_name, mesh_idx, mesh.tex_coords[1].len()
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        let gl = context.gl();
        let shader = self.shader.as_ref().unwrap();
        let fbo = self.fbo.as_ref().unwrap();

        // Bind FBO
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(fbo));
        gl.viewport(0, 0, width as i32, height as i32);

        // Reset GL state that may have been left by 2D compositor.
        // CRITICAL: unbind textures from all units to prevent feedback loop.
        // The FBO texture may still be bound as a texture input from the 2D compositor's
        // previous frame. Rendering to an FBO whose texture is also bound as input is
        // undefined behavior in WebGL and silently discards draw calls.
        for unit in 0..4 {
            gl.active_texture(WebGl2RenderingContext::TEXTURE0 + unit);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
        }
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.disable(WebGl2RenderingContext::BLEND);
        gl.disable(WebGl2RenderingContext::SCISSOR_TEST);
        gl.disable(WebGl2RenderingContext::STENCIL_TEST);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        gl.enable(WebGl2RenderingContext::DEPTH_TEST);
        gl.depth_func(WebGl2RenderingContext::LEQUAL);
        if clear_fbo {
            gl.clear_color(0.2, 0.2, 0.2, 1.0);
            gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT | WebGl2RenderingContext::DEPTH_BUFFER_BIT);
        } else {
            // Only clear depth for additional cameras (so new geometry occludes properly)
            gl.clear(WebGl2RenderingContext::DEPTH_BUFFER_BIT);
        }

        // Enable backface culling
        // Y-flip in projection inverts winding → cull FRONT instead of BACK
        gl.enable(WebGl2RenderingContext::CULL_FACE);
        gl.cull_face(WebGl2RenderingContext::FRONT);
        gl.front_face(WebGl2RenderingContext::CCW);

        // Use 3D shader
        gl.use_program(Some(&shader.program));

        // Set up camera
        let (view_matrix, camera_pos) = self.build_view_matrix(scene, runtime_state);
        let projection_matrix = self.build_projection_matrix(scene, width as f32 / height as f32, runtime_state);

        gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &view_matrix);
        gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &projection_matrix);
        gl.uniform3f(shader.u_camera_pos.as_ref(), camera_pos[0], camera_pos[1], camera_pos[2]);

        // Set up lighting (pass camera pos for headlight direction)
        self.setup_lights(gl, shader, scene, &camera_pos, runtime_state);

        // Set texture samplers
        gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);   // unit 0 = base/diffuse
        gl.uniform1i(shader.u_lightmap_tex.as_ref(), 1);   // unit 1 = secondary layer
        gl.uniform1i(shader.u_layer2_tex.as_ref(), 2);     // unit 2 = third layer
        gl.uniform1i(shader.u_specular_tex.as_ref(), 3);   // unit 3 = specular map
        gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);   // default: no extra layers
        gl.uniform1i(shader.u_layer2_blend.as_ref(), 0);
        gl.uniform1i(shader.u_has_specular_map.as_ref(), 0);
        gl.uniform1i(shader.u_has_env_map.as_ref(), 0);
        gl.uniform1f(shader.u_reflectivity.as_ref(), 0.0);
        // Default texture transform = identity
        let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0f32];
        gl.uniform_matrix4fv_with_f32_array(shader.u_tex_transform.as_ref(), false, &identity);
        gl.uniform_matrix4fv_with_f32_array(shader.u_wrap_transform.as_ref(), false, &identity);
        gl.uniform1i(shader.u_uv_proj_mode.as_ref(), 0);    // default: mesh UVs
        gl.uniform1i(shader.u_skinning_enabled.as_ref(), 0); // default: no skinning
        gl.uniform1i(shader.u_shader_mode.as_ref(), 0);     // default: phong
        gl.uniform1f(shader.u_toon_steps.as_ref(), 3.0);    // default toon steps

        // Apply fog from runtime state or default off
        if let Some(rs) = runtime_state {
            if rs.fog_enabled {
                gl.uniform1i(shader.u_fog_enabled.as_ref(), 1);
                gl.uniform1f(shader.u_fog_near.as_ref(), rs.fog_near);
                gl.uniform1f(shader.u_fog_far.as_ref(), rs.fog_far);
                gl.uniform3f(shader.u_fog_color.as_ref(), rs.fog_color.0, rs.fog_color.1, rs.fog_color.2);
                gl.uniform1i(shader.u_fog_mode.as_ref(), rs.fog_mode as i32);
            } else {
                gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
            }

            // Apply background color from member's bgColor (parsed from 3DPR).
            // Only on the first pass (clear_fbo=true) — subsequent camera passes
            // (overlays, arrowcam) must NOT re-clear or they wipe the main scene.
            //
            // ALWAYS clear, regardless of whether background_color is explicitly
            // set. Skipping the clear left the FBO showing stale contents from
            // previous frames / uninitialised GPU memory (which on some GPUs
            // appears as solid white) — see ClubMarian where the world member
            // stores `bgColor = rgb(0, 0, 0)` but no explicit 3DPR background,
            // so the unset Option became "no clear" and the scene composited
            // over GPU garbage. Default to black matching Director's behaviour
            // for a freshly-initialised member; movies that need a different
            // background set it via Lingo (`member.bgColor = ...`) which feeds
            // back into runtime_state.background_color.
            if clear_fbo {
                let (r, g, b) = rs.background_color.unwrap_or((0, 0, 0));
                gl.clear_color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
                gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT | WebGl2RenderingContext::DEPTH_BUFFER_BIT);
            }
        } else {
            gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
        }

        // Skinning defaults to off (enabled when bone data is present)
        gl.uniform1i(shader.u_has_texcoord2.as_ref(), 0);
        gl.uniform1i(shader.u_texcoord2_direct.as_ref(), 0);

        // Draw camera backdrops (Director `addBackdrop`) BEHIND the scene: after the
        // colour clear, before any models, with depth test off so all geometry
        // occludes them. Only on the clearing (primary) pass — extra-camera passes
        // (clear_fbo=false) must not redraw them. The FBO is already bound, cleared,
        // and feedback-safe here, which avoids the stale/uninitialised-white that a
        // separate pre-pass produced. After drawing, the 3D camera matrices and GL
        // state are restored for the model loop.
        if clear_fbo {
            if let Some(rs) = runtime_state {
                let cam_key = self.active_camera.as_deref()
                    .unwrap_or("defaultview").to_ascii_lowercase();
                let backdrops = rs.camera_backdrops.get(&cam_key)
                    .filter(|b| !b.is_empty())
                    .or_else(|| {
                        // The sprite only has an active_camera when the movie called
                        // addCamera. When it didn't (it just used member.camera[1],
                        // like the estate explore), active_camera is None and the
                        // renderer defaults to DefaultView — fall back to whichever
                        // single camera owns backdrops. With an explicit camera, match
                        // strictly so a multi-camera movie doesn't cross backdrops.
                        if self.active_camera.is_none() {
                            rs.camera_backdrops.values().find(|b| !b.is_empty())
                        } else {
                            None
                        }
                    });
                if let Some(backdrops) = backdrops {
                    self.draw_backdrops_inline(gl, shader, &member_key, backdrops, width, height);
                    // Restore camera matrices + render state for the model loop.
                    gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &view_matrix);
                    gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &projection_matrix);
                    // Re-enable fog for the models (the backdrop disabled it; near/far/
                    // color/mode were set before the backdrop and are untouched).
                    gl.uniform1i(shader.u_fog_enabled.as_ref(), if rs.fog_enabled { 1 } else { 0 });
                    gl.enable(WebGl2RenderingContext::DEPTH_TEST);
                    gl.depth_mask(true);
                    gl.enable(WebGl2RenderingContext::CULL_FACE);
                    gl.cull_face(WebGl2RenderingContext::FRONT);
                    gl.front_face(WebGl2RenderingContext::CCW);
                    gl.disable(WebGl2RenderingContext::BLEND);
                }
            }
        }

        // Traverse scene graph and draw model nodes
        if self.member_data.contains_key(&member_key) {
            // Get set of nodes explicitly detached by Lingo (parent = VOID)
            let detached_nodes: std::collections::HashSet<&str> = runtime_state
                .map(|rs| rs.detached_nodes.iter().map(|s| s.as_str()).collect())
                .unwrap_or_default();

            // Check if active camera has a rootNode filter
            let root_node_filter: Option<String> = runtime_state.and_then(|rs| {
                self.active_camera.as_ref()
                    .and_then(|cam| rs.camera_root_nodes.get(&cam.to_ascii_lowercase()))
                    .cloned()
            });

            let model_nodes: Vec<&W3dNode> = scene.nodes.iter()
                .filter(|n| n.node_type == W3dNodeType::Model)
                .filter(|n| {
                    // Skip directly detached nodes
                    if detached_nodes.contains(n.name.as_str()) { return false; }

                    // Skip #particle models — their resource is a billboard placeholder;
                    // the particle system itself is drawn by render_particles, not as a
                    // static quad here.
                    let res = if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name };
                    if scene.model_resources.get(res)
                        .and_then(|r| r.primitive_type.as_deref())
                        .map(|t| t.eq_ignore_ascii_case("particle"))
                        .unwrap_or(false)
                    {
                        return false;
                    }

                    if let Some(ref root) = root_node_filter {
                        // Camera has rootNode: only render nodes in that subtree
                        self.is_child_of(scene, &n.name, root)
                    } else {
                        // No rootNode: render world-visible models only.
                        // Skip models whose parent (or ancestor) is detached — they belong
                        // to a different camera's rootNode subtree (e.g., overlay HUD models
                        // parented to a detached "overlays" camera).
                        !self.has_detached_ancestor(scene, &n.parent_name, &detached_nodes)
                    }
                })
                .collect();

            // One-time diagnostic logging per member
            if !self.logged_members.contains(&member_key) {
                self.logged_members.insert(member_key);
                let gpu_data = self.member_data.get(&member_key);
                let mesh_group_keys: Vec<String> = gpu_data.map(|d| d.mesh_groups.keys().cloned().collect()).unwrap_or_default();
                let model_names: Vec<String> = model_nodes.iter().map(|n| {
                    let res = if !n.model_resource_name.is_empty() { &n.model_resource_name } else { &n.resource_name };
                    format!("{}→{}", n.name, res)
                }).collect();
                log(&format!(
                    "[3D] Scene {:?}: {} model_nodes={:?}, mesh_groups={:?}, textures={}",
                    member_key, model_nodes.len(), model_names, mesh_group_keys,
                    gpu_data.map(|d| d.textures.len()).unwrap_or(0)
                ));
                // Log motion summary (count only, not per-track)
                log(&format!(
                    "[3D] {} motions, skeletons={:?}",
                    scene.motions.len(),
                    scene.skeletons.iter().map(|s| format!("{}({}b)", s.name, s.bones.len())).collect::<Vec<_>>(),
                ));
            }

            // Evaluate motion animations each frame
            self.motion_transforms.clear();
            self.motion_replace_transforms.clear();
            let multi_player = runtime_state.map(|rs| rs.bones_players.len() > 1).unwrap_or(false);
            if !scene.motions.is_empty() && multi_player {
                // Multiple per-model keyframe/bones players animating at once
                // (Splat pac-man: footA plays "footA-Key", footB plays "footB-Key").
                // The single member-level current_motion can only carry one, so apply
                // EACH model's own motion at its own clock. A single-track object
                // keyframe motion is applied to the model the player is on (so
                // identically-sourced clones with different names still animate);
                // a multi-track skeletal motion applies each track to its named bone.
                if let Some(rs) = runtime_state {
                    for (model_name, bp) in &rs.bones_players {
                        if !bp.animation_playing { continue; }
                        let motion_name = match &bp.current_motion { Some(m) => m, None => continue };
                        let motion = match scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(motion_name)) {
                            Some(m) => m, None => continue,
                        };
                        let duration = motion.duration();
                        let eff_end = if bp.animation_end_time >= 0.0 { bp.animation_end_time.min(duration) } else { duration };
                        let eff_start = bp.animation_start_time.min(eff_end);
                        let range = eff_end - eff_start;
                        if range <= 0.0 { continue; }
                        let t = if bp.animation_loop {
                            eff_start + ((bp.animation_time - eff_start) % range + range) % range
                        } else {
                            bp.animation_time.clamp(eff_start, eff_end)
                        };
                        let apply_to_model = motion.tracks.len() == 1;
                        for track in &motion.tracks {
                            let mut kf = track.evaluate(t);
                            if kf.scale_x.abs() < 1e-6 { kf.scale_x = 1.0; }
                            if kf.scale_y.abs() < 1e-6 { kf.scale_y = 1.0; }
                            if kf.scale_z.abs() < 1e-6 { kf.scale_z = 1.0; }
                            let m = keyframe_to_column_major_matrix(&kf);
                            if apply_to_model {
                                // Single-track object keyframe → replace the model's local transform.
                                // Key by lowercase: bones_players keys are lowercased but scene node
                                // names keep their original case (e.g. "footA"), so the node-draw
                                // lookup must match case-insensitively or the feet never animate.
                                self.motion_replace_transforms.insert(model_name.to_ascii_lowercase(), m);
                            } else {
                                // Multi-track skeletal → multiply each track onto its bone.
                                self.motion_transforms.insert(track.bone_name.clone(), m);
                            }
                        }
                    }
                }
            } else if !scene.motions.is_empty() {
                // Determine which motion to play: use runtime current_motion, or fallback to first
                let is_playing = runtime_state.map(|rs| rs.animation_playing).unwrap_or(true);
                let play_rate = runtime_state.map(|rs| rs.play_rate).unwrap_or(1.0);
                let anim_scale = runtime_state.map(|rs| rs.animation_scale).unwrap_or(1.0);
                let is_loop = runtime_state.map(|rs| rs.animation_loop).unwrap_or(true);
                let start_time = runtime_state.map(|rs| rs.animation_start_time).unwrap_or(0.0);
                let end_time = runtime_state.map(|rs| rs.animation_end_time).unwrap_or(-1.0);

                let current_motion_name = runtime_state.and_then(|rs| rs.current_motion.as_deref());

                // Detect motion change — sync animation_time from runtime state
                let motion_changed = current_motion_name != self.last_motion_name.as_deref();
                if motion_changed {
                    self.last_motion_name = current_motion_name.map(|s| s.to_string());
                    // Sync initial time from runtime state (set by play() offset)
                    self.animation_time = runtime_state.map(|rs| rs.animation_time).unwrap_or(0.0);
                    self.motion_ended = false;
                    // Sync blend state from runtime
                    self.blend_weight = runtime_state.map(|rs| rs.blend_weight).unwrap_or(1.0);
                    self.blend_elapsed = runtime_state.map(|rs| rs.blend_elapsed).unwrap_or(0.0);
                    self.blend_duration = runtime_state.map(|rs| rs.blend_duration).unwrap_or(0.0);
                }

                // The per-frame dt advance now lives on runtime_state in
                // events::tick_w3d_animations so that Lingo readers (the
                // bone.worldTransform getter used to pin the head to bone[6])
                // see the same time as the renderer. Mirror it here.
                self.animation_time = runtime_state.map(|rs| rs.animation_time).unwrap_or(self.animation_time);
                self.blend_weight = runtime_state.map(|rs| rs.blend_weight).unwrap_or(self.blend_weight);
                self.blend_elapsed = runtime_state.map(|rs| rs.blend_elapsed).unwrap_or(self.blend_elapsed);
                self.blend_duration = runtime_state.map(|rs| rs.blend_duration).unwrap_or(self.blend_duration);
                let _ = (play_rate, anim_scale);

                let motion = if let Some(name) = current_motion_name {
                    // Director is case-insensitive. ClubMarian queues
                    // "root-skeleton-Motion0" while the W3D file stores
                    // "root-skeleton-motion0" — a strict `==` here was
                    // dropping the motion silently.
                    scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(name))
                } else {
                    None // Don't apply a motion until the game explicitly calls play()
                };

                if let Some(motion) = motion {
                    let duration = motion.duration();
                    // Effective end time: use end_time if specified, else full duration
                    let eff_end = if end_time >= 0.0 { (end_time / 1.0).min(duration) } else { duration };
                    let eff_start = start_time.min(eff_end);
                    let range = eff_end - eff_start;

                    if range > 0.0 {
                        let t = if is_loop {
                            // Loop within [start_time, end_time]
                            eff_start + ((self.animation_time - eff_start) % range + range) % range
                        } else {
                            self.animation_time.clamp(eff_start, eff_end)
                        };

                        for track in &motion.tracks {
                            let mut kf = track.evaluate(t);
                            if kf.scale_x.abs() < 1e-6 { kf.scale_x = 1.0; }
                            if kf.scale_y.abs() < 1e-6 { kf.scale_y = 1.0; }
                            if kf.scale_z.abs() < 1e-6 { kf.scale_z = 1.0; }
                            let m = keyframe_to_column_major_matrix(&kf);
                            self.motion_transforms.insert(track.bone_name.clone(), m);
                        }

                        // Check if non-looping motion has ended
                        if !is_loop && self.animation_time >= eff_end && !self.motion_ended {
                            self.motion_ended = true;
                        }
                    }
                }
            }

            if model_nodes.is_empty() {
                // No model nodes — fallback: draw all meshes with identity transform
                self.draw_all_meshes_fallback(gl, shader, scene, &member_key);
            } else {
                // Classify nodes into opaque, cutout, and transparent for proper order.
                // - opaque (material opacity ≥ 1, no alpha texture): pass 1, depth write.
                // - cutout (opacity ≥ 1 but the texture carries alpha, e.g. frog01's
                //   Flash bark/leaf textures): pass 1b, alpha-tested, depth write — so it
                //   occludes the translucent water planes instead of being sorted behind
                //   them (the logs were turning blue when they drifted off-centre).
                // - transparent (opacity < 1, e.g. water2 blend=50): pass 2, back-to-front.
                let mut transparent_nodes: Vec<(&W3dNode, f32)> = Vec::new(); // (node, distance_to_camera)
                let mut cutout_nodes: Vec<&W3dNode> = Vec::new();

                // Sort: skybox nodes first so they render before scene geometry
                let mut sorted_model_nodes: Vec<&W3dNode> = model_nodes.iter().copied().collect();
                sorted_model_nodes.sort_by_key(|n| {
                    if n.name.starts_with("SB_") && n.parent_name.contains("SkyBox") { 0 } else { 1 }
                });

                // PASS 1: Render opaque geometry (skybox first, then scene)
                gl.uniform1f(shader.u_alpha_threshold.as_ref(), 0.0);
                for model_node in &sorted_model_nodes {
                    if let Some(rs) = runtime_state {
                        if let Some(&vis_mode) = rs.node_visibility.get(&model_node.name) {
                            if vis_mode == 0 { continue; } // #none → skip
                        }
                    }
                    // Check if this model is transparent
                    let opacity = self.get_model_opacity(scene, model_node, runtime_state);
                    // Translucent (blend<100) OR a script-marked `transparent` shader
                    // (soft alpha blend, e.g. the galaxy glow plane at blend=100) →
                    // transparent pass, sorted back-to-front. Without the transparent-shader
                    // branch such a plane fell to the cutout pass and rendered as a hard
                    // opaque disk instead of a soft glow.
                    // Director gates transparency on shader.blend (default 100 = opaque)
                    // and shader.transparent, NOT on the W3D material's opacity field. So a
                    // low material opacity only means "translucent" when the surface isn't
                    // an opaque textured solid — otherwise finalDrive's `chassis` (material
                    // opacity 0.2, opaque camo texture) rendered 20% see-through, letting you
                    // look through the car body at the passengers.
                    let is_transparent = Self::model_uses_transparent_shader(model_node, runtime_state)
                        || (opacity < 0.999
                            && !self.model_has_opaque_texture(scene, model_node, &member_key, runtime_state));
                    if is_transparent {
                        let world_matrix = self.accumulate_transform_with_state(scene, model_node, runtime_state);
                        let dx = world_matrix[12] - camera_pos[0];
                        let dy = world_matrix[13] - camera_pos[1];
                        let dz = world_matrix[14] - camera_pos[2];
                        transparent_nodes.push((model_node, dx*dx + dy*dy + dz*dz));
                        continue;
                    }
                    if self.model_has_alpha_texture(scene, model_node, &member_key, runtime_state) {
                        // Opaque material but alpha-keyed texture → cutout (alpha-tested
                        // opaque draw); keep depth writes so it occludes translucent water.
                        cutout_nodes.push(model_node);
                        continue;
                    }

                    self.draw_model_node(gl, shader, scene, model_node, &member_key, runtime_state, &view_matrix, &projection_matrix, false);
                }

                // PASS 1b: Cutout geometry — opaque pass with alpha-test discard so
                // transparent texels write neither colour nor depth (hard edges, but
                // correct occlusion vs. the translucent water planes).
                if !cutout_nodes.is_empty() {
                    gl.depth_mask(true);
                    gl.uniform1f(shader.u_alpha_threshold.as_ref(), 0.5);
                    for model_node in &cutout_nodes {
                        self.draw_model_node(gl, shader, scene, model_node, &member_key, runtime_state, &view_matrix, &projection_matrix, false);
                    }
                    gl.uniform1f(shader.u_alpha_threshold.as_ref(), 0.0);
                }

                // PASS 2: Render transparent geometry (back-to-front, no depth writes)
                if !transparent_nodes.is_empty() {
                    transparent_nodes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    gl.depth_mask(false); // Disable depth writes for transparent objects
                    gl.enable(WebGl2RenderingContext::BLEND);
                    gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);

                    for (model_node, _dist) in &transparent_nodes {
                        self.draw_model_node(gl, shader, scene, model_node, &member_key, runtime_state, &view_matrix, &projection_matrix, true);
                    }

                    gl.depth_mask(true);
                    gl.disable(WebGl2RenderingContext::BLEND);
                }
            }
        }

        // Render ShaderInker outlines (after geometry, before particles)
        let _ = self.render_inker_outlines(context, scene, &member_key, &view_matrix, &projection_matrix, runtime_state);

        // Re-activate main shader after outline pass (particles need it or their own shader)
        if let Some(ref shader) = self.shader {
            gl.use_program(Some(&shader.program));
        }

        // Render particles (after opaque geometry), alpha-blended.
        let _ = self.render_particles(context, &member_key, runtime_state, &view_matrix, &projection_matrix);

        // Note: overlays are rendered AFTER all camera passes, not per-camera

        // Apply post-processing effects
        if runtime_state.map(|rs| rs.bloom_enabled).unwrap_or(false) {
            let threshold = runtime_state.map(|rs| rs.bloom_threshold).unwrap_or(0.5);
            let intensity = runtime_state.map(|rs| rs.bloom_intensity).unwrap_or(0.5);
            let _ = self.apply_bloom(context, threshold, intensity);
        }

        // Restore state
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);

        Ok(self.fbo_texture.as_ref())
    }

    /// Ensure overlay quad buffers exist (created once, reused every frame)
    fn ensure_overlay_quad(&mut self, gl: &WebGl2RenderingContext) {
        if self.overlay_quad_vbo.is_some() { return; }
        let verts: [f32; 36] = [
            0.0, 0.0, 0.0,  0.0, 0.0, 1.0,
            1.0, 0.0, 0.0,  0.0, 0.0, 1.0,
            1.0, 1.0, 0.0,  0.0, 0.0, 1.0,
            0.0, 0.0, 0.0,  0.0, 0.0, 1.0,
            1.0, 1.0, 0.0,  0.0, 0.0, 1.0,
            0.0, 1.0, 0.0,  0.0, 0.0, 1.0,
        ];
        let uvs: [f32; 12] = [
            0.0, 0.0,  1.0, 0.0,  1.0, 1.0,
            0.0, 0.0,  1.0, 1.0,  0.0, 1.0,
        ];
        let vbo = gl.create_buffer();
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, vbo.as_ref());
        unsafe {
            let view = js_sys::Float32Array::view(&verts);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER, &view, WebGl2RenderingContext::STATIC_DRAW);
        }
        self.overlay_quad_vbo = vbo;

        let uv_buf = gl.create_buffer();
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, uv_buf.as_ref());
        unsafe {
            let view = js_sys::Float32Array::view(&uvs);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER, &view, WebGl2RenderingContext::STATIC_DRAW);
        }
        self.overlay_quad_uv = uv_buf;
    }

    /// Render overlays to the existing FBO (called after all camera passes)
    pub fn render_overlays_to_fbo(
        &mut self,
        context: &WebGL2Context,
        member_key: &(i32, i32),
        overlays: &[crate::player::cast_member::CameraOverlay],
        width: u32,
        height: u32,
    ) {
        if overlays.is_empty() { return; }
        let gl = context.gl();
        self.ensure_overlay_quad(&gl);
        let shader = match self.shader.as_ref() { Some(s) => s, None => return };
        let gpu_data = match self.member_data.get(member_key) { Some(d) => d, None => return };
        let fbo = match self.fbo.as_ref() { Some(f) => f, None => return };

        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(fbo));
        gl.viewport(0, 0, width as i32, height as i32);
        gl.use_program(Some(&shader.program));

        // Set up 2D orthographic state
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);

        let w = width as f32;
        let h = height as f32;
        // Ortho projection: (0,0)=top-left in screen space
        // FBO is Y-flipped when composited, so use positive Y (no flip here)
        let ortho: [f32; 16] = [
            2.0/w,  0.0,    0.0, 0.0,
            0.0,    2.0/h,  0.0, 0.0,
            0.0,    0.0,   -1.0, 0.0,
           -1.0,   -1.0,    0.0, 1.0,
        ];
        let identity: [f32; 16] = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
        gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &ortho);
        gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &identity);
        gl.uniform1i(shader.u_num_lights.as_ref(), 0);
        gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
        gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);
        gl.uniform1i(shader.u_layer2_blend.as_ref(), 0);
        gl.uniform1i(shader.u_has_specular_map.as_ref(), 0);
        gl.uniform1i(shader.u_shader_mode.as_ref(), 0);
        gl.uniform1i(shader.u_has_vertex_color.as_ref(), 0);
        // Signal overlay mode: u_skinning_enabled = -1 → skip CLOD UV remap in vertex shader
        gl.uniform1i(shader.u_skinning_enabled.as_ref(), -1);
        // Reset texture transform to identity for overlays
        let ov_identity = [1.0f32,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
        gl.uniform_matrix4fv_with_f32_array(shader.u_tex_transform.as_ref(), false, &ov_identity);
        gl.uniform_matrix4fv_with_f32_array(shader.u_wrap_transform.as_ref(), false, &ov_identity);
        gl.uniform1i(shader.u_uv_proj_mode.as_ref(), 0);  // overlays use authored UVs
        gl.uniform4f(shader.u_emissive_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
        gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
        gl.uniform4f(shader.u_ambient_color.as_ref(), 0.0, 0.0, 0.0, 1.0);

        // Bind quad VBOs once
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, self.overlay_quad_vbo.as_ref());
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 3, WebGl2RenderingContext::FLOAT, false, 24, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(1, 3, WebGl2RenderingContext::FLOAT, false, 24, 12);

        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, self.overlay_quad_uv.as_ref());
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_with_i32(2, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);

        for overlay in overlays {
            if overlay.source_texture.is_empty() || overlay.blend <= 0.0 { continue; }
            let tex = match gpu_data.textures.get(&overlay.source_texture_lower) {
                Some(t) => t,
                None => continue,
            };
            let (tex_w, tex_h) = gpu_data.texture_sizes
                .get(&overlay.source_texture_lower)
                .map(|&(w, h)| (w as f32, h as f32))
                .unwrap_or((64.0, 64.0));

            gl.active_texture(WebGl2RenderingContext::TEXTURE0);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
            // Use NEAREST filtering for overlays — crisp pixel-perfect text/HUD rendering
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::NEAREST as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::NEAREST as i32);
            gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
            gl.uniform1i(shader.u_has_texture.as_ref(), 1);
            gl.uniform1f(shader.u_opacity.as_ref(), (overlay.blend / 100.0) as f32);

            let x = overlay.loc[0] as f32;
            let y = overlay.loc[1] as f32;
            let sx = (overlay.scale * overlay.scale_x) as f32;
            let sy = (overlay.scale * overlay.scale_y) as f32;
            let rx = overlay.reg_point[0] as f32;
            let ry = overlay.reg_point[1] as f32;
            let rot_rad = (overlay.rotation as f32).to_radians();
            let cos_r = rot_rad.cos();
            let sin_r = rot_rad.sin();

            // 2D transform: Scale → Rotate → Translate (with regPoint offset).
            // Director's camera.overlay[n].regPoint is in SCALED/destination pixels,
            // NOT source-texture pixels: e.g. Rasterwerks' rifle scope reticle sets
            // reg = GetScale(#center) = texCenter × scale (= 256×128 × 3.06 = 783×392)
            // so the screen rect is `loc - regPoint .. loc - regPoint + texSize×scale`,
            // centred on loc. The reg term must therefore NOT be multiplied by the
            // scale again — doing so pushed the scaled reticle far off-screen while
            // every scale=1 HUD layer (rx*sx == rx) was unaffected, so only the scope
            // tube vanished. translate = loc − R·regPoint.
            let sw = sx * tex_w;
            let sh = sy * tex_h;
            let model: [f32; 16] = [
                cos_r * sw, sin_r * sw, 0.0, 0.0,
               -sin_r * sh, cos_r * sh, 0.0, 0.0,
                0.0,        0.0,        1.0, 0.0,
                x - rx * cos_r + ry * sin_r,
                y - rx * sin_r - ry * cos_r,
                0.0, 1.0,
            ];
            gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &model);
            gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 6);
            // Restore texture filtering so 3D rendering is not affected
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::LINEAR_MIPMAP_LINEAR as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::LINEAR as i32);
        }

        // Restore state
        gl.disable_vertex_attrib_array(0);
        gl.disable_vertex_attrib_array(1);
        gl.disable_vertex_attrib_array(2);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, None);
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
    }

    // Old render_overlays_static removed — functionality merged into render_overlays_to_fbo

    /// Render camera backdrops into the FBO. Backdrops are positioned 2D images
    /// drawn BEHIND the 3D scene (Director 11.5 `addBackdrop`): each is a textured
    /// quad placed by loc/scale/regPoint/rotation, with loc measured from the
    /// sprite's upper-left corner. This is the backdrop counterpart of
    /// render_overlays_to_fbo (which draws on top). It must run before the scene's
    /// geometry: it clears the FBO to the member background colour, fills it with
    /// the backdrop images, and leaves the colour buffer intact so the subsequent
    /// `render_scene_with_state_ex(clear_fbo=false)` only clears depth and composites
    /// the models on top.
    /// Draw camera backdrops into the CURRENTLY-BOUND, ALREADY-CLEARED FBO, in the
    /// middle of render_scene_with_state_ex (after the clear, before the models).
    /// Each backdrop is a positioned 2D quad (loc/scale/regPoint/rotation, loc from
    /// the sprite's upper-left per the Director dictionary), drawn with depth test
    /// off so all geometry occludes it. The sky is shown unlit/full-bright: emissive
    /// is forced to white so the textured-path lighting clamps to 1 and the texture
    /// shows as-is, which also means u_num_lights is left untouched (the model loop
    /// keeps its lighting). The caller restores u_view/u_projection + GL state after.
    fn draw_backdrops_inline(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        member_key: &(i32, i32),
        backdrops: &[crate::player::cast_member::CameraOverlay],
        width: u32,
        height: u32,
    ) {
        let gpu_data = match self.member_data.get(member_key) { Some(d) => d, None => return };

        // Bind the default VAO before touching vertex attrib pointers. Otherwise the
        // vertexAttribPointer calls below would write onto whatever VAO is currently
        // bound — which is the 2D compositor's sprite-quad VAO (left bound from the
        // previous sprite). The 3D scene still renders (models bind their own VAOs),
        // but the compositor would then draw every sprite, including this 3D one, with
        // a corrupted quad → a degenerate / full-screen white quad ("all white").
        gl.bind_vertex_array(None);

        // 2D orthographic state — depth off (behind everything), no cull, alpha blend.
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        // CRITICAL: also disable depth WRITES. On ANGLE/D3D (Windows) disabling the
        // depth test alone does not reliably stop depth writes, so the full-screen
        // backdrop quad would stamp its depth (~0.5) over the whole buffer and then
        // every house surface farther than that fails the LEQUAL test and vanishes —
        // leaving only the sky ("fades to white"). The skybox model path does the
        // same. The caller restores depth_mask(true) before the model loop.
        gl.depth_mask(false);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);

        let w = width as f32;
        let h = height as f32;
        // Ortho: (0,0)=top-left in sprite space. FBO is Y-flipped when composited, so
        // use positive Y here (matches render_overlays_to_fbo).
        let ortho: [f32; 16] = [
            2.0/w,  0.0,    0.0, 0.0,
            0.0,    2.0/h,  0.0, 0.0,
            0.0,    0.0,   -1.0, 0.0,
           -1.0,   -1.0,    0.0, 1.0,
        ];
        let identity: [f32; 16] = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
        gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &ortho);
        gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &identity);
        // Unlit full-bright: emissive=1 → lighting clamps to 1 → final = texture.
        gl.uniform4f(shader.u_emissive_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
        gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
        gl.uniform4f(shader.u_ambient_color.as_ref(), 0.0, 0.0, 0.0, 1.0);
        gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);
        gl.uniform1i(shader.u_layer2_blend.as_ref(), 0);
        gl.uniform1i(shader.u_has_specular_map.as_ref(), 0);
        gl.uniform1i(shader.u_shader_mode.as_ref(), 0);
        gl.uniform1i(shader.u_has_vertex_color.as_ref(), 0);
        gl.uniform1i(shader.u_has_texcoord2.as_ref(), 0);
        // The 2D backdrop is the scene background — never fogged (the caller
        // re-enables fog for the models afterward).
        gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
        // u_skinning_enabled = -1 → vertex shader skips CLOD UV remap for the quad.
        gl.uniform1i(shader.u_skinning_enabled.as_ref(), -1);
        gl.uniform1i(shader.u_uv_proj_mode.as_ref(), 0);
        gl.uniform_matrix4fv_with_f32_array(shader.u_tex_transform.as_ref(), false, &identity);
        gl.uniform_matrix4fv_with_f32_array(shader.u_wrap_transform.as_ref(), false, &identity);

        // Bind quad VBOs once
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, self.overlay_quad_vbo.as_ref());
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 3, WebGl2RenderingContext::FLOAT, false, 24, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(1, 3, WebGl2RenderingContext::FLOAT, false, 24, 12);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, self.overlay_quad_uv.as_ref());
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_with_i32(2, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);

        for backdrop in backdrops {
            if backdrop.source_texture.is_empty() || backdrop.blend <= 0.0 { continue; }
            let tex = match gpu_data.textures.get(&backdrop.source_texture_lower) {
                Some(t) => t,
                None => continue,
            };
            let (tex_w, tex_h) = gpu_data.texture_sizes
                .get(&backdrop.source_texture_lower)
                .map(|&(w, h)| (w as f32, h as f32))
                .unwrap_or((64.0, 64.0));

            gl.active_texture(WebGl2RenderingContext::TEXTURE0);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
            gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
            gl.uniform1i(shader.u_has_texture.as_ref(), 1);
            gl.uniform1f(shader.u_opacity.as_ref(), (backdrop.blend / 100.0) as f32);

            let x = backdrop.loc[0] as f32;
            let y = backdrop.loc[1] as f32;
            let sx = (backdrop.scale * backdrop.scale_x) as f32;
            let sy = (backdrop.scale * backdrop.scale_y) as f32;
            let rx = backdrop.reg_point[0] as f32;
            let ry = backdrop.reg_point[1] as f32;
            let rot_rad = (backdrop.rotation as f32).to_radians();
            let cos_r = rot_rad.cos();
            let sin_r = rot_rad.sin();

            let sw = sx * tex_w;
            let sh = sy * tex_h;
            let model: [f32; 16] = [
                cos_r * sw, sin_r * sw, 0.0, 0.0,
               -sin_r * sh, cos_r * sh, 0.0, 0.0,
                0.0,        0.0,        1.0, 0.0,
                x - rx * sx * cos_r + ry * sy * sin_r,
                y - rx * sx * sin_r - ry * sy * cos_r,
                0.0, 1.0,
            ];
            gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &model);
            gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 6);
        }

        gl.disable_vertex_attrib_array(0);
        gl.disable_vertex_attrib_array(1);
        gl.disable_vertex_attrib_array(2);
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, None);
    }

    /// Fallback: draw all meshes with identity transform when no scene graph
    fn draw_all_meshes_fallback(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        scene: &W3dScene,
        member_key: &(i32, i32),
    ) {
        let identity = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &identity);
        self.bind_default_material(gl, shader, scene);

        if let Some(gpu_data) = self.member_data.get(member_key) {
            for mesh_group in gpu_data.mesh_groups.values() {
                for mesh_buf in mesh_group {
                    mesh_buf.bind(gl);
                    mesh_buf.draw(gl);
                    mesh_buf.unbind(gl);
                }
            }
            for mesh_buf in &gpu_data.all_meshes {
                mesh_buf.bind(gl);
                mesh_buf.draw(gl);
                mesh_buf.unbind(gl);
            }
        }
    }

    /// Accumulate world transform by walking parent chain, using runtime overrides when available
    fn accumulate_transform_with_state(
        &self,
        scene: &W3dScene,
        node: &W3dNode,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> [f32; 16] {
        // Bone motion tracks share names with skeleton/model roots (for example "Bip01").
        // Applying those tracks to Model nodes with skeletons would animate the root twice:
        // once through skinning and again through u_model.
        // But Model nodes WITHOUT skeletons need motion overrides for keyframe animation.
        let allow_motion_override = if node.node_type == W3dNodeType::Model {
            // Allow keyframe animation on models that have no skeleton.
            // For W3D model nodes the skinned resource key can live in
            // model_resource_name instead of resource_name; if we only check
            // resource_name, a root bone track like "bip01" gets applied twice:
            // once in skinning and again through u_model.
            let skeleton_key = if !node.model_resource_name.is_empty() {
                node.model_resource_name.as_str()
            } else {
                node.resource_name.as_str()
            };
            !scene.skeletons.iter().any(|s| s.name == skeleton_key && s.bones.len() > 1)
        } else {
            true
        };

        // A model explicitly placed via Lingo `transform.position` (e.g. a cloned
        // On the Run bonus, repositioned after cloning) has a runtime transform
        // override. When such a model ALSO runs a keyframePlayer motion, place it
        // with the runtime transform and apply the keyframe LOCALLY (runtime *
        // motion) — otherwise the spin would be applied to the stale parsed base
        // (the template's position) and the model renders nowhere near where
        // `worldPosition` reports it. Models WITHOUT a runtime override (Splat's
        // per-part keyframes) keep `motion * base`, relative to the rest pose.
        let runtime_override: Option<[f32; 16]> =
            runtime_state.and_then(|rs| get_runtime_transform(rs, &node.name));
        // Get this node's transform: motion (combined with base), runtime override, or parsed
        let node_transform = if allow_motion_override {
            if let Some(km) = (!self.motion_replace_transforms.is_empty())
                .then(|| self.motion_replace_transforms.get(&node.name.to_ascii_lowercase()))
                .flatten()
            {
                // Lookup is case-insensitive (bones_players keys are lowercased;
                // node names aren't).
                match runtime_override {
                    Some(rt) => mat4_multiply_col_major(&rt, km),
                    None => mat4_multiply_col_major(km, &node.transform),
                }
            } else if let Some(motion_t) = self.motion_transforms.get(&node.name) {
                match runtime_override {
                    Some(rt) => mat4_multiply_col_major(&rt, motion_t),
                    None => mat4_multiply_col_major(motion_t, &node.transform),
                }
            } else {
                runtime_override.unwrap_or(node.transform)
            }
        } else {
            runtime_override.unwrap_or(node.transform)
        };

        let mut chain = vec![node_transform];
        let mut current_parent = node.parent_name.as_str();

        // Walk up parent chain. Director node names are case-insensitive, and
        // get_runtime_transform already looks them up that way — but the parent
        // NODE lookup must match it. A case-sensitive `==` here fails to find a
        // parent whose stored name differs in case from the child's parent_name
        // (cloneModelFromCastmember preserves the SOURCE member's casing for
        // re-parented sub-nodes), which silently breaks the chain and renders the
        // node at its raw local transform — e.g. the frog's deep limb hierarchy
        // (frog→axe→body→lhip→ll→…→lf) collapsing toward the origin.
        while !current_parent.is_empty()
            && !current_parent.eq_ignore_ascii_case("world")
            && current_parent != "<world>"
        {
            if let Some(parent_node) = scene.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(current_parent)) {
                let parent_t = runtime_state
                    .and_then(|rs| get_runtime_transform(rs, &parent_node.name))
                    .unwrap_or(parent_node.transform);
                chain.push(parent_t);
                current_parent = parent_node.parent_name.as_str();
            } else {
                break;
            }
        }

        // Multiply from root to leaf: parent * ... * node
        let mut result = IDENTITY_4X4;
        for t in chain.into_iter().rev() {
            result = mat4_multiply_col_major(&result, &t);
        }
        result
    }

    /// Draw a single model node (extracted for opaque/transparent pass reuse).
    fn draw_model_node(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        scene: &W3dScene,
        model_node: &W3dNode,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
        view_matrix: &[f32; 16],
        projection_matrix: &[f32; 16],
        force_blend: bool,
    ) {
        let resource = if !model_node.model_resource_name.is_empty() {
            &model_node.model_resource_name
        } else {
            &model_node.resource_name
        };
        let res_info = scene.model_resources.get(resource);

        // Rasterwerks tags its skybox nodes `SB_*` under a `*SkyBox*` parent; other
        // movies just name a big camera-enclosing box "skybox" (unicraft clones one
        // from a cast member and scales it ×1000). Both need the same treatment:
        // render inside-out (no cull), camera-centered, past the normal far plane —
        // otherwise the box's inner faces are culled/clipped and the starfield
        // background is missing (only the foreground galaxy plane shows).
        let is_skybox = (model_node.name.starts_with("SB_") && model_node.parent_name.contains("SkyBox"))
            || model_node.name.to_ascii_lowercase().contains("skybox");
        let mut vis_mode = 1u8; // default #front

        if let Some(gpu_data) = self.member_data.get(member_key) {
            let has_skeleton_data = self.setup_skinning_for_resource(
                gl, shader, scene, resource, &model_node.name, gpu_data, runtime_state,
            );

            let world_matrix = self.accumulate_transform_with_state(scene, model_node, runtime_state);
            if has_skeleton_data {
                let has_runtime_model_override = runtime_state
                    .map(|rs| rs.node_transforms.contains_key(&model_node.name))
                    .unwrap_or(false);

                if has_runtime_model_override {
                    // If Lingo is explicitly driving model.transform, use that
                    // authored transform directly. Applying the legacy skinned
                    // basis correction here would rotate the whole skeleton a
                    // second time for scripted movies like the dinosaur test.
                    gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &world_matrix);
                } else {
                    // Passive W3D skinned content still needs the historical
                    // Z-up -> render-basis correction.
                    let mut m = world_matrix;
                    for col in 0..3 {
                        let o = col * 4;
                        let r1 = m[o + 1];
                        let r2 = m[o + 2];
                        m[o + 1] = r2;
                        m[o + 2] = -r1;
                    }
                    gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &m);
                }
            } else {
                gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &world_matrix);
            }

            // Per-model visibility culling
            // Only explicitly set #both or #back modes change GL state.
            // Default (#front / no entry) keeps the global cull_face(FRONT).
            vis_mode = if is_skybox {
                gl.disable(WebGl2RenderingContext::CULL_FACE);
                gl.depth_mask(false);
                // Rasterwerks draws its skybox through a dedicated sky camera
                // (rootNode = detached "NodeSkyBox") so it's parallax-free and never
                // clipped. Approximate that inline: strip the view translation so the
                // cube is camera-centered (no parallax), and extend the far plane —
                // the cube is scaled to 32000 (faces at ±16000), far beyond the main
                // camera's far plane, so it was being clipped to nothing.
                let mut sky_view = *view_matrix;
                sky_view[12] = 0.0;
                sky_view[13] = 0.0;
                sky_view[14] = 0.0;
                gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &sky_view);
                let near = projection_matrix[14] / (projection_matrix[10] - 1.0);
                // Generous far: the box is camera-centered and drawn depth-masked, so
                // precision is irrelevant; it only must not clip a large scaled skybox
                // (Rasterwerks ≈32000; unicraft's is scaled ×1000 to hundreds of thousands).
                let far = 8_000_000.0f32;
                let nf = 1.0 / (near - far);
                let mut sky_proj = *projection_matrix;
                sky_proj[10] = (far + near) * nf;
                sky_proj[14] = 2.0 * far * near * nf;
                gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &sky_proj);
                3u8
            } else {
                let mode = runtime_state
                    .and_then(|rs| rs.node_visibility.get(&model_node.name))
                    .copied()
                    .unwrap_or(1); // no entry = default #front
                if mode == 2 || mode == 3 {
                    // #back or #both — disable culling (show both sides)
                    gl.disable(WebGl2RenderingContext::CULL_FACE);
                }
                mode
            };

            if let Some(mesh_group) = gpu_data.mesh_groups.get(resource) {
                for (mesh_idx, mesh_buf) in mesh_group.iter().enumerate() {
                    let bound = self.bind_material_for_mesh(
                        gl, shader, scene, model_node,
                        res_info, mesh_idx, member_key, runtime_state, force_blend,
                    );
                    if !bound {
                        self.bind_material(gl, shader, scene, model_node, member_key, runtime_state, force_blend);
                    }
                    // Reflection map last so the per-mesh candidate search can't clobber it.
                    self.apply_reflection_map(gl, shader, scene, model_node, member_key, runtime_state);

                    if mesh_buf.has_bones && has_skeleton_data {
                        gl.uniform1i(shader.u_skinning_enabled.as_ref(), 1);
                    } else {
                        gl.uniform1i(shader.u_skinning_enabled.as_ref(), 0);
                    }
                    gl.uniform1i(shader.u_has_vertex_color.as_ref(),
                        if mesh_buf.has_vertex_colors { 1 } else { 0 });
                    gl.uniform1i(shader.u_has_texcoord2.as_ref(),
                        if mesh_buf.has_texcoord2 { 1 } else { 0 });
                    gl.uniform1i(
                        shader.u_texcoord2_direct.as_ref(),
                        if mesh_buf.texcoord2_direct { 1 } else { 0 },
                    );
                    // Force the non-textured fragment path for UV-less meshes
                    // (e.g. ClubMarian's heightmap terrain built via
                    // `newMesh(name, faces, verts, 0_uvs, ...)` — without
                    // UVs the diffuse texture would otherwise sample at (0,0)
                    // and tint the whole surface with one texel).
                    if !mesh_buf.has_texcoord {
                        gl.uniform1i(shader.u_has_texture.as_ref(), 0);
                    }

                    mesh_buf.bind(gl);
                    mesh_buf.draw(gl);
                    mesh_buf.unbind(gl);
                }
            } else {
                // Log missing mesh data — deduplicate by model name
                use std::sync::Mutex;
                use std::collections::HashSet;
                static LOGGED_MISS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
                if let Ok(mut guard) = LOGGED_MISS.lock() {
                    let set = guard.get_or_insert_with(HashSet::new);
                    if set.insert(model_node.name.clone()) {
                        console_warn!(
                            "[W3D-MISS] model=\"{}\" resource=\"{}\" (res=\"{}\", mres=\"{}\") — NOT in mesh_groups({} keys). parent=\"{}\"",
                            model_node.name, resource, model_node.resource_name,
                            model_node.model_resource_name, gpu_data.mesh_groups.len(), model_node.parent_name,
                        );
                    }
                }
            }
        }
        // Restore culling/depth state if changed
        if is_skybox {
            gl.enable(WebGl2RenderingContext::CULL_FACE);
            gl.cull_face(WebGl2RenderingContext::FRONT); // restore default
            gl.depth_mask(true);
            // Restore the world view/projection for subsequent (non-skybox) models.
            gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, view_matrix);
            gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, projection_matrix);
        } else if vis_mode >= 2 {
            // Restore default culling after #back or #both
            gl.enable(WebGl2RenderingContext::CULL_FACE);
            gl.cull_face(WebGl2RenderingContext::FRONT);
        }
    }

    /// Get the opacity of a model node's material (for transparency sorting).
    /// Look up a per-model shader override.  Returns the first available:
    /// mesh-specific index → index 0 fallback → lowest set index → None.
    ///
    /// The lowest-set-index fallback handles Director's 2-sided #plane idiom:
    /// a plane has a front (mesh 0) and back (mesh 1) face, and movies texture
    /// only the face that points at the camera after the model's rotation —
    /// e.g. frog01's water does `model("water").shaderList[2] = waterS` (1-based
    /// → index 1) and leaves shaderList[1] unset. Because the water is rotated
    /// -90° about X, the VISIBLE face is mesh 0, which would otherwise resolve to
    /// no override and render the default checker. Falling back to any set shader
    /// puts the intended texture on the visible face. Models that set index 0
    /// (whole-list assignment) or every index are unaffected.
    fn node_shader_override<'a>(
        rs: &'a crate::player::cast_member::Shockwave3dRuntimeState,
        node_name: &str,
        mesh_idx: Option<usize>,
    ) -> Option<&'a String> {
        rs.node_shaders.get(node_name).and_then(|m| {
            match mesh_idx {
                Some(idx) => m.get(&idx)
                    // Whole-model fallback: a `model.shaderList = shader` assignment
                    // (applied to every mesh) is stored as the SOLE override at index
                    // 0. Only then does a mesh without its own entry inherit it. When
                    // the script set specific indices (`shaderList[1]`, `shaderList[2]`,
                    // …), an unset mesh keeps its DEFAULT resource shader — otherwise the
                    // LEGO minifig's legs (mesh 2, no override) inherited the head shader
                    // (mesh 0) and rendered as yellow skin instead of blue legs.
                    .or_else(|| if m.len() == 1 { m.get(&0) } else { None }),
                // Whole-model query (no mesh index): return a representative override —
                // mesh 0, else the lowest index. Used by opacity / transparent-shader /
                // material lookups that need the model's primary shader (e.g. unicraft's
                // galaxy glow plane, whose `.transparent = 1` shader must still be found).
                // The per-mesh render path passes Some(idx), so this can't re-leak the
                // LEGO legs (that's guarded by the Some branch above).
                None => m.get(&0).or_else(|| m.iter().min_by_key(|(k, _)| **k).map(|(_, v)| v)),
            }
        })
    }

    /// True if the model's effective shader was explicitly marked `.transparent = 1`
    /// by the script (tracked in runtime_state.transparent_shaders). Such a model
    /// alpha-blends softly and belongs in the transparent pass even at full opacity.
    fn model_uses_transparent_shader(
        model_node: &W3dNode,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        let rs = match runtime_state { Some(rs) => rs, None => return false };
        if rs.transparent_shaders.is_empty() { return false; }
        let name = Self::node_shader_override(rs, &model_node.name, None)
            .cloned()
            .unwrap_or_else(|| model_node.shader_name.clone());
        if name.is_empty() { return false; }
        rs.transparent_shaders.iter().any(|s| s.eq_ignore_ascii_case(&name))
    }

    fn get_model_opacity(
        &self,
        scene: &W3dScene,
        model_node: &W3dNode,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> f32 {
        // 1. Check node-level shader override
        let effective_shader_name = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, &model_node.name, None))
            .cloned()
            .unwrap_or_else(|| model_node.shader_name.clone());
        if !effective_shader_name.is_empty() {
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, &effective_shader_name) {
                if let Some(mat) = Self::find_material_ci(&scene.materials, &w3d_shader.material_name) {
                    return mat.opacity;
                }
                if let Some(mat) = Self::find_material_ci(&scene.materials, &w3d_shader.name) {
                    return mat.opacity;
                }
            }
        }
        // 2. Check per-mesh shader bindings from model resource
        let resource = if !model_node.model_resource_name.is_empty() {
            &model_node.model_resource_name
        } else {
            &model_node.resource_name
        };
        if let Some(res_info) = scene.model_resources.get(resource) {
            for binding in &res_info.shader_bindings {
                for shader_name in &binding.mesh_bindings {
                    if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, shader_name) {
                        let mat = if !w3d_shader.material_name.is_empty() {
                            Self::find_material_ci(&scene.materials, &w3d_shader.material_name)
                        } else {
                            Self::find_material_ci(&scene.materials, &w3d_shader.name)
                        };
                        if let Some(mat) = mat {
                            if mat.opacity < 0.999 {
                                return mat.opacity; // Any transparent mesh → whole model is transparent
                            }
                        }
                    }
                }
            }
        }
        // 3. Check per-mesh runtime shader overrides
        if let Some(rs) = runtime_state {
            if let Some(shader_map) = rs.node_shaders.get(&model_node.name) {
                for shader_name in shader_map.values() {
                    if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, shader_name) {
                        let mat = if !w3d_shader.material_name.is_empty() {
                            Self::find_material_ci(&scene.materials, &w3d_shader.material_name)
                        } else {
                            Self::find_material_ci(&scene.materials, &w3d_shader.name)
                        };
                        if let Some(mat) = mat {
                            if mat.opacity < 0.999 {
                                return mat.opacity;
                            }
                        }
                    }
                }
            }
        }
        1.0 // Default opaque
    }

    /// Check if a model's shader references any texture that has alpha data.
    /// Used to route such models to the transparent rendering pass.
    /// True if the model is covered by a fully-OPAQUE diffuse texture (a bound texture
    /// layer whose image has no alpha). Director gates transparency on shader.blend
    /// (default 100 = opaque), NOT on the W3D material's opacity field — so a textured
    /// solid like finalDrive's `chassis` (material opacity 0.2, opaque camo texture)
    /// must render solid, not 20% see-through. A genuinely translucent surface is a
    /// plain colour material or an alpha texture, which this returns false for.
    fn model_has_opaque_texture(
        &self,
        scene: &W3dScene,
        model_node: &W3dNode,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        let gpu_data = match self.member_data.get(member_key) { Some(d) => d, None => return false };
        let mut shader_names: Vec<String> = Vec::new();
        if let Some(rs) = runtime_state {
            if let Some(m) = rs.node_shaders.get(&model_node.name) { shader_names.extend(m.values().cloned()); }
        }
        if !model_node.shader_name.is_empty() { shader_names.push(model_node.shader_name.clone()); }
        let resource = if !model_node.model_resource_name.is_empty() { &model_node.model_resource_name } else { &model_node.resource_name };
        if let Some(res_info) = scene.model_resources.get(resource) {
            for b in &res_info.shader_bindings { for s in &b.mesh_bindings { shader_names.push(s.clone()); } }
        }
        for shader_name in &shader_names {
            if let Some(sh) = Self::find_shader_ci(&scene.shaders, shader_name) {
                for layer in &sh.texture_layers {
                    let lname = layer.name.to_lowercase();
                    // A loaded texture that is NOT flagged as carrying alpha = opaque cover.
                    if gpu_data.textures.contains_key(&lname) && !gpu_data.alpha_textures.contains(&lname) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn model_has_alpha_texture(
        &self,
        scene: &W3dScene,
        model_node: &W3dNode,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        let gpu_data = match self.member_data.get(member_key) {
            Some(d) => d,
            None => return false,
        };
        if gpu_data.alpha_textures.is_empty() { return false; }

        // Collect all shader names that affect this model
        let mut shader_names: Vec<String> = Vec::new();

        // 1. Per-node shader override
        if let Some(rs) = runtime_state {
            if let Some(shader_map) = rs.node_shaders.get(&model_node.name) {
                shader_names.extend(shader_map.values().cloned());
            }
        }
        // 2. Node-level shader
        if !model_node.shader_name.is_empty() {
            shader_names.push(model_node.shader_name.clone());
        }
        // 3. Model resource shader bindings
        let resource = if !model_node.model_resource_name.is_empty() {
            &model_node.model_resource_name
        } else {
            &model_node.resource_name
        };
        if let Some(res_info) = scene.model_resources.get(resource) {
            for binding in &res_info.shader_bindings {
                for mesh_shader in &binding.mesh_bindings {
                    shader_names.push(mesh_shader.clone());
                }
            }
        }

        // Check if any shader's texture layers reference an alpha texture
        for shader_name in &shader_names {
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, shader_name) {
                for layer in &w3d_shader.texture_layers {
                    if gpu_data.alpha_textures.contains(&layer.name.to_lowercase()) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Compile post-processing shader for bloom (lazy init)
    fn ensure_pp_shader(&mut self, context: &WebGL2Context) -> Result<(), JsValue> {
        if self.pp_shader.is_some() { return Ok(()); }

        let vs = r#"#version 300 es
layout(location = 0) in vec2 a_pos;
out vec2 v_uv;
void main() {
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;
        let fs = r#"#version 300 es
precision mediump float;
in vec2 v_uv;
uniform sampler2D u_input;
uniform vec2 u_resolution;
uniform vec2 u_direction;  // (1,0) for horizontal, (0,1) for vertical
uniform float u_threshold;
uniform float u_intensity;
uniform int u_mode;        // 0=bright-pass, 1=blur, 2=composite, 3=adjustColor, 4=nightVision
uniform mat4 u_color_matrix; // for adjustColor mode
out vec4 frag_color;

void main() {
    vec4 color = texture(u_input, v_uv);
    if (u_mode == 0) {
        // Bright-pass: extract pixels above threshold
        float lum = dot(color.rgb, vec3(0.299, 0.587, 0.114));
        frag_color = (lum > u_threshold) ? vec4(color.rgb - u_threshold, 1.0) : vec4(0.0, 0.0, 0.0, 1.0);
    } else if (u_mode == 1) {
        // 9-tap Gaussian blur
        vec2 texel = u_direction / u_resolution;
        vec3 result = color.rgb * 0.227;
        result += texture(u_input, v_uv + texel * 1.0).rgb * 0.1945;
        result += texture(u_input, v_uv - texel * 1.0).rgb * 0.1945;
        result += texture(u_input, v_uv + texel * 2.0).rgb * 0.1216;
        result += texture(u_input, v_uv - texel * 2.0).rgb * 0.1216;
        result += texture(u_input, v_uv + texel * 3.0).rgb * 0.0540;
        result += texture(u_input, v_uv - texel * 3.0).rgb * 0.0540;
        result += texture(u_input, v_uv + texel * 4.0).rgb * 0.0162;
        result += texture(u_input, v_uv - texel * 4.0).rgb * 0.0162;
        frag_color = vec4(result, 1.0);
    } else if (u_mode == 2) {
        // Composite: add bloom on top of original
        frag_color = vec4(color.rgb * u_intensity, 1.0);
    } else if (u_mode == 3) {
        // AdjustColor: apply 4x4 color transform matrix
        frag_color = u_color_matrix * color;
        frag_color.a = color.a;
    } else if (u_mode == 4) {
        // NightVision: green monochrome + noise + brightness boost
        float lum = dot(color.rgb, vec3(0.299, 0.587, 0.114));
        float noise = fract(sin(dot(v_uv * u_resolution, vec2(12.9898, 78.233))) * 43758.5453) * 0.05;
        float green = clamp(lum * 2.0 + noise, 0.0, 1.0);
        frag_color = vec4(green * 0.1, green, green * 0.1, 1.0);
    } else if (u_mode == 5) {
        // Depth of field: blur based on distance from focus (simplified)
        vec2 texel = 1.0 / u_resolution;
        float blur_radius = u_threshold; // reuse threshold as blur radius
        vec3 result = vec3(0.0);
        float total = 0.0;
        for (int x = -2; x <= 2; x++) {
            for (int y = -2; y <= 2; y++) {
                float w = 1.0 / (1.0 + float(x*x + y*y));
                result += texture(u_input, v_uv + vec2(float(x), float(y)) * texel * blur_radius).rgb * w;
                total += w;
            }
        }
        frag_color = vec4(result / total, 1.0);
    } else {
        frag_color = color;
    }
}
"#;
        let vs_compiled = context.compile_shader(WebGl2RenderingContext::VERTEX_SHADER, vs)?;
        let fs_compiled = context.compile_shader(WebGl2RenderingContext::FRAGMENT_SHADER, fs)?;
        let program = context.link_program(&vs_compiled, &fs_compiled)?;
        let gl = context.gl();
        let u = |name: &str| gl.get_uniform_location(&program, name);

        self.pp_shader = Some(PostProcessShader {
            u_input_tex: u("u_input"),
            u_resolution: u("u_resolution"),
            u_direction: u("u_direction"),
            u_threshold: u("u_threshold"),
            u_intensity: u("u_intensity"),
            u_mode: u("u_mode"),
            u_color_matrix: u("u_color_matrix"),
            program,
        });

        // Create fullscreen triangle VAO
        let vao = context.create_vertex_array()?;
        gl.bind_vertex_array(Some(&vao));
        let vbo = context.create_buffer()?;
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
        // Oversized triangle that covers the entire viewport
        let verts: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
        unsafe {
            let array = js_sys::Float32Array::view(&verts);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);
        gl.bind_vertex_array(None);
        self.fullscreen_vao = Some(vao);

        Ok(())
    }

    /// Create bloom FBOs at half resolution (lazy init / resize)
    fn ensure_bloom_fbos(&mut self, context: &WebGL2Context, width: u32, height: u32) -> Result<(), JsValue> {
        let bw = width / 2;
        let bh = height / 2;
        if bw == self.bloom_width && bh == self.bloom_height && self.bloom_fbo_a.is_some() {
            return Ok(());
        }
        let gl = context.gl();

        // Create two ping-pong FBOs for blur passes
        for is_b in [false, true] {
            let fbo = gl.create_framebuffer().ok_or("bloom fbo")?;
            let tex = gl.create_texture().ok_or("bloom tex")?;
            gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(&fbo));
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&tex));
            gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                WebGl2RenderingContext::TEXTURE_2D, 0,
                WebGl2RenderingContext::RGBA as i32,
                bw as i32, bh as i32, 0,
                WebGl2RenderingContext::RGBA,
                WebGl2RenderingContext::UNSIGNED_BYTE,
                None,
            )?;
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::LINEAR as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::LINEAR as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
            gl.framebuffer_texture_2d(
                WebGl2RenderingContext::FRAMEBUFFER, WebGl2RenderingContext::COLOR_ATTACHMENT0,
                WebGl2RenderingContext::TEXTURE_2D, Some(&tex), 0,
            );
            if is_b {
                self.bloom_fbo_b = Some(fbo);
                self.bloom_tex_b = Some(tex);
            } else {
                self.bloom_fbo_a = Some(fbo);
                self.bloom_tex_a = Some(tex);
            }
        }
        self.bloom_width = bw;
        self.bloom_height = bh;
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
        Ok(())
    }

    /// Apply bloom post-processing to the main FBO.
    /// Reads from fbo_texture, writes blurred bright areas back additively.
    pub fn apply_bloom(
        &mut self,
        context: &WebGL2Context,
        threshold: f32,
        intensity: f32,
    ) -> Result<(), JsValue> {
        self.ensure_pp_shader(context)?;
        self.ensure_bloom_fbos(context, self.fbo_width, self.fbo_height)?;

        let gl = context.gl();
        let pp = self.pp_shader.as_ref().unwrap();
        let vao = self.fullscreen_vao.as_ref().unwrap();
        let bw = self.bloom_width as f32;
        let bh = self.bloom_height as f32;

        gl.use_program(Some(&pp.program));
        gl.uniform1i(pp.u_input_tex.as_ref(), 0);
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::CULL_FACE);

        // Pass 1: Bright-pass extract → bloom_fbo_a
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, self.bloom_fbo_a.as_ref());
        gl.viewport(0, 0, bw as i32, bh as i32);
        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, self.fbo_texture.as_ref());
        gl.uniform1i(pp.u_mode.as_ref(), 0); // bright-pass
        gl.uniform1f(pp.u_threshold.as_ref(), threshold);
        gl.uniform2f(pp.u_resolution.as_ref(), bw, bh);
        gl.bind_vertex_array(Some(vao));
        gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 3);

        // Pass 2: Horizontal blur → bloom_fbo_b
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, self.bloom_fbo_b.as_ref());
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, self.bloom_tex_a.as_ref());
        gl.uniform1i(pp.u_mode.as_ref(), 1); // blur
        gl.uniform2f(pp.u_direction.as_ref(), 1.0, 0.0);
        gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 3);

        // Pass 3: Vertical blur → bloom_fbo_a
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, self.bloom_fbo_a.as_ref());
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, self.bloom_tex_b.as_ref());
        gl.uniform2f(pp.u_direction.as_ref(), 0.0, 1.0);
        gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 3);

        // Pass 4: Composite — additive blend bloom onto main FBO
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, self.fbo.as_ref());
        gl.viewport(0, 0, self.fbo_width as i32, self.fbo_height as i32);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, self.bloom_tex_a.as_ref());
        gl.uniform1i(pp.u_mode.as_ref(), 2); // composite
        gl.uniform1f(pp.u_intensity.as_ref(), intensity);
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(WebGl2RenderingContext::ONE, WebGl2RenderingContext::ONE);
        gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 3);
        gl.disable(WebGl2RenderingContext::BLEND);

        gl.bind_vertex_array(None);
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
        Ok(())
    }

    /// Detect cubemap textures from naming convention (e.g., "envmap_posx", "envmap_negx", etc.)
    /// and create WebGL cube map textures.
    /// Detect cubemap textures from naming convention and create WebGL cube map textures.
    fn detect_and_create_cubemaps(
        &self,
        context: &WebGL2Context,
        scene: &W3dScene,
    ) -> HashMap<String, WebGlTexture> {
        let suffixes = ["_posx", "_negx", "_posy", "_negy", "_posz", "_negz"];
        let gl_faces = [
            WebGl2RenderingContext::TEXTURE_CUBE_MAP_POSITIVE_X,
            WebGl2RenderingContext::TEXTURE_CUBE_MAP_NEGATIVE_X,
            WebGl2RenderingContext::TEXTURE_CUBE_MAP_POSITIVE_Y,
            WebGl2RenderingContext::TEXTURE_CUBE_MAP_NEGATIVE_Y,
            WebGl2RenderingContext::TEXTURE_CUBE_MAP_POSITIVE_Z,
            WebGl2RenderingContext::TEXTURE_CUBE_MAP_NEGATIVE_Z,
        ];
        let mut cube_maps = HashMap::new();

        // Find base names that have all 6 faces in the raw texture data
        let mut candidates: HashMap<String, u8> = HashMap::new();
        for name in scene.texture_images.keys() {
            let lower = name.to_lowercase();
            for (i, suffix) in suffixes.iter().enumerate() {
                if lower.ends_with(suffix) {
                    let base = lower[..lower.len() - suffix.len()].to_string();
                    let entry = candidates.entry(base).or_insert(0);
                    *entry |= 1 << i;
                }
            }
        }

        let gl = context.gl();
        for (base_name, mask) in &candidates {
            if *mask != 0x3F { continue; } // Need all 6 faces

            let cube_tex = match gl.create_texture() {
                Some(t) => t,
                None => continue,
            };
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_CUBE_MAP, Some(&cube_tex));

            let mut all_ok = true;
            for (i, suffix) in suffixes.iter().enumerate() {
                let face_name = format!("{}{}", base_name, suffix);
                let face_data = scene.texture_images.iter()
                    .find(|(k, _)| k.to_lowercase() == face_name)
                    .map(|(_, v)| v);

                if let Some(data) = face_data {
                    // Decode face image to RGBA
                    if let Ok(img) = image::load_from_memory(data) {
                        let rgba = img.to_rgba8();
                        let w = rgba.width() as i32;
                        let h = rgba.height() as i32;
                        let raw = rgba.into_raw();
                        let _ = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                            gl_faces[i], 0,
                            WebGl2RenderingContext::RGBA as i32,
                            w, h, 0,
                            WebGl2RenderingContext::RGBA,
                            WebGl2RenderingContext::UNSIGNED_BYTE,
                            Some(&raw),
                        );
                    } else {
                        all_ok = false;
                    }
                } else {
                    all_ok = false;
                }
            }

            if all_ok {
                gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_CUBE_MAP, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::LINEAR as i32);
                gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_CUBE_MAP, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::LINEAR as i32);
                gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_CUBE_MAP, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
                gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_CUBE_MAP, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
                log(&format!(
                    "[3D-CUBEMAP] Created cubemap: \"{}\"", base_name
                ));
                cube_maps.insert(base_name.clone(), cube_tex);
            }
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_CUBE_MAP, None);
        }

        cube_maps
    }

    /// Process render-to-texture requests: render scene from specified camera into named texture.
    pub fn process_render_targets(
        &mut self,
        context: &WebGL2Context,
        member_key: (i32, i32),
        scene: &W3dScene,
        width: u32,
        height: u32,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> Result<(), JsValue> {
        let targets: Vec<(String, String)> = runtime_state
            .map(|rs| rs.render_targets.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        if targets.is_empty() { return Ok(()); }

        for (cam_name, tex_name) in &targets {
            // Temporarily set this camera as active
            let prev_camera = self.active_camera.clone();
            self.active_camera = Some(cam_name.clone());

            // Ensure RTT FBO exists at the right size
            self.ensure_rtt_fbo(context, width, height)?;

            let gl = context.gl();

            // Render scene to RTT FBO
            gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, self.rtt_fbo.as_ref());
            gl.viewport(0, 0, width as i32, height as i32);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT | WebGl2RenderingContext::DEPTH_BUFFER_BIT);

            // Render the scene (this will use the RTT FBO since it's bound)
            // We can't call render_scene_with_state_ex recursively, so we just copy the main FBO
            // For a proper implementation, we'd need to refactor the render loop.
            // For now: copy the main FBO texture into the named texture.
            gl.bind_framebuffer(WebGl2RenderingContext::READ_FRAMEBUFFER, self.fbo.as_ref());
            gl.bind_framebuffer(WebGl2RenderingContext::DRAW_FRAMEBUFFER, self.rtt_fbo.as_ref());
            gl.blit_framebuffer(
                0, 0, width as i32, height as i32,
                0, 0, width as i32, height as i32,
                WebGl2RenderingContext::COLOR_BUFFER_BIT,
                WebGl2RenderingContext::NEAREST,
            );

            // Now copy RTT texture into the named texture in MemberGpuData
            if let Some(gpu_data) = self.member_data.get_mut(&member_key) {
                let tex_key = tex_name.to_lowercase();
                if let Some(existing_tex) = gpu_data.textures.get(&tex_key) {
                    // Copy RTT result into existing texture via blit
                    // For simplicity, just replace the texture reference
                    // (proper impl would use glCopyTexSubImage2D)
                }
                // Insert/replace the RTT texture as the named texture
                if let Some(ref rtt_tex) = self.rtt_texture {
                    // Create a copy texture and blit into it
                    let copy_tex = gl.create_texture().ok_or("rtt copy")?;
                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&copy_tex));
                    gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                        WebGl2RenderingContext::TEXTURE_2D, 0,
                        WebGl2RenderingContext::RGBA as i32,
                        width as i32, height as i32, 0,
                        WebGl2RenderingContext::RGBA,
                        WebGl2RenderingContext::UNSIGNED_BYTE,
                        None,
                    )?;
                    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::LINEAR as i32);
                    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::LINEAR as i32);

                    // Copy from RTT FBO to the new texture
                    gl.bind_framebuffer(WebGl2RenderingContext::READ_FRAMEBUFFER, self.rtt_fbo.as_ref());
                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&copy_tex));
                    gl.copy_tex_sub_image_2d(
                        WebGl2RenderingContext::TEXTURE_2D, 0,
                        0, 0, 0, 0,
                        width as i32, height as i32,
                    );
                    gpu_data.textures.insert(tex_key, copy_tex);
                    gpu_data.texture_sizes.insert(tex_name.to_lowercase(), (width, height));
                }
            }

            // Restore camera
            self.active_camera = prev_camera;
            gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);
        }
        Ok(())
    }

    /// Ensure render-to-texture FBO exists
    fn ensure_rtt_fbo(&mut self, context: &WebGL2Context, width: u32, height: u32) -> Result<(), JsValue> {
        if self.rtt_width == width && self.rtt_height == height && self.rtt_fbo.is_some() {
            return Ok(());
        }
        let gl = context.gl();
        let fbo = gl.create_framebuffer().ok_or("rtt fbo")?;
        let tex = gl.create_texture().ok_or("rtt tex")?;
        let depth = gl.create_renderbuffer().ok_or("rtt depth")?;

        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(&fbo));

        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&tex));
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            WebGl2RenderingContext::TEXTURE_2D, 0,
            WebGl2RenderingContext::RGBA as i32,
            width as i32, height as i32, 0,
            WebGl2RenderingContext::RGBA,
            WebGl2RenderingContext::UNSIGNED_BYTE,
            None,
        )?;
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::LINEAR as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::LINEAR as i32);
        gl.framebuffer_texture_2d(WebGl2RenderingContext::FRAMEBUFFER, WebGl2RenderingContext::COLOR_ATTACHMENT0, WebGl2RenderingContext::TEXTURE_2D, Some(&tex), 0);

        gl.bind_renderbuffer(WebGl2RenderingContext::RENDERBUFFER, Some(&depth));
        gl.renderbuffer_storage(WebGl2RenderingContext::RENDERBUFFER, WebGl2RenderingContext::DEPTH_COMPONENT16, width as i32, height as i32);
        gl.framebuffer_renderbuffer(WebGl2RenderingContext::FRAMEBUFFER, WebGl2RenderingContext::DEPTH_ATTACHMENT, WebGl2RenderingContext::RENDERBUFFER, Some(&depth));

        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);

        self.rtt_fbo = Some(fbo);
        self.rtt_texture = Some(tex);
        self.rtt_depth = Some(depth);
        self.rtt_width = width;
        self.rtt_height = height;
        Ok(())
    }

    /// Compile outline shader for ShaderInker (lazy init)
    fn ensure_outline_shader(&mut self, context: &WebGL2Context) -> Result<(), JsValue> {
        if self.outline_shader.is_some() { return Ok(()); }

        let vs = r#"#version 300 es
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;

uniform mat4 u_model;
uniform mat4 u_view;
uniform mat4 u_projection;
uniform float u_outline_width;

void main() {
    // Expand vertex along normal for outline thickness
    vec3 expanded = a_position + a_normal * u_outline_width;
    gl_Position = u_projection * u_view * u_model * vec4(expanded, 1.0);
}
"#;
        let fs = r#"#version 300 es
precision mediump float;
uniform vec4 u_outline_color;
out vec4 frag_color;
void main() {
    frag_color = u_outline_color;
}
"#;
        let vs_compiled = context.compile_shader(WebGl2RenderingContext::VERTEX_SHADER, vs)?;
        let fs_compiled = context.compile_shader(WebGl2RenderingContext::FRAGMENT_SHADER, fs)?;
        let program = context.link_program(&vs_compiled, &fs_compiled)?;
        let gl = context.gl();
        let u = |name: &str| gl.get_uniform_location(&program, name);

        self.outline_shader = Some(OutlineShader {
            u_model: u("u_model"),
            u_view: u("u_view"),
            u_projection: u("u_projection"),
            u_outline_width: u("u_outline_width"),
            u_outline_color: u("u_outline_color"),
            program,
        });
        Ok(())
    }

    /// Render outlines for models using ShaderInker.
    /// Called after the main geometry pass, draws back-faces expanded along normals.
    fn render_inker_outlines(
        &mut self,
        context: &WebGL2Context,
        scene: &W3dScene,
        member_key: &(i32, i32),
        view_matrix: &[f32; 16],
        projection_matrix: &[f32; 16],
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> Result<(), JsValue> {
        use crate::director::chunks::w3d::types::W3dShaderType;

        // Check if any model uses ShaderInker
        let has_inker = scene.nodes.iter().any(|n| {
            if n.node_type != W3dNodeType::Model { return false; }
            let shader_name = runtime_state
                .and_then(|rs| Self::node_shader_override(rs, &n.name, None))
                .unwrap_or(&n.shader_name);
            Self::find_shader_ci(&scene.shaders, shader_name)
                .map(|s| s.shader_type == W3dShaderType::Inker)
                .unwrap_or(false)
        });
        if !has_inker { return Ok(()); }

        self.ensure_outline_shader(context)?;
        let gl = context.gl();
        let outline = self.outline_shader.as_ref().unwrap();

        gl.use_program(Some(&outline.program));
        gl.uniform_matrix4fv_with_f32_array(outline.u_view.as_ref(), false, view_matrix);
        gl.uniform_matrix4fv_with_f32_array(outline.u_projection.as_ref(), false, projection_matrix);

        // Render back-faces only (front-face culling gives outline effect)
        gl.enable(WebGl2RenderingContext::CULL_FACE);
        gl.cull_face(WebGl2RenderingContext::BACK); // Cull back = draw front → flip for outline
        // Actually for outline: cull FRONT faces, draw BACK faces expanded outward
        gl.cull_face(WebGl2RenderingContext::FRONT);

        for model_node in scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model) {
            let shader_name = runtime_state
                .and_then(|rs| Self::node_shader_override(rs, &model_node.name, None))
                .unwrap_or(&model_node.shader_name);
            let w3d_shader = match Self::find_shader_ci(&scene.shaders, shader_name) {
                Some(s) if s.shader_type == W3dShaderType::Inker => s,
                _ => continue,
            };

            let width = if w3d_shader.outline_width > 0.0 { w3d_shader.outline_width } else { 0.02 };
            let color = w3d_shader.outline_color;
            gl.uniform1f(outline.u_outline_width.as_ref(), width);
            gl.uniform4f(outline.u_outline_color.as_ref(), color[0], color[1], color[2], color[3]);

            let world_matrix = self.accumulate_transform_with_state(scene, model_node, runtime_state);
            gl.uniform_matrix4fv_with_f32_array(outline.u_model.as_ref(), false, &world_matrix);

            let resource = if !model_node.model_resource_name.is_empty() {
                &model_node.model_resource_name
            } else {
                &model_node.resource_name
            };

            if let Some(gpu_data) = self.member_data.get(member_key) {
                if let Some(mesh_group) = gpu_data.mesh_groups.get(resource) {
                    for mesh_buf in mesh_group {
                        mesh_buf.bind(gl);
                        mesh_buf.draw(gl);
                        mesh_buf.unbind(gl);
                    }
                }
            }
        }

        // Restore culling for main shader
        gl.cull_face(WebGl2RenderingContext::FRONT); // Back to Y-flipped culling
        Ok(())
    }

    /// Case-insensitive shader lookup (W3D files have inconsistent casing).
    fn find_shader_ci<'a>(shaders: &'a [W3dShader], name: &str) -> Option<&'a W3dShader> {
        shaders.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Case-insensitive material lookup.
    fn find_material_ci<'a>(materials: &'a [W3dMaterial], name: &str) -> Option<&'a W3dMaterial> {
        materials.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// Find the first shader that references a material by name.
    fn find_shader_for_material_ci<'a>(scene: &'a W3dScene, material_name: &str) -> Option<&'a W3dShader> {
        scene.shaders.iter().find(|s| s.material_name.eq_ignore_ascii_case(material_name))
    }

    /// Resolve a candidate name to a shader, allowing either shader names or material names.
    fn resolve_shader_candidate_ci<'a>(scene: &'a W3dScene, candidate: &str) -> Option<&'a W3dShader> {
        Self::find_shader_ci(&scene.shaders, candidate)
            .or_else(|| Self::find_shader_for_material_ci(scene, candidate))
    }

    /// Resolve a candidate name to a material, allowing either material names or shader names.
    fn resolve_material_candidate_ci<'a>(scene: &'a W3dScene, candidate: &str) -> Option<&'a W3dMaterial> {
        Self::find_material_ci(&scene.materials, candidate)
            .or_else(|| {
                Self::find_shader_ci(&scene.shaders, candidate)
                    .and_then(|s| Self::find_material_ci(&scene.materials, &s.material_name))
            })
    }

    /// Resolve all texture layers for a shader: diffuse, extra blend layers, and specular map.
    /// Categorizes layers by tex_mode: 0/5 = diffuse, 6 = specular, others = diffuse.
    /// Extra layers (beyond the first diffuse) are returned with proper blend modes.
    fn find_texture_layers<'a>(
        layers: &[crate::director::chunks::w3d::types::W3dTextureLayer],
        gpu_data: &'a MemberGpuData,
    ) -> TextureBindResult<'a> {
        let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
        let mut result = TextureBindResult {
            diffuse: None,
            diffuse_tex_transform: identity,
            diffuse_wrap_transform: identity,
            diffuse_wrap: (1, 1), // default: repeat
            diffuse_tex_mode: 0,
            extra_layers: Vec::new(),
            specular: None,
        };

        let mut diffuse_name = String::new();

        for (layer_idx, layer) in layers.iter().enumerate() {
            if layer.name.is_empty() { continue; }
            let lower = layer.name.to_lowercase();
            let tex = gpu_data.textures.get(&lower);
            let tex = match tex {
                Some(t) => t,
                None => continue,
            };

            // tex_mode 4 = reflection / environment map (Director `reflectionMap`).
            // Sampled with sphere-mapped UVs (not the mesh's authored UVs), so it
            // must be kept out of the diffuse/extra-layer slots here. It is bound
            // separately as a final step via apply_reflection_map() so the per-mesh
            // candidate search can't clobber its u_layer2_blend signal.
            if layer.tex_mode == 4 {
                continue;
            }

            // tex_mode 6 = specular map
            if layer.tex_mode == 6 {
                if result.specular.is_none() {
                    result.specular = Some(tex);
                }
                continue;
            }

            // Diffuse base texture: first non-empty layer whose name doesn't
            // look like a baked lighting layer. Phosphor Beta / Rasterwerks
            // shifts its base texture to textureList[2] (slot 0 in the layer
            // array left empty); a strict "position 0 only" rule misses it
            // and the model renders as if untextured.
            let is_baked_lighting = lower.contains("lightmap") || lower.contains("shadow");
            if !is_baked_lighting && result.diffuse.is_none() {
                result.diffuse = Some(tex);
                diffuse_name = lower;
                if layer.tex_transform != identity {
                    result.diffuse_tex_transform = layer.tex_transform;
                }
                result.diffuse_wrap_transform = layer.wrap_transform;
                result.diffuse_wrap = (layer.repeat_s, layer.repeat_t);
                result.diffuse_tex_mode = layer.tex_mode;
                continue;
            }

            // Subsequent non-specular textures are extra blend layers (up to 2)
            if result.extra_layers.len() < 2 {
                // Skip duplicate layers (same texture as diffuse) — W3D files often
                // have the same texture in multiple layers as placeholders
                if lower == diffuse_name {
                    continue;
                }

                // Blend mode mapping:
                //   blend_func 0 = #multiply / GL_REPLACE (ambiguous)
                //   blend_func 1 = #add / GL_ADD
                //   blend_func 2 = #replace / GL_MODULATE
                //   blend_func 3 = #blend / GL_DECAL
                let blend = if lower.contains("lightmap") && !lower.contains("shadow") {
                    // Lightmap-only meshes (empty textureList[1], lightmap in textureList[2])
                    // should shade as material color multiplied by light intensity.
                    let lightmap_only = layer_idx > 0
                        && layers[..layer_idx].iter().all(|prev| prev.name.is_empty());
                    if lightmap_only { 1 } else { 2 }
                } else {
                    match layer.blend_func {
                        1 => 2,  // #add / GL_ADD → our add mode
                        2 => 1,  // #replace / GL_MODULATE → our multiply mode
                        _ => 1,  // #multiply → multiply
                    }
                };

                result.extra_layers.push(TextureLayerBinding {
                    tex,
                    blend,
                    intensity: layer.intensity,
                    wrap: (layer.repeat_s, layer.repeat_t),
                });
            }
        }

        // Do not promote textureList[2+] into diffuse when textureList[1] is empty.
        // Director uses that layout for lightmap-only meshes, which should render via
        // the non-textured material path plus the extra lightmap layer.

        result
    }

    /// Bind resolved texture layers to GPU: diffuse (unit 0), extra layers (units 1-2), specular (unit 3).
    /// Returns true if a diffuse texture was bound.
    fn bind_texture_layers(
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        result: &TextureBindResult,
    ) -> bool {
        let mut tex_bound = false;
        if let Some(tex) = result.diffuse {
            gl.active_texture(WebGl2RenderingContext::TEXTURE0);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
            gl.uniform1i(shader.u_has_texture.as_ref(), 1);
            // Upload texture coordinate transform
            gl.uniform_matrix4fv_with_f32_array(shader.u_tex_transform.as_ref(), false, &result.diffuse_tex_transform);
            // Set wrap mode per layer: 0=clamp, 1=repeat (default)
            let wrap_s = if result.diffuse_wrap.0 == 0 { WebGl2RenderingContext::CLAMP_TO_EDGE } else { WebGl2RenderingContext::REPEAT };
            let wrap_t = if result.diffuse_wrap.1 == 0 { WebGl2RenderingContext::CLAMP_TO_EDGE } else { WebGl2RenderingContext::REPEAT };
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, wrap_s as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, wrap_t as i32);
            // Forward the tex_mode to the vertex shader so #wrapPlanar etc. branch
            gl.uniform1i(shader.u_uv_proj_mode.as_ref(), result.diffuse_tex_mode as i32);
            gl.uniform_matrix4fv_with_f32_array(shader.u_wrap_transform.as_ref(), false, &result.diffuse_wrap_transform);
            tex_bound = true;
        } else {
            gl.uniform1i(shader.u_uv_proj_mode.as_ref(), 0);
        }

        // Extra layer 0 → unit 1
        // Extra layers (shadow/lightmap) are authored per-mesh — UV 0..1 covers
        // the whole mesh and sampling outside that range should never wrap to
        // the opposite edge (visible as dark seams along cliff/floor edges in
        // Phosphor). Force CLAMP_TO_EDGE regardless of the W3D file's repeat
        // flag, which is typically left at REPEAT (the default) by authoring
        // tools.
        if let Some(layer) = result.extra_layers.get(0) {
            gl.active_texture(WebGl2RenderingContext::TEXTURE1);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(layer.tex));
            gl.uniform1i(shader.u_has_lightmap.as_ref(), layer.blend);
            gl.uniform1f(shader.u_lightmap_intensity.as_ref(), layer.intensity);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        } else {
            gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);
        }

        // Extra layer 1 → unit 2
        if let Some(layer) = result.extra_layers.get(1) {
            gl.active_texture(WebGl2RenderingContext::TEXTURE2);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(layer.tex));
            gl.uniform1i(shader.u_layer2_blend.as_ref(), layer.blend);
            gl.uniform1f(shader.u_layer2_intensity.as_ref(), layer.intensity);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
            gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        } else {
            gl.uniform1i(shader.u_layer2_blend.as_ref(), 0);
        }

        // Specular map → unit 3
        if let Some(tex) = result.specular {
            gl.active_texture(WebGl2RenderingContext::TEXTURE3);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
            gl.uniform1i(shader.u_has_specular_map.as_ref(), 1);
        } else {
            gl.uniform1i(shader.u_has_specular_map.as_ref(), 0);
        }

        tex_bound
    }

    /// Resolve a model's primary shader name the same way the Lingo `model.shader`
    /// getter (get_model_prop) does, so reflection binding targets the exact shader
    /// the movie assigned `reflectionMap` to. Precedence: runtime override →
    /// model-resource first-mesh binding (prefer non-DefaultShader) → node
    /// shader_name → model-index→shader-index.
    fn resolve_model_primary_shader(
        scene: &W3dScene,
        model_node: &W3dNode,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> Option<String> {
        // 1) Runtime override (model.shader = s1 / shaderList[1] = ref)
        if let Some(name) = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, &model_node.name, None))
        {
            return Some(name.clone());
        }
        // 2) Model-resource first-mesh shader binding (prefer non-DefaultShader)
        let resource = if !model_node.model_resource_name.is_empty() {
            &model_node.model_resource_name
        } else {
            &model_node.resource_name
        };
        if let Some(res) = scene.model_resources.get(resource) {
            let mut fallback = String::new();
            for binding in &res.shader_bindings {
                if !binding.mesh_bindings.is_empty() && !binding.mesh_bindings[0].is_empty() {
                    let name = &binding.mesh_bindings[0];
                    if !name.eq_ignore_ascii_case("DefaultShader") {
                        return Some(name.clone());
                    } else if fallback.is_empty() {
                        fallback = name.clone();
                    }
                }
            }
            if !fallback.is_empty() { return Some(fallback); }
        }
        // 3) Node's shader_name
        if !model_node.shader_name.is_empty() {
            return Some(model_node.shader_name.clone());
        }
        // 4) Model index → shader index
        let mi = scene.nodes.iter()
            .filter(|n| n.node_type == W3dNodeType::Model)
            .position(|n| n.name == model_node.name);
        if let Some(mi) = mi {
            if mi < scene.shaders.len() {
                return Some(scene.shaders[mi].name.clone());
            }
        }
        None
    }

    /// Bind the model's reflection / environment map as the FINAL material step.
    /// Director's `reflectionMap` helper puts the texture on the third layer with
    /// tex_mode 4 (#reflection); we sample it sphere-mapped in the fragment shader
    /// and signal it via u_layer2_blend = 5. This runs after bind_material[_for_mesh]
    /// so the per-mesh path's multi-candidate texture search (which calls
    /// bind_texture_layers repeatedly and resets u_layer2_blend) cannot clobber it.
    /// Untextured surfaces (e.g. tinted glass) therefore still get their reflection.
    fn apply_reflection_map(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        scene: &W3dScene,
        model_node: &W3dNode,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) {
        // Resolve the model's SINGLE primary shader exactly as `model.shader`
        // (get_model_prop) does — that is the shader the reflectionMap helper was
        // assigned to. Scanning every shader the model's resource references is
        // wrong: house models share one model-resource whose bindings include the
        // glass's `roofshad`, so a broad scan applied the reflection (and its 50%
        // sky blend) to every surface and washed the scene white.
        let shader_name = match Self::resolve_model_primary_shader(scene, model_node, runtime_state) {
            Some(s) => s,
            None => return,
        };
        let refl = Self::find_shader_ci(&scene.shaders, &shader_name)
            .and_then(|sh| sh.texture_layers.iter()
                .find(|l| l.tex_mode == 4 && !l.name.is_empty())
                .map(|l| (l.name.clone(), l.blend_const)));
        let (tex_name, blend_const) = match refl { Some(x) => x, None => return };
        let gpu_data = match self.member_data.get(member_key) { Some(d) => d, None => return };
        let tex = match gpu_data.textures.get(&tex_name.to_lowercase()) {
            Some(t) => t,
            None => return,
        };
        gl.active_texture(WebGl2RenderingContext::TEXTURE2);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        gl.uniform1i(shader.u_layer2_blend.as_ref(), 5);
        gl.uniform1f(shader.u_layer2_intensity.as_ref(), blend_const.clamp(0.0, 1.0));
    }

    /// Bind material properties for a model node
    fn bind_material(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        scene: &W3dScene,
        model_node: &W3dNode,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
        force_blend: bool,
    ) {
        // Resolve shader → material chain:
        // 1. Check runtime shader override (node_shaders)
        // 2. ModelNode has a shader_name
        // 3. Shader has a material_name → find in scene.materials
        // 4. Shader has texture_layers → bind first diffuse texture
        let mut mat_found = false;
        let mut tex_bound = false;

        // Check runtime shader override first
        let effective_shader_name = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, &model_node.name, None))
            .cloned()
            .unwrap_or_else(|| model_node.shader_name.clone());

        if !effective_shader_name.is_empty() {
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, &effective_shader_name) {
                // Find material: try shader's material_name, then shader name itself
                let mat = if !w3d_shader.material_name.is_empty() {
                    Self::find_material_ci(&scene.materials, &w3d_shader.material_name)
                } else { None }
                    .or_else(|| Self::find_material_ci(&scene.materials, &w3d_shader.name));
                if let Some(mat) = mat {
                    self.set_material_uniforms(gl, shader, mat);
                    mat_found = true;
                }

                // Bind texture layers
                if let Some(gpu_data) = self.member_data.get(member_key) {
                    let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                    tex_bound = Self::bind_texture_layers(gl, shader, &layers);
                }
            }
        }

        // If no shader on node, try model resource shader bindings
        if !mat_found {
            let resource = if !model_node.model_resource_name.is_empty() {
                &model_node.model_resource_name
            } else {
                &model_node.resource_name
            };
            if let Some(res_info) = scene.model_resources.get(resource) {
                if let Some(binding) = res_info.shader_bindings.first() {
                    // Resolve binding name → shader → material
                    if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, &binding.name) {
                        if let Some(mat) = Self::find_material_ci(&scene.materials, &w3d_shader.material_name) {
                            self.set_material_uniforms(gl, shader, mat);
                            mat_found = true;
                        }
                        // Bind texture layers from shader binding
                        if !tex_bound {
                            if let Some(gpu_data) = self.member_data.get(member_key) {
                                let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                                tex_bound = Self::bind_texture_layers(gl, shader, &layers);
                            }
                        }
                    }
                }
            }
        }

        if !mat_found {
            self.bind_default_material(gl, shader, scene);
        }
        if !tex_bound {
            gl.uniform1i(shader.u_has_texture.as_ref(), 0);
        }

        // Set shader mode based on shader type (NPR support)
        let w3d_shader_opt = Self::find_shader_ci(&scene.shaders, &effective_shader_name);
        if let Some(w3d_shader) = w3d_shader_opt {
            use crate::director::chunks::w3d::types::W3dShaderType;
            match w3d_shader.shader_type {
                W3dShaderType::Painter => {
                    gl.uniform1i(shader.u_shader_mode.as_ref(), 1);
                    let steps = if w3d_shader.toon_steps > 0 { w3d_shader.toon_steps as f32 } else { 3.0 };
                    gl.uniform1f(shader.u_toon_steps.as_ref(), steps);
                }
                _ => {
                    gl.uniform1i(shader.u_shader_mode.as_ref(), 0);
                }
            }
        } else {
            gl.uniform1i(shader.u_shader_mode.as_ref(), 0);
        }

        // IFX default: when a texture is bound and useDiffuseWithTexture is false,
        // force diffuse to white (1,1,1) so lighting doesn't attenuate the textured surface.
        // Shaders with useDiffuseWithTexture=true (e.g., lightmap clones) keep their actual diffuse.
        if tex_bound {
            let use_diffuse = w3d_shader_opt.map(|s| s.use_diffuse_with_texture).unwrap_or(false);
            if !use_diffuse {
                gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
            }
        }

        // Apply blend mode based on material opacity and first texture layer's blend function
        let first_blend_func = self.get_first_blend_func(scene, model_node, runtime_state);
        let opacity = w3d_shader_opt
            .and_then(|s| Self::find_material_ci(&scene.materials, &s.material_name))
            .map(|m| m.opacity)
            .unwrap_or(1.0);
        Self::apply_blend_mode(gl, shader, opacity, first_blend_func, force_blend);
    }

    /// Get the first texture layer's blend_func for a model node
    fn get_first_blend_func(&self, scene: &W3dScene, node: &W3dNode, runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>) -> u8 {
        let effective_shader = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, &node.name, None))
            .cloned()
            .unwrap_or_else(|| node.shader_name.clone());
        Self::find_shader_ci(&scene.shaders, &effective_shader)
            .and_then(|s| s.texture_layers.first())
            .map(|l| l.blend_func)
            .unwrap_or(0)
    }

    /// Bind material for a specific mesh index using model resource shader bindings
    fn bind_material_for_mesh(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        scene: &W3dScene,
        model_node: &W3dNode,
        res_info: Option<&ModelResourceInfo>,
        mesh_idx: usize,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
        force_blend: bool,
    ) -> bool {
        // Check per-mesh shader override first (from Lingo shaderList[I] = shaderRef)
        if let Some(override_name) = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, &model_node.name, Some(mesh_idx)))
        {
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, override_name) {
                let mat = if !w3d_shader.material_name.is_empty() {
                    Self::find_material_ci(&scene.materials, &w3d_shader.material_name)
                } else { None }
                    .or_else(|| Self::find_material_ci(&scene.materials, &w3d_shader.name));
                if let Some(m) = mat {
                    self.set_material_uniforms(gl, shader, m);
                } else {
                    gl.uniform4f(shader.u_diffuse_color.as_ref(), 0.8, 0.8, 0.8, 1.0);
                    gl.uniform4f(shader.u_ambient_color.as_ref(), 0.2, 0.2, 0.2, 1.0);
                    gl.uniform4f(shader.u_specular_color.as_ref(), 0.0, 0.0, 0.0, 1.0);
                    gl.uniform4f(shader.u_emissive_color.as_ref(), 0.0, 0.0, 0.0, 1.0);
                    gl.uniform1f(shader.u_shininess.as_ref(), 0.0);
                    gl.uniform1f(shader.u_opacity.as_ref(), 1.0);
                }
                let mut tex_bound = false;
                let mut has_lightmap_layer = false;
                if let Some(gpu_data) = self.member_data.get(member_key) {
                    let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                    has_lightmap_layer = !layers.extra_layers.is_empty();
                    tex_bound = Self::bind_texture_layers(gl, shader, &layers);
                }
                let is_prim = res_info.and_then(|r| r.primitive_type.as_ref()).is_some();
                if !tex_bound && is_prim {
                    // Fall back to Director's default checkerboard for primitives only
                    if let Some(tex) = &self.default_checker_texture {
                        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
                        gl.uniform1i(shader.u_has_texture.as_ref(), 1);
                        gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
                        tex_bound = true;
                    } else {
                        gl.uniform1i(shader.u_has_texture.as_ref(), 0);
                    }
                }
                // IFX default: white diffuse for textured models unless useDiffuseWithTexture
                if tex_bound && !w3d_shader.use_diffuse_with_texture {
                    gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
                }
                let first_bf = w3d_shader.texture_layers.first().map(|l| l.blend_func).unwrap_or(0);
                let opacity = mat.map(|m| m.opacity).unwrap_or(1.0);
                Self::apply_blend_mode(gl, shader, opacity, first_bf, force_blend);
                return true;
            }
        }

        let res_info = match res_info {
            Some(r) => r,
            None => return false,
        };

        let mut candidate_names: Vec<String> = Vec::new();
        for binding in &res_info.shader_bindings {
            if mesh_idx < binding.mesh_bindings.len() && !binding.mesh_bindings[mesh_idx].is_empty() {
                candidate_names.push(binding.mesh_bindings[mesh_idx].clone());
            }
        }

        // If this mesh slot has no explicit shader, inherit the lowest-indexed
        // mesh's shader BEFORE falling back to the resource's auto-generated
        // "<res>_Shader" default. Director's #plane is 2 meshes; a resource that
        // carries a single shader (e.g. frog01's cloned wheel: shaderList[1]=wheelS)
        // puts it on mesh 0 only, leaving mesh 1's binding empty. Director shows the
        // same shader on both faces ([wheelS, wheelS]); without this, dirplayer's
        // mesh 1 fell back to the default shader (checker/untextured), so the
        // camera-facing wheel face rendered with no wheel texture → "no wheels".
        // Mirrors node_shader_override's lowest-index fallback for the resource path.
        if candidate_names.is_empty() {
            for binding in &res_info.shader_bindings {
                if let Some(first) = binding.mesh_bindings.iter().find(|b| !b.is_empty()) {
                    candidate_names.push(first.clone());
                    break;
                }
            }
        }

        // Per-mesh: query THIS mesh's override (not the whole model). Passing None here
        // leaked mesh 0's shader onto every unset mesh — the LEGO minifig legs (mesh 2,
        // no override) inherited the head shader (mesh 0) and rendered yellow. A single
        // whole-model `shaderList = shader` is still honored: node_shader_override's
        // Some(idx) branch returns mesh 0 when it's the sole override.
        let effective_shader_name = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, &model_node.name, Some(mesh_idx)))
            .cloned()
            .unwrap_or_else(|| model_node.shader_name.clone());
        if !effective_shader_name.is_empty() {
            candidate_names.push(effective_shader_name);
        }

        for binding in &res_info.shader_bindings {
            if !binding.name.is_empty() {
                candidate_names.push(binding.name.clone());
            }
        }

        let mut best_material: Option<&W3dMaterial> = None;
        let mut best_blend_func = 0u8;

        for candidate in &candidate_names {
            if candidate.is_empty() {
                continue;
            }

            let w3d_shader = Self::resolve_shader_candidate_ci(scene, candidate);
            let mat = Self::resolve_material_candidate_ci(scene, candidate)
                .or_else(|| {
                    w3d_shader.and_then(|s| {
                        if !s.material_name.is_empty() {
                            Self::find_material_ci(&scene.materials, &s.material_name)
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| w3d_shader.and_then(|s| Self::find_material_ci(&scene.materials, &s.name)));

            // Skip DefaultShader as best_material when there are more specific candidates.
            // DefaultShader often has white default material that overrides model-specific
            // materials (e.g., cloned models with yellow emissive from their source member).
            if best_material.is_none() && !(candidate.eq_ignore_ascii_case("DefaultShader") && candidate_names.len() > 1) {
                best_material = mat;
                best_blend_func = w3d_shader
                    .and_then(|s| s.texture_layers.first())
                    .map(|l| l.blend_func)
                    .unwrap_or(0);
            }

            let mut tex_bound = false;
            if let (Some(gpu_data), Some(w3d_shader)) = (self.member_data.get(member_key), w3d_shader) {
                let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                tex_bound = Self::bind_texture_layers(gl, shader, &layers);
            }

            if tex_bound {
                if let Some(m) = mat {
                    self.set_material_uniforms(gl, shader, m);
                }
                // IFX default: white diffuse for textured models unless useDiffuseWithTexture
                let use_diffuse = w3d_shader.map(|s| s.use_diffuse_with_texture).unwrap_or(false);
                if !use_diffuse {
                    gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
                }
                let first_bf = w3d_shader
                    .and_then(|s| s.texture_layers.first())
                    .map(|l| l.blend_func)
                    .unwrap_or(0);
                let opacity = mat.map(|m| m.opacity).unwrap_or(1.0);
                Self::apply_blend_mode(gl, shader, opacity, first_bf, force_blend);
                return true;
            }
        }

        // Log multi-mesh models that end up without texture
        if res_info.shader_bindings.iter().any(|b| b.mesh_bindings.len() > 1) {
            use std::sync::Mutex;
            use std::collections::HashSet;
            static LOGGED_NOTEX2: Mutex<Option<HashSet<String>>> = Mutex::new(None);
            let key = format!("{}:{}", model_node.name, mesh_idx);
            if let Ok(mut guard) = LOGGED_NOTEX2.lock() {
                let set = guard.get_or_insert_with(HashSet::new);
                if set.insert(key) {
                    let has_best = best_material.is_some();
                    log(&format!(
                        "[W3D-NOTEX-MESH] model=\"{}\" mesh={} candidates={:?} has_best_material={} → using material-only (no texture)",
                        model_node.name, mesh_idx, candidate_names, has_best,
                    ));
                }
            }
        }

        // No textured binding found — use best material.  Apply Director's
        // default checker only for newModelResource primitives (box/sphere/etc).
        let is_primitive = res_info.primitive_type.is_some();
        if let Some(mat) = best_material {
            self.set_material_uniforms(gl, shader, mat);
            if is_primitive {
                if let Some(tex) = &self.default_checker_texture {
                    gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
                    gl.uniform1i(shader.u_has_texture.as_ref(), 1);
                    gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
                    gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
                }
            } else {
                gl.uniform1i(shader.u_has_texture.as_ref(), 0);
            }
            Self::apply_blend_mode(gl, shader, mat.opacity, best_blend_func, force_blend);
            return true;
        }

        false
    }

    fn set_material_uniforms(&self, gl: &WebGl2RenderingContext, shader: &Shader3d, mat: &W3dMaterial) {
        gl.uniform4f(shader.u_diffuse_color.as_ref(), mat.diffuse[0], mat.diffuse[1], mat.diffuse[2], mat.diffuse[3]);
        gl.uniform4f(shader.u_ambient_color.as_ref(), mat.ambient[0], mat.ambient[1], mat.ambient[2], mat.ambient[3]);
        gl.uniform4f(shader.u_specular_color.as_ref(), mat.specular[0], mat.specular[1], mat.specular[2], mat.specular[3]);
        gl.uniform4f(shader.u_emissive_color.as_ref(), mat.emissive[0], mat.emissive[1], mat.emissive[2], mat.emissive[3]);
        // IFX maps material reflectivity to shader shininess (scaled by 100)
        let shininess = if mat.shininess > 0.0 { mat.shininess } else { mat.reflectivity * 100.0 };
        gl.uniform1f(shader.u_shininess.as_ref(), shininess);
        gl.uniform1f(shader.u_opacity.as_ref(), mat.opacity);
    }

    /// Set GL blend mode based on material opacity and shader blend function.
    /// `force_blend` = true when drawing in the transparent pass (models with alpha textures).
    fn apply_blend_mode(gl: &WebGl2RenderingContext, shader: &Shader3d, opacity: f32, first_layer_blend_func: u8, force_blend: bool) {
        // #replace (2) first layer → texture shown unlit (as-is). See the fragment
        // shader's u_texture_unlit branch. All other modes shade normally.
        gl.uniform1i(shader.u_texture_unlit.as_ref(), if first_layer_blend_func == 2 { 1 } else { 0 });
        if opacity < 1.0 || first_layer_blend_func == 1 || force_blend {
            gl.enable(WebGl2RenderingContext::BLEND);
            if first_layer_blend_func == 1 {
                // #add — additive blending (for glow/lightbox effects)
                gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE);
            } else {
                // #multiply / default — standard alpha blending
                gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);
            }
        } else {
            gl.disable(WebGl2RenderingContext::BLEND);
        }
    }

    /// Compute and upload bone matrices for skinning. Returns true if skinning data was uploaded.
    /// `model_name` selects the per-model bonesPlayer state — each skinned model in a member
    /// animates independently (multiple cloned bots in one G3D scene must not share a clock).
    fn setup_skinning_for_resource(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        scene: &W3dScene,
        resource_name: &str,
        model_name: &str,
        gpu_data: &MemberGpuData,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        // Only skin models that have a matching skeleton — no fallback to first()
        // to prevent walls/weapons from being skinned with the character skeleton.
        // Director is case-insensitive — script-side cloned resources can vary
        // case from the parsed W3D file.
        let skeleton = scene.skeletons.iter().find(|s| s.name.eq_ignore_ascii_case(resource_name));
        let skeleton = match skeleton {
            Some(s) if s.bones.len() > 1 => s,
            _ => return false,
        };

        // The biped mesh is bound to the MOTION's frame 0, not the skeleton HTree rest —
        // so build inv_bind from the motion's first frame for it. (Other rigs bind to rest.)
        let bind_to_motion_frame0 = resource_name.to_ascii_lowercase().contains("biped");

        // Compute inverse bind matrices fresh (bypass cache to ensure correct transpose)
        let inv_bind_fresh = {
            let bind_motion = if bind_to_motion_frame0 {
                let cmn = runtime_state.and_then(|rs| rs.bones_player(model_name))
                    .filter(|b| b.current_motion.is_some())
                    .and_then(|b| b.current_motion.as_deref())
                    .or_else(|| runtime_state.and_then(|rs| rs.current_motion.as_deref()));
                cmn.and_then(|name| scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(name)))
            } else { None };
            let rest = crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, bind_motion, 0.0);
            rest.iter().map(|m| {
                // Proper column-major affine inverse: R^-1 = R^T, t^-1 = -R^T * t
                let (r00,r01,r02) = (m[0], m[4], m[8]);
                let (r10,r11,r12) = (m[1], m[5], m[9]);
                let (r20,r21,r22) = (m[2], m[6], m[10]);
                let (tx,ty,tz) = (m[12], m[13], m[14]);
                let itx = -(r00*tx + r10*ty + r20*tz);
                let ity = -(r01*tx + r11*ty + r21*tz);
                let itz = -(r02*tx + r12*ty + r22*tz);
                [r00,r01,r02,0.0, r10,r11,r12,0.0, r20,r21,r22,0.0, itx,ity,itz,1.0]
            }).collect::<Vec<_>>()
        };
        let inv_bind = &inv_bind_fresh;

        // Per-MODEL bonesPlayer state is the source of truth ONCE a motion has been
        // play()'d on that model. A bare entry created only by a setter (rootLock /
        // playRate, e.g. the dino) has no motion and a frozen clock — treat it as
        // absent so we fall back ENTIRELY to the legacy member fields (auto-play +
        // the advancing legacy clock). Otherwise its frozen time=0 shadowed the
        // legacy clock and the model rendered stuck on frame 0.
        let bp = runtime_state.and_then(|rs| rs.bones_player(model_name))
            .filter(|b| b.current_motion.is_some());
        let current_motion_name = bp.and_then(|b| b.current_motion.as_deref())
            .or_else(|| runtime_state.and_then(|rs| rs.current_motion.as_deref()));
        let is_loop = bp.map(|b| b.animation_loop)
            .or_else(|| runtime_state.map(|rs| rs.animation_loop)).unwrap_or(true);
        let root_lock = bp.map(|b| b.root_lock)
            .or_else(|| runtime_state.map(|rs| rs.root_lock)).unwrap_or(false);
        let motion = if let Some(name) = current_motion_name {
            scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(name))
        } else {
            None // Don't apply a motion until the game explicitly calls play()
        };
        // Manual per-bone overrides (bonesPlayer.bone[i].transform = t), keyed by
        // "modelname:boneindex". updateBoneRotation re-sets these each frame to
        // animate procedurally (the SweeTarts snake's S-wiggle), so we must skin
        // even when the played motion is sparse or absent.
        let bone_overrides: std::collections::HashMap<usize, [f32; 16]> = runtime_state
            .map(|rs| {
                let prefix = format!("{}:", model_name.to_ascii_lowercase());
                rs.bone_transform_overrides.iter()
                    .filter_map(|(k, v)| {
                        k.strip_prefix(&prefix)
                            .and_then(|i| i.parse::<usize>().ok())
                            .map(|i| (i, *v))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Skip skinning if motion has too few tracks for the skeleton — unless
        // manual bone overrides are driving the pose.
        let min_tracks = (skeleton.bones.len() / 2).max(2);
        if bone_overrides.is_empty()
            && motion.map(|m| m.tracks.len() < min_tracks).unwrap_or(true)
        {
            return false;
        }
        let time = bp.map(|b| b.animation_time).unwrap_or(self.animation_time);
        let duration = motion.map(|m| m.duration()).unwrap_or(0.0);
        let end_time = bp.map(|b| b.animation_end_time)
            .or_else(|| runtime_state.map(|rs| rs.animation_end_time)).unwrap_or(-1.0);
        let start_time = bp.map(|b| b.animation_start_time)
            .or_else(|| runtime_state.map(|rs| rs.animation_start_time)).unwrap_or(0.0);
        let eff_end = if end_time >= 0.0 { (end_time).min(duration) } else { duration };
        let eff_start = start_time.min(eff_end);
        let range = eff_end - eff_start;
        let t = if range > 0.0 {
            if is_loop {
                eff_start + ((time - eff_start) % range + range) % range
            } else {
                time.clamp(eff_start, eff_end)
            }
        } else { 0.0 };
        let world_matrices = crate::director::chunks::w3d::skeleton::build_bone_matrices_ex(
            skeleton, motion, t, root_lock,
            if bone_overrides.is_empty() { None } else { Some(&bone_overrides) },
        );

        // [root-relativize] Director keeps a 3ds-Max biped's ROOT at identity IN THE SKIN
        // (the root COM drives the model node, not the deformation). dirplayer's posed
        // skeleton is instead pre-rotated by the root COM — verified against Director:
        // dirplayer's bone[i] world == Rz(-122°) × Director's, the SAME factor for every
        // bone (the root's COM). Strip it by relativizing each posed bone to the posed
        // ROOT: skin[b] = inverse(root) × world[b] × inv_bind[b]. Algebra cancels to
        // (b-relative-to-root) × inv_bind[b], so the inv_bind (mesh-consistent dir/quat
        // T-pose) is untouched — no distortion (changing the bind DOES distort). This is
        // the bot "aims right, faces ~NW" fix; rigid bodies/non-skinned models are
        // unaffected (only skinned models reach here).
        let affine_inv = |m: &[f32; 16]| -> [f32; 16] {
            let (r00, r01, r02) = (m[0], m[4], m[8]);
            let (r10, r11, r12) = (m[1], m[5], m[9]);
            let (r20, r21, r22) = (m[2], m[6], m[10]);
            let (tx, ty, tz) = (m[12], m[13], m[14]);
            let itx = -(r00 * tx + r10 * ty + r20 * tz);
            let ity = -(r01 * tx + r11 * ty + r21 * tz);
            let itz = -(r02 * tx + r12 * ty + r22 * tz);
            [r00, r01, r02, 0.0, r10, r11, r12, 0.0, r20, r21, r22, 0.0, itx, ity, itz, 1.0]
        };
        // Relativize by a FIXED idle-pose root, NOT the per-frame posed root. The idle
        // root strips the biped COM convention while KEEPING each frame's run deviation
        // (the per-frame posed root removed the run's small turn too → bots looked
        // "slightly off while moving"). The bot mesh is authored at "Idle_Rest", so use
        // that motion's frame-0 root as the fixed reference; models with no idle motion
        // (dino/frog) fall back to the per-frame posed root (idle-dominant → no residual).
        let idle_root_mats = scene.motions.iter()
            .find(|m| m.name.to_ascii_lowercase().contains("idle_rest"))
            .or_else(|| scene.motions.iter().find(|m| m.name.to_ascii_lowercase().contains("idle")))
            .map(|im| crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, Some(im), 0.0));
        // Only models with an idle-rest motion (the biped actors/bots) are relativized;
        // everything else (dino, frog01, ClubMarian, …) keeps the original skin — no
        // relativization — so this can't regress them.
        let root_relinv = match &idle_root_mats {
            Some(m) if !m.is_empty() => affine_inv(&m[0]),
            _ => IDENTITY_4X4,
        };

        // Check for motion blending (crossfade) — per-model blend state.
        let blend_weight = bp.map(|b| b.blend_weight).unwrap_or(self.blend_weight);
        let prev_motion_name = bp.and_then(|b| b.previous_motion.as_deref())
            .or_else(|| runtime_state.and_then(|rs| rs.previous_motion.as_deref()));
        let blending = blend_weight < 1.0 && prev_motion_name.is_some();

        let bone_count = skeleton.bones.len().min(48);
        // Initialize ALL 48 uniform slots to identity — bone indices can reference
        // any slot 0-47, even beyond the skeleton's actual bone count.
        let uniform_slots = 48;
        let mut skinning_matrices = vec![0.0f32; uniform_slots * 16];
        for i in 0..uniform_slots {
            skinning_matrices[i * 16]      = 1.0; // m[0][0]
            skinning_matrices[i * 16 + 5]  = 1.0; // m[1][1]
            skinning_matrices[i * 16 + 10] = 1.0; // m[2][2]
            skinning_matrices[i * 16 + 15] = 1.0; // m[3][3]
        }

        if blending {
            let prev_motion = prev_motion_name.and_then(|n| scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(n)));
            let prev_matrices = crate::director::chunks::w3d::skeleton::build_bone_matrices_ex(
                skeleton, prev_motion, t, root_lock,
                if bone_overrides.is_empty() { None } else { Some(&bone_overrides) },
            );
            for i in 0..bone_count {
                let cur_rel = mat4_multiply_col_major(&root_relinv, &world_matrices[i]);
                let prev_rel = mat4_multiply_col_major(&root_relinv, &prev_matrices[i]);
                let cur = mat4_multiply_col_major(&cur_rel, &inv_bind[i]);
                let prev = mat4_multiply_col_major(&prev_rel, &inv_bind[i]);
                for j in 0..16 {
                    skinning_matrices[i * 16 + j] = prev[j] + (cur[j] - prev[j]) * blend_weight;
                }
            }
        } else {
            for i in 0..bone_count {
                let rel = mat4_multiply_col_major(&root_relinv, &world_matrices[i]);
                let final_mat = mat4_multiply_col_major(&rel, &inv_bind[i]);
                skinning_matrices[i * 16..i * 16 + 16].copy_from_slice(&final_mat);
            }
        }

        // Debug: log root bone matrices once
        {
            static BONE_LOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !BONE_LOG.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let w = &world_matrices[0];
                let ib = &inv_bind[0];
                log(&format!(
                    "[3D-BONE0] rootLock={} world_pos=({:.1},{:.1},{:.1}) inv_bind_pos=({:.1},{:.1},{:.1}) skin_pos=({:.2},{:.2},{:.2})",
                    root_lock, w[12], w[13], w[14], ib[12], ib[13], ib[14],
                    skinning_matrices[12], skinning_matrices[13], skinning_matrices[14]
                ));
            }
        }

        gl.uniform_matrix4fv_with_f32_array(
            shader.u_bone_matrices.as_ref(),
            false,
            &skinning_matrices,
        );
        true
    }

    fn bind_default_material(&self, gl: &WebGl2RenderingContext, shader: &Shader3d, scene: &W3dScene) {
        if let Some(mat) = scene.materials.first() {
            self.set_material_uniforms(gl, shader, mat);
        } else {
            gl.uniform4f(shader.u_diffuse_color.as_ref(), 0.5, 0.5, 0.5, 1.0);
            gl.uniform4f(shader.u_ambient_color.as_ref(), 0.125, 0.125, 0.125, 1.0);
            gl.uniform4f(shader.u_specular_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
            gl.uniform4f(shader.u_emissive_color.as_ref(), 0.0, 0.0, 0.0, 1.0);
            gl.uniform1f(shader.u_shininess.as_ref(), 0.0);
            gl.uniform1f(shader.u_opacity.as_ref(), 1.0);
        }
        gl.uniform1i(shader.u_has_texture.as_ref(), 0);
    }

    /// Check if a node is a child (direct or indirect) of a given root node
    fn is_child_of(&self, scene: &W3dScene, node_name: &str, root_name: &str) -> bool {
        if node_name == root_name { return true; }
        let mut current = node_name;
        for _ in 0..20 { // max depth to prevent infinite loops
            if let Some(node) = scene.nodes.iter().find(|n| n.name == current) {
                if node.parent_name == root_name { return true; }
                if node.parent_name.is_empty() { return false; }
                current = &node.parent_name;
            } else {
                return false;
            }
        }
        false
    }

    /// Check if any ancestor in the parent chain is in the detached set
    fn has_detached_ancestor(&self, scene: &W3dScene, parent_name: &str, detached: &std::collections::HashSet<&str>) -> bool {
        if parent_name.is_empty() || parent_name == "World" { return false; }
        if detached.contains(parent_name) { return true; }
        // Walk up parent chain
        for _ in 0..10 {
            if let Some(node) = scene.nodes.iter().find(|n| n.name == parent_name) {
                if node.parent_name.is_empty() || node.parent_name == "World" { return false; }
                if detached.contains(node.parent_name.as_str()) { return true; }
                return self.has_detached_ancestor(scene, &node.parent_name, detached);
            }
            return false;
        }
        false
    }

    /// Build view matrix from scene's ViewNode (or default camera)
    fn build_view_matrix(
        &self,
        scene: &W3dScene,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> ([f32; 16], [f32; 3]) {
        // 1. Determine which camera to use
        let default_cam = "DefaultView".to_string();
        let cam_name = self.active_camera.as_ref().unwrap_or(&default_cam);

        // 2. Find the camera node (case-insensitive), fall back to first view node
        let view_node = scene.nodes.iter()
            .find(|n| n.node_type == W3dNodeType::View && n.name.eq_ignore_ascii_case(cam_name))
            .or_else(|| scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View));

        if let Some(node) = view_node {
            let world_t = self.accumulate_transform_with_state(scene, node, runtime_state);
            let cam_pos = [world_t[12], world_t[13], world_t[14]];
            return (invert_transform(&world_t), cam_pos);
        }
        let cam_name = view_node.map(|n| n.name.as_str()).unwrap_or("DefaultView");

        // 3. Check runtime transform for this camera (case-insensitive)
        if let Some(rs) = runtime_state {
            if let Some(cam_t) = get_runtime_transform(rs, cam_name) {
                let cam_pos = [cam_t[12], cam_t[13], cam_t[14]];
                return (invert_transform(&cam_t), cam_pos);
            }
        }

        // Use world transform (accumulated through parent chain)
        if let Some(node) = view_node {
            let world_t = self.accumulate_transform_with_state(scene, node, runtime_state);
            let has_position = world_t[12].abs() > 0.01 || world_t[13].abs() > 0.01 || world_t[14].abs() > 0.01;
            if has_position {
                let cam_pos = [world_t[12], world_t[13], world_t[14]];
                let view = invert_transform(&world_t);
                return (view, cam_pos);
            }
        }

        // Default camera: looking at origin from a reasonable distance
        let cam_pos = [0.0, 0.0, 100.0];
        let view = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, -100.0, 1.0,
        ];
        (view, cam_pos)
    }

    /// Build perspective projection matrix from ViewNode
    fn build_projection_matrix(&self, scene: &W3dScene, _fbo_aspect: f32,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> [f32; 16] {
        // Guard against a degenerate render-target aspect (0 / NaN / inf). The
        // projection is driven by the real sprite/FBO aspect (_fbo_aspect = w/h); a
        // 0-sized rect (e.g. briefly, before the score sizes a W3D sprite) would give
        // 0/inf/NaN and collapse the matrix, blanking the scene. Fall back to 4:3.
        let fbo_aspect = if _fbo_aspect.is_finite() && _fbo_aspect > 0.0 { _fbo_aspect } else { 4.0 / 3.0 };
        let default_cam = "DefaultView".to_string();
        let cam_name = self.active_camera.as_ref().unwrap_or(&default_cam);
        // Find camera node (case-insensitive), fall back to first view node
        let view_node = scene.nodes.iter()
            .find(|n| n.node_type == W3dNodeType::View && n.name.eq_ignore_ascii_case(cam_name))
            .or_else(|| scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View));

        let (fov, near, far, aspect) = if let Some(node) = view_node {
            let mut f = node.far_plane;
            // Director's default camera `yon` is effectively unbounded, but dirplayer
            // stores a small default (10000) on empty 3D members and previously clamped
            // any far > 100000 back down to 10000. Large-coordinate scenes (e.g. the
            // unicraft galaxy — camera at ~180000 units, geometry at the origin) were
            // then entirely far-clipped and rendered black (picking still worked because
            // raycasts ignore the far plane). When the stored far is that default /
            // invalid / over-large sentinel, fit it to the furthest geometry from the
            // camera instead. Explicitly-set, in-range far values (e.g. gameplay `yon`)
            // are left alone — no per-frame scan and no override.
            let needs_fit = f <= 0.0 || f > 100000.0 || (f - 10000.0).abs() < 0.5;
            if needs_fit {
                let cam_world = self.accumulate_transform_with_state(scene, node, runtime_state);
                let cp = [cam_world[12], cam_world[13], cam_world[14]];
                let mut scene_far = 0.0f32;
                for m in scene.nodes.iter().filter(|nn| nn.node_type == W3dNodeType::Model) {
                    let wt = self.accumulate_transform_with_state(scene, m, runtime_state);
                    let d = ((wt[12]-cp[0]).powi(2) + (wt[13]-cp[1]).powi(2) + (wt[14]-cp[2]).powi(2)).sqrt();
                    if d > scene_far { scene_far = d; }
                }
                // Margin for object radius / geometry beyond node origin, floored so tiny
                // scenes keep a sane far, capped so depth precision stays usable.
                f = (scene_far * 1.5).clamp(10000.0, 4_000_000.0);
            }
            let mut n = node.near_plane;
            if n <= 0.0 { n = 1.0; }
            // Director scales the projection plane to fit the SPRITE rect, so the
            // aspect must come from the sprite/FBO — NOT the camera's stored screen
            // size. W3dNode.screen_width/height are an unparsed 640x480 default that
            // never updates, which forced every movie to 4:3 (fine for 4:3 sprites,
            // but horizontally stretched for square sprites like the estate explore).
            let cam_aspect = fbo_aspect;
            // Each camera uses its own FOV/near/far settings
            let (fov, n, f) = (node.fov, n, f);
            (fov.to_radians(), n, f, cam_aspect)
        } else {
            (34.516f32.to_radians(), 1.0, 10000.0, fbo_aspect)
        };

        // Check for orthographic projection mode
        let is_ortho = runtime_state
            .and_then(|rs| rs.camera_projection_mode.get(&cam_name.to_ascii_lowercase()))
            .map(|&m| m == 1)
            .unwrap_or(false);

        let mut proj = if is_ortho {
            let ortho_h = runtime_state
                .and_then(|rs| rs.camera_ortho_height.get(&cam_name.to_ascii_lowercase()))
                .copied()
                .unwrap_or(100.0);
            let half_h = ortho_h * 0.5;
            let half_w = half_h * aspect;
            orthographic(-half_w, half_w, -half_h, half_h, near, far)
        } else {
            perspective(fov, aspect, near, far)
        };
        // Flip Y: FBO renders with OpenGL Y-up but composited as 2D sprite with Y-down
        proj[5] = -proj[5];
        proj
    }

    /// Set up lighting uniforms from scene lights
    fn setup_lights(&self, gl: &WebGl2RenderingContext, shader: &Shader3d, scene: &W3dScene, camera_pos: &[f32; 3],
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) {
        let mut positions = [0.0f32; 24]; // 8 * 3
        let mut colors = [0.0f32; 24];
        let mut types = [0i32; 8];
        let mut attenuations = [0.0f32; 24]; // 8 * 3 (constant, linear, quadratic)
        let mut directions = [0.0f32; 24];   // 8 * 3 (direction for spot/directional)
        let mut spot_angles = [0.0f32; 8];   // cone angle in radians
        let mut global_ambient = [0.0f32, 0.0, 0.0];
        let mut num_lights = 0i32;

        // dirplayer injects fallback default lights (Default*/UI*) so an unlit scene
        // is still visible. When the movie supplies its OWN directional/spot lighting
        // (e.g. frog01's spot/spot2), the synthetic *directional* fallback floods the
        // scene and washes out the intended mood — so suppress it in that case. Baked
        // content lights (e.g. AmbientLightResource from the .w3d) are kept.
        //
        // The AMBIENT fallback (DefaultAmbient) is NOT suppressed: it stands in for
        // Director's always-present default ambient in an empty 3D member, and movies
        // configure it directly (e.g. `w.light(1).color = rgb(255,255,255)`) as the
        // scene's base fill. Dropping it left ambient-lit scenes (unicraft's galaxy)
        // fully black.
        let is_fallback_light = |name: &str| matches!(name,
            "DefaultAmbient" | "DefaultDirectional" | "UIAmbient" | "UIDirectional");
        let is_fallback_directional = |name: &str| matches!(name,
            "DefaultDirectional" | "UIDirectional");
        let has_movie_light = scene.lights.iter().any(|l|
            l.enabled && !is_fallback_light(&l.name)
            && matches!(l.light_type, W3dLightType::Directional | W3dLightType::Spot));

        if scene.lights.is_empty() {
            // Default: one directional light from above-right
            positions[0] = 0.5;
            positions[1] = 1.0;
            positions[2] = 0.7;
            colors[0] = 1.0;
            colors[1] = 1.0;
            colors[2] = 1.0;
            types[0] = 1; // directional
            attenuations[0] = 1.0; // constant = 1 (no falloff for directional)
            directions[0] = -0.5; directions[1] = -1.0; directions[2] = -0.7; // matches position
            num_lights = 1;
        } else {
            // Collect detached nodes to skip lights that are removeFromWorld()
            let detached = runtime_state.map(|rs| &rs.detached_nodes);

            // Sort lights: ambient first (handled separately), then directional, then point/spot.
            // This ensures important scene lights (SunLight, KeyLight) aren't pushed out by
            // weapon/effect point lights that may not even be in the world.
            let mut sorted_lights: Vec<&W3dLight> = scene.lights.iter().collect();
            sorted_lights.sort_by_key(|l| match l.light_type {
                W3dLightType::Ambient => 0,
                W3dLightType::Directional => 1,
                W3dLightType::Spot => 2,
                W3dLightType::Point => 3,
            });

            for light in &sorted_lights {
                if !light.enabled {
                    continue;
                }
                // Suppress only the synthetic fallback *directional* key when the movie
                // lights itself; keep the ambient fallback as Director's base fill.
                if has_movie_light && is_fallback_directional(&light.name) {
                    continue;
                }
                // Skip lights that have been removed from world
                if let Some(detached_set) = detached {
                    if detached_set.contains(&light.name) {
                        continue;
                    }
                }
                // Also skip lights whose node has empty parent (detached)
                let light_node = scene.nodes.iter().find(|n| n.name == light.name);
                if let Some(node) = light_node {
                    if node.parent_name.is_empty() {
                        continue;
                    }
                }
                // When camera has rootNode, only use lights in that subtree.
                // E.g., arrowcam with rootNode=pointarrow has no child lights,
                // so its pass uses only emissive (no scene lighting wash-out).
                if let Some(ref cam) = self.active_camera {
                    if let Some(rs) = runtime_state {
                        if let Some(root) = rs.camera_root_nodes.get(&cam.to_ascii_lowercase()) {
                            if !self.is_child_of(scene, &light.name, root) {
                                continue;
                            }
                        }
                    }
                }

                let li = num_lights as usize;
                let lt = match light.light_type {
                    W3dLightType::Ambient => {
                        global_ambient[0] += light.color[0];
                        global_ambient[1] += light.color[1];
                        global_ambient[2] += light.color[2];
                        continue;
                    }
                    W3dLightType::Directional => 1,
                    W3dLightType::Point => 2,
                    W3dLightType::Spot => 3,
                };
                if li >= 8 { continue; } // Max 8 non-ambient lights

                // Per-light attenuation from W3dLight (constant, linear, quadratic)
                attenuations[li * 3]     = light.attenuation[0]; // constant
                attenuations[li * 3 + 1] = light.attenuation[1]; // linear
                attenuations[li * 3 + 2] = light.attenuation[2]; // quadratic
                // Ensure attenuation sum > 0 (prevent division by zero)
                if attenuations[li * 3] + attenuations[li * 3 + 1] + attenuations[li * 3 + 2] < 0.001 {
                    attenuations[li * 3] = 1.0; // default constant = 1
                }

                // Spot angle (degrees → radians)
                spot_angles[li] = if lt == 3 { light.spot_angle.to_radians() } else { 0.0 };

                if let Some(light_node) = scene.nodes.iter().find(|n| {
                    n.node_type == W3dNodeType::Light && (n.resource_name == light.name || n.name == light.name)
                }) {
                    let world_t = self.accumulate_transform_with_state(scene, light_node, runtime_state);
                    if lt == 1 {
                        // Directional: direction = -Z axis of world transform
                        positions[li * 3]     = -world_t[8];
                        positions[li * 3 + 1] = -world_t[9];
                        positions[li * 3 + 2] = -world_t[10];
                        directions[li * 3]     = -world_t[8];
                        directions[li * 3 + 1] = -world_t[9];
                        directions[li * 3 + 2] = -world_t[10];
                    } else {
                        // Point/Spot: world position from translation
                        positions[li * 3]     = world_t[12];
                        positions[li * 3 + 1] = world_t[13];
                        positions[li * 3 + 2] = world_t[14];
                        // Spot direction = -Z axis of light transform
                        directions[li * 3]     = -world_t[8];
                        directions[li * 3 + 1] = -world_t[9];
                        directions[li * 3 + 2] = -world_t[10];
                    }
                } else {
                    positions[li * 3] = 0.5;
                    positions[li * 3 + 1] = 1.0;
                    positions[li * 3 + 2] = 0.7;
                    directions[li * 3] = -0.5;
                    directions[li * 3 + 1] = -1.0;
                    directions[li * 3 + 2] = -0.7;
                }
                colors[li * 3] = light.color[0];
                colors[li * 3 + 1] = light.color[1];
                colors[li * 3 + 2] = light.color[2];
                types[li] = lt;
                num_lights += 1;
            }
        }

        let n = num_lights.max(1) as usize;
        gl.uniform1i(shader.u_num_lights.as_ref(), num_lights);
        gl.uniform3fv_with_f32_array(shader.u_light_pos.as_ref(), &positions[..n * 3]);
        gl.uniform3fv_with_f32_array(shader.u_light_color.as_ref(), &colors[..n * 3]);
        gl.uniform1iv_with_i32_array(shader.u_light_type.as_ref(), &types[..n]);
        gl.uniform3fv_with_f32_array(shader.u_light_atten.as_ref(), &attenuations[..n * 3]);
        gl.uniform3fv_with_f32_array(shader.u_light_dir.as_ref(), &directions[..n * 3]);
        gl.uniform1fv_with_f32_array(shader.u_light_spot_angle.as_ref(), &spot_angles[..n]);
        gl.uniform3f(shader.u_global_ambient.as_ref(), global_ambient[0], global_ambient[1], global_ambient[2]);
    }

    /// Get the FBO texture (for use as sprite texture in 2D pipeline)
    pub fn get_fbo_texture(&self) -> Option<&WebGlTexture> {
        self.fbo_texture.as_ref()
    }

    pub fn fbo_size(&self) -> (u32, u32) {
        (self.fbo_width, self.fbo_height)
    }
}

// ─── Texture decode + upload (free function) ───

/// Decode image data (raw RGBA, DXT, JPEG/PNG) and upload as a WebGL2 texture.
/// Free function to avoid borrow conflicts when called during incremental updates.
fn decode_and_upload_texture_impl(context: &WebGL2Context, data: &[u8], flip_v: bool) -> Option<(WebGlTexture, u32, u32, bool)> {
    if data.len() < 4 { return None; }

    // Detection priority: JPEG/PNG magic → DXT header → raw RGBA (our own format)
    // Raw RGBA must be checked LAST because its 8-byte header (u32 w, u32 h) can
    // accidentally match the first bytes of DXT/JPEG/PNG data, causing misidentification
    // (e.g., a DXT texture whose first 8 bytes happen to decode as valid small dimensions).
    let (width, height, rgba_data) = if data.len() >= 2
        && (data[0] == 0xFF && data[1] == 0xD8       // JPEG magic
            || data[0] == 0x89 && data[1] == 0x50)    // PNG magic
    {
        let img = match image::load_from_memory(data) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                let header: Vec<String> = data.iter().take(8).map(|b| format!("{:02X}", b)).collect();
                console_warn!(
                    "[3D-TEX-DECODE] Failed to decode {} bytes, header=[{}]: {}",
                    data.len(), header.join(" "), e
                );
                return None;
            }
        };
        let w = img.width();
        let h = img.height();
        let mut rgba = img.into_raw();
        // Director W3D stores rgba4444 / 4444 textures as an RGB JPEG followed by a
        // separate alpha continuation block: [width u32][height u32][zlibLen u32]
        // [zlib-compressed 8-bit grayscale alpha]. image::load_from_memory only
        // decodes the leading JPEG (alpha defaults to 255), so the icon's
        // transparent background renders black. Recover the alpha here: locate the
        // JPEG's EOI, parse the trailing block, inflate it, and write it into the
        // alpha channel (0 = transparent, 255 = opaque).
        if data[0] == 0xFF && data[1] == 0xD8 {
            if let Some(eoi) = data.windows(2).position(|b| b[0] == 0xFF && b[1] == 0xD9) {
                let tail = &data[eoi + 2..];
                if tail.len() >= 14 {
                    let aw = u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]);
                    let ah = u32::from_le_bytes([tail[4], tail[5], tail[6], tail[7]]);
                    let alen = u32::from_le_bytes([tail[8], tail[9], tail[10], tail[11]]) as usize;
                    if aw == w && ah == h && tail[12] == 0x78 && tail.len() >= 12 + alen {
                        use std::io::Read;
                        let mut alpha = Vec::new();
                        let mut dec = flate2::read::ZlibDecoder::new(&tail[12..12 + alen]);
                        if dec.read_to_end(&mut alpha).is_ok() && alpha.len() >= (w * h) as usize {
                            for i in 0..(w * h) as usize {
                                rgba[i * 4 + 3] = alpha[i];
                            }
                        }
                    }
                }
            }
        }
        (w, h, rgba)
    } else if is_dxt_texture(data) {
        // DXT compressed texture — decode to RGBA
        match decode_dxt_to_rgba(data) {
            Some((w, h, rgba)) => (w, h, rgba),
            None => return None,
        }
    } else if data.len() >= 8 {
        // Raw RGBA format (from newTexture #fromImageObject):
        // first 4 bytes = width LE, next 4 bytes = height LE, rest = RGBA
        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let expected = 8 + (w as usize) * (h as usize) * 4;
        if w > 0 && w <= 4096 && h > 0 && h <= 4096 && data.len() == expected {
            (w, h, data[8..].to_vec())
        } else {
            // Last resort: try image library decode for other formats
            let img = match image::load_from_memory(data) {
                Ok(img) => img.to_rgba8(),
                Err(e) => {
                    let header: Vec<String> = data.iter().take(8).map(|b| format!("{:02X}", b)).collect();
                    console_warn!(
                        "[3D-TEX-DECODE] Failed to decode {} bytes, header=[{}]: {}",
                        data.len(), header.join(" "), e
                    );
                    return None;
                }
            };
            let w = img.width();
            let h = img.height();
            (w, h, img.into_raw())
        }
    } else {
        return None;
    };

    // Vertically flip the decoded image when requested (the caller sets this for
    // the SkyLine* textures, which are authored upside-down in the W3D asset).
    let rgba_data = if flip_v && height > 0 {
        let row = (width as usize) * 4;
        let mut out = vec![0u8; rgba_data.len()];
        for y in 0..(height as usize) {
            let src = y * row;
            let dst = (height as usize - 1 - y) * row;
            if src + row <= rgba_data.len() && dst + row <= out.len() {
                out[dst..dst + row].copy_from_slice(&rgba_data[src..src + row]);
            }
        }
        out
    } else {
        rgba_data
    };

    let gl = context.gl();
    let texture = gl.create_texture()?;
    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&texture));

    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, WebGl2RenderingContext::LINEAR_MIPMAP_LINEAR as i32);
    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, WebGl2RenderingContext::LINEAR as i32);
    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::REPEAT as i32);
    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::REPEAT as i32);

    gl.pixel_storei(WebGl2RenderingContext::UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0);

    // Verify data size matches expected
    let expected_size = (width as usize) * (height as usize) * 4;
    if rgba_data.len() != expected_size {
        console_warn!(
            "[3D-TEX] Size mismatch! {}x{} expects {} bytes but got {}",
            width, height, expected_size, rgba_data.len()
        );
        return None;
    }

    let upload_result = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
        WebGl2RenderingContext::TEXTURE_2D,
        0,
        WebGl2RenderingContext::RGBA as i32,
        width as i32,
        height as i32,
        0,
        WebGl2RenderingContext::RGBA,
        WebGl2RenderingContext::UNSIGNED_BYTE,
        Some(&rgba_data),
    );
    if let Err(ref e) = upload_result {
        console_warn!("[3D-TEX] tex_image_2d failed: {:?}", e);
    }
    gl.generate_mipmap(WebGl2RenderingContext::TEXTURE_2D);
    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
    // Detect if texture has meaningful alpha (any pixel alpha < 250)
    let has_alpha = rgba_data.chunks(4).any(|p| p[3] < 250);
    Some((texture, width, height, has_alpha))
}

// ─── DXT texture decompression ───

/// Check if data looks like a DXT compressed texture.
/// IFX stores DXT textures with a small header: width(u16), height(u16), format(u8), then blocks.
fn is_dxt_texture(data: &[u8]) -> bool {
    if data.len() < 5 { return false; }
    let w = u16::from_le_bytes([data[0], data[1]]) as u32;
    let h = u16::from_le_bytes([data[2], data[3]]) as u32;
    if w == 0 || h == 0 || w > 4096 || h > 4096 { return false; }
    // DXT1: 8 bytes per 4x4 block = 0.5 bytes per pixel
    let blocks_w = (w + 3) / 4;
    let blocks_h = (h + 3) / 4;
    let dxt1_size = (blocks_w * blocks_h * 8) as usize;
    let dxt3_5_size = (blocks_w * blocks_h * 16) as usize;
    // Check if data matches DXT1 or DXT3/5 size (with 5-byte header)
    data.len() == 5 + dxt1_size || data.len() == 5 + dxt3_5_size
}

/// Decode DXT compressed texture to RGBA. Returns (width, height, rgba_pixels).
fn decode_dxt_to_rgba(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if data.len() < 5 { return None; }
    let w = u16::from_le_bytes([data[0], data[1]]) as u32;
    let h = u16::from_le_bytes([data[2], data[3]]) as u32;
    let _format = data[4];
    let block_data = &data[5..];

    let blocks_w = (w + 3) / 4;
    let blocks_h = (h + 3) / 4;
    let dxt1_expected = (blocks_w * blocks_h * 8) as usize;
    let is_dxt1 = block_data.len() == dxt1_expected;

    let mut rgba = vec![0u8; (w * h * 4) as usize];

    for by in 0..blocks_h {
        for bx in 0..blocks_w {
            let block_idx = (by * blocks_w + bx) as usize;
            if is_dxt1 {
                let offset = block_idx * 8;
                if offset + 8 > block_data.len() { break; }
                decode_dxt1_block(&block_data[offset..offset+8], &mut rgba, bx * 4, by * 4, w, h);
            } else {
                // DXT3/DXT5: skip 8-byte alpha block, decode 8-byte color block
                let offset = block_idx * 16;
                if offset + 16 > block_data.len() { break; }
                decode_dxt1_block(&block_data[offset+8..offset+16], &mut rgba, bx * 4, by * 4, w, h);
            }
        }
    }

    Some((w, h, rgba))
}

/// Decode a single DXT1 4x4 color block into RGBA pixels.
fn decode_dxt1_block(block: &[u8], rgba: &mut [u8], start_x: u32, start_y: u32, img_w: u32, img_h: u32) {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);

    let r0 = ((c0 >> 11) & 0x1F) as u8;
    let g0 = ((c0 >> 5) & 0x3F) as u8;
    let b0 = (c0 & 0x1F) as u8;
    let r1 = ((c1 >> 11) & 0x1F) as u8;
    let g1 = ((c1 >> 5) & 0x3F) as u8;
    let b1 = (c1 & 0x1F) as u8;

    // Expand to 8-bit
    let colors: [[u8; 4]; 4] = if c0 > c1 {
        [
            [(r0 << 3) | (r0 >> 2), (g0 << 2) | (g0 >> 4), (b0 << 3) | (b0 >> 2), 255],
            [(r1 << 3) | (r1 >> 2), (g1 << 2) | (g1 >> 4), (b1 << 3) | (b1 >> 2), 255],
            [((2*r0 as u16 + r1 as u16)/3) as u8 * 8 / 8, ((2*g0 as u16 + g1 as u16)/3) as u8 * 4 / 4, ((2*b0 as u16 + b1 as u16)/3) as u8 * 8 / 8, 255],
            [((r0 as u16 + 2*r1 as u16)/3) as u8 * 8 / 8, ((g0 as u16 + 2*g1 as u16)/3) as u8 * 4 / 4, ((b0 as u16 + 2*b1 as u16)/3) as u8 * 8 / 8, 255],
        ]
    } else {
        [
            [(r0 << 3) | (r0 >> 2), (g0 << 2) | (g0 >> 4), (b0 << 3) | (b0 >> 2), 255],
            [(r1 << 3) | (r1 >> 2), (g1 << 2) | (g1 >> 4), (b1 << 3) | (b1 >> 2), 255],
            [((r0 as u16 + r1 as u16)/2) as u8 * 8 / 8, ((g0 as u16 + g1 as u16)/2) as u8 * 4 / 4, ((b0 as u16 + b1 as u16)/2) as u8 * 8 / 8, 255],
            [0, 0, 0, 0], // Transparent black for DXT1 with alpha
        ]
    };

    for py in 0..4u32 {
        for px in 0..4u32 {
            let x = start_x + px;
            let y = start_y + py;
            if x >= img_w || y >= img_h { continue; }
            let bit_idx = (py * 4 + px) * 2;
            let byte_idx = 4 + (bit_idx / 8) as usize;
            let bit_offset = bit_idx % 8;
            let color_idx = ((block[byte_idx] >> bit_offset) & 3) as usize;
            let pixel_offset = ((y * img_w + x) * 4) as usize;
            rgba[pixel_offset..pixel_offset+4].copy_from_slice(&colors[color_idx]);
        }
    }
}

// ─── Bone data helpers ───

/// Pack variable-length bone indices into fixed vec4 (as f32 for vertex attribute).
fn pack_bone_vec4_f32(indices: &[Vec<u32>]) -> Vec<[f32; 4]> {
    indices.iter().map(|v| {
        let mut out = [0.0f32; 4];
        for (i, &idx) in v.iter().take(4).enumerate() {
            // Clamp to max bone uniform array size to prevent out-of-bounds GPU access
            out[i] = (idx as f32).min(47.0);
        }
        out
    }).collect()
}

/// Pack variable-length bone weights into fixed vec4, normalized to sum to 1.
fn pack_bone_weights_vec4(weights: &[Vec<f32>]) -> Vec<[f32; 4]> {
    weights.iter().map(|v| {
        let mut out = [0.0f32; 4];
        for (i, &w) in v.iter().take(4).enumerate() {
            out[i] = w.max(0.0); // clamp negatives to 0 (bad IQ can produce negative weights)
        }
        // Normalize so weights sum to 1.0
        let sum: f32 = out.iter().sum();
        if sum > 0.001 {
            for w in out.iter_mut() {
                *w /= sum;
            }
        } else {
            // No weights — assign full weight to bone 0
            out[0] = 1.0;
        }
        out
    }).collect()
}

// ─── Matrix math helpers ───

const IDENTITY_4X4: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0,
    0.0, 0.0, 0.0, 1.0,
];

/// Convert a W3dKeyframe (quaternion + position + scale) to a column-major 4x4 matrix
fn keyframe_to_column_major_matrix(kf: &crate::director::chunks::w3d::types::W3dKeyframe) -> [f32; 16] {
    let (qx, qy, qz, qw) = (kf.rot_x, kf.rot_y, kf.rot_z, kf.rot_w);
    let (sx, sy, sz) = (kf.scale_x, kf.scale_y, kf.scale_z);

    // Quaternion to rotation matrix (column-major)
    let x2 = qx + qx; let y2 = qy + qy; let z2 = qz + qz;
    let xx = qx * x2; let xy = qx * y2; let xz = qx * z2;
    let yy = qy * y2; let yz = qy * z2; let zz = qz * z2;
    let wx = qw * x2; let wy = qw * y2; let wz = qw * z2;

    [
        (1.0 - (yy + zz)) * sx,  (xy + wz) * sx,           (xz - wy) * sx,           0.0,  // col 0
        (xy - wz) * sy,           (1.0 - (xx + zz)) * sy,  (yz + wx) * sy,           0.0,  // col 1
        (xz + wy) * sz,           (yz - wx) * sz,           (1.0 - (xx + yy)) * sz,  0.0,  // col 2
        kf.pos_x,                 kf.pos_y,                 kf.pos_z,                 1.0,  // col 3
    ]
}

/// Case-insensitive lookup in node_transforms (Director is case-insensitive for node names).
fn get_runtime_transform(rs: &crate::player::cast_member::Shockwave3dRuntimeState, name: &str) -> Option<[f32; 16]> {
    if let Some(m) = rs.node_transforms.get(name) {
        return Some(*m);
    }
    for (key, val) in &rs.node_transforms {
        if key.eq_ignore_ascii_case(name) {
            return Some(*val);
        }
    }
    None
}

fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1.0 / (fov_y / 2.0).tan();
    let nf = 1.0 / (near - far);
    [
        f / aspect, 0.0, 0.0, 0.0,
        0.0, f, 0.0, 0.0,
        0.0, 0.0, (far + near) * nf, -1.0,
        0.0, 0.0, 2.0 * far * near * nf, 0.0,
    ]
}

fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> [f32; 16] {
    let rl = right - left;
    let tb = top - bottom;
    let fn_ = far - near;
    [
        2.0 / rl, 0.0, 0.0, 0.0,
        0.0, 2.0 / tb, 0.0, 0.0,
        0.0, 0.0, -2.0 / fn_, 0.0,
        -(right + left) / rl, -(top + bottom) / tb, -(far + near) / fn_, 1.0,
    ]
}

/// Multiply two 4x4 row-major matrices: result = a * b
fn mat4_multiply_row_major(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0f32; 16];
    for row in 0..4 {
        for col in 0..4 {
            r[row * 4 + col] =
                a[row * 4 + 0] * b[0 * 4 + col] +
                a[row * 4 + 1] * b[1 * 4 + col] +
                a[row * 4 + 2] * b[2 * 4 + col] +
                a[row * 4 + 3] * b[3 * 4 + col];
        }
    }
    r
}


/// Convert row-major matrix to column-major for OpenGL uniforms
fn row_major_to_column_major(m: &[f32; 16]) -> [f32; 16] {
    [
        m[0], m[4], m[8],  m[12],
        m[1], m[5], m[9],  m[13],
        m[2], m[6], m[10], m[14],
        m[3], m[7], m[11], m[15],
    ]
}

/// Invert a 4x4 affine transform and output column-major for OpenGL.
/// Director/IFX transforms are stored in COLUMN-MAJOR order:
///   m[0..3] = column 0 (X-axis), m[4..7] = column 1 (Y-axis),
///   m[8..11] = column 2 (Z-axis), m[12..15] = column 3 (translation)
/// Used for view matrix: view = inverse(camera_world_transform).
fn invert_transform(m: &[f32; 16]) -> [f32; 16] {
    // Column-major: R[row][col] = m[col*4 + row]
    // R as a math matrix:
    //   R[0][0]=m[0]  R[0][1]=m[4]  R[0][2]=m[8]
    //   R[1][0]=m[1]  R[1][1]=m[5]  R[1][2]=m[9]
    //   R[2][0]=m[2]  R[2][1]=m[6]  R[2][2]=m[10]
    let tx = m[12]; let ty = m[13]; let tz = m[14];

    // R^T: swap rows and columns
    // R^T[0][0]=m[0]  R^T[0][1]=m[1]  R^T[0][2]=m[2]
    // R^T[1][0]=m[4]  R^T[1][1]=m[5]  R^T[1][2]=m[6]
    // R^T[2][0]=m[8]  R^T[2][1]=m[9]  R^T[2][2]=m[10]

    // -R^T * t (using R^T rows)
    let itx = -(m[0] * tx + m[1] * ty + m[2] * tz);
    let ity = -(m[4] * tx + m[5] * ty + m[6] * tz);
    let itz = -(m[8] * tx + m[9] * ty + m[10] * tz);

    // Output column-major: columns of R^T
    [
        m[0], m[4], m[8],  0.0,  // R^T column 0
        m[1], m[5], m[9],  0.0,  // R^T column 1
        m[2], m[6], m[10], 0.0,  // R^T column 2
        itx,  ity,  itz,   1.0,  // translation
    ]
}

/// Generate spherical UV coordinates from vertex positions.
fn generate_spherical_uvs(positions: &[[f32; 3]]) -> Vec<[f32; 2]> {
    if positions.is_empty() {
        return Vec::new();
    }
    // Compute center
    let n = positions.len() as f32;
    let cx = positions.iter().map(|p| p[0]).sum::<f32>() / n;
    let cy = positions.iter().map(|p| p[1]).sum::<f32>() / n;
    let cz = positions.iter().map(|p| p[2]).sum::<f32>() / n;

    positions.iter().map(|p| {
        let dx = p[0] - cx;
        let dy = p[1] - cy;
        let dz = p[2] - cz;
        let len = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-8);
        let nx = dx / len;
        let ny = dy / len;
        let nz = dz / len;
        let u = 0.5 + nz.atan2(nx) / (2.0 * std::f32::consts::PI);
        let v = 0.5 - ny.asin() / std::f32::consts::PI;
        [u, v]
    }).collect()
}

/// Generate cylindrical UV coordinates from vertex positions.
fn generate_cylindrical_uvs(positions: &[[f32; 3]]) -> Vec<[f32; 2]> {
    if positions.is_empty() {
        return Vec::new();
    }
    let n = positions.len() as f32;
    let cx = positions.iter().map(|p| p[0]).sum::<f32>() / n;
    let cz = positions.iter().map(|p| p[2]).sum::<f32>() / n;
    let min_y = positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
    let max_y = positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
    let height = (max_y - min_y).max(0.001);

    positions.iter().map(|p| {
        let dx = p[0] - cx;
        let dz = p[2] - cz;
        let u = 0.5 + dz.atan2(dx) / (2.0 * std::f32::consts::PI);
        let v = (p[1] - min_y) / height;
        [u, v]
    }).collect()
}

/// Generate UVs using the specified mode: 0=planar, 1=spherical, 2=cylindrical, 3=reflection.
fn generate_uvs_by_mode(positions: &[[f32; 3]], mode: Option<u8>) -> Vec<[f32; 2]> {
    match mode {
        Some(1) => generate_spherical_uvs(positions),
        Some(2) => generate_cylindrical_uvs(positions),
        _ => generate_planar_uvs(positions), // 0=planar or default
    }
}

/// Generate planar UV coordinates from vertex positions (bounding-box normalized).
fn generate_planar_uvs(positions: &[[f32; 3]]) -> Vec<[f32; 2]> {
    if positions.is_empty() {
        return Vec::new();
    }
    // Find bounding box
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for p in positions {
        if p[0] < min_x { min_x = p[0]; }
        if p[0] > max_x { max_x = p[0]; }
        if p[1] < min_y { min_y = p[1]; }
        if p[1] > max_y { max_y = p[1]; }
    }
    let range_x = (max_x - min_x).max(0.001);
    let range_y = (max_y - min_y).max(0.001);

    positions.iter().map(|p| {
        [(p[0] - min_x) / range_x, (p[1] - min_y) / range_y]
    }).collect()
}

/// Multiply two column-major 4x4 matrices: result = A * B
fn mat4_multiply_col_major(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut r = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] =
                a[0 * 4 + row] * b[col * 4 + 0] +
                a[1 * 4 + row] * b[col * 4 + 1] +
                a[2 * 4 + row] * b[col * 4 + 2] +
                a[3 * 4 + row] * b[col * 4 + 3];
        }
    }
    r
}

